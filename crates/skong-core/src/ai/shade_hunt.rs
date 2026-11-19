//! Going after the Shade (travel there, fight it, reclaim the Geo, focus to heal) and
//! picking up loose Geo.
use crate::controller::{Gait, MoveInput, Side};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};
use crate::props::ledge_below;
use crate::stakes::{DeathKind, Spell};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::interest::PoiTarget;
use super::stakes::{ShadeSense, StakeEvent, body_centre, try_spell};
use super::{Attack, AttackDir, Motion};

#[derive(Clone, Copy, Debug)]
pub(crate) enum HuntPhase {
    Travel(Travel),
    Fight,
    /// An act between exchanges: recoil / thorns / a spell.
    Act(Option<ActWait>),
    /// The Shade burst: take the Geo back.
    Reclaim(Option<ActWait>),
    /// Heal up after the fight.
    Focus(Option<ActWait>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadeHunt {
    pub phase: HuntPhase,
    /// Hits he can still take.
    pub masks: u32,
    elapsed: f32,
    attack_cd: f32,
    dash_cd: f32,
    jump_hold: f32,
    double_jumped: bool,
    goal_at: Vec2,
    /// Shade geo when last seen (for the reason line).
    geo: u32,
}

impl ShadeHunt {
    pub fn start(ctx: &mut Ctx) -> Self {
        ctx.stakes.force_hunt = false;
        let shade = ctx.stakes.sense.shade.unwrap_or_default();
        let goal = Self::ground_under(ctx, shade.pos);
        let style = if ctx.mind.needs.energy > 0.3 { TravelStyle::Normal } else { TravelStyle::Stroll };
        ctx.mind.excite(0.2);
        Self {
            phase: HuntPhase::Travel(Travel::to(ctx, goal, style)),
            masks: ctx.cfg.stakes.masks.max(1),
            elapsed: 0.0,
            attack_cd: 0.0,
            dash_cd: 0.0,
            jump_hold: 0.0,
            double_jumped: false,
            goal_at: goal,
            geo: shade.geo,
        }
    }

    /// The point on the ledge right under the Shade (where he can stand and fight).
    fn ground_under(ctx: &Ctx, at: Vec2) -> Vec2 {
        ledge_below(ctx.p.terrain, at).map_or(at, |(y, _)| Vec2::new(at.x, y))
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.elapsed += ctx.dt();
        self.attack_cd -= ctx.dt();
        self.dash_cd -= ctx.dt();
        let cfg = ctx.cfg.stakes;

        // The Shade's blows.
        let hits = std::mem::take(&mut ctx.stakes.sense.shade_hits);
        if hits > 0 && !matches!(self.phase, HuntPhase::Reclaim(_) | HuntPhase::Focus(_)) {
            self.masks = self.masks.saturating_sub(hits);
            ctx.mind.hurt(0.12 * hits as f32);
            ctx.mind.startle(0.2);
            if self.masks == 0 {
                ctx.stakes.pending_death = Some((DeathKind::Fight, true));
                ctx.stakes.shade_ready_at = ctx.now + cfg.shade_retry;
                return Status::Done;
            }
            ctx.nav.stop();
            let off = ctx.stakes.sense.shade.map(|s| s.pos - body_centre(ctx));
            let act = match try_spell(ctx, off, true, false) {
                Some((spell, w)) => {
                    self.strike(ctx, spell);
                    Some(w)
                }
                None => ActWait::named(ctx, "recoil"),
            };
            self.phase = HuntPhase::Act(act);
        }

        let shade = ctx.stakes.sense.shade;
        match (&mut self.phase, shade) {
            (HuntPhase::Reclaim(act), _) => {
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    ctx.mind.excite(0.5);
                    ctx.mind.emotions.hurt *= 0.5;
                    ctx.mind.adjust(|n| &mut n.boredom, -0.3);
                    let heal = try_spell(ctx, None, false, true).map(|(_, w)| w);
                    if heal.is_none() {
                        return Status::Done;
                    }
                    self.phase = HuntPhase::Focus(heal);
                }
                Status::Running
            }
            (HuntPhase::Focus(act), _) => {
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    ctx.mind.emotions.hurt *= 0.3;
                    return Status::Done;
                }
                Status::Running
            }
            (_, None) => {
                // Gone: beaten (if we were fighting) or reclaimed elsewhere.
                ctx.nav.stop();
                ctx.stakes.shade_ready_at = ctx.now + cfg.shade_retry;
                if matches!(self.phase, HuntPhase::Travel(_)) {
                    return Status::Done;
                }
                self.phase = HuntPhase::Reclaim(ActWait::named(ctx, "reclaim_shade"));
                Status::Running
            }
            (_, Some(_)) if self.elapsed > cfg.shade_fight_secs || ctx.senses.stuck => {
                ctx.nav.stop();
                ctx.stakes.shade_ready_at = ctx.now + cfg.shade_retry;
                ctx.mind.annoy(0.2);
                Status::Done
            }
            (HuntPhase::Travel(t), Some(sh)) => {
                self.geo = sh.geo;
                let near = sh.pos.distance(body_centre(ctx)) < 260.0 * ctx.stakes.sense.scale.max(0.1);
                if near {
                    ctx.nav.stop();
                    self.phase = HuntPhase::Fight;
                    return Status::Running;
                }
                let goal = Self::ground_under(ctx, sh.pos);
                if goal.distance(self.goal_at) > 120.0 {
                    t.retarget(NavGoal::Point(goal));
                    self.goal_at = goal;
                }
                match t.tick(ctx) {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Arrived => {
                        self.phase = HuntPhase::Fight;
                        Status::Running
                    }
                    TravelStep::Failed if sh.pos.distance(ctx.p.knight.pos) < 600.0 => {
                        self.phase = HuntPhase::Fight;
                        Status::Running
                    }
                    TravelStep::Failed => {
                        ctx.stakes.shade_ready_at = ctx.now + cfg.shade_retry;
                        Status::Done
                    }
                }
            }
            (HuntPhase::Act(act), Some(_)) => {
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    self.phase = HuntPhase::Fight;
                }
                Status::Running
            }
            (HuntPhase::Fight, Some(sh)) => {
                self.fight(ctx, sh);
                Status::Running
            }
        }
    }

    /// A blow (nail or spell) that reaches the Shade.
    fn strike(&mut self, ctx: &mut Ctx, spell: Spell) {
        let from = body_centre(ctx);
        if let Some(sh) = ctx.stakes.sense.shade
            && sh.vulnerable
            && spell.reaches(sh.pos - from, ctx.stakes.sense.scale.max(0.1))
            && spell.damage() > 0
        {
            ctx.out.stakes.push(StakeEvent::StrikeShade { damage: spell.damage(), from });
        }
    }

    /// Chase / jump at the Shade and slash it; now and then a spell or nail art.
    fn fight(&mut self, ctx: &mut Ctx, sh: ShadeSense) {
        if !ctx.free_to_move() {
            return;
        }
        let b = ctx.cfg.battle;
        let k = ctx.p.knight;
        let grounded = k.is_grounded();
        let centre = body_centre(ctx);
        let off = sh.pos - centre;
        let side = Side::of(off.x).unwrap_or(k.facing);

        if self.attack_cd <= 0.0
            && let Some((spell, w)) = try_spell(ctx, Some(off), false, false)
        {
            ctx.nav.stop();
            self.strike(ctx, spell);
            self.attack_cd = b.attack_cooldown;
            self.phase = HuntPhase::Act(Some(w));
            return;
        }

        let mut input = MoveInput { gait: Gait::Run, ..Default::default() };
        if off.x.abs() > b.attack_range * 0.5 {
            input.x = side.sign();
        }
        if grounded {
            self.double_jumped = false;
            if off.y < -70.0 && off.x.abs() < 200.0 {
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
            self.dash_cd = 1.5;
        }
        if off.length() <= b.attack_range * 1.1 && self.attack_cd <= 0.0 {
            let dir = if off.y < -off.x.abs() {
                AttackDir::Up
            } else if !grounded && off.y > off.x.abs() {
                AttackDir::Down
            } else {
                AttackDir::Forward
            };
            let hit = sh.vulnerable;
            ctx.out.attack = Some(Attack { dir, side, hit });
            self.attack_cd = b.attack_cooldown;
            if hit {
                ctx.out.stakes.push(StakeEvent::StrikeShade { damage: 1, from: centre });
                ctx.stakes.spells.gain(1);
            }
        }
        ctx.out.face = Some(side);
        ctx.out.motion = Motion::Direct(input);
    }

    pub fn reason(&self) -> String {
        match self.phase {
            HuntPhase::Travel(_) => format!("shade: going to reclaim {} geo", self.geo),
            HuntPhase::Fight | HuntPhase::Act(_) => format!("shade: fighting ({} masks)", self.masks),
            HuntPhase::Reclaim(_) => "shade: reclaimed!".into(),
            HuntPhase::Focus(_) => "shade: focusing to heal".into(),
        }
    }
}

