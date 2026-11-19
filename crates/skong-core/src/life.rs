//! "Lives": small weighted state machines that keep a long act step alive instead of
//! holding one frame (the seated knight glances around, leans back, checks his map, nods
//! off…). Defined in act files under `"lives"`, run by an act step `{"life": "seated"}`.
//!
//! * A life is a list of states; each state is a short sequence of act-style steps
//!   (`clip`, `hold`, `repeat`, `until`, `interruptible`).
//! * The next state is drawn by weight × mood bias (`tired`, `curious`) × the step's
//!   per-state `bias`; repeating the state just played is damped.
//! * The step's budget (`hold`) is only checked between states, so a started state always
//!   finishes (the map gets closed, a doze ends with waking up).
//! * Signals (e.g. "woken") end a sub-step waiting on them, like act steps.
use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::acts::Step;
use crate::anim::{AnimEvent, Animator, ClipId, ClipLibrary};

#[derive(Debug, Deserialize)]
pub(crate) struct LifeJson {
    /// Clip shown while the life starts (and the step's nominal clip).
    base: String,
    states: Vec<StateJson>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StateJson {
    name: String,
    weight: f32,
    /// Weight multiplier `1 + tired * k` (negative: less likely when tired).
    #[serde(default)]
    tired: f32,
    /// Weight multiplier `1 + curious * k`.
    #[serde(default)]
    curious: f32,
    steps: Vec<crate::acts::StepJson>,
}

/// One state of a life.
#[derive(Clone, Debug)]
pub struct LifeState {
    pub name: String,
    pub weight: f32,
    pub tired: f32,
    pub curious: f32,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug)]
pub struct Life {
    pub name: String,
    pub base: ClipId,
    pub states: Vec<LifeState>,
}

impl Life {
    pub(crate) fn from_json(name: &str, j: LifeJson, clips: &ClipLibrary) -> Result<Life> {
        let base = clips.id(&j.base).with_context(|| format!("life '{name}': unknown base clip '{}'", j.base))?;
        if j.states.is_empty() {
            bail!("life '{name}' has no states");
        }
        let mut states = Vec::new();
        for s in j.states {
            if s.steps.is_empty() {
                bail!("life '{name}': state '{}' has no steps", s.name);
            }
            let ctx = format!("life '{name}' state '{}'", s.name);
            let steps = s.steps.into_iter().map(|st| st.resolve(&ctx, clips, &HashMap::new())).collect::<Result<Vec<_>>>()?;
            if steps.iter().any(|st| st.life.is_some()) {
                bail!("{ctx}: lives cannot nest");
            }
            states.push(LifeState { name: s.name, weight: s.weight.max(0.0), tired: s.tired, curious: s.curious, steps });
        }
        Ok(Life { name: name.to_string(), base, states })
    }

    /// Selection weight of state `i` for `mood`, with the step's per-state `bias`.
    pub fn weight(&self, i: usize, mood: Mood, bias: &HashMap<String, f32>) -> f32 {
        let s = &self.states[i];
        let m = (1.0 + mood.tired * s.tired).max(0.0) * (1.0 + mood.curious * s.curious).max(0.0);
        s.weight * m * bias.get(&s.name).copied().unwrap_or(1.0)
    }
}

/// What biases a life's choices, each 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mood {
    pub tired: f32,
    pub curious: f32,
}

impl Default for Mood {
    fn default() -> Self {
        Self { tired: 0.3, curious: 0.3 }
    }
}

/// The state just played is not picked again right away (unless it is the only one):
/// back-to-back idles would hold one frame too long.
const REPEAT_DAMPING: f32 = 0.0;

/// Runs one life step.
#[derive(Clone, Debug)]
pub struct LifeRun {
    /// Seconds left before the life wraps up (checked between states).
    budget: f32,
    state: usize,
    sub: usize,
    /// Seconds left in a timed sub-step phase.
    timer: f32,
    holding: bool,
    repeats_left: u32,
    /// States played, in order (for tests / debug).
    pub history: Vec<usize>,
    done: bool,
    mood: Mood,
}

