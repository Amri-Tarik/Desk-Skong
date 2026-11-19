//! One live visitor: footing, animation and the arrive → wait → talk → leave cycle.
use crate::ai::rng::Rng;
use crate::ai::{Visitor, VisitorState};
use crate::anim::{Animator, ClipId, ClipLibrary, WrapMode};
use crate::assets::SpriteKey;
use crate::desktop::WindowId;
use crate::geom::{RectF, Vec2};
use crate::props::ledge_below;
use crate::terrain::Terrain;

use super::{Arrival, Faces, Leave, NpcSet};

/// Where a visitor appears (computed by the app from terrain / nav / monitors).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Feet (or seat) point. Drop arrivals start this far above and fall onto the ledge.
    pub feet: Vec2,
    pub owner: Option<WindowId>,
    /// Walk arrivals: walk from `feet` to this x.
    pub walk_to: Option<f32>,
}

/// Things the app must render or know about.
#[derive(Clone, Debug, PartialEq)]
pub enum NpcEvent {
    /// Show this line in a speech bubble over the visitor for `secs`.
    Say { text: String, secs: f32 },
    /// The current line is over (close its bubble).
    LineDone,
    /// The conversation with the knight ended.
    ConversationOver,
    /// Open a dream portal here (portal departures).
    OpenPortal(Vec2),
    /// Fully gone: drop the visitor.
    Gone,
}

/// Coarse phase (for the brain and the app).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpcPhase {
    Arriving,
    Waiting,
    Talking,
    Leaving,
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Speaking,
    Ending,
    Gap,
}

#[derive(Clone, Debug)]
enum State {
    /// Fading in (and walking to `to_x` for walk arrivals).
    Enter { to_x: Option<f32> },
    /// Playing the landing clips after a drop.
    Landing { step: usize },
    Waiting { left: f32, hum_in: f32 },
    Talking { lines: Vec<(String, Option<String>)>, i: usize, t: f32, stage: Stage },
    /// After the conversation: a moment before leaving.
    Lingering { left: f32 },
    Exit { to_x: Option<f32> },
    Gone,
}

/// Context for one tick.
pub struct NpcEnv<'a> {
    pub terrain: &'a Terrain,
    /// Knight feet position.
    pub knight: Vec2,
    /// Pixels per game unit (render scale).
    pub ppu: f32,
    /// Area of the monitor the visitor is on (walk departures head for its nearest edge).
    pub area: RectF,
}

pub struct Npc {
    pub id: u32,
    pub def: usize,
    /// Feet (or seat) point.
    pub pos: Vec2,
    pub owner: Option<WindowId>,
    /// Offset from the owner window's top-left (riding it like a bench).
    rel: Vec2,
    /// Falling speed (px/s) while not supported.
    vy: Option<f32>,
    /// Looking right (side-view art is mirrored then).
    pub looking_right: bool,
    /// Opacity 0..1 (fade in / out).
    pub alpha: f32,
    pub greeted: bool,
    anim: Animator,
    state: State,
    rng: Rng,
    /// The knight is to the right (talk clips by side).
    knight_right: bool,
}

const FADE_IN: f32 = 0.6;
const FADE_OUT: f32 = 0.9;
const GAP: f32 = 0.5;

impl Npc {
    pub fn new(id: u32, def: usize, set: &NpcSet, at: Placement, seed: u32, ppu: f32) -> Self {
        let d = &set.defs[def];
        let c = &set.clips[def];
        let mut rng = Rng::new(seed);
        let (pos, vy, clip) = match d.arrive {
            Arrival::Drop => (at.feet - Vec2::new(0.0, 2.5 * ppu), Some(0.0), c.fall.unwrap_or(c.idle)),
            Arrival::Walk => (at.feet, None, c.walk.unwrap_or(c.idle)),
            Arrival::Perch | Arrival::Portal => (at.feet, None, c.idle),
        };
        let mut anim = Animator::new(clip);
        anim.play(set.lib(def), clip, true);
        let looking_right = at.walk_to.is_some_and(|x| x > at.feet.x);
        let _ = rng.next_u32();
        Self {
            id,
            def,
            pos,
            owner: at.owner,
            rel: Vec2::ZERO,
            vy,
            looking_right,
            alpha: if d.arrive == Arrival::Drop { 1.0 } else { 0.0 },
            greeted: false,
            anim,
            state: State::Enter { to_x: at.walk_to },
            rng,
            knight_right: false,
        }
    }

    pub fn phase(&self) -> NpcPhase {
        match self.state {
            State::Enter { .. } | State::Landing { .. } => NpcPhase::Arriving,
            State::Waiting { .. } | State::Lingering { .. } => NpcPhase::Waiting,
            State::Talking { .. } => NpcPhase::Talking,
            State::Exit { .. } => NpcPhase::Leaving,
            State::Gone => NpcPhase::Gone,
        }
    }

