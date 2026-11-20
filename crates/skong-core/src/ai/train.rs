//! Training with a teaching visitor (Great Nailsage Sly): after their conversation the
//! visitor runs a short lesson (`crate::npc::lesson`) and the knight follows it. He stands
//! beside the teacher and watches each instruction and demonstration, practises the drill
//! when it's his turn (slashes into the open air, up and down slashes, dashes, jumps, the
//! nail arts), reports back ([`Intent::lesson`]), takes the praise or the correction with
//! the line's reaction, and bows out at the end.
//!
//! Every drill goes through the usual outputs: [`Attack`]s with `hit: false`, knight acts
//! by name, and [`Motion::Direct`] inputs. The lesson ends early (politely) when the user
//! starts typing, goes away, or a fullscreen app comes up.
use crate::controller::{MoveInput, MoveState, Side};
use crate::nav::TravelStyle;

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::greet::{DREAM_NAIL_CHANCE, Visitor, VisitorState, beside, stand_point, startable};
use super::{Attack, AttackDir, Motion};

/// Where the teacher is in the lesson.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LessonStage {
    /// Announcing / demonstrating the drill: watch.
    #[default]
    Instruct,
    /// The knight's turn.
    Practice,
    /// Praise or a correction (the line's `react` is the knight's answer).
    Feedback,
    /// The closing line.
    Closing,
}

/// What a teaching visitor wants right now (part of [`Visitor`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LessonCue {
    /// Drill number (1-based), echoed in [`LessonSignal::Drilled`].
    pub serial: u32,
    /// `slash`, `up_slash`, `down_slash`, `dash`, `jump`, or a knight act's name.
    pub drill: String,
    pub stage: LessonStage,
}

/// The knight's side of the lesson, for the app to pass to the teacher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LessonSignal {
    /// Drill `serial` is done (`clean`: every repetition came off).
    Drilled { serial: u32, clean: bool },
    /// He has to stop (the user needs the screen, something urgent): end the lesson.
    Stop,
}

/// [`Intent::lesson`]: a signal for visitor `visitor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LessonReport {
    pub visitor: u32,
    pub signal: LessonSignal,
}

/// Seconds between two slashes of a drill.
const SLASH_GAP: f32 = 0.45;
/// A moment to square up (face the open air) before each drill.
const READY_SECS: f32 = 0.3;
/// A jump that hasn't left the ground by then didn't happen.
const TAKEOFF_SECS: f32 = 0.5;
/// Longest a single repetition may take.
const REP_SECS: f32 = 2.5;
/// Longest a whole drill may take.
const DRILL_SECS: f32 = 10.0;
/// Room (game units) a travelling drill wants in front of him.
const ROOM_UNITS: f32 = 3.5;

#[derive(Clone, Debug, PartialEq)]
enum DrillKind {
    Slash,
    UpSlash,
    DownSlash,
    Dash,
    Jump,
    /// A knight act by name (`great_slash`, `cyclone_slash`, `dash_slash`...).
    Act(String),
}

impl DrillKind {
    fn parse(s: &str) -> Self {
        match s {
            "slash" => DrillKind::Slash,
            "up_slash" => DrillKind::UpSlash,
            "down_slash" => DrillKind::DownSlash,
            "dash" => DrillKind::Dash,
            "jump" => DrillKind::Jump,
            other => DrillKind::Act(other.to_string()),
        }
    }

    fn reps(&self) -> u32 {
        match self {
            DrillKind::Slash => 3,
            DrillKind::UpSlash | DrillKind::DownSlash | DrillKind::Dash | DrillKind::Jump => 2,
            DrillKind::Act(_) => 1,
        }
    }

    /// Drills that carry him along the ledge.
    fn travels(&self) -> bool {
        match self {
            DrillKind::Dash => true,
            DrillKind::Act(n) => n == "dash_slash",
            _ => false,
        }
    }
}

/// One drill in progress.
#[derive(Clone, Debug)]
struct Drill {
    kind: DrillKind,
    serial: u32,
    /// Repetitions done (and how many came off).
    reps: u32,
    good: u32,
    /// Seconds into the drill / into the current repetition.
    t: f32,
    rep_t: f32,
    /// The side he works towards (away from the teacher, where there's room).
    side: Side,
    cooldown: f32,
    /// Current repetition: jump pressed, left the ground, slashed / dashed / double-jumped.
    pressed: bool,
    airborne: bool,
    did: bool,
    jump_hold: f32,
    act: Option<ActWait>,
}

