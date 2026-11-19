//! Acts: data-driven choreographies (sit → open map → scribble → close → get up) built
//! from Knight clips. The AI picks acts by mood tags; [`ActPlayer`] runs one by driving
//! an [`Animator`] and reporting any locomotion the step wants.
//!
//! Step semantics:
//! * looping clip — plays for `hold` seconds (default: one pass), or until `until` signal;
//! * one-shot clip — plays `repeat` times, then holds the last frame for `hold` seconds;
//! * `move` — the controller should walk with that gait while the step runs.
//! * `life` — run a weighted state machine from the file's `lives` (see [`crate::life`])
//!   for `hold` seconds instead of one clip (`bias` scales its states' weights by name).
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::anim::{AnimEvent, Animator, ClipId, ClipLibrary};
use crate::assets::read_json;
use crate::controller::Gait;
use crate::life::{Life, LifeJson, LifeRun, Mood};

#[derive(Debug, Deserialize)]
struct ActsFile {
    /// Lives (weighted state machines) usable by `life` steps in this and later files.
    #[serde(default)]
    lives: std::collections::BTreeMap<String, LifeJson>,
    #[serde(default)]
    acts: Vec<ActJson>,
}

#[derive(Debug, Deserialize)]
struct ActJson {
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    requires: Requirement,
    #[serde(default = "one")]
    weight: f32,
    #[serde(default)]
    cooldown: f32,
    /// A held pose with an ending (see [`Act::linger`]).
    #[serde(default)]
    linger: bool,
    steps: Vec<StepJson>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StepJson {
    /// Optional for `life` steps (defaults to the life's base clip).
    #[serde(default)]
    clip: Option<String>,
    #[serde(default)]
    life: Option<String>,
    #[serde(default)]
    bias: HashMap<String, f32>,
    #[serde(default)]
    hold: Option<[f32; 2]>,
    #[serde(default)]
    repeat: Option<[u32; 2]>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default, rename = "move")]
    movement: Option<String>,
    #[serde(default)]
    turn: Option<String>,
    #[serde(default = "yes")]
    interruptible: bool,
}

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}

