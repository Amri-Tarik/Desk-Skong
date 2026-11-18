//! Battling the cursor: approach, issue a challenge, then chase / jump / dash at it and
//! slash when in reach. Clicks on the knight are the cursor's hits: recoil, and two in
//! quick succession stun it. First to the configured score wins; the knight gives up
//! politely when the user starts typing, leaves, or the fight drags on.
use crate::controller::{Gait, MoveInput, Side};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::percept::{CursorSense, Poke};
use super::{Attack, AttackDir, Motion};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Won,
    Lost,
    GaveUp,
}

/// Running score, for overlays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BattleScore {
    pub knight: u32,
    pub cursor: u32,
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Approach,
    Challenge(Option<ActWait>),
    Fight,
    /// Took a hit: short recoil act, then back to the fight.
    Recoil(Option<ActWait>),
    Stunned(Option<ActWait>),
    Finale(Outcome, Option<ActWait>),
}

#[derive(Clone, Debug)]
pub(crate) struct Battle {
    phase: Phase,
    pub score: BattleScore,
    elapsed: f32,
    attack_cd: f32,
    dash_cd: f32,
    jump_hold: f32,
    double_jumped: bool,
    last_hit_taken: f32,
    travel: Option<Travel>,
    goal_at: Vec2,
}

impl Battle {
    pub fn start(_ctx: &mut Ctx) -> Self {
        Self {
            phase: Phase::Approach,
            score: BattleScore::default(),
            elapsed: 0.0,
            attack_cd: 0.0,
            dash_cd: 0.0,
            jump_hold: 0.0,
            double_jumped: false,
            last_hit_taken: f32::NEG_INFINITY,
            travel: None,
            goal_at: Vec2::ZERO,
        }
    }

    pub fn outcome(&self) -> Option<Outcome> {
        match self.phase {
            Phase::Finale(o, _) => Some(o),
            _ => None,
        }
    }

