//! "Lives": small weighted state machines that keep a long act step alive instead of
//! holding one frame (the seated knight breathes, glances around, leans back, checks his
//! map, nods off…). Defined in act files under `"lives"`, run by an act step
//! `{"life": "seated"}`.
//!
//! * A life is a list of states; each state is a short sequence of act-style steps
//!   (`clip`, `hold`, `repeat`, `until`, `interruptible`).
//! * The next state is drawn by weight × mood bias (`tired`, `curious`, `lively`) × the
//!   step's per-state `bias`; the state just played is not picked again right away.
//! * `pace` states (plain idle) stretch their holds when calm and shrink them when lively,
//!   so a calm knight changes pose a few times a minute and a mischievous one fidgets.
//! * `min` + `body`: the `body` sub-step repeats until the state has lasted `min` seconds
//!   (a map check always includes reading). `exit`: the sub-step an urgent wrap-up jumps to
//!   (closing the map, sitting up, waking).
//! * The step's budget (`hold`) is only checked between states, so a started state always
//!   finishes; [`LifeRun::wrap_up`] ends it early, gracefully (or via `exit` when urgent).
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
    /// Weight multiplier `1 + lively * k` (mischief: fidgets).
    #[serde(default)]
    lively: f32,
    /// Holds stretch when calm, shrink when lively.
    #[serde(default)]
    pace: bool,
    /// Minimum duration [lo, hi] s, reached by repeating the `body` sub-step.
    #[serde(default)]
    min: Option<[f32; 2]>,
    #[serde(default)]
    body: usize,
    /// Sub-step an urgent wrap-up jumps to.
    #[serde(default)]
    exit: Option<usize>,
    steps: Vec<crate::acts::StepJson>,
}

/// One state of a life.
#[derive(Clone, Debug)]
pub struct LifeState {
    pub name: String,
    pub weight: f32,
    pub tired: f32,
    pub curious: f32,
    pub lively: f32,
    pub pace: bool,
    pub min: Option<(f32, f32)>,
    pub body: usize,
    pub exit: Option<usize>,
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
            let ctx = format!("life '{name}' state '{}'", s.name);
            let steps = s.steps.into_iter().map(|st| st.resolve(&ctx, clips, &HashMap::new())).collect::<Result<Vec<_>>>()?;
            if steps.is_empty() {
                bail!("{ctx}: no steps");
            }
            if steps.iter().any(|st| st.life.is_some()) {
                bail!("{ctx}: lives cannot nest");
            }
            if s.body >= steps.len() || s.exit.is_some_and(|e| e >= steps.len()) {
                bail!("{ctx}: `body` / `exit` out of range");
            }
            states.push(LifeState {
                name: s.name,
                weight: s.weight.max(0.0),
                tired: s.tired,
                curious: s.curious,
                lively: s.lively,
                pace: s.pace,
                min: s.min.map(|[lo, hi]| (lo, hi.max(lo))),
                body: s.body,
                exit: s.exit,
                steps,
            });
        }
        Ok(Life { name: name.to_string(), base, states })
    }

    /// Selection weight of state `i` for `mood`, with the step's per-state `bias`.
    pub fn weight(&self, i: usize, mood: Mood, bias: &HashMap<String, f32>) -> f32 {
        let s = &self.states[i];
        let f = |m: f32, k: f32| (1.0 + m * k).max(0.0);
        s.weight * f(mood.tired, s.tired) * f(mood.curious, s.curious) * f(mood.lively, s.lively) * bias.get(&s.name).copied().unwrap_or(1.0)
    }
}

/// What biases a life's choices, each 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mood {
    pub tired: f32,
    pub curious: f32,
    /// Mischief / playfulness: shorter idles, more fidgets.
    pub lively: f32,
}

impl Default for Mood {
    fn default() -> Self {
        Self { tired: 0.3, curious: 0.3, lively: 0.2 }
    }
}

impl Mood {
    /// Hold multiplier for `pace` states: 1.3 when calm, 0.4 when fully lively.
    pub fn pace(self) -> f32 {
        1.3 - 0.9 * self.lively.clamp(0.0, 1.0)
    }
}

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
    /// Seconds spent in the current state, and the minimum it must last.
    state_t: f32,
    state_min: f32,
    /// Urgent wrap-up: skip `min`, go out through `exit`.
    urgent: bool,
    /// States played, in order (for tests / debug).
    pub history: Vec<usize>,
    done: bool,
    mood: Mood,
}

