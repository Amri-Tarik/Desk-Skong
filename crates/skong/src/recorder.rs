//! Flight recorder: the last ~20 s of the knight's movement, one row per tick, kept in
//! memory and written to a text file in the logs folder when asked (panel → "Save trace")
//! or when a move/stop flicker is detected (input toggling on and off many times a second
//! while grounded: the "walks very slowly" bug).
//!
//! Each row is what decided the step: the brain's behaviour and reason, the motion kind
//! the brain asked for, the input it produced, the input that reached the controller
//! (after acts, dives and manual mode), and the controller's state, position and speed.
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use skong_core::controller::{MoveInput, MoveState, Side};
use skong_core::geom::Vec2;

/// Seconds kept.
const KEEP_SECS: f64 = 20.0;
/// Flicker detection window (s).
const WINDOW: f64 = 1.0;
/// Grounded input on/off toggles within the window that count as a flicker.
const TOGGLES: usize = 6;
/// Behaviour switches within two windows that count as dithering.
const SWITCHES: usize = 4;
/// Facing flips within the window that count as a flicker.
const FLIPS: usize = 6;
/// At most one automatic save this often (s).
const AUTO_COOLDOWN: f64 = 90.0;
/// Keep recording this long after a trigger before saving, so the file shows how it went on.
const AFTER_SECS: f64 = 3.0;

/// Where the brain's motion came from this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Hold,
    Navigate,
    Direct,
    Manual,
    Forced,
}

impl Source {
    fn tag(self) -> &'static str {
        match self {
            Source::Hold => "hold",
            Source::Navigate => "nav",
            Source::Direct => "direct",
            Source::Manual => "manual",
            Source::Forced => "forced",
        }
    }
}

/// What the brain said this tick (set from `think`).
#[derive(Clone, Debug, Default)]
pub struct Thought {
    pub behaviour: &'static str,
    pub reason: Arc<str>,
    pub source: Source,
    /// The input the brain's motion produced, before acts / dives / overrides.
    pub x: f32,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub t: f64,
    pub thought: Thought,
    pub input: MoveInput,
    pub state: MoveState,
    pub grounded: bool,
    pub pos: Vec2,
    pub vel: Vec2,
    pub facing: Side,
    pub act: Option<String>,
    pub following: bool,
    pub held: bool,
    pub note: Option<String>,
}

#[derive(Default)]
pub struct Recorder {
    frames: VecDeque<Frame>,
    t: f64,
    thought: Thought,
    /// Notes for the next frame (gauntlet stage changes, switches, …).
    note: Option<String>,
    last_auto: Option<f64>,
    /// A trigger is waiting: save at this time, with this cause.
    pending: Option<(f64, String)>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The brain's side of this tick.
    pub fn think(&mut self, behaviour: &'static str, reason: &str, source: Source, input: &MoveInput) {
        let reason = if &*self.thought.reason == reason { self.thought.reason.clone() } else { Arc::from(reason) };
        self.thought = Thought { behaviour, reason, source, x: input.x };
    }

    /// Input that didn't come from the brain (manual keys, go-to-cursor).
    pub fn bypass(&mut self, source: Source, input: &MoveInput) {
        self.thought = Thought { behaviour: "-", reason: self.thought.reason.clone(), source, x: input.x };
    }

    /// A short note shown on the next row.
    pub fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        match self.note.as_mut() {
            Some(n) => {
                n.push_str("; ");
                n.push_str(&text);
            }
            None => self.note = Some(text),
        }
    }

    /// One row, after the controller stepped. Returns the path of a trace saved because a
    /// flicker was detected.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, dt: f32, input: &MoveInput, state: MoveState, grounded: bool, pos: Vec2, vel: Vec2, facing: Side, act: Option<String>, following: bool, held: bool) -> Option<PathBuf> {
        self.t += f64::from(dt);
        let frame = Frame {
            t: self.t,
            thought: self.thought.clone(),
            input: *input,
            state,
            grounded,
            pos,
            vel,
            facing,
            act,
            following,
            held,
            note: self.note.take(),
        };
        self.frames.push_back(frame);
        while self.frames.front().is_some_and(|f| self.t - f.t > KEEP_SECS) {
            self.frames.pop_front();
        }
        if let Some((at, _)) = &self.pending {
            if self.t >= *at {
                let (_, cause) = self.pending.take().unwrap_or_default();
                return self.save(&cause);
            }
            return None;
        }
        let cooled = self.last_auto.is_none_or(|t| self.t - t > AUTO_COOLDOWN);
        if cooled && let Some(cause) = detect(&self.frames, self.t) {
            log::warn!("flight recorder: {cause}; saving a trace in {AFTER_SECS} s");
            self.last_auto = Some(self.t);
            self.note(format!("TRIGGER: {cause}"));
            self.pending = Some((self.t + AFTER_SECS, cause));
        }
        None
    }

    /// Writes the buffer to the logs folder (or the temp folder without one).
    pub fn save(&self, cause: &str) -> Option<PathBuf> {
        let dir = crate::logfile::dir().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let _ = std::fs::create_dir_all(&dir);
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let crash = crate::logfile::crash_name(unix);
        let path = dir.join(format!("trace-{}", crash.split_once('-').map_or(crash.as_str(), |(_, rest)| rest)));
        match std::fs::write(&path, render(&self.frames, cause)) {
            Ok(()) => {
                log::warn!("flight recorder: saved {} ({} ticks)", path.display(), self.frames.len());
                crate::logfile::prune(&dir, "trace-", 10);
                Some(path)
            }
            Err(e) => {
                log::warn!("flight recorder: couldn't write {}: {e}", path.display());
                None
            }
        }
    }
}