#[derive(Clone, Debug)]
enum Phase {
    /// Beside the teacher: watch, answer feedback, wait for his turn.
    Attend,
    /// Walk back to the teacher after a drill that carried him off.
    Return(Travel),
    Drill(Box<Drill>),
}

#[derive(Clone, Debug)]
pub(crate) struct Train {
    pub id: u32,
    name: String,
    phase: Phase,
    /// Last drill reported done.
    done: u32,
    /// Last feedback / closing line answered (serial, stage).
    answered: Option<(u32, LessonStage)>,
    /// Answering a line (or bowing at the end).
    act: Option<ActWait>,
    /// The lesson ended (finished or stopped): the brain sends no `Stop` when switching.
    pub ended: bool,
    /// Don't try walking back before this time (the way back failed).
    return_ready_at: f64,
    drills: u32,
    /// The after-lesson dream nail was considered.
    dream_decided: bool,
}

impl Train {
    pub fn start(ctx: &mut Ctx, v: &Visitor) -> Self {
        ctx.mind.excite(0.2);
        ctx.mind.adjust(|n| &mut n.boredom, -0.3);
        Self { id: v.id, name: v.name.clone(), phase: Phase::Attend, done: 0, answered: None, act: None, ended: false, return_ready_at: 0.0, drills: 0, dream_decided: false }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let teaching = ctx.senses.visitors.iter().find(|v| v.id == self.id && v.state == VisitorState::Teaching).cloned();
        let Some(v) = teaching else {
            // Lesson over: finish the bow (or whatever answer is playing), then carry on.
            self.ended = true;
            ctx.nav.stop();
            if let Some(w) = &mut self.act
                && !w.tick(ctx)
            {
                return Status::Running;
            }
            if !self.dream_decided {
                // Afterwards, sometimes: dream-nail the teacher to hear what he really thought.
                self.dream_decided = true;
                let lingering = ctx.senses.visitors.iter().find(|v| v.id == self.id && v.state == VisitorState::Waiting).map(|v| v.pos.x);
                let chance = DREAM_NAIL_CHANCE * (0.5 + ctx.mind.personality.curiosity);
                if let Some(x) = lingering.filter(|_| self.drills > 0)
                    && ctx.rng.chance(chance)
                    && let Some(w) = ActWait::named(ctx, "dream_nail")
                {
                    ctx.face_toward(x);
                    self.act = Some(w);
                    ctx.out.dream_visitor = Some(self.id);
                    return Status::Running;
                }
            }
            if self.drills > 0 {
                ctx.mind.adjust(|n| &mut n.mischief, -0.3);
                ctx.mind.excite(0.15);
            }
            return Status::Done;
        };
        let s = ctx.senses;
        if s.user_busy || s.user_away || s.fullscreen {
            // The screen is needed: thank the teacher another time.
            self.stop(ctx);
            return Status::Done;
        }
        let Some(cue) = v.lesson.clone() else { return Status::Running };
        ctx.out.waiting = false;
        match &mut self.phase {
            Phase::Drill(d) => {
                if let Some(clean) = d.tick(ctx) {
                    let serial = d.serial;
                    self.done = serial;
                    self.drills += 1;
                    ctx.mind.adjust(|n| &mut n.boredom, -0.1);
                    ctx.out.lesson = Some(LessonReport { visitor: self.id, signal: LessonSignal::Drilled { serial, clean } });
                    self.phase = Phase::Attend;
                }
            }
            Phase::Return(t) => match t.tick(ctx) {
                TravelStep::Moving => {}
                TravelStep::Arrived => self.phase = Phase::Attend,
                TravelStep::Failed => {
                    self.return_ready_at = ctx.now + 8.0;
                    self.phase = Phase::Attend;
                }
            },
            Phase::Attend => self.attend(ctx, &v, &cue),
        }
        Status::Running
    }

