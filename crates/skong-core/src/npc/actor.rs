//! One live visitor: footing, animation and the arrive → wait → talk (→ teach) → leave
//! cycle.
use crate::ai::rng::Rng;
use crate::ai::{LessonCue, LessonStage, Visitor, VisitorState};
use crate::anim::{Animator, ClipId, ClipLibrary, WrapMode};
use crate::assets::SpriteKey;
use crate::desktop::WindowId;
use crate::geom::{RectF, Vec2};
use crate::props::ledge_below;
use crate::terrain::Terrain;

use super::lesson::Lesson;
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
    /// The lesson after the conversation ended (`finished`: every drill was taught).
    LessonOver { finished: bool },
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
    Teaching,
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
    /// The lesson after the conversation.
    Teaching(Box<Teach>),
    /// After the conversation: a moment before leaving.
    Lingering { left: f32 },
    /// `hold`: seconds of the leave clip still to play before moving / fading.
    Exit { to_x: Option<f32>, hold: f32 },
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TeachStage {
    /// The lesson's opening demo (drawing the nail), step by step.
    Open { step: usize },
    /// Announcing the drill.
    Instruct,
    /// Showing how it's done, step by step.
    Demo { step: usize },
    /// Watching the knight do it.
    Practice,
    /// Praise or a correction.
    Feedback,
    /// A breath before the next drill.
    Gap,
    /// The last word.
    Closing,
}

/// A lesson in progress.
#[derive(Clone, Debug)]
struct Teach {
    lesson: Lesson,
    /// Current drill.
    i: usize,
    stage: TeachStage,
    t: f32,
    /// Length of the current stage (line reading time, demo step, practice patience).
    len: f32,
    /// Drills in a row the knight never did.
    misses: u32,
    /// The knight's answer to the line being said.
    react: Option<String>,
    /// The demo being played (Open / Demo stages).
    demo: Vec<(ClipId, Option<f32>)>,
}

impl Teach {
    fn placeholder() -> Self {
        let lesson = Lesson { drills: vec![], praise: vec![], critique: vec![], closing: vec![], abandon: vec![], open_demo: None, practice_secs: 1.0 };
        Self { lesson, i: 0, stage: TeachStage::Gap, t: 0.0, len: 0.0, misses: 0, react: None, demo: vec![] }
    }

    /// Drill number (1-based) — what the knight reports back.
    fn serial(&self) -> u32 {
        self.i as u32 + 1
    }

    fn react(&self) -> Option<String> {
        matches!(self.stage, TeachStage::Feedback | TeachStage::Closing).then(|| self.react.clone()).flatten()
    }

    fn cue(&self) -> Option<LessonCue> {
        let stage = match self.stage {
            TeachStage::Open { .. } | TeachStage::Instruct | TeachStage::Demo { .. } => LessonStage::Instruct,
            TeachStage::Practice => LessonStage::Practice,
            TeachStage::Feedback | TeachStage::Gap => LessonStage::Feedback,
            TeachStage::Closing => LessonStage::Closing,
        };
        let drill = self.lesson.drills.get(self.i).map_or_else(String::new, |d| d.drill.clone());
        Some(LessonCue { serial: self.serial(), drill, stage })
    }
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
    /// A lesson to teach once the conversation is over.
    lesson: Option<Lesson>,
}

