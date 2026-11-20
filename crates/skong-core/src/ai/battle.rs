//! Battling the cursor: approach, issue a challenge, then chase / jump / dash at it and
//! slash when in reach (pogo off a low cursor, dive onto one far below: `super::combat`). Clicks on the knight are the cursor's hits: recoil, and two in
//! quick succession stun it. First to the configured score wins; the knight gives up
//! politely when the user starts typing, leaves, or the fight drags on.
use crate::controller::{Gait, MoveInput, Side};
use crate::geom::Vec2;
use crate::stakes::SpellCues;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::percept::{CursorSense, Poke};
use super::combat::{DIVE_RADIUS, Kit, KitMove, Mark, POGO_SWING_CD};
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
    /// A spell or nail art (see `ai::stakes`), then back to the fight.
    Special(Option<ActWait>),
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
    last_hit_taken: f64,
    travel: Option<Travel>,
    goal_at: Vec2,
    /// Seconds the cursor has stayed out of slash reach while fighting.
    out_of_reach: f32,
    /// Pogo off the cursor, dive onto it (see [`super::combat`]).
    kit: Kit,
    /// Clicked once too often: he started this one (see `ai::handling`).
    pub provoked: bool,
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
            last_hit_taken: f64::NEG_INFINITY,
            travel: None,
            goal_at: Vec2::ZERO,
            out_of_reach: 0.0,
            kit: Kit::new(_ctx.now),
            provoked: false,
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
            Phase::Special(_) => "special",
            Phase::Finale(Outcome::Won, _) => "won",
            Phase::Finale(Outcome::Lost, _) => "lost",
            Phase::Finale(Outcome::GaveUp, _) => "gave up",
        };
        let why = if self.provoked { "provoked, " } else { "" };
        format!("{why}{phase} {}-{}", self.score.knight, self.score.cursor)
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.elapsed += ctx.dt();
        self.attack_cd -= ctx.dt();
        self.dash_cd -= ctx.dt();

        if let Phase::Finale(_, act) = &mut self.phase {
            ctx.face_cursor();
            return if act.as_mut().is_none_or(|w| w.tick(ctx)) { Status::Done } else { Status::Running };
        }

        if ctx.senses.stuck {
            // Running into a wall chasing the cursor: not worth it.
            return self.finish(ctx, Outcome::GaveUp);
        }
        let b = ctx.cfg.battle;
        let cursor = match ctx.senses.cursor {
            Some(c) if !ctx.senses.user_busy && !ctx.senses.user_away && self.elapsed <= b.max_duration && c.far_for <= b.give_up_secs => c,
            _ => return self.finish(ctx, Outcome::GaveUp),
        };
        if self.out_of_reach > 2.0 * ctx.cfg.stakes.unreachable_secs {
            // Out of reach (up on a ledge he can't get to): no frantic chase.
            ctx.mind.emotions.excitement *= 0.3;
            return self.finish(ctx, Outcome::GaveUp);
        }

        // The cursor's hits: clicks on the knight.
        let hits = ctx.p.pokes.iter().filter(|p| matches!(p, Poke::Click { .. })).count() as u32;
        if hits > 0 && !matches!(self.phase, Phase::Approach) {
            self.score.cursor += hits;
            ctx.mind.hurt(0.1 * hits as f32);
            if self.score.cursor >= b.lose_hits {
                return self.finish(ctx, Outcome::Lost);
            }
            let quick = ((ctx.now - self.last_hit_taken) as f32) < b.stun_window;
            self.last_hit_taken = ctx.now;
            ctx.nav.stop();
            self.phase = if quick {
                ctx.mind.annoy(0.3);
                Phase::Stunned(ActWait::named(ctx, "stunned"))
            } else {
                match super::stakes::try_spell(ctx, Some(cursor.offset), true, false) {
                    Some((spell, w)) => {
                        self.score.knight += u32::from(spell.reaches(cursor.offset, ctx.stakes.sense.scale.max(0.1)));
                        Phase::Special(Some(w))
                    }
                    None => Phase::Recoil(ActWait::named(ctx, "recoil")),
                }
            };
        }

        match &mut self.phase {
            Phase::Approach => self.approach(ctx, cursor),
            Phase::Challenge(act) | Phase::Recoil(act) | Phase::Stunned(act) | Phase::Special(act) => {
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
        let b = ctx.cfg.battle;
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        if let Some(land) = self.kit.dive_landed(ctx) {
            // The slam catches a cursor resting near where he lands.
            let near = (c.pos.x - land.x).abs() <= DIVE_RADIUS * ppu && (c.pos.y - land.y).abs() <= 2.5 * ppu;
            if near && c.speed() < b.dodge_speed {
                self.score.knight += 1;
                ctx.mind.excite(0.15);
            }
        }
        if !ctx.free_to_move() {
            return;
        }
        let grounded = k.is_grounded();
        let off = c.offset;
        let side = Side::of(off.x).unwrap_or(k.facing);
        self.out_of_reach = if c.dist() > b.attack_range * 1.6 { self.out_of_reach + ctx.dt() } else { 0.0 };
        // A cursor above him calls for the wraiths.
        let cues = SpellCues { flying: off.y < -1.4 * ppu, ..Default::default() };
        if self.attack_cd <= 0.0
            && let Some((spell, w)) = super::stakes::try_spell_cued(ctx, Some(off), false, false, cues)
        {
            // Spells and nail arts land like a slash would (a fast cursor dodges them).
            self.attack_cd = b.attack_cooldown;
            self.score.knight += u32::from(spell.reaches(off, ctx.stakes.sense.scale.max(0.1)) && c.speed() < b.dodge_speed);
            self.phase = Phase::Special(Some(w));
            return;
        }
        let mut input = MoveInput { gait: Gait::Sprint, ..Default::default() };

        // Pogo off it, or dive onto it from above. A cursor down by his legs (below his
        // chest, where a forward slash is awkward) is jumped over and pogoed.
        let hit = c.speed() < b.dodge_speed;
        let low = off.y > 0.0;
        match self.kit.step(ctx, &Mark::point(off), self.attack_cd <= 0.0, false, low, &mut input) {
            Some(KitMove::PogoSwing) => {
                ctx.out.attack = Some(Attack { dir: AttackDir::Down, side, hit });
                self.attack_cd = POGO_SWING_CD;
                if hit {
                    self.score.knight += 1;
                    ctx.stakes.spells.gain(1);
                    ctx.mind.excite(0.1);
                    self.kit.bounce(ctx.now, &mut input);
                }
                return Self::direct(ctx, side, input);
            }
            Some(KitMove::PogoJump) => {
                self.jump_hold = 0.35;
                return Self::direct(ctx, side, input);
            }
            Some(KitMove::PogoSteer) => {
                if self.jump_hold > 0.0 {
                    input.jump_held = true;
                    self.jump_hold -= ctx.dt();
                }
                return Self::direct(ctx, side, input);
            }
            Some(_) => return Self::direct(ctx, side, input),
            None => {}
        }

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

        let resting_down = !grounded && off.y > off.x.abs() && !self.kit.may_pogo(ctx.now);
        if c.dist() <= b.attack_range && self.attack_cd <= 0.0 && !resting_down {
            let dir = if off.y < -off.x.abs() {
                AttackDir::Up
            } else if !grounded && off.y > off.x.abs() {
                AttackDir::Down
            } else {
                AttackDir::Forward
            };
            ctx.out.attack = Some(Attack { dir, side, hit });
            self.attack_cd = b.attack_cooldown;
            if hit {
                self.score.knight += 1;
                ctx.stakes.spells.gain(1);
                ctx.mind.excite(0.1);
                if dir == AttackDir::Down && !grounded {
                    self.kit.bounce(ctx.now, &mut input);
                }
            }
        }
        Self::direct(ctx, side, input);
    }

    fn direct(ctx: &mut Ctx, side: Side, input: MoveInput) {
        ctx.out.face = Some(side);
        ctx.out.motion = Motion::Direct(input);
    }

    fn finish(&mut self, ctx: &mut Ctx, outcome: Outcome) -> Status {
        ctx.nav.stop();
        let (act, mischief) = match outcome {
            Outcome::Won => {
                super::stakes::victory(ctx);
                ctx.mind.excite(0.4);
                ctx.mind.adjust(|n| &mut n.boredom, -0.3);
                ("victory", -0.7)
            }
            Outcome::Lost => {
                ctx.mind.hurt(0.7);
                // A bad beating is lethal: the death plays out in `Dead` instead.
                if super::stakes::lethal_loss(ctx, self.score) { ("", -0.5) } else { ("bow_down", -0.5) } // knocked out
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