impl LifeRun {
    pub fn start(life: &Life, budget: f32, mood: Mood, bias: &HashMap<String, f32>, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) -> Self {
        let mut r = Self { budget, state: 0, sub: 0, timer: 0.0, holding: false, repeats_left: 1, history: Vec::new(), done: false, mood };
        r.next_state(life, mood, bias, clips, anim, rng);
        r
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Name of the state playing.
    pub fn state_name<'a>(&self, life: &'a Life) -> &'a str {
        &life.states[self.state].name
    }

    pub fn interruptible(&self, life: &Life) -> bool {
        self.done || life.states[self.state].steps[self.sub].interruptible
    }

    /// End the current sub-step if it waits on `name`.
    #[allow(clippy::too_many_arguments)]
    pub fn signal(&mut self, life: &Life, bias: &HashMap<String, f32>, clips: &ClipLibrary, anim: &mut Animator, name: &str, rng: &mut impl FnMut() -> f32) {
        if !self.done && life.states[self.state].steps[self.sub].until.as_deref() == Some(name) {
            self.next_sub(life, self.mood, bias, clips, anim, rng);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        life: &Life,
        mood: Mood,
        bias: &HashMap<String, f32>,
        clips: &ClipLibrary,
        anim: &mut Animator,
        dt: f32,
        anim_events: &[AnimEvent],
        rng: &mut impl FnMut() -> f32,
    ) {
        self.mood = mood;
        if self.done {
            return;
        }
        self.budget -= dt;
        let step = &life.states[self.state].steps[self.sub];
        if self.holding || clips.get(step.clip).wrap.loops() {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.next_sub(life, mood, bias, clips, anim, rng);
            }
        } else if anim_events.contains(&AnimEvent::Finished(step.clip)) || anim.is_finished() {
            self.repeats_left = self.repeats_left.saturating_sub(1);
            if self.repeats_left > 0 {
                anim.play(clips, step.clip, true);
            } else if let Some(h) = step.hold {
                self.holding = true;
                self.timer = sample(h, rng);
            } else {
                self.next_sub(life, mood, bias, clips, anim, rng);
            }
        }
    }

    fn next_sub(&mut self, life: &Life, mood: Mood, bias: &HashMap<String, f32>, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) {
        self.sub += 1;
        if self.sub < life.states[self.state].steps.len() {
            self.enter_sub(life, clips, anim, rng);
        } else if self.budget <= 0.0 {
            self.done = true;
        } else {
            self.next_state(life, mood, bias, clips, anim, rng);
        }
    }

    fn next_state(&mut self, life: &Life, mood: Mood, bias: &HashMap<String, f32>, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) {
        let last = self.history.last().copied();
        let mut w: Vec<f32> = (0..life.states.len()).map(|i| life.weight(i, mood, bias) * if Some(i) == last { REPEAT_DAMPING } else { 1.0 }).collect();
        if w.iter().all(|&x| x <= 0.0) {
            w = (0..life.states.len()).map(|i| life.weight(i, mood, bias)).collect();
        }
        let total: f32 = w.iter().sum();
        let mut pick = rng() * total;
        self.state = w.iter().position(|&x| {
            pick -= x;
            pick < 0.0 && x > 0.0
        })
        .unwrap_or_else(|| w.iter().rposition(|&x| x > 0.0).unwrap_or(0));
        self.history.push(self.state);
        self.sub = 0;
        self.enter_sub(life, clips, anim, rng);
    }

    fn enter_sub(&mut self, life: &Life, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) {
        let step = &life.states[self.state].steps[self.sub];
        anim.play(clips, step.clip, true);
        self.holding = false;
        let (lo, hi) = step.repeat;
        self.repeats_left = lo + ((hi - lo + 1) as f32 * rng()) as u32 % (hi - lo + 1);
        self.timer = match (&step.until, step.hold) {
            (Some(_), Some((_, hi))) => hi,
            (_, Some(h)) => sample(h, rng),
            (_, None) => clips.get(step.clip).duration(),
        };
    }
}