    /// Current frame.
    pub fn sprite(&self, set: &NpcSet) -> Option<SpriteKey> {
        self.anim.current(set.lib(self.def))
    }

    pub fn clip(&self) -> ClipId {
        self.anim.clip()
    }

    /// Mirror the (left-facing) art this frame.
    pub fn flipped(&self, set: &NpcSet) -> bool {
        match set.defs[self.def].faces {
            Faces::Left => self.looking_right,
            Faces::Right => !self.looking_right,
            Faces::Front => false,
        }
    }

    /// Can a conversation start now?
    pub fn can_talk(&self) -> bool {
        !self.greeted && matches!(self.state, State::Waiting { .. })
    }

    /// Start talking (the knight arrived next to us). `lines` come from
    /// [`super::TalkContext::pick`]. Returns false if not possible right now.
    pub fn begin_talk(&mut self, lines: Vec<(String, Option<String>)>, knight_x: f32, set: &NpcSet, events: &mut Vec<NpcEvent>) -> bool {
        if !self.can_talk() || lines.is_empty() {
            return false;
        }
        self.knight_right = knight_x > self.pos.x;
        self.looking_right = self.knight_right;
        self.state = State::Talking { lines, i: 0, t: 0.0, stage: Stage::Speaking };
        self.start_line(set, events);
        true
    }

    /// Leave now (e.g. nobody can reach us, or the app shuts visits off).
    pub fn leave(&mut self, set: &NpcSet, env: &NpcEnv, events: &mut Vec<NpcEvent>) {
        if !matches!(self.state, State::Exit { .. } | State::Gone) {
            if matches!(self.state, State::Talking { .. }) {
                events.push(NpcEvent::LineDone);
            }
            self.state = self.start_exit(set, env, events);
        }
    }

    /// The brain's view of this visitor.
    pub fn percept(&self, set: &NpcSet, ppu: f32) -> Visitor {
        let d = &set.defs[self.def];
        let (state, line, react) = match &self.state {
            State::Talking { lines, i, stage, .. } => {
                let react = (*stage == Stage::Speaking).then(|| lines.get(*i).and_then(|l| l.1.clone())).flatten();
                (VisitorState::Talking, Some(*i), react)
            }
            _ => {
                let s = match self.phase() {
                    NpcPhase::Arriving => VisitorState::Arriving,
                    NpcPhase::Waiting => VisitorState::Waiting,
                    _ => VisitorState::Leaving,
                };
                (s, None, None)
            }
        };
        Visitor {
            id: self.id,
            name: d.name.clone(),
            pos: self.pos,
            state,
            line,
            react,
            greet_react: d.greet_react.clone(),
            greeted: self.greeted,
            excitement: d.excitement,
            stand_off: d.stand_off * ppu,
        }
    }