impl StepJson {
    /// Validate against `clips` and the lives known so far (name -> (index, base clip)).
    pub(crate) fn resolve(self, ctx: &str, clips: &ClipLibrary, lives: &HashMap<String, (usize, ClipId)>) -> Result<Step> {
        let s = self;
        let life = match s.life.as_deref() {
            None => None,
            Some(l) => Some(*lives.get(l).with_context(|| format!("{ctx}: unknown life '{l}'"))?),
        };
        let clip = match (&s.clip, life) {
            (Some(c), _) => clips.id(c).with_context(|| format!("{ctx}: unknown clip '{c}'"))?,
            (None, Some((_, base))) => base,
            (None, None) => bail!("{ctx}: step without clip"),
        };
        let gait = match s.movement.as_deref() {
            None => None,
            Some("walk") => Some(Gait::Walk),
            Some("run") => Some(Gait::Run),
            Some("sprint") => Some(Gait::Sprint),
            Some(m) => bail!("{ctx}: unknown move '{m}'"),
        };
        let turn = match s.turn.as_deref() {
            None => None,
            Some(t) => Some(clips.id(t).with_context(|| format!("{ctx}: unknown turn clip '{t}'"))?),
        };
        let repeat = s.repeat.map_or((1, 1), |[lo, hi]| (lo.max(1), hi.max(lo.max(1))));
        Ok(Step {
            clip,
            hold: s.hold.map(|[lo, hi]| (lo, hi.max(lo))),
            repeat,
            until: s.until,
            gait,
            interruptible: s.interruptible,
            turn,
            life: life.map(|(i, _)| i),
            bias: s.bias,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Requirement {
    #[default]
    Grounded,
    Any,
    /// Seated: perched on a ledge edge (legs dangling) or on a placed bench. Sitting
    /// clips are a front-facing pose with dangling legs.
    Seat,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub clip: ClipId,
    pub hold: Option<(f32, f32)>,
    pub repeat: (u32, u32),
    pub until: Option<String>,
    pub gait: Option<Gait>,
    pub interruptible: bool,
    /// Clip played when a walking step turns around (e.g. "Map Turn"), then the step clip resumes.
    pub turn: Option<ClipId>,
    /// Index of the life this step runs (see [`ActLibrary::life`]).
    pub life: Option<usize>,
    /// Per-state weight multipliers for the life.
    pub bias: HashMap<String, f32>,
}

#[derive(Clone, Debug)]
pub struct Act {
    pub name: String,
    pub tags: Vec<String>,
    pub requires: Requirement,
    pub weight: f32,
    /// Seconds before the act may be picked again.
    pub cooldown: f32,
    /// A held pose with an ending (look down, look up, gaze...): when the pose step ends the
    /// player may hold it a little longer ([`ActPlayer::allow_linger`]) so that an identical
    /// act chosen next continues the pose ([`ActPlayer::fuse`]) instead of playing the ending
    /// and the start again.
    pub linger: bool,
    pub steps: Vec<Step>,
}

impl Act {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActId(pub u16);

#[derive(Debug, Default)]
pub struct ActLibrary {
    acts: Vec<Act>,
    by_name: HashMap<String, ActId>,
    lives: Vec<Life>,
    life_ids: HashMap<String, (usize, ClipId)>,
}

impl ActLibrary {
    /// Load and validate against the clip library (unknown clips are an error). Act files
    /// in an `acts/` folder next to `path` (e.g. `data/acts/stakes.json`) are added too.
    pub fn load(path: &Path, clips: &ClipLibrary) -> Result<ActLibrary> {
        let mut lib = ActLibrary::default();
        lib.add_file(path, clips)?;
        let dir = path.parent().map(|d| d.join("acts"));
        let mut extra: Vec<_> = dir.and_then(|d| std::fs::read_dir(d).ok()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        extra.sort();
        for p in extra {
            lib.add_file(&p, clips)?;
        }
        Ok(lib)
    }

    fn add_file(&mut self, path: &Path, clips: &ClipLibrary) -> Result<()> {
        let raw: ActsFile = read_json(path)?;
        let lib = self;
        for (name, j) in raw.lives {
            let life = Life::from_json(&name, j, clips)?;
            if lib.life_ids.insert(name.clone(), (lib.lives.len(), life.base)).is_some() {
                bail!("duplicate life '{name}'");
            }
            lib.lives.push(life);
        }
        for a in raw.acts {
            if a.steps.is_empty() {
                bail!("act '{}' has no steps", a.name);
            }
            let ctx = format!("act '{}'", a.name);
            let steps = a.steps.into_iter().map(|s| s.resolve(&ctx, clips, &lib.life_ids)).collect::<Result<Vec<_>>>()?;
            let id = ActId(lib.acts.len() as u16);
            if lib.by_name.insert(a.name.clone(), id).is_some() {
                bail!("duplicate act '{}'", a.name);
            }
            lib.acts.push(Act { name: a.name, tags: a.tags, requires: a.requires, weight: a.weight, cooldown: a.cooldown, linger: a.linger, steps });
        }
        Ok(())
    }

    pub fn id(&self, name: &str) -> Option<ActId> {
        self.by_name.get(name).copied()
    }

    pub fn life(&self, i: usize) -> &Life {
        &self.lives[i]
    }

    pub fn life_id(&self, name: &str) -> Option<usize> {
        self.life_ids.get(name).map(|&(i, _)| i)
    }

    pub fn get(&self, id: ActId) -> &Act {
        &self.acts[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = (ActId, &Act)> {
        self.acts.iter().enumerate().map(|(i, a)| (ActId(i as u16), a))
    }

    pub fn len(&self) -> usize {
        self.acts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.acts.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActStatus {
    /// Running; `gait` is set when the current step wants the character to walk.
    Running { gait: Option<Gait> },
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Clip playing (one-shot: until finished; looping: until timer).
    Playing,
    /// One-shot finished, holding its last frame until timer.
    Holding,
}

/// Runs one act. Deterministic given its seed.
#[derive(Clone, Debug)]
pub struct ActPlayer {
    act: ActId,
    step: usize,
    phase: Phase,
    /// Seconds left in the current timed phase (looping play or hold).
    timer: f32,
    repeats_left: u32,
    done: bool,
    rng: u32,
    /// Playing the step's turn clip (see [`ActPlayer::turned`]).
    turning: bool,
    /// The current `life` step's state machine.
    life: Option<LifeRun>,
    /// Biases life choices (set by the app from the brain's needs).
    mood: Mood,
    /// Wrapping up: after the current life step, go straight to the act's last step.
    wrapping: bool,
    /// How long a lingering pose may be held (0 = never linger; see [`Self::allow_linger`]).
    linger_max: f32,
    /// Seconds left holding the pose past its step, waiting to see whether the same pose
    /// is chosen next.
    lingering: Option<f32>,
    /// Playing the ending after a linger (see [`Self::is_released`]).
    released: bool,
}

impl ActPlayer {
    pub fn start(acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, act: ActId, seed: u32) -> Self {
        let mut p = Self { act, step: 0, phase: Phase::Playing, timer: 0.0, repeats_left: 1, done: false, rng: seed | 1, turning: false, life: None, mood: Mood::default(), wrapping: false, linger_max: 0.0, lingering: None, released: false };
        p.enter_step(acts, clips, anim);
        p
    }

    pub fn act(&self) -> ActId {
        self.act
    }

    /// Let an [`Act::linger`] pose be held up to `secs` past its step instead of going
    /// straight into its ending. While lingering the act counts as finished for whoever
    /// chose it ([`Self::is_lingering`]); [`Self::fuse`] continues the pose with the next
    /// act, [`Self::release`] (or the time running out) plays the ending.
    pub fn allow_linger(mut self, secs: f32) -> Self {
        self.linger_max = secs.max(0.0);
        self
    }

    /// Holding the pose after its step ended (see [`Self::allow_linger`]).
    pub fn is_lingering(&self) -> bool {
        self.lingering.is_some() && !self.done
    }

    /// Playing the ending of a pose that lingered: the act was already reported finished,
    /// so it should not be cut again.
    pub fn is_released(&self) -> bool {
        self.released && !self.done
    }

    /// The act chosen next while lingering: when it holds the same pose (the same act, or
    /// any act with a timed step on the same clip before its ending), continue the pose as
    /// that act's step instead of playing the ending and the start again. Returns false
    /// (and changes nothing) otherwise.
    pub fn fuse(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, next: ActId) -> bool {
        if !self.is_lingering() {
            return false;
        }
        let pose = acts.get(self.act).steps[self.step].clip;
        let steps = &acts.get(next).steps;
        let body = &steps[..steps.len().saturating_sub(1)];
        let Some(i) = body.iter().position(|s| s.clip == pose && s.hold.is_some() && s.life.is_none() && s.gait.is_none() && s.until.is_none()) else {
            return false;
        };
        let hold = steps[i].hold.unwrap_or((1.0, 1.0));
        self.act = next;
        self.step = i;
        self.lingering = None;
        self.released = false;
        self.turning = false;
        self.repeats_left = 1;
        // No restart: a looping pose plays on, a one-shot pose keeps its last frame.
        anim.play(clips, pose, false);
        self.phase = if clips.get(pose).wrap.loops() { Phase::Playing } else { Phase::Holding };
        self.timer = self.sample(hold);
        true
    }

    /// End a lingering pose now: play the act's ending.
    pub fn release(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator) {
        if self.is_lingering() {
            self.lingering = None;
            self.released = true;
            self.advance(acts, clips, anim);
        }
    }

    pub fn step_index(&self) -> usize {
        self.step
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Asked to stop. During a `life` step the act ends gracefully instead of being cut:
    /// not urgent, the current state plays out (bounded by its minimum); urgent, it leaves
    /// through the state's exit clip (closing the map, sitting up). Then the act's last step
    /// (Get Off) plays. Returns false when the current step is not a life step (the caller
    /// cuts or replaces the act as before).
    pub fn wrap_up(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, urgent: bool) -> bool {
        let Some(l) = acts.get(self.act).steps.get(self.step).and_then(|s| s.life) else { return false };
        let Some(run) = self.life.as_mut().filter(|_| !self.done) else { return false };
        self.wrapping = true;
        let mut seed = self.rng;
        run.wrap_up(acts.life(l), urgent, clips, anim, &mut || xorshift(&mut seed));
        self.rng = seed;
        true
    }

    /// Wrapping up (see [`Self::wrap_up`]).
    pub fn is_wrapping(&self) -> bool {
        self.wrapping
    }

    /// Bias for `life` steps (tired → dozing and nodding, curious → map checks and glances).
    pub fn set_mood(&mut self, mood: Mood) {
        self.mood = mood;
    }

    /// Name of the life state playing, when the current step is a `life` step.
    pub fn life_state<'a>(&self, acts: &'a ActLibrary) -> Option<&'a str> {
        let l = acts.get(self.act).steps.get(self.step)?.life?;
        self.life.as_ref().filter(|_| !self.done).map(|r| r.state_name(acts.life(l)))
    }

    /// Gait the current step wants the character to walk with, if any.
    pub fn gait(&self, acts: &ActLibrary) -> Option<Gait> {
        if self.done { None } else { acts.get(self.act).steps[self.step].gait }
    }

    /// Whether the current step may be cut short.
    pub fn interruptible(&self, acts: &ActLibrary) -> bool {
        if let (Some(run), Some(l)) = (&self.life, acts.get(self.act).steps.get(self.step).and_then(|s| s.life)) {
            return self.done || (acts.get(self.act).steps[self.step].interruptible && run.interruptible(acts.life(l)));
        }
        self.done || acts.get(self.act).steps[self.step].interruptible
    }

    /// External signal (e.g. "woken"). Ends the current step if it waits on it.
    pub fn signal(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, name: &str) {
        if self.done {
            return;
        }
        let step = &acts.get(self.act).steps[self.step];
        if let (Some(run), Some(l)) = (self.life.as_mut(), step.life) {
            let mut seed = self.rng;
            run.signal(acts.life(l), &step.bias, clips, anim, name, &mut || xorshift(&mut seed));
            self.rng = seed;
            return;
        }
        if acts.get(self.act).steps[self.step].until.as_deref() == Some(name) {
            self.next_step(acts, clips, anim);
        }
    }

    /// The walking character just turned around: play the step's turn clip (if any),
    /// then resume the step clip. The step's timer keeps running.
    pub fn turned(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator) {
        if self.done {
            return;
        }
        if let Some(t) = acts.get(self.act).steps[self.step].turn {
            anim.play(clips, t, true);
            self.turning = true;
        }
    }

    /// Advance timers. Call after `anim.update(...)` with the events it produced this tick.
    pub fn update(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, dt: f32, anim_events: &[AnimEvent]) -> ActStatus {
        if self.done {
            return ActStatus::Done;
        }
        if let Some(left) = self.lingering.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.release(acts, clips, anim);
            }
            return if self.done { ActStatus::Done } else { ActStatus::Running { gait: None } };
        }
        let step = &acts.get(self.act).steps[self.step];
        if let (Some(run), Some(l)) = (self.life.as_mut(), step.life) {
            let mut seed = self.rng;
            run.update(acts.life(l), self.mood, &step.bias, clips, anim, dt, anim_events, &mut || xorshift(&mut seed));
            self.rng = seed;
            if run.is_done() {
                if self.wrapping && self.step + 1 < acts.get(self.act).steps.len() {
                    // Skip to the way out (Get Off).
                    self.step = acts.get(self.act).steps.len() - 2;
                }
                self.next_step(acts, clips, anim);
            }
            return if self.done { ActStatus::Done } else { ActStatus::Running { gait: acts.get(self.act).steps[self.step].gait } };
        }
        let looping = clips.get(step.clip).wrap.loops();
        if self.turning {
            let t = step.turn.unwrap_or(step.clip);
            if anim_events.contains(&AnimEvent::Finished(t)) || anim.is_finished() || anim.clip() != t {
                self.turning = false;
                anim.play(clips, step.clip, false);
            } else if !looping {
                return ActStatus::Running { gait: step.gait };
            }
        }
        match self.phase {
            Phase::Playing if looping => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.next_step(acts, clips, anim);
                }
            }
            Phase::Playing => {
                let finished = anim_events.contains(&AnimEvent::Finished(step.clip)) || anim.is_finished();
                if finished {
                    self.repeats_left = self.repeats_left.saturating_sub(1);
                    if self.repeats_left > 0 {
                        anim.play(clips, step.clip, true);
                    } else if let Some(h) = step.hold {
                        self.phase = Phase::Holding;
                        self.timer = self.sample(h);
                        if self.timer <= 0.0 {
                            self.next_step(acts, clips, anim);
                        }
                    } else {
                        self.next_step(acts, clips, anim);
                    }
                }
            }
            Phase::Holding => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.next_step(acts, clips, anim);
                }
            }
        }
        if self.done {
            ActStatus::Done
        } else {
            ActStatus::Running { gait: acts.get(self.act).steps[self.step].gait }
        }
    }

    fn next_step(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator) {
        // A lingering pose: the timed, standing step right before the act's ending.
        let act = acts.get(self.act);
        let s = &act.steps[self.step];
        if act.linger && self.linger_max > 0.0 && !self.released && self.step + 2 == act.steps.len() && s.hold.is_some() && s.life.is_none() && s.gait.is_none() && s.until.is_none() {
            self.lingering = Some(self.linger_max);
            return;
        }
        self.advance(acts, clips, anim);
    }

    fn advance(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator) {
        self.step += 1;
        if self.step >= acts.get(self.act).steps.len() {
            self.done = true;
            self.step = acts.get(self.act).steps.len() - 1;
            return;
        }
        self.enter_step(acts, clips, anim);
    }

    fn enter_step(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator) {
        let step = acts.get(self.act).steps[self.step].clone();
        let clip = clips.get(step.clip);
        anim.play(clips, step.clip, true);
        self.phase = Phase::Playing;
        self.turning = false;
        self.life = None;
        if let Some(l) = step.life {
            let budget = self.sample(step.hold.unwrap_or((10.0, 30.0)));
            let mut seed = self.rng;
            self.life = Some(LifeRun::start(acts.life(l), budget, self.mood, &step.bias, clips, anim, &mut || xorshift(&mut seed)));
            self.rng = seed;
            return;
        }
        self.repeats_left = self.sample_u(step.repeat);
        // Looping clips run for `hold` (default one full pass); `until` steps use the
        // upper bound of `hold` as a timeout.
        self.timer = match (&step.until, step.hold) {
            (Some(_), Some((_, hi))) => hi,
            (_, Some(h)) => self.sample(h),
            (_, None) => clip.duration(),
        };
    }

    fn next_rand(&mut self) -> f32 {
        xorshift(&mut self.rng)
    }

    fn sample(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.next_rand()
    }

    fn sample_u(&mut self, (lo, hi): (u32, u32)) -> u32 {
        lo + ((hi - lo + 1) as f32 * self.next_rand()) as u32 % (hi - lo + 1)
    }
}

/// xorshift32 step, returning 0..1.
fn xorshift(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::SpriteBank;

    fn libs() -> (ClipLibrary, ActLibrary) {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let bank = SpriteBank::load_metadata(&data.join("hk")).expect("bank");
        let clips = ClipLibrary::load(&data.join("hk/animations/Knight.json"), &bank).expect("clips");
        let acts = ActLibrary::load(&data.join("acts.json"), &clips).expect("acts.json must reference real clips");
        (clips, acts)
    }

    fn run(clips: &ClipLibrary, acts: &ActLibrary, name: &str, max_s: f32) -> (Vec<String>, f32) {
        let id = acts.id(name).expect("act");
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(acts, clips, &mut anim, id, 42);
        let mut seen = vec![clips.get(anim.clip()).name.clone()];
        let mut ev = Vec::new();
        let dt = 1.0 / 120.0;
        let mut t = 0.0;
        while t < max_s {
            ev.clear();
            anim.update(clips, dt, &mut ev);
            if p.update(acts, clips, &mut anim, dt, &ev) == ActStatus::Done {
                return (seen, t);
            }
            let name = &clips.get(anim.clip()).name;
            if seen.last() != Some(name) {
                seen.push(name.clone());
            }
            t += dt;
        }
        (seen, t)
    }

    #[test]
    fn every_act_terminates() {
        let (clips, acts) = libs();
        for (_, a) in acts.iter() {
            let (_, t) = run(&clips, &acts, &a.name, 700.0);
            assert!(t < 700.0, "act '{}' never finished", a.name);
        }
    }

    #[test]
    fn read_map_sequence() {
        let (clips, acts) = libs();
        let (seen, _) = run(&clips, &acts, "read_map", 60.0);
        assert_eq!(seen.first().map(String::as_str), Some("Sit"));
        assert!(seen.contains(&"Sit Map Open".to_string()));
        assert_eq!(seen.last().map(String::as_str), Some("Get Off"));
    }

    #[test]
    fn nap_wakes_on_signal() {
        let (clips, acts) = libs();
        let id = acts.id("nap").expect("nap");
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, id, 7);
        let mut ev = Vec::new();
        // run until asleep
        for _ in 0..120 * 10 {
            ev.clear();
            anim.update(&clips, 1.0 / 120.0, &mut ev);
            p.update(&acts, &clips, &mut anim, 1.0 / 120.0, &ev);
        }
        assert_eq!(clips.get(anim.clip()).name, "Sitting Asleep Breathe");
        p.signal(&acts, &clips, &mut anim, "woken");
        assert_eq!(clips.get(anim.clip()).name, "Wake To Sit");
        assert!(!p.interruptible(&acts));
    }

    #[test]
    fn seated_acts_wrap_up_through_get_off() {
        let (clips, acts) = libs();
        let id = acts.id("rest").expect("rest");
        for (urgent, max_s) in [(true, 2.5), (false, 25.0)] {
            for seed in 1..6 {
                let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
                let mut p = ActPlayer::start(&acts, &clips, &mut anim, id, seed);
                let (dt, mut ev) = (1.0 / 60.0, Vec::new());
                let mut step = |p: &mut ActPlayer, anim: &mut Animator| {
                    ev.clear();
                    anim.update(&clips, dt, &mut ev);
                    p.update(&acts, &clips, anim, dt, &ev)
                };
                for _ in 0..(60.0 * (3.0 + seed as f32)) as usize {
                    step(&mut p, &mut anim);
                }
                assert!(p.life_state(&acts).is_some(), "in the seated life");
                assert!(p.wrap_up(&acts, &clips, &mut anim, urgent));
                let mut seen = vec![];
                let mut t = 0.0;
                while step(&mut p, &mut anim) != ActStatus::Done && t < 60.0 {
                    t += dt;
                    let n = clips.get(anim.clip()).name.clone();
                    if seen.last() != Some(&n) {
                        seen.push(n);
                    }
                }
                assert!(t < max_s, "urgent={urgent} seed {seed}: took {t:.1}s: {seen:?}");
                assert_eq!(seen.last().map(String::as_str), Some("Get Off"), "{seen:?}");
            }
        }
        // Not a life step: the caller decides (cut or exit act).
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, acts.id("look_up").expect("act"), 1);
        assert!(!p.wrap_up(&acts, &clips, &mut anim, true));
    }

    #[test]
    fn walking_step_plays_its_turn_clip_then_resumes() {
        let (clips, acts) = libs();
        let id = acts.id("map_stroll").expect("act");
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, id, 3);
        let mut ev = Vec::new();
        let dt = 1.0 / 120.0;
        while p.gait(&acts).is_none() {
            ev.clear();
            anim.update(&clips, dt, &mut ev);
            p.update(&acts, &clips, &mut anim, dt, &ev);
        }
        p.turned(&acts, &clips, &mut anim);
        assert_eq!(clips.get(anim.clip()).name, "Map Turn");
        let mut seen = vec![];
        for _ in 0..30 {
            ev.clear();
            anim.update(&clips, dt, &mut ev);
            p.update(&acts, &clips, &mut anim, dt, &ev);
            seen.push(clips.get(anim.clip()).name.clone());
        }
        assert_eq!(seen.last().map(String::as_str), Some("Map Walk"), "{seen:?}");
        assert!(p.gait(&acts).is_some());
        // Steps without a turn clip ignore it.
        let lantern = acts.id("lantern_patrol").expect("act");
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, lantern, 3);
        p.turned(&acts, &clips, &mut anim);
        assert_eq!(clips.get(anim.clip()).name, "Lantern Idle");
    }

    #[test]
    fn fidgets_end_on_idle_transitions() {
        let (clips, acts) = libs();
        for (act, last) in [("look_up", "LookUpToIdle"), ("look_down", "LookDownToIdle"), ("pick_up_trinket", "GetUpToIdle"), ("ascend", "Collect StandToIdle")] {
            let a = acts.get(acts.id(act).expect("act"));
            assert_eq!(clips.get(a.steps.last().expect("steps").clip).name, last);
        }
    }

    /// Ticks `p` until `until` holds (or `max_s` passes); returns the clips seen.
    fn tick_until(clips: &ClipLibrary, acts: &ActLibrary, anim: &mut Animator, p: &mut ActPlayer, max_s: f32, until: impl Fn(&ActPlayer, ActStatus) -> bool) -> Vec<String> {
        let (dt, mut ev, mut t) = (1.0 / 120.0, Vec::new(), 0.0);
        let mut seen = vec![clips.get(anim.clip()).name.clone()];
        while t < max_s {
            ev.clear();
            anim.update(clips, dt, &mut ev);
            let status = p.update(acts, clips, anim, dt, &ev);
            let n = &clips.get(anim.clip()).name;
            if seen.last() != Some(n) {
                seen.push(n.clone());
            }
            if until(p, status) {
                break;
            }
            t += dt;
        }
        assert!(t < max_s, "timed out: {seen:?}");
        seen
    }

    #[test]
    fn repeated_pose_acts_fuse_instead_of_restarting() {
        let (clips, acts) = libs();
        let down = acts.id("look_down").expect("act");
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, down, 5).allow_linger(3.0);
        // Seven looks down in a row: one start, one ending.
        for round in 0..7 {
            let seen = tick_until(&clips, &acts, &mut anim, &mut p, 6.0, |p, _| p.is_lingering());
            assert_eq!(seen.last().map(String::as_str), Some("LookDown"), "round {round}: {seen:?}");
            assert!(!seen.iter().any(|c| c == "LookDownToIdle" || (c == "Idle" && round > 0)), "round {round}: {seen:?}");
            assert!(p.fuse(&acts, &clips, &mut anim, down), "round {round}");
            assert!(!p.is_lingering());
            assert_eq!(clips.get(anim.clip()).name, "LookDown");
        }
        tick_until(&clips, &acts, &mut anim, &mut p, 6.0, |p, _| p.is_lingering());
        // Another pose does not fuse; the linger runs out into the ending.
        assert!(!p.fuse(&acts, &clips, &mut anim, acts.id("look_up").expect("act")));
        let seen = tick_until(&clips, &acts, &mut anim, &mut p, 4.0, |_, s| s == ActStatus::Done);
        assert_eq!(seen.last().map(String::as_str), Some("LookDownToIdle"), "{seen:?}");
    }

    #[test]
    fn released_linger_plays_the_ending_once() {
        let (clips, acts) = libs();
        let up = acts.id("look_up").expect("act");
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, up, 9).allow_linger(3.0);
        tick_until(&clips, &acts, &mut anim, &mut p, 6.0, |p, _| p.is_lingering());
        p.release(&acts, &clips, &mut anim);
        assert!(p.is_released() && !p.is_lingering());
        assert_eq!(clips.get(anim.clip()).name, "LookUpToIdle");
        assert!(!p.fuse(&acts, &clips, &mut anim, up), "only a lingering pose fuses");
        let seen = tick_until(&clips, &acts, &mut anim, &mut p, 3.0, |_, s| s == ActStatus::Done);
        assert_eq!(seen.iter().filter(|c| *c == "LookUpToIdle").count(), 1, "{seen:?}");
        // Without the opt-in (e.g. scripted acts) the act ends as before.
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, up, 9);
        let seen = tick_until(&clips, &acts, &mut anim, &mut p, 6.0, |p, s| {
            assert!(!p.is_lingering());
            s == ActStatus::Done
        });
        assert_eq!(seen.last().map(String::as_str), Some("LookUpToIdle"), "{seen:?}");
    }

    #[test]
    fn map_stroll_requests_walking() {
        let (clips, acts) = libs();
        let id = acts.id("map_stroll").expect("act");
        let mut anim = Animator::new(clips.id("Idle").expect("Idle"));
        let mut p = ActPlayer::start(&acts, &clips, &mut anim, id, 3);
        let mut ev = Vec::new();
        let mut walked = false;
        for _ in 0..120 * 20 {
            ev.clear();
            anim.update(&clips, 1.0 / 120.0, &mut ev);
            if let ActStatus::Running { gait: Some(Gait::Walk) } = p.update(&acts, &clips, &mut anim, 1.0 / 120.0, &ev) {
                walked = true;
                assert_eq!(clips.get(anim.clip()).name, "Map Walk");
            }
        }
        assert!(walked);
    }
}