fn sample((lo, hi): (f32, f32), rng: &mut impl FnMut() -> f32) -> f32 {
    lo + (hi - lo) * rng()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::acts::ActLibrary;
    use crate::assets::SpriteBank;

    fn libs() -> (ClipLibrary, ActLibrary) {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let bank = SpriteBank::load_metadata(&data.join("hk")).expect("bank");
        let clips = ClipLibrary::load(&data.join("hk/animations/Knight.json"), &bank).expect("clips");
        let acts = ActLibrary::load(&data.join("acts.json"), &clips).expect("acts");
        (clips, acts)
    }

    fn rng(seed: u32) -> impl FnMut() -> f32 {
        let mut x = seed | 1;
        move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as f32 / (1u32 << 24) as f32
        }
    }

    struct Trace {
        /// (state name, clip name, seconds since start) at every clip change.
        clips: Vec<(String, String, f32)>,
        /// Longest time one sprite stayed on screen outside the doze state.
        longest_still: f32,
        secs: f32,
        states: Vec<String>,
    }

    fn run(seed: u32, budget: f32, mood: Mood, bias: &HashMap<String, f32>, max_s: f32) -> Trace {
        let (clips, acts) = libs();
        let life = acts.life(acts.life_id("seated").expect("seated life"));
        let mut anim = Animator::new(life.base);
        let mut r = rng(seed);
        let mut run = LifeRun::start(life, budget, mood, bias, &clips, &mut anim, &mut r);
        let (dt, mut t, mut ev) = (1.0 / 60.0, 0.0, Vec::new());
        let mut trace = Trace { clips: vec![], longest_still: 0.0, secs: 0.0, states: vec![] };
        let (mut key, mut since) = (anim.current(&clips), 0.0);
        while !run.is_done() && t < max_s {
            ev.clear();
            anim.update(&clips, dt, &mut ev);
            run.update(life, mood, bias, &clips, &mut anim, dt, &ev, &mut r);
            t += dt;
            let name = clips.get(anim.clip()).name.clone();
            if trace.clips.last().map(|c| &c.1) != Some(&name) {
                trace.clips.push((run.state_name(life).to_string(), name, t));
            }
            if anim.current(&clips) != key || run.state_name(life) == "doze" {
                key = anim.current(&clips);
                since = t;
            }
            trace.longest_still = trace.longest_still.max(t - since);
        }
        trace.secs = t;
        trace.states = run.history.iter().map(|&i| life.states[i].name.clone()).collect();
        trace
    }

    fn only(name: &str) -> HashMap<String, f32> {
        ["idle", "glance", "look_down", "lean", "map", "nod", "doze"].iter().map(|s| (s.to_string(), if *s == name { 1.0 } else { 0.0 })).collect()
    }

    #[test]
    fn weights_follow_mood_and_bias() {
        let (_, acts) = libs();
        let life = acts.life(acts.life_id("seated").unwrap());
        let i = |n: &str| life.states.iter().position(|s| s.name == n).unwrap();
        let none = HashMap::new();
        let rested = Mood { tired: 0.0, curious: 0.0 };
        let tired = Mood { tired: 1.0, curious: 0.0 };
        let curious = Mood { tired: 0.0, curious: 1.0 };
        assert!(life.weight(i("doze"), tired, &none) > 5.0 * life.weight(i("doze"), rested, &none));
        assert!(life.weight(i("map"), curious, &none) > 2.0 * life.weight(i("map"), rested, &none));
        assert!(life.weight(i("map"), tired, &none) < life.weight(i("map"), rested, &none));
        let lean = HashMap::from([("lean".to_string(), 4.0)]);
        assert_eq!(life.weight(i("lean"), rested, &lean), 4.0 * life.weight(i("lean"), rested, &none));
        assert_eq!(life.weight(i("idle"), rested, &only("map")), 0.0);
    }

    #[test]
    fn sitting_varies_and_never_freezes_awake() {
        for seed in 1..20 {
            let t = run(seed, 30.0, Mood::default(), &HashMap::new(), 120.0);
            assert!(t.secs >= 30.0, "budget respected: {}", t.secs);
            assert!(t.longest_still <= 4.3, "seed {seed}: one frame for {:.1}s: {:?}", t.longest_still, t.clips);
            let distinct: std::collections::HashSet<_> = t.states.iter().collect();
            assert!(distinct.len() >= 3, "seed {seed}: {:?}", t.states);
            assert!(t.states.windows(2).all(|w| w[0] != w[1]), "no back-to-back repeats: {:?}", t.states);
        }
    }

    #[test]
    fn mood_biases_what_he_does() {
        let count = |mood: Mood, name: &str| -> usize {
            (1..16).map(|s| run(s, 60.0, mood, &HashMap::new(), 400.0).states.iter().filter(|n| *n == name).count()).sum()
        };
        let tired = Mood { tired: 1.0, curious: 0.0 };
        let curious = Mood { tired: 0.0, curious: 1.0 };
        assert!(count(tired, "doze") + count(tired, "nod") > 3 * (count(curious, "doze") + count(curious, "nod")).max(1));
        assert!(count(curious, "map") > 2 * count(tired, "map").max(1));
    }

    #[test]
    fn started_states_finish_past_the_budget() {
        // Budget gone at once: the map still gets closed before the life ends.
        let t = run(5, 0.0, Mood::default(), &only("map"), 30.0);
        assert_eq!(t.states, vec!["map"]);
        let names: Vec<&str> = t.clips.iter().map(|c| c.1.as_str()).collect();
        assert!(names.contains(&"Map Update") && names.last() == Some(&"Sit Map Close"), "{names:?}");
    }

    #[test]
    fn woken_ends_a_doze() {
        let (clips, acts) = libs();
        let life = acts.life(acts.life_id("seated").unwrap());
        let bias = only("doze");
        let mut anim = Animator::new(life.base);
        let mut r = rng(9);
        let mut run = LifeRun::start(life, 0.0, Mood::default(), &bias, &clips, &mut anim, &mut r);
        let mut ev = Vec::new();
        for _ in 0..120 {
            ev.clear();
            anim.update(&clips, 1.0 / 60.0, &mut ev);
            run.update(life, Mood::default(), &bias, &clips, &mut anim, 1.0 / 60.0, &ev, &mut r);
        }
        assert_eq!(clips.get(anim.clip()).name, "Sitting Asleep Breathe");
        assert!(run.interruptible(life));
        run.signal(life, &bias, &clips, &mut anim, "woken", &mut r);
        assert_eq!(clips.get(anim.clip()).name, "Wake To Sit");
        assert!(!run.interruptible(life), "waking up is not cut short");
    }
}