// --- Loot ---------------------------------------------------------------------------

/// Walk over the Geo lying around to pick it up.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Loot {
    travel: Option<Travel>,
    target: Vec2,
    elapsed: f32,
    picked: u32,
}

impl Loot {
    pub fn start(_ctx: &mut Ctx) -> Self {
        Self { travel: None, target: Vec2::ZERO, elapsed: 0.0, picked: 0 }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.elapsed += ctx.dt();
        if self.elapsed > 30.0 {
            ctx.stakes.loot_ready_at = ctx.now + 20.0;
            return Status::Done;
        }
        let k = ctx.p.knight.pos;
        let range = ctx.cfg.stakes.loot_range;
        let still_there = ctx.stakes.sense.coins.iter().any(|c| c.distance(self.target) < 12.0);
        if self.travel.is_none() || !still_there {
            if self.travel.is_some() {
                self.picked += 1;
            }
            let next = ctx
                .stakes
                .sense
                .coins
                .iter()
                .copied()
                .filter(|c| c.distance(k) < range && !ctx.interest.is_blocked(PoiTarget::Spot(*c)))
                .min_by(|a, b| a.distance(k).total_cmp(&b.distance(k)));
            let Some(coin) = next else {
                ctx.nav.stop();
                return Status::Done;
            };
            self.target = coin;
            let style = if ctx.mind.arousal() > 0.3 { TravelStyle::Hurry } else { TravelStyle::Normal };
            self.travel = Some(Travel::to(ctx, coin, style));
        }
        if let Some(t) = &mut self.travel {
            match t.tick(ctx) {
                TravelStep::Moving => {}
                TravelStep::Arrived => {
                    // Standing on it: the app picks it up; if not (just out of reach), step on.
                    if k.distance(self.target) > 6.0 {
                        let dx = self.target.x - k.x;
                        ctx.out.motion = Motion::Direct(MoveInput { x: dx.signum(), gait: Gait::Walk, ..Default::default() });
                    }
                    if self.elapsed > 0.0 && k.distance(self.target) > 40.0 {
                        ctx.interest.block(PoiTarget::Spot(self.target), ctx.now + 60.0);
                        self.travel = None;
                    }
                }
                TravelStep::Failed => {
                    ctx.interest.block(PoiTarget::Spot(self.target), ctx.now + 60.0);
                    self.travel = None;
                }
            }
        }
        Status::Running
    }

    pub fn reason(&self) -> String {
        format!("loot: picking up geo ({} so far)", self.picked)
    }
}