    pub fn tick(&mut self, dt: f32, set: &NpcSet, env: &NpcEnv, events: &mut Vec<NpcEvent>) {
        let d = &set.defs[self.def];
        let c = &set.clips[self.def];
        let lib = set.lib(self.def);
        self.follow(env, dt);
        let mut anim_events = Vec::new();
        self.anim.update(lib, dt, &mut anim_events);

        // Face the knight when he's around (side-view art only matters).
        let near_knight = (env.knight.x - self.pos.x).abs() < 600.0 && (env.knight.y - self.pos.y).abs() < 300.0;
        let state = std::mem::replace(&mut self.state, State::Gone);
        self.state = match state {
            State::Enter { to_x } => {
                self.alpha = (self.alpha + dt / FADE_IN).min(1.0);
                if self.vy.is_some() {
                    State::Enter { to_x } // still falling
                } else if d.arrive == Arrival::Drop {
                    match c.land.first() {
                        Some(&clip) => {
                            self.anim.play(lib, clip, true);
                            State::Landing { step: 0 }
                        }
                        None => self.start_wait(set),
                    }
                } else if let Some(x) = to_x {
                    let step = d.walk_speed * env.ppu * dt;
                    self.looking_right = x > self.pos.x;
                    if (x - self.pos.x).abs() <= step {
                        self.pos.x = x;
                        self.resync_rel(env);
                        self.start_wait(set)
                    } else {
                        self.pos.x += step * (x - self.pos.x).signum();
                        self.resync_rel(env);
                        State::Enter { to_x }
                    }
                } else if self.alpha >= 1.0 {
                    self.start_wait(set)
                } else {
                    State::Enter { to_x }
                }
            }
            State::Landing { step } => {
                if self.anim.is_finished() || once_secs(lib, self.anim.clip()) <= 0.0 {
                    match c.land.get(step + 1) {
                        Some(&clip) => {
                            self.anim.play(lib, clip, true);
                            State::Landing { step: step + 1 }
                        }
                        None => self.start_wait(set),
                    }
                } else {
                    State::Landing { step }
                }
            }
            State::Waiting { left, hum_in } => {
                if near_knight {
                    self.looking_right = env.knight.x > self.pos.x;
                }
                let (left, mut hum_in) = (left - dt, hum_in - dt);
                if left <= 0.0 {
                    self.start_exit(set, env, events)
                } else {
                    if hum_in <= 0.0 && !d.idle_lines.is_empty() {
                        let text = d.idle_lines[self.rng.next_u32() as usize % d.idle_lines.len()].clone();
                        let secs = crate::bubble::read_secs(&text).min(3.5);
                        events.push(NpcEvent::Say { text, secs });
                        hum_in = self.rng.range(d.idle_every[0], d.idle_every[1]) + secs;
                    }
                    State::Waiting { left, hum_in }
                }
            }
            State::Talking { lines, i, t, stage } => self.talk_tick(lines, i, t + dt, stage, set, events),
            State::Lingering { left } => {
                if near_knight {
                    self.looking_right = env.knight.x > self.pos.x;
                }
                if left - dt <= 0.0 { self.start_exit(set, env, events) } else { State::Lingering { left: left - dt } }
            }
            State::Exit { to_x } => match to_x {
                Some(x) => {
                    let step = d.walk_speed * env.ppu * dt;
                    self.looking_right = x > self.pos.x;
                    self.pos.x += step * (x - self.pos.x).signum();
                    self.resync_rel(env);
                    if !env.area.contains(Vec2::new(self.pos.x, env.area.top + 1.0)) {
                        self.alpha -= dt / FADE_OUT;
                    }
                    if (x - self.pos.x).abs() <= step || self.alpha <= 0.0 { self.gone(events) } else { State::Exit { to_x } }
                }
                None => {
                    self.alpha -= dt / FADE_OUT;
                    if self.alpha <= 0.0 { self.gone(events) } else { State::Exit { to_x } }
                }
            },
            State::Gone => State::Gone,
        };
    }

    fn gone(&mut self, events: &mut Vec<NpcEvent>) -> State {
        self.alpha = 0.0;
        events.push(NpcEvent::Gone);
        State::Gone
    }

    fn start_wait(&mut self, set: &NpcSet) -> State {
        let d = &set.defs[self.def];
        self.anim.play(set.lib(self.def), set.clips[self.def].idle, false);
        self.alpha = 1.0;
        let hum_in = self.rng.range(1.5, 4.0);
        State::Waiting { left: self.rng.range(d.stay[0], d.stay[1]), hum_in }
    }

    fn start_exit(&mut self, set: &NpcSet, env: &NpcEnv, events: &mut Vec<NpcEvent>) -> State {
        let d = &set.defs[self.def];
        let c = &set.clips[self.def];
        let lib = set.lib(self.def);
        match d.leave {
            Leave::Walk => {
                let left_edge = self.pos.x - env.area.left <= env.area.right - self.pos.x;
                let margin = 3.0 * env.ppu;
                let to = if left_edge { env.area.left - margin } else { env.area.right + margin };
                self.anim.play(lib, c.walk.unwrap_or(c.idle), true);
                self.looking_right = to > self.pos.x;
                State::Exit { to_x: Some(to) }
            }
            Leave::Fade => {
                self.anim.play(lib, c.idle, false);
                State::Exit { to_x: None }
            }
            Leave::Portal => {
                self.anim.play(lib, c.idle, false);
                events.push(NpcEvent::OpenPortal(self.pos));
                State::Exit { to_x: None }
            }
        }
    }

    fn talk_clips(&self, set: &NpcSet) -> (ClipId, Option<ClipId>) {
        let c = &set.clips[self.def];
        if self.knight_right {
            (c.talk_right.unwrap_or(c.talk), c.talk_end_right.or(c.talk_end))
        } else {
            (c.talk, c.talk_end)
        }
    }

    fn start_line(&mut self, set: &NpcSet, events: &mut Vec<NpcEvent>) {
        let State::Talking { lines, i, .. } = &self.state else { return };
        let Some((text, _)) = lines.get(*i) else { return };
        let text = text.clone();
        let (talk, _) = self.talk_clips(set);
        self.anim.play(set.lib(self.def), talk, true);
        events.push(NpcEvent::Say { secs: crate::bubble::read_secs(&text), text });
    }