fn moving(f: &Frame) -> bool {
    f.input.x != 0.0
}

/// A move/stop flicker, facing flicker or behaviour dithering in the recent past.
fn detect(frames: &VecDeque<Frame>, now: f64) -> Option<String> {
    let recent: Vec<&Frame> = frames.iter().rev().take_while(|f| now - f.t <= WINDOW).collect();
    if recent.len() < 10 || recent.iter().any(|f| f.held) {
        return None;
    }
    let toggles = recent.windows(2).filter(|w| w[0].grounded && w[1].grounded && moving(w[0]) != moving(w[1])).count();
    if toggles >= TOGGLES {
        return Some(format!("move/stop flicker: input toggled {toggles}x in {WINDOW} s while grounded ({})", recent[0].thought.behaviour));
    }
    let flips = recent.windows(2).filter(|w| w[0].facing != w[1].facing).count();
    if flips >= FLIPS {
        return Some(format!("facing flicker: turned {flips}x in {WINDOW} s ({})", recent[0].thought.behaviour));
    }
    let longer: Vec<&Frame> = frames.iter().rev().take_while(|f| now - f.t <= 2.0 * WINDOW).collect();
    let switches = longer.windows(2).filter(|w| w[0].thought.behaviour != w[1].thought.behaviour).count();
    if switches >= SWITCHES {
        return Some(format!("dithering: behaviour switched {switches}x in {} s", 2.0 * WINDOW));
    }
    None
}

fn render(frames: &VecDeque<Frame>, cause: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Desk-Skong flight recorder: {cause}");
    let _ = writeln!(s, "{} ticks, newest last. x = brain input -> input reaching the controller. Reasons are printed when they change.", frames.len());
    let _ = writeln!(s);
    let _ = writeln!(s, "{:>8} {:<12} {:<6} {:>15} {:<7} {:<6} {:<26} {:>12} {:>7} {:<1} {:<3} act / note", "t", "behaviour", "src", "x brain->ctl", "gait", "flags", "state", "pos", "vel.x", "f", "nav");
    let mut reason: Option<&Arc<str>> = None;
    for f in frames {
        if reason.is_none_or(|r| !Arc::ptr_eq(r, &f.thought.reason) && **r != *f.thought.reason) {
            let _ = writeln!(s, "{:>8} reason: {}", "", f.thought.reason);
            reason = Some(&f.thought.reason);
        }
        let i = &f.input;
        let mut flags = String::new();
        for (on, c) in [(i.jump_held, 'J'), (i.jump_pressed, 'j'), (i.dash_pressed, 'd'), (i.down, 'v'), (i.up, '^'), (i.dive_pressed, 'D'), (i.superdash_held, 'S'), (f.held, 'H')] {
            if on {
                flags.push(c);
            }
        }
        let state = format!("{:?}", f.state);
        let state: String = state.chars().take(26).collect();
        let gait = format!("{:?}", f.input.gait);
        let mut tail = f.act.clone().map(|a| format!("act {a}")).unwrap_or_default();
        if let Some(n) = &f.note {
            if !tail.is_empty() {
                tail.push_str(" | ");
            }
            tail.push_str(n);
        }
        let _ = writeln!(
            s,
            "{:>8.3} {:<12} {:<6} {:>6.2} -> {:>5.2} {:<7} {:<6} {:<26} {:>5.0},{:>6.0} {:>7.1} {:<1} {:<3} {}",
            f.t,
            f.thought.behaviour,
            f.thought.source.tag(),
            f.thought.x,
            i.x,
            gait,
            flags,
            state,
            f.pos.x,
            f.pos.y,
            f.vel.x,
            if f.facing == Side::Right { 'R' } else { 'L' },
            if f.following { "yes" } else { "" },
            tail
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rec: &mut Recorder, ticks: usize, x: impl Fn(usize) -> f32) {
        for k in 0..ticks {
            let input = MoveInput { x: x(k), ..MoveInput::default() };
            rec.think("gauntlet", "readying", Source::Navigate, &input);
            let _ = rec.frame(1.0 / 60.0, &input, MoveState::Grounded, true, Vec2::ZERO, Vec2::ZERO, Side::Right, None, true, false);
        }
    }

    #[test]
    fn steady_walking_is_not_a_flicker() {
        let mut rec = Recorder::new();
        run(&mut rec, 300, |_| 1.0);
        assert!(rec.pending.is_none());
        assert!(detect(&rec.frames, rec.t).is_none());
    }

    #[test]
    fn move_stop_every_few_ticks_is_caught() {
        let mut rec = Recorder::new();
        run(&mut rec, 120, |k| if (k / 3) % 2 == 0 { 1.0 } else { 0.0 });
        let cause = rec.pending.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
        assert!(cause.contains("move/stop flicker"), "{cause}");
    }

    #[test]
    fn only_the_last_twenty_seconds_are_kept_and_render_lists_them() {
        let mut rec = Recorder::new();
        run(&mut rec, 60 * 30, |_| 1.0);
        assert!(rec.frames.len() <= 60 * 20 + 1);
        let text = render(&rec.frames, "test");
        assert!(text.contains("reason: readying"));
        assert_eq!(text.matches("reason:").count(), 1, "the reason prints once while unchanged");
    }
}
