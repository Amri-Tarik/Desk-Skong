//! Acts: data-driven choreographies (sit → open map → scribble → close → get up) built
//! from Knight clips. The AI picks acts by mood tags; [`ActPlayer`] runs one by driving
//! an [`Animator`] and reporting any locomotion the step wants.
//!
//! Step semantics:
//! * looping clip — plays for `hold` seconds (default: one pass), or until `until` signal;
//! * one-shot clip — plays `repeat` times, then holds the last frame for `hold` seconds;
//! * `move` — the controller should walk with that gait while the step runs.
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::anim::{AnimEvent, Animator, ClipId, ClipLibrary};
use crate::assets::read_json;
use crate::controller::Gait;

#[derive(Debug, Deserialize)]
struct ActsFile {
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
    steps: Vec<StepJson>,
}

#[derive(Debug, Deserialize)]
struct StepJson {
    clip: String,
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
}

#[derive(Clone, Debug)]
pub struct Act {
    pub name: String,
    pub tags: Vec<String>,
    pub requires: Requirement,
    pub weight: f32,
    /// Seconds before the act may be picked again.
    pub cooldown: f32,
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
        for a in raw.acts {
            if a.steps.is_empty() {
                bail!("act '{}' has no steps", a.name);
            }
            let steps = a
                .steps
                .into_iter()
                .map(|s| {
                    let clip = clips.id(&s.clip).with_context(|| format!("act '{}': unknown clip '{}'", a.name, s.clip))?;
                    let gait = match s.movement.as_deref() {
                        None => None,
                        Some("walk") => Some(Gait::Walk),
                        Some("run") => Some(Gait::Run),
                        Some("sprint") => Some(Gait::Sprint),
                        Some(m) => bail!("act '{}': unknown move '{m}'", a.name),
                    };
                    let turn = match s.turn.as_deref() {
                        None => None,
                        Some(t) => Some(clips.id(t).with_context(|| format!("act '{}': unknown turn clip '{t}'", a.name))?),
                    };
                    let repeat = s.repeat.map_or((1, 1), |[lo, hi]| (lo.max(1), hi.max(lo.max(1))));
                    Ok(Step { clip, hold: s.hold.map(|[lo, hi]| (lo, hi.max(lo))), repeat, until: s.until, gait, interruptible: s.interruptible, turn })
                })
                .collect::<Result<Vec<_>>>()?;
            let id = ActId(lib.acts.len() as u16);
            if lib.by_name.insert(a.name.clone(), id).is_some() {
                bail!("duplicate act '{}'", a.name);
            }
            lib.acts.push(Act { name: a.name, tags: a.tags, requires: a.requires, weight: a.weight, cooldown: a.cooldown, steps });
        }
        Ok(())
    }

    pub fn id(&self, name: &str) -> Option<ActId> {
        self.by_name.get(name).copied()
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
}

impl ActPlayer {
    pub fn start(acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, act: ActId, seed: u32) -> Self {
        let mut p = Self { act, step: 0, phase: Phase::Playing, timer: 0.0, repeats_left: 1, done: false, rng: seed | 1, turning: false };
        p.enter_step(acts, clips, anim);
        p
    }

    pub fn act(&self) -> ActId {
        self.act
    }

    pub fn step_index(&self) -> usize {
        self.step
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Gait the current step wants the character to walk with, if any.
    pub fn gait(&self, acts: &ActLibrary) -> Option<Gait> {
        if self.done { None } else { acts.get(self.act).steps[self.step].gait }
    }

    /// Whether the current step may be cut short.
    pub fn interruptible(&self, acts: &ActLibrary) -> bool {
        self.done || acts.get(self.act).steps[self.step].interruptible
    }

    /// External signal (e.g. "woken"). Ends the current step if it waits on it.
    pub fn signal(&mut self, acts: &ActLibrary, clips: &ClipLibrary, anim: &mut Animator, name: &str) {
        if self.done {
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
        let step = &acts.get(self.act).steps[self.step];
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
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn sample(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.next_rand()
    }

    fn sample_u(&mut self, (lo, hi): (u32, u32)) -> u32 {
        lo + ((hi - lo + 1) as f32 * self.next_rand()) as u32 % (hi - lo + 1)
    }
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
        assert_eq!(clips.get(anim.clip()).name, "Sitting Asleep");
        p.signal(&acts, &clips, &mut anim, "woken");
        assert_eq!(clips.get(anim.clip()).name, "Wake To Sit");
        assert!(!p.interruptible(&acts));
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