const FADE_IN: f32 = 0.4;
const FADE_OUT: f32 = 0.9;
const GAP: f32 = 0.5;
/// Pause between drills.
const DRILL_GAP: f32 = 0.6;
/// A lesson ends after this many drills in a row the knight never did.
const MAX_MISSES: u32 = 2;

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
            alpha: 0.0,
            greeted: false,
            anim,
            state: State::Enter { to_x: at.walk_to },
            rng,
            knight_right: false,
            lesson: None,
        }
    }

    pub fn phase(&self) -> NpcPhase {
        match self.state {
            State::Enter { .. } | State::Landing { .. } => NpcPhase::Arriving,
            State::Waiting { .. } | State::Lingering { .. } => NpcPhase::Waiting,
            State::Talking { .. } => NpcPhase::Talking,
            State::Teaching(_) => NpcPhase::Teaching,
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
        let d = &set.defs[self.def];
        let c = &set.clips[self.def];
        let clip = self.anim.clip();
        // Some talk clips are drawn facing the other way than the rest of the art.
        let talk = clip == c.talk || Some(clip) == c.talk_end;
        let base = match d.faces {
            Faces::Left => self.looking_right,
            Faces::Right => !self.looking_right,
            Faces::Front => false,
        };
        base != (talk && d.talk_mirrored && d.faces != Faces::Front)
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

    /// Teach this lesson once the conversation (not yet started) is over.
    pub fn set_lesson(&mut self, lesson: Lesson) {
        self.lesson = Some(lesson);
    }

    /// Leave now (e.g. nobody can reach us, or the app shuts visits off).
    pub fn leave(&mut self, set: &NpcSet, env: &NpcEnv, events: &mut Vec<NpcEvent>) {
        if !matches!(self.state, State::Exit { .. } | State::Gone) {
            if matches!(self.state, State::Talking { .. } | State::Teaching(_)) {
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
            State::Teaching(t) => (VisitorState::Teaching, None, t.react()),
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
            lesson: match &self.state {
                State::Teaching(t) => t.cue(),
                _ => None,
            },
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
            State::Teaching(teach) => self.teach_tick(teach, dt, set, env, events),
            State::Lingering { left } => {
                if near_knight {
                    self.looking_right = env.knight.x > self.pos.x;
                }
                if left - dt <= 0.0 { self.start_exit(set, env, events) } else { State::Lingering { left: left - dt } }
            }
            State::Exit { to_x, hold } if hold > 0.0 => {
                // The leave clip (a bow...) plays first.
                let hold = hold - dt;
                if hold <= 0.0 && to_x.is_some() {
                    self.anim.play(lib, c.walk.unwrap_or(c.idle), true);
                    self.looking_right = to_x.is_some_and(|x| x > self.pos.x);
                }
                State::Exit { to_x, hold }
            }
            State::Exit { to_x, .. } => match to_x {
                Some(x) => {
                    let step = d.walk_speed * env.ppu * dt;
                    self.looking_right = x > self.pos.x;
                    self.pos.x += step * (x - self.pos.x).signum();
                    self.resync_rel(env);
                    if !env.area.contains(Vec2::new(self.pos.x, env.area.top + 1.0)) {
                        self.alpha -= dt / FADE_OUT;
                    }
                    if (x - self.pos.x).abs() <= step || self.alpha <= 0.0 { self.gone(events) } else { State::Exit { to_x, hold: 0.0 } }
                }
                None => {
                    self.alpha -= dt / FADE_OUT;
                    if self.alpha <= 0.0 { self.gone(events) } else { State::Exit { to_x, hold: 0.0 } }
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
        // A leave clip (a bow) plays first, then they walk / fade.
        let hold = c.leave.map_or(0.0, |l| once_secs(lib, l).max(0.05));
        let rest = c.leave.unwrap_or(c.idle);
        match d.leave {
            Leave::Walk => {
                let left_edge = self.pos.x - env.area.left <= env.area.right - self.pos.x;
                let margin = 3.0 * env.ppu;
                let to = if left_edge { env.area.left - margin } else { env.area.right + margin };
                if hold <= 0.0 {
                    self.anim.play(lib, c.walk.unwrap_or(c.idle), true);
                    self.looking_right = to > self.pos.x;
                } else {
                    self.anim.play(lib, rest, true);
                }
                State::Exit { to_x: Some(to), hold }
            }
            Leave::Fade => {
                self.anim.play(lib, rest, c.leave.is_some());
                State::Exit { to_x: None, hold }
            }
            Leave::Portal => {
                self.anim.play(lib, rest, c.leave.is_some());
                events.push(NpcEvent::OpenPortal(self.pos));
                State::Exit { to_x: None, hold }
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
                    match self.lesson.take() {
                        Some(lesson) => self.start_lesson(lesson, set, events),
                        None => State::Lingering { left: self.rng.range(3.0, 6.0) },
                    }
                }
            }
        }
    }

    /// The knight finished drill `serial` (`clean`: every repetition done).
    pub fn drilled(&mut self, serial: u32, clean: bool, set: &NpcSet, events: &mut Vec<NpcEvent>) {
        let State::Teaching(t) = &mut self.state else { return };
        if t.stage != TeachStage::Practice || t.serial() != serial {
            return;
        }
        t.misses = 0;
        let mut t = std::mem::replace(t, Box::new(Teach::placeholder()));
        self.feedback(&mut t, clean, set, events);
        self.state = State::Teaching(t);
    }

    /// Cut the lesson short (the knight was called away, the user needs the screen): a
    /// short parting word, then leave as usual.
    pub fn end_lesson(&mut self, events: &mut Vec<NpcEvent>) {
        let State::Teaching(t) = &self.state else { return };
        let line = (!t.lesson.abandon.is_empty()).then(|| t.lesson.abandon[self.rng.next_u32() as usize % t.lesson.abandon.len()].clone());
        events.push(NpcEvent::LineDone);
        events.push(NpcEvent::LessonOver { finished: false });
        let mut left = self.rng.range(1.5, 3.0);
        if let Some(text) = line {
            let secs = crate::bubble::read_secs(&text);
            left += secs;
            events.push(NpcEvent::Say { text, secs });
        }
        self.state = State::Lingering { left };
    }

    fn start_lesson(&mut self, lesson: Lesson, set: &NpcSet, events: &mut Vec<NpcEvent>) -> State {
        let mut t = Box::new(Teach { lesson, i: 0, stage: TeachStage::Instruct, t: 0.0, len: 0.0, misses: 0, react: None, demo: Vec::new() });
        let open = t.lesson.open_demo.as_ref().and_then(|n| set.clips[self.def].demos.get(n)).cloned().unwrap_or_default();
        if open.is_empty() {
            self.instruct(&mut t, set, events);
        } else {
            t.demo = open;
            self.demo_step(&mut t, TeachStage::Open { step: 0 }, set);
        }
        State::Teaching(t)
    }

    fn stance(&self, set: &NpcSet) -> ClipId {
        let c = &set.clips[self.def];
        c.stance.unwrap_or(c.idle)
    }

    /// Say a line in the stance (or the given clip) for its reading time.
    fn teach_say(&mut self, t: &mut Teach, stage: TeachStage, text: String, react: Option<String>, set: &NpcSet, events: &mut Vec<NpcEvent>) {
        self.anim.play(set.lib(self.def), self.stance(set), false);
        t.stage = stage;
        t.t = 0.0;
        t.len = crate::bubble::read_secs(&text);
        t.react = react;
        events.push(NpcEvent::Say { secs: t.len, text });
    }

    fn instruct(&mut self, t: &mut Teach, set: &NpcSet, events: &mut Vec<NpcEvent>) {
        let text = t.lesson.drills[t.i].say.clone();
        self.teach_say(t, TeachStage::Instruct, text, None, set, events);
    }

    /// Play step `stage` of `t.demo` (Open / Demo stages).
    fn demo_step(&mut self, t: &mut Teach, stage: TeachStage, set: &NpcSet) {
        let (TeachStage::Open { step } | TeachStage::Demo { step }) = stage else { return };
        let Some(&(clip, secs)) = t.demo.get(step) else { return };
        let lib = set.lib(self.def);
        self.anim.play(lib, clip, true);
        t.stage = stage;
        t.t = 0.0;
        t.len = secs.unwrap_or_else(|| once_secs(lib, clip)).max(0.05);
        t.react = None;
    }

    fn practice(&mut self, t: &mut Teach, set: &NpcSet) {
        self.anim.play(set.lib(self.def), self.stance(set), false);
        t.stage = TeachStage::Practice;
        t.t = 0.0;
        t.len = t.lesson.practice_secs;
        t.react = None;
    }

    fn feedback(&mut self, t: &mut Teach, clean: bool, set: &NpcSet, events: &mut Vec<NpcEvent>) {
        // A strict teacher: even good work gets a correction now and then.
        let praise = if clean { !self.rng.chance(0.25) } else { self.rng.chance(0.15) };
        let pool = if praise { &t.lesson.praise } else { &t.lesson.critique };
        let (text, react) = pool[self.rng.next_u32() as usize % pool.len()].clone();
        self.teach_say(t, TeachStage::Feedback, text, react, set, events);
    }

    fn teach_tick(&mut self, mut t: Box<Teach>, dt: f32, set: &NpcSet, env: &NpcEnv, events: &mut Vec<NpcEvent>) -> State {
        t.t += dt;
        // Face the knight, except while demonstrating (the nail swings into the open air).
        let knight_right = env.knight.x > self.pos.x;
        self.looking_right = if matches!(t.stage, TeachStage::Open { .. } | TeachStage::Demo { .. }) { !knight_right } else { knight_right };
        if t.t < t.len {
            return State::Teaching(t);
        }
        match t.stage {
            TeachStage::Open { step } | TeachStage::Demo { step } if step + 1 < t.demo.len() => {
                let next = if matches!(t.stage, TeachStage::Open { .. }) { TeachStage::Open { step: step + 1 } } else { TeachStage::Demo { step: step + 1 } };
                self.demo_step(&mut t, next, set);
            }
            TeachStage::Open { .. } => self.instruct(&mut t, set, events),
            TeachStage::Demo { .. } => self.practice(&mut t, set),
            TeachStage::Instruct => {
                events.push(NpcEvent::LineDone);
                let demo = t.lesson.drills[t.i].demo.as_ref().and_then(|n| set.clips[self.def].demos.get(n)).cloned().unwrap_or_default();
                if demo.is_empty() {
                    self.practice(&mut t, set);
                } else {
                    t.demo = demo;
                    self.demo_step(&mut t, TeachStage::Demo { step: 0 }, set);
                }
            }
            TeachStage::Practice => {
                // The knight never did it.
                t.misses += 1;
                if t.misses >= MAX_MISSES {
                    self.state = State::Teaching(t);
                    self.end_lesson(events);
                    return std::mem::replace(&mut self.state, State::Gone);
                }
                self.feedback(&mut t, false, set, events);
            }
            TeachStage::Feedback => {
                events.push(NpcEvent::LineDone);
                self.anim.play(set.lib(self.def), self.stance(set), false);
                t.stage = TeachStage::Gap;
                t.t = 0.0;
                t.len = DRILL_GAP;
                t.react = None;
            }
            TeachStage::Gap => {
                t.i += 1;
                if t.i < t.lesson.drills.len() {
                    self.instruct(&mut t, set, events);
                } else {
                    let pool = &t.lesson.closing;
                    let (text, react) = pool[self.rng.next_u32() as usize % pool.len()].clone();
                    self.teach_say(&mut t, TeachStage::Closing, text, react, set, events);
                }
            }
            TeachStage::Closing => {
                events.push(NpcEvent::LineDone);
                events.push(NpcEvent::LessonOver { finished: true });
                self.anim.play(set.lib(self.def), self.stance(set), false);
                return State::Lingering { left: self.rng.range(2.5, 4.0) };
            }
        }
        State::Teaching(t)
    }

    /// Ride the owner window; fall onto the next ledge when unsupported.
    fn follow(&mut self, env: &NpcEnv, dt: f32) {
        if let Some(r) = self.owner.and_then(|id| env.terrain.window_rect(id))
            && self.vy.is_none()
        {
            self.pos = Vec2::new(r.left, r.top) + self.rel;
        }
        let supported = env.terrain.ledges.iter().any(|l| (l.y - self.pos.y).abs() < 1.5 && l.x0 - 1.0 <= self.pos.x && self.pos.x <= l.x1 + 1.0);
        let walking_off = matches!(self.state, State::Exit { to_x: Some(_), .. }) && !env.area.contains(Vec2::new(self.pos.x, env.area.top + 1.0));
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
