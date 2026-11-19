//! Pestering the cursor: not a fight, just showing off around it. He buzzes about it with
//! hops that cross under it, loops that go up one side, dash over the top and fall down the
//! other, quick dash-bys back and forth, and the odd Cyclone Slash spin, then gets bored of
//! it. Nothing scores; a click on him ends the game (he flinches and lets it be).
use crate::controller::{Gait, MoveInput, Side};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::percept::{CursorSense, Poke};
use super::{Attack, AttackDir, Motion};

/// Close enough to start playing (px from the body center).
const PLAY_RANGE: f32 = 320.0;
/// Seconds per trick before switching to another.
const TRICK_SECS: [f32; 2] = [2.5, 4.5];
/// Total play time.
const PLAY_SECS: [f32; 2] = [10.0, 18.0];
/// The cursor wandered off: give up after this long beyond `LOST_RANGE`.
const LOST_RANGE: f32 = 900.0;
const LOST_SECS: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trick {
    /// Short hops crossing under the cursor, side to side.
    Hops,
    /// Up one side (jump + double jump), dash over the top, drop down the other side.
    Loop,
    /// Run and dash past under it, turn, dash back.
    DashBy,
    /// Jump up at it and flick the nail past it (never touching it).
    Flick,
}

impl Trick {
    fn label(self) -> &'static str {
        match self {
            Trick::Hops => "hops",
            Trick::Loop => "loop",
            Trick::DashBy => "dash-by",
            Trick::Flick => "flick",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Approach,
    Play,
    /// A one-off act (cyclone spin, flinch, a satisfied look), then `next`.
    Act(Option<ActWait>, bool),
}

#[derive(Clone, Debug)]
pub(crate) struct Pester {
    phase: Phase,
    trick: Trick,
    trick_left: f32,
    play_left: f32,
    lost_for: f32,
    /// Side of the cursor the current pass is heading to.
    side: Side,
    jump_hold: f32,
    double_done: bool,
    dashed: bool,
    flick_cd: f32,
    spun: bool,
    travel: Option<Travel>,
    goal_at: Vec2,
}

impl Pester {
    pub fn start(ctx: &mut Ctx) -> Self {
        let play = ctx.range(PLAY_SECS);
        Self {
            phase: Phase::Approach,
            trick: Trick::Hops,
            trick_left: 0.0,
            play_left: play,
            lost_for: 0.0,
            side: if ctx.rng.chance(0.5) { Side::Left } else { Side::Right },
            jump_hold: 0.0,
            double_done: false,
            dashed: false,
            flick_cd: 0.0,
            spun: false,
            travel: None,
            goal_at: Vec2::ZERO,
        }
    }

    pub fn reason(&self) -> String {
        match self.phase {
            Phase::Approach => "pester: buzzing over to the cursor".into(),
            Phase::Play => format!("pester: {} around the cursor ({:.0}s left)", self.trick.label(), self.play_left.max(0.0)),
            Phase::Act(..) => "pester: showing off".into(),
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let dt = ctx.dt();
        if let Phase::Act(act, done) = &mut self.phase {
            ctx.face_cursor();
            if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                if *done {
                    return Status::Done;
                }
                self.phase = Phase::Play;
            }
            return Status::Running;
        }
        let s = ctx.senses;
        let cursor = match s.cursor {
            Some(c) if !s.user_busy && !s.user_away && !s.fullscreen => c,
            _ => return Status::Done,
        };
        if ctx.p.pokes.iter().any(|p| matches!(p, Poke::Click { .. })) {
            // Swatted: fair enough.
            ctx.nav.stop();
            ctx.mind.annoy(0.05);
            return self.finish(ctx, "recoil");
        }
        self.lost_for = if cursor.dist() > LOST_RANGE { self.lost_for + dt } else { 0.0 };
        if self.lost_for > LOST_SECS || ctx.senses.stuck {
            return Status::Done;
        }
        match self.phase {
            Phase::Approach => self.approach(ctx, cursor),
            Phase::Play => {
                self.play_left -= dt;
                if self.play_left <= 0.0 && ctx.grounded() {
                    ctx.mind.adjust(|n| &mut n.mischief, -0.6);
                    ctx.mind.adjust(|n| &mut n.boredom, -0.4);
                    ctx.mind.excite(0.1);
                    return self.finish(ctx, "look_up");
                }
                self.play(ctx, cursor);
            }
            Phase::Act(..) => {}
        }
        Status::Running
    }

    fn finish(&mut self, ctx: &mut Ctx, act: &str) -> Status {
        self.phase = Phase::Act(ActWait::named(ctx, act), true);
        Status::Running
    }

    fn approach(&mut self, ctx: &mut Ctx, c: CursorSense) {
        if c.dist() <= PLAY_RANGE {
            ctx.nav.stop();
            self.travel = None;
            self.phase = Phase::Play;
            ctx.mind.excite(0.2);
            return;
        }
        let goal = NavGoal::Point(c.pos);
        match &mut self.travel {
            None => {
                self.travel = Some(Travel::new(goal, TravelStyle::Hurry, 10.0));
                self.goal_at = c.pos;
            }
            Some(t) if self.goal_at.distance(c.pos) > 120.0 => {
                t.retarget(goal);
                self.goal_at = c.pos;
            }
            Some(_) => {}
        }
        if let Some(t) = &mut self.travel
            && t.tick(ctx) != TravelStep::Moving
        {
            // No route: play from wherever he is if it's close-ish, else forget it.
            self.travel = None;
            self.phase = Phase::Play;
        }
    }