    fn talk_tick(&mut self, lines: Vec<(String, Option<String>)>, i: usize, t: f32, stage: Stage, set: &NpcSet, events: &mut Vec<NpcEvent>) -> State {
        let lib = set.lib(self.def);
        let (_, end) = self.talk_clips(set);
        match stage {
            Stage::Speaking => {
                let secs = lines.get(i).map_or(0.0, |l| crate::bubble::read_secs(&l.0));
                if t < secs {
                    return State::Talking { lines, i, t, stage };
                }
                events.push(NpcEvent::LineDone);
                match end {
                    Some(e) => {
                        self.anim.play(lib, e, true);
                        State::Talking { lines, i, t: 0.0, stage: Stage::Ending }
                    }
                    None => {
                        self.anim.play(lib, set.clips[self.def].idle, false);
                        State::Talking { lines, i, t: 0.0, stage: Stage::Gap }
                    }
                }
            }
            Stage::Ending => {
                if t < once_secs(lib, self.anim.clip()).min(1.6) && !self.anim.is_finished() {
                    return State::Talking { lines, i, t, stage };
                }
                self.anim.play(lib, set.clips[self.def].idle, false);
                State::Talking { lines, i, t: 0.0, stage: Stage::Gap }
            }
            Stage::Gap => {
                if t < GAP {
                    return State::Talking { lines, i, t, stage };
                }
                if i + 1 < lines.len() {
                    self.state = State::Talking { lines, i: i + 1, t: 0.0, stage: Stage::Speaking };
                    self.start_line(set, events);
                    std::mem::replace(&mut self.state, State::Gone)
                } else {
                    self.greeted = true;
                    events.push(NpcEvent::ConversationOver);
                    State::Lingering { left: self.rng.range(3.0, 6.0) }
                }
            }
        }
    }

    /// Ride the owner window; fall onto the next ledge when unsupported.
    fn follow(&mut self, env: &NpcEnv, dt: f32) {
        if let Some(r) = self.owner.and_then(|id| env.terrain.window_rect(id))
            && self.vy.is_none()
        {
            self.pos = Vec2::new(r.left, r.top) + self.rel;
        }
        let supported = env.terrain.ledges.iter().any(|l| (l.y - self.pos.y).abs() < 1.5 && l.x0 - 1.0 <= self.pos.x && self.pos.x <= l.x1 + 1.0);
        let walking_off = matches!(self.state, State::Exit { to_x: Some(_) }) && !env.area.contains(Vec2::new(self.pos.x, env.area.top + 1.0));
        if supported || walking_off || matches!(self.state, State::Gone) {
            if self.vy.take().is_some() {
                self.land(env);
            }
            return;
        }
        // Unsupported: fall (game-like gravity, capped).
        let g = 60.0 * env.ppu;
        let vy = (self.vy.unwrap_or(0.0) + g * dt).min(30.0 * env.ppu);
        let target = ledge_below(env.terrain, self.pos).map(|(y, _)| y);
        let ny = self.pos.y + vy * dt;
        match target {
            Some(y) if ny >= y => {
                self.pos.y = y;
                self.vy = None;
                self.land(env);
            }
            Some(_) => {
                self.pos.y = ny;
                self.vy = Some(vy);
            }
            None => {
                // Nothing below: rest on the bottom of the play area.
                self.pos.y = env.terrain.bounds.bottom;
                self.vy = None;
                self.owner = None;
            }
        }
    }

    fn land(&mut self, env: &NpcEnv) {
        self.owner = env
            .terrain
            .ledges
            .iter()
            .find(|l| (l.y - self.pos.y).abs() < 1.5 && l.x0 - 1.0 <= self.pos.x && self.pos.x <= l.x1 + 1.0)
            .and_then(|l| l.owner);
        self.resync_rel(env);
    }

    fn resync_rel(&mut self, env: &NpcEnv) {
        if let Some(r) = self.owner.and_then(|id| env.terrain.window_rect(id)) {
            self.rel = self.pos - Vec2::new(r.left, r.top);
        }
    }

    /// Call once after creation so a visitor placed on a window rides it.
    pub fn attach(&mut self, env: &NpcEnv) {
        self.resync_rel(env);
    }
}

/// Seconds a clip plays before it loops (LoopSection: its intro) or ends.
pub fn once_secs(lib: &ClipLibrary, clip: ClipId) -> f32 {
    let c = lib.get(clip);
    let frames = match c.wrap {
        WrapMode::LoopSection if c.loop_start > 0 => c.loop_start,
        WrapMode::Single | WrapMode::RandomFrame => 1,
        _ => c.frames.len(),
    };
    frames as f32 / c.fps.max(0.001)
}