    fn attend(&mut self, ctx: &mut Ctx, v: &Visitor, cue: &LessonCue) {
        // Answer the teacher's feedback / closing line once (look down, bow...).
        if matches!(cue.stage, LessonStage::Feedback | LessonStage::Closing)
            && self.answered != Some((cue.serial, cue.stage))
            && let Some(r) = v.react.as_deref()
        {
            self.answered = Some((cue.serial, cue.stage));
            if self.act.is_none() {
                ctx.face_toward(v.pos.x);
                self.act = startable(ctx, r);
            }
        }
        if let Some(w) = &mut self.act {
            if !w.tick(ctx) {
                return;
            }
            self.act = None;
        }
        if cue.stage == LessonStage::Practice && cue.serial > self.done && ctx.grounded() {
            let side = self.work_side(ctx, v, &DrillKind::parse(&cue.drill));
            self.phase = Phase::Drill(Box::new(Drill::new(&cue.drill, cue.serial, side)));
            ctx.nav.stop();
            return;
        }
        let goal = stand_point(ctx, v);
        if !beside(ctx, v, goal) && ctx.grounded() && ctx.now >= self.return_ready_at {
            self.phase = Phase::Return(Travel::to(ctx, goal, TravelStyle::Normal));
            return;
        }
        ctx.nav.stop();
        ctx.face_toward(v.pos.x);
    }

    /// Away from the teacher; for drills that travel, towards the side with more room.
    fn work_side(&self, ctx: &Ctx, v: &Visitor, kind: &DrillKind) -> Side {
        let k = ctx.p.knight;
        let away = Side::of(k.pos.x - v.pos.x).unwrap_or(k.facing);
        if !kind.travels() {
            return away;
        }
        let Some(s) = k.support else { return away };
        let room = |side: Side| if side == Side::Right { s.x1 - k.pos.x } else { k.pos.x - s.x0 };
        let need = ROOM_UNITS * k.params.px_per_unit;
        if room(away) < need && room(away.opposite()) > room(away) { away.opposite() } else { away }
    }

    fn stop(&mut self, ctx: &mut Ctx) {
        if !self.ended {
            self.ended = true;
            ctx.nav.stop();
            ctx.out.lesson = Some(LessonReport { visitor: self.id, signal: LessonSignal::Stop });
        }
    }

    pub fn reason(&self) -> String {
        match &self.phase {
            Phase::Attend if self.act.is_some() => format!("train with {}: answering", self.name),
            Phase::Attend => format!("train with {}: watching ({} drills done)", self.name, self.drills),
            Phase::Return(_) => format!("train with {}: back to the teacher", self.name),
            Phase::Drill(d) => format!("train with {}: drill {} {:?} ({}/{})", self.name, d.serial, d.kind, d.reps, d.kind.reps()),
        }
    }
}

impl Drill {
    fn new(drill: &str, serial: u32, side: Side) -> Self {
        Drill {
            kind: DrillKind::parse(drill),
            serial,
            reps: 0,
            good: 0,
            t: 0.0,
            rep_t: 0.0,
            side,
            cooldown: READY_SECS,
            pressed: false,
            airborne: false,
            did: false,
            jump_hold: 0.0,
            act: None,
        }
    }