impl LifeRun {
    pub fn start(life: &Life, budget: f32, mood: Mood, bias: &HashMap<String, f32>, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) -> Self {
        let mut r = Self {
            budget,
            state: 0,
            sub: 0,
            timer: 0.0,
            holding: false,
            repeats_left: 1,
            state_t: 0.0,
            state_min: 0.0,
            urgent: false,
            history: Vec::new(),
            done: false,
            mood,
        };
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
            self.state_min = 0.0; // a signal (being woken) beats the minimum
            self.next_sub(life, self.mood, bias, clips, anim, rng);
        }
    }

    /// Finish soon. Not urgent: the current state still plays to its natural end, but no
    /// longer than its minimum (a sleep is cut to its minimum, then he wakes). Urgent: jump
    /// to the state's `exit` sub-step (close the map, sit up, wake) or stop right away.
    pub fn wrap_up(&mut self, life: &Life, urgent: bool, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) {
        self.budget = 0.0;
        if self.done {
            return;
        }
        let s = &life.states[self.state];
        if urgent && !self.urgent {
            self.urgent = true;
            match s.exit {
                Some(e) if self.sub < e => {
                    self.sub = e;
                    self.enter_sub(life, s.pace, clips, anim, rng);
                }
                Some(_) => {}
                None => self.done = true,
            }
        } else if !urgent && self.sub == s.body && (self.holding || clips.get(s.steps[self.sub].clip).wrap.loops()) {
            self.timer = self.timer.min((self.state_min - self.state_t).max(0.3));
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
        self.state_t += dt;
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
        let s = &life.states[self.state];
        if self.sub == s.body && !self.urgent && self.state_t < self.state_min {
            // Not long enough yet: another round of the body (read the map again).
            self.enter_sub(life, s.pace, clips, anim, rng);
            return;
        }
        self.sub += 1;
        if self.sub < s.steps.len() {
            self.enter_sub(life, s.pace, clips, anim, rng);
        } else if self.budget <= 0.0 || self.urgent {
            self.done = true;
        } else {
            self.next_state(life, mood, bias, clips, anim, rng);
        }
    }

    fn next_state(&mut self, life: &Life, mood: Mood, bias: &HashMap<String, f32>, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) {
        let last = self.history.last().copied();
        let mut w: Vec<f32> = (0..life.states.len()).map(|i| if Some(i) == last { 0.0 } else { life.weight(i, mood, bias) }).collect();
        if w.iter().all(|&x| x <= 0.0) {
            w = (0..life.states.len()).map(|i| life.weight(i, mood, bias)).collect();
        }
        let total: f32 = w.iter().sum();
        let mut pick = rng() * total;
        self.state = w
            .iter()
            .position(|&x| {
                pick -= x;
                pick < 0.0 && x > 0.0
            })
            .unwrap_or_else(|| w.iter().rposition(|&x| x > 0.0).unwrap_or(0));
        self.history.push(self.state);
        let s = &life.states[self.state];
        self.sub = 0;
        self.state_t = 0.0;
        self.state_min = s.min.map_or(0.0, |m| sample(m, rng));
        self.enter_sub(life, s.pace, clips, anim, rng);
    }

    fn enter_sub(&mut self, life: &Life, pace: bool, clips: &ClipLibrary, anim: &mut Animator, rng: &mut impl FnMut() -> f32) {
        let step = &life.states[self.state].steps[self.sub];
        anim.play(clips, step.clip, true);
        self.holding = false;
        let (lo, hi) = step.repeat;
        self.repeats_left = lo + ((hi - lo + 1) as f32 * rng()) as u32 % (hi - lo + 1);
        let k = if pace { self.mood.pace() } else { 1.0 };
        self.timer = match (&step.until, step.hold) {
            (Some(_), Some((_, hi))) => hi,
            (_, Some(h)) => sample(h, rng) * k,
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
        let rested = Mood { tired: 0.0, curious: 0.0, lively: 0.0 };
        let tired = Mood { tired: 1.0, curious: 0.0, lively: 0.0 };
        let curious = Mood { tired: 0.0, curious: 1.0, lively: 0.0 };
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
            let t = run(seed, 90.0, Mood::default(), &HashMap::new(), 200.0);
            assert!(t.secs >= 90.0, "budget respected: {}", t.secs);
            assert!(t.longest_still <= 4.3, "seed {seed}: one frame for {:.1}s: {:?}", t.longest_still, t.clips);
            let distinct: std::collections::HashSet<_> = t.states.iter().collect();
            assert!(distinct.len() >= 3, "seed {seed}: {:?}", t.states);
            assert!(t.states.windows(2).all(|w| w[0] != w[1]), "no back-to-back repeats: {:?}", t.states);
        }
    }

    #[test]
    fn mood_biases_what_he_does() {
        // Share of the states started that are `names`.
        let share = |mood: Mood, names: &[&str]| -> f32 {
            let runs: Vec<Trace> = (1..10).map(|s| run(s, 300.0, mood, &HashMap::new(), 600.0)).collect();
            let all: usize = runs.iter().map(|r| r.states.len()).sum();
            let hit: usize = runs.iter().map(|r| r.states.iter().filter(|n| names.contains(&n.as_str())).count()).sum();
            hit as f32 / all.max(1) as f32
        };
        let tired = Mood { tired: 1.0, curious: 0.0, lively: 0.0 };
        let curious = Mood { tired: 0.0, curious: 1.0, lively: 0.0 };
        let (t, c) = (share(tired, &["doze", "nod"]), share(curious, &["doze", "nod"]));
        assert!(t > 3.0 * c, "sleepy share: tired {t:.2} vs curious {c:.2}");
        let (c, t) = (share(curious, &["map"]), share(tired, &["map"]));
        assert!(c > 2.0 * t, "map share: curious {c:.2} vs tired {t:.2}");
    }

    #[test]
    fn started_states_finish_past_the_budget() {
        // Budget gone at once: the map still gets closed before the life ends.
        let t = run(5, 0.0, Mood::default(), &only("map"), 30.0);
        assert_eq!(t.states, vec!["map"]);
        let names: Vec<&str> = t.clips.iter().map(|c| c.1.as_str()).collect();
        assert!(names.contains(&"Map Update") && names.last() == Some(&"Sit Map Close"), "{names:?}");
        assert!(t.secs >= 4.5, "a map check lasts its minimum: {:.1}s", t.secs);
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
    /// Transitions (states started) per minute over a long sit.
    fn per_minute(mood: Mood) -> f32 {
        let (n, secs) = (1..8).map(|s| run(s, 300.0, mood, &HashMap::new(), 400.0)).fold((0, 0.0), |(n, t), r| (n + r.states.len(), t + r.secs));
        n as f32 / (secs / 60.0)
    }

    #[test]
    fn calm_sits_slowly_mischief_fidgets() {
        let calm = per_minute(Mood { tired: 0.4, curious: 0.2, lively: 0.0 });
        let lively = per_minute(Mood { tired: 0.1, curious: 0.4, lively: 1.0 });
        assert!((2.5..=6.5).contains(&calm), "calm: {calm:.1}/min");
        assert!(lively >= 10.0, "mischievous: {lively:.1}/min");
    }

    fn start_map(seed: u32) -> (ClipLibrary, ActLibrary, Animator, LifeRun) {
        let (clips, acts) = libs();
        let l = acts.life_id("seated").unwrap();
        let mut anim = Animator::new(acts.life(l).base);
        let run = LifeRun::start(acts.life(l), 60.0, Mood::default(), &only("map"), &clips, &mut anim, &mut rng(seed));
        (clips, acts, anim, run)
    }

    /// Steps until the life ends; returns (seconds, clip names seen).
    fn finish(clips: &ClipLibrary, life: &Life, anim: &mut Animator, run: &mut LifeRun, mut t: f32) -> (f32, Vec<String>) {
        let (mut ev, mut seen, mut r) = (Vec::new(), Vec::<String>::new(), rng(3));
        while !run.is_done() && t < 60.0 {
            ev.clear();
            anim.update(clips, 1.0 / 60.0, &mut ev);
            run.update(life, Mood::default(), &only("map"), clips, anim, 1.0 / 60.0, &ev, &mut r);
            t += 1.0 / 60.0;
            let n = clips.get(anim.clip()).name.clone();
            if seen.last() != Some(&n) {
                seen.push(n);
            }
        }
        (t, seen)
    }

    #[test]
    fn graceful_wrap_up_keeps_the_minimum_then_closes() {
        let (clips, acts, mut anim, mut run) = start_map(4);
        let life = acts.life(acts.life_id("seated").unwrap());
        let (t, _) = finish_for(&clips, life, &mut anim, &mut run, 0.5);
        run.wrap_up(life, false, &clips, &mut anim, &mut rng(1));
        let (end, seen) = finish(&clips, life, &mut anim, &mut run, t);
        assert!(end >= 4.5, "map not cut before its minimum: {end:.1}s");
        assert!(end < 8.0, "but wraps up promptly: {end:.1}s");
        assert_eq!(seen.last().map(String::as_str), Some("Sit Map Close"), "{seen:?}");
        assert_eq!(run.history.len(), 1, "no new state after a wrap-up");
    }

    #[test]
    fn urgent_wrap_up_exits_through_the_close_clip() {
        let (clips, acts, mut anim, mut run) = start_map(4);
        let life = acts.life(acts.life_id("seated").unwrap());
        let (t, _) = finish_for(&clips, life, &mut anim, &mut run, 0.8);
        run.wrap_up(life, true, &clips, &mut anim, &mut rng(1));
        assert_eq!(clips.get(anim.clip()).name, "Sit Map Close");
        let (end, _) = finish(&clips, life, &mut anim, &mut run, t);
        assert!(end - t < 0.6, "exit clip only: {:.2}s", end - t);
        // Quick states without an exit stop at once.
        let mut anim2 = Animator::new(life.base);
        let mut glance = LifeRun::start(life, 60.0, Mood::default(), &only("glance"), &clips, &mut anim2, &mut rng(2));
        glance.wrap_up(life, true, &clips, &mut anim2, &mut rng(1));
        assert!(glance.is_done());
    }

    /// Steps for `secs` (without the life ending).
    fn finish_for(clips: &ClipLibrary, life: &Life, anim: &mut Animator, run: &mut LifeRun, secs: f32) -> (f32, ()) {
        let (mut ev, mut r, mut t) = (Vec::new(), rng(5), 0.0);
        while t < secs {
            ev.clear();
            anim.update(clips, 1.0 / 60.0, &mut ev);
            run.update(life, Mood::default(), &only("map"), clips, anim, 1.0 / 60.0, &ev, &mut r);
            t += 1.0 / 60.0;
        }
        (t, ())
    }
}