    pub fn detail(&self) -> String {
        let phase = match self.phase {
            Phase::Approach => "approach",
            Phase::Challenge(_) => "challenge",
            Phase::Fight => "fight",
            Phase::Recoil(_) => "recoil",
            Phase::Stunned(_) => "stunned",
            Phase::Finale(Outcome::Won, _) => "won",
            Phase::Finale(Outcome::Lost, _) => "lost",
            Phase::Finale(Outcome::GaveUp, _) => "gave up",
        };
        format!("{phase} {}-{}", self.score.knight, self.score.cursor)
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.elapsed += ctx.dt();
        self.attack_cd -= ctx.dt();
        self.dash_cd -= ctx.dt();

        if let Phase::Finale(_, act) = &mut self.phase {
            ctx.face_cursor();
            return if act.as_mut().is_none_or(|w| w.tick(ctx)) { Status::Done } else { Status::Running };
        }

        let b = ctx.cfg.battle;
        let cursor = match ctx.senses.cursor {
            Some(c) if !ctx.senses.user_busy && !ctx.senses.user_away && self.elapsed <= b.max_duration && c.far_for <= b.give_up_secs => c,
            _ => return self.finish(ctx, Outcome::GaveUp),
        };

        // The cursor's hits: clicks on the knight.
        let hits = ctx.p.pokes.iter().filter(|p| matches!(p, Poke::Click { .. })).count() as u32;
        if hits > 0 && !matches!(self.phase, Phase::Approach) {
            self.score.cursor += hits;
            ctx.mind.hurt(0.1 * hits as f32);
            if self.score.cursor >= b.lose_hits {
                return self.finish(ctx, Outcome::Lost);
            }
            let quick = ctx.now - self.last_hit_taken < b.stun_window;
            self.last_hit_taken = ctx.now;
            ctx.nav.stop();
            self.phase = if quick {
                ctx.mind.annoy(0.3);
                Phase::Stunned(ActWait::named(ctx, "stunned"))
            } else {
                Phase::Recoil(ActWait::named(ctx, "recoil"))
            };
        }

        match &mut self.phase {
            Phase::Approach => self.approach(ctx, cursor),
            Phase::Challenge(act) | Phase::Recoil(act) | Phase::Stunned(act) => {
                ctx.face_cursor();
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    if matches!(self.phase, Phase::Challenge(_)) {
                        ctx.mind.excite(0.3);
                    }
                    self.phase = Phase::Fight;
                }
            }
            Phase::Fight => self.fight(ctx, cursor),
            Phase::Finale(..) => {}
        }
        if self.score.knight >= b.win_hits {
            return self.finish(ctx, Outcome::Won);
        }
        Status::Running
    }

    fn approach(&mut self, ctx: &mut Ctx, c: CursorSense) {
        if c.dist() <= ctx.cfg.battle.challenge_range {
            ctx.nav.stop();
            self.travel = None;
            ctx.face_cursor();
            self.phase = Phase::Challenge(ActWait::named(ctx, "challenge"));
            return;
        }
        let goal = NavGoal::Point(c.pos);
        let style = if ctx.mind.arousal() > 0.4 { TravelStyle::Hurry } else { TravelStyle::Normal };
        match &mut self.travel {
            None => {
                self.travel = Some(Travel::new(goal, style, 12.0));
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
            // Can't path there: charge straight at it if it's close-ish.
            self.travel = None;
            self.phase = Phase::Fight;
        }
    }

    fn fight(&mut self, ctx: &mut Ctx, c: CursorSense) {
        if !ctx.free_to_move() {
            return;
        }
        let b = ctx.cfg.battle;
        let k = ctx.p.knight;
        let grounded = k.is_grounded();
        let off = c.offset;
        let side = Side::of(off.x).unwrap_or(k.facing);
        let mut input = MoveInput { gait: Gait::Sprint, ..Default::default() };

        if off.x.abs() > b.attack_range * 0.5 {
            input.x = side.sign();
        }
        if grounded {
            self.double_jumped = false;
            if off.y < -70.0 && off.x.abs() < 260.0 {
                input.jump_pressed = true;
                self.jump_hold = (-off.y / 500.0).clamp(0.08, 0.35);
            }
        } else if !self.double_jumped && k.vel.y > -60.0 && off.y < -90.0 && k.can_double_jump() {
            input.jump_pressed = true;
            self.double_jumped = true;
            self.jump_hold = 0.2;
        }
        if self.jump_hold > 0.0 {
            input.jump_held = true;
            self.jump_hold -= ctx.dt();
        }
        if off.x.abs() > 320.0 && off.y.abs() < 140.0 && self.dash_cd <= 0.0 && k.can_dash() {
            input.dash_pressed = true;
            self.dash_cd = 1.2;
        }

        if c.dist() <= b.attack_range && self.attack_cd <= 0.0 {
            let dir = if off.y < -off.x.abs() {
                AttackDir::Up
            } else if !grounded && off.y > off.x.abs() {
                AttackDir::Down
            } else {
                AttackDir::Forward
            };
            let hit = c.speed() < b.dodge_speed;
            ctx.out.attack = Some(Attack { dir, side, hit });
            self.attack_cd = b.attack_cooldown;
            if hit {
                self.score.knight += 1;
                ctx.mind.excite(0.1);
            }
        }
        ctx.out.face = Some(side);
        ctx.out.motion = Motion::Direct(input);
    }

    fn finish(&mut self, ctx: &mut Ctx, outcome: Outcome) -> Status {
        ctx.nav.stop();
        let (act, mischief) = match outcome {
            Outcome::Won => {
                ctx.mind.excite(0.4);
                ctx.mind.adjust(|n| &mut n.boredom, -0.3);
                ("victory", -0.7)
            }
            Outcome::Lost => {
                ctx.mind.hurt(0.7);
                ("bow_down", -0.5) // knocked out
            }
            Outcome::GaveUp => {
                ctx.mind.annoy(0.15);
                ("", -0.3)
            }
        };
        ctx.mind.adjust(|n| &mut n.mischief, mischief);
        let wait = ActWait::named(ctx, act);
        self.phase = Phase::Finale(outcome, wait);
        Status::Running
    }
}