    fn next_trick(&mut self, ctx: &mut Ctx) {
        let options = [(Trick::Hops, 1.0), (Trick::Loop, 1.2), (Trick::DashBy, 0.8), (Trick::Flick, 0.7)];
        let mut next = ctx.rng.pick_weighted(&options).unwrap_or(Trick::Hops);
        if next == self.trick {
            next = ctx.rng.pick_weighted(&options).unwrap_or(Trick::Hops);
        }
        self.trick = next;
        self.trick_left = ctx.range(TRICK_SECS);
    }

    fn play(&mut self, ctx: &mut Ctx, c: CursorSense) {
        if !ctx.free_to_move() {
            return;
        }
        let dt = ctx.dt();
        self.trick_left -= dt;
        self.flick_cd -= dt;
        let k = ctx.p.knight;
        let grounded = k.is_grounded();
        if grounded {
            self.double_done = false;
            self.dashed = false;
        }
        if self.trick_left <= 0.0 && grounded {
            // Between tricks, once: a Cyclone Slash spin right under it.
            if !self.spun && c.offset.x.abs() < 160.0 && ctx.rng.chance(0.35) {
                self.spun = true;
                if let Some(w) = ActWait::named(ctx, "cyclone_slash") {
                    self.phase = Phase::Act(Some(w), false);
                    self.trick_left = 0.5;
                    return;
                }
            }
            self.next_trick(ctx);
        }
        // Where the cursor is relative to the knight's feet / body center.
        let off = c.offset;
        let toward = Side::of(off.x).unwrap_or(k.facing);
        // Target x of the current pass: past the cursor on `self.side`.
        let pass_x = c.pos.x + self.side.sign() * 110.0;
        let to_pass = pass_x - k.pos.x;
        let reached = to_pass * self.side.sign() <= 0.0;
        let mut input = MoveInput { gait: Gait::Run, ..Default::default() };
        let press = |input: &mut MoveInput| {
            input.jump_pressed = true;
            input.jump_held = true;
        };
        match self.trick {
            Trick::Hops => {
                if reached && grounded {
                    self.side = self.side.opposite();
                }
                input.x = (to_pass / 60.0).clamp(-1.0, 1.0);
                if grounded && to_pass.abs() > 30.0 {
                    press(&mut input);
                    self.jump_hold = 0.06;
                }
            }
            Trick::Loop => {
                // Up the near side, over the top, down the far side.
                let near_x = c.pos.x - self.side.sign() * 90.0;
                let above = k.pos.y < c.pos.y - 10.0;
                if grounded {
                    let d = near_x - k.pos.x;
                    input.x = (d / 40.0).clamp(-1.0, 1.0);
                    if d.abs() < 40.0 {
                        press(&mut input);
                        self.jump_hold = 0.35;
                    }
                } else if !above && !self.double_done && k.vel.y > -80.0 && k.can_double_jump() {
                    press(&mut input);
                    self.double_done = true;
                    self.jump_hold = 0.2;
                } else if above && !self.dashed && k.can_dash() {
                    input.x = self.side.sign();
                    input.dash_pressed = true;
                    self.dashed = true;
                } else if self.dashed {
                    input.x = (to_pass / 60.0).clamp(-1.0, 1.0);
                }
                if self.dashed && grounded {
                    self.side = self.side.opposite();
                }
            }
            Trick::DashBy => {
                input.gait = Gait::Sprint;
                if reached {
                    self.side = self.side.opposite();
                }
                input.x = self.side.sign();
                if grounded && off.x.abs() < 90.0 && (c.pos.x - k.pos.x) * self.side.sign() > 0.0 && k.can_dash() {
                    input.dash_pressed = true;
                }
            }
            Trick::Flick => {
                let beside = c.pos.x - toward.sign() * 70.0;
                input.x = ((beside - k.pos.x) / 50.0).clamp(-1.0, 1.0);
                if grounded && (beside - k.pos.x).abs() < 50.0 && off.y < -40.0 {
                    press(&mut input);
                    self.jump_hold = (-off.y / 500.0).clamp(0.08, 0.35);
                }
                // A playful slash past it at the top of the jump (never a hit).
                if !grounded && k.vel.y > -120.0 && self.flick_cd <= 0.0 && c.dist() < 200.0 {
                    let dir = if off.y < -off.x.abs() { AttackDir::Up } else { AttackDir::Forward };
                    ctx.out.attack = Some(Attack { dir, side: toward, hit: false });
                    self.flick_cd = 0.9;
                }
            }
        }
        if self.jump_hold > 0.0 {
            input.jump_held = true;
            self.jump_hold -= dt;
        }
        ctx.out.face = Some(toward);
        ctx.out.motion = Motion::Direct(input);
    }
}