    /// `Some(clean)` once the drill is over.
    fn tick(&mut self, ctx: &mut Ctx) -> Option<bool> {
        let dt = ctx.dt();
        self.t += dt;
        self.cooldown -= dt;
        ctx.nav.stop();
        let want = self.kind.reps();
        if self.reps >= want && self.cooldown <= 0.0 && ctx.grounded() && ctx.p.act.is_none() {
            return Some(self.good >= want);
        }
        if self.t > DRILL_SECS {
            // Ran out of time (couldn't jump here, an act never started...).
            return Some(false);
        }
        // Square up first: face the open air, then begin.
        if self.reps == 0 && !self.pressed && self.act.is_none() {
            ctx.out.face = Some(self.side);
            if self.t < READY_SECS {
                return None;
            }
        }
        if self.reps >= want {
            return None; // let the last one settle
        }
        let k = ctx.p.knight;
        let grounded = k.is_grounded();
        match self.kind.clone() {
            DrillKind::Slash | DrillKind::UpSlash => {
                ctx.out.face = Some(self.side);
                if self.cooldown <= 0.0 && grounded && ctx.p.act.is_none() {
                    let dir = if self.kind == DrillKind::Slash { AttackDir::Forward } else { AttackDir::Up };
                    ctx.out.attack = Some(Attack { dir, side: self.side, hit: false });
                    self.rep(true, SLASH_GAP);
                }
            }
            DrillKind::DownSlash => self.airborne_rep(ctx, |d, ctx| {
                // At the top of the jump, on the way down: strike below.
                if !d.did && ctx.p.knight.vel.y > -60.0 {
                    ctx.out.attack = Some(Attack { dir: AttackDir::Down, side: d.side, hit: false });
                    d.did = true;
                }
            }),
            DrillKind::Jump => self.airborne_rep(ctx, |d, ctx| {
                // A double jump near the top, if he has the wings.
                let k = ctx.p.knight;
                let mut input = MoveInput::default();
                if !d.did && k.vel.y > -80.0 && k.can_double_jump() {
                    input.jump_pressed = true;
                    input.jump_held = true;
                    d.jump_hold = 0.2;
                    d.did = true;
                    ctx.out.motion = Motion::Direct(input);
                } else if !d.did && !k.abilities.double_jump {
                    d.did = true; // a clean single jump is all he can do
                }
            }),
            DrillKind::Dash => {
                // Out and back again.
                let dir = if self.reps.is_multiple_of(2) { self.side } else { self.side.opposite() };
                let mut input = MoveInput::default();
                if !self.pressed {
                    if self.cooldown <= 0.0 && grounded && ctx.free_to_move() && k.can_dash() {
                        input.x = dir.sign();
                        input.dash_pressed = true;
                        self.pressed = true;
                        self.rep_t = 0.0;
                    } else if self.cooldown <= 0.0 {
                        self.rep_t += dt;
                        if self.rep_t > REP_SECS {
                            self.rep(false, 0.3);
                        }
                    }
                    ctx.out.face = Some(dir);
                } else {
                    self.rep_t += dt;
                    self.did |= matches!(k.state, MoveState::Dashing { .. });
                    if self.rep_t > 0.45 && !matches!(k.state, MoveState::Dashing { .. }) {
                        let ok = self.did;
                        self.rep(ok, 0.5);
                    }
                }
                ctx.out.motion = Motion::Direct(input);
            }
            DrillKind::Act(name) => {
                if self.act.is_none() {
                    self.act = ActWait::named(ctx, &name);
                    if self.act.is_none() {
                        self.rep(false, 0.0); // no such act
                        return None;
                    }
                }
                if let Some(w) = &mut self.act {
                    if !w.started() {
                        ctx.out.face = Some(self.side);
                    }
                    if w.tick(ctx) {
                        let ok = w.started();
                        self.act = None;
                        self.rep(ok, 0.4);
                    }
                }
            }
        }
        None
    }

    fn rep(&mut self, ok: bool, gap: f32) {
        self.reps += 1;
        self.good += ok as u32;
        self.cooldown = gap;
        self.pressed = false;
        self.airborne = false;
        self.did = false;
        self.rep_t = 0.0;
    }

    /// A repetition that jumps: press, leave the ground, do `mid_air` while airborne, and
    /// count it on landing (good if `mid_air` did its thing).
    fn airborne_rep(&mut self, ctx: &mut Ctx, mid_air: impl Fn(&mut Drill, &mut Ctx)) {
        let dt = ctx.dt();
        let k = ctx.p.knight;
        if !self.pressed {
            if self.cooldown <= 0.0 && k.is_grounded() && ctx.free_to_move() {
                ctx.out.motion = Motion::Direct(MoveInput { jump_pressed: true, jump_held: true, ..Default::default() });
                self.pressed = true;
                self.jump_hold = 0.3;
                self.rep_t = 0.0;
            }
            ctx.out.face = Some(self.side);
            return;
        }
        self.rep_t += dt;
        if !self.airborne {
            if !k.is_grounded() {
                self.airborne = true;
            } else if self.rep_t > TAKEOFF_SECS {
                self.rep(false, 0.4); // never left the ground
                return;
            }
        }
        ctx.out.motion = Motion::Direct(MoveInput { jump_held: self.jump_hold > 0.0, ..Default::default() });
        self.jump_hold -= dt;
        if self.airborne && !k.is_grounded() {
            mid_air(self, ctx);
        }
        if self.airborne && k.is_grounded() || self.rep_t > REP_SECS {
            let ok = self.did && self.airborne;
            self.rep(ok, 0.4);
        }
    }
}

/// The visitor (if any) who is teaching the knight right now.
pub(crate) fn teacher(visitors: &[Visitor]) -> Option<&Visitor> {
    visitors.iter().find(|v| v.state == VisitorState::Teaching)
}
