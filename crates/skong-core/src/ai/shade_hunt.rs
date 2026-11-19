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
    /// Seconds the Shade has stayed out of slash reach while fighting.
    out_of_reach: f32,
    /// The route to the Shade was checked (`Navigator::reaches`).
    checked: bool,
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
            out_of_reach: 0.0,
            checked: false,
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
                if !self.checked {
                    self.checked = true;
                    let under = Self::ground_under(ctx, sh.pos);
                    if !ctx.nav.reaches(ctx.p.knight, under, TravelStyle::Hurry, 150.0) {
                        // No way to get under it: don't even try (no frantic chase).
                        ctx.interest.block(PoiTarget::Spot(under), ctx.now + 2.0 * cfg.shade_retry);
                        ctx.stakes.shade_ready_at = ctx.now + 2.0 * cfg.shade_retry;
                        log::debug!("brain: the shade can't be reached; leaving it for now");
                        return Status::Done;
                    }
                }
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
            (HuntPhase::Fight, Some(_)) if self.out_of_reach > cfg.unreachable_secs => {
                // Can't get at it: no frantic chase. Calm down and leave it for a while.
                ctx.nav.stop();
                let under = Self::ground_under(ctx, shade.map_or(ctx.p.knight.pos, |s| s.pos));
                ctx.interest.block(PoiTarget::Spot(under), ctx.now + 2.0 * cfg.shade_retry);
                ctx.stakes.shade_ready_at = ctx.now + 2.0 * cfg.shade_retry;
                ctx.mind.emotions.excitement *= 0.3;
                ctx.mind.annoy(0.15);
                log::debug!("brain: the shade is out of reach; leaving it for now");
                Status::Done
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
        if off.length() > b.attack_range * 1.6 {
            self.out_of_reach += ctx.dt();
        } else {
            self.out_of_reach = 0.0;
        }

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

/// Gather the Geo lying around: nearest-first by an effort-weighted distance (climbing
/// costs more than walking, so a cluster on his ledge goes before a coin up high), hurry
/// for a big pile and stroll for stragglers, give up on a coin he can't reach (it fades
/// on its own), and a happy little act after a big haul.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Loot {
    travel: Option<Travel>,
    target: Vec2,
    elapsed: f32,
    pub picked: u32,
    /// Coins in sight when the gathering began.
    pile: usize,
    cheer: Option<ActWait>,
}

/// Coins picked up in one go that earn a happy act.
const BIG_HAUL: u32 = 6;

/// Effort to walk from `from` to `to`: vertical moves (jumps, drops) cost extra.
pub(crate) fn effort(from: Vec2, to: Vec2) -> f32 {
    (to.x - from.x).abs() + 2.5 * (to.y - from.y).abs()
}

/// The next coin to go for (cheapest effort, not blocked, within `range`).
pub(crate) fn next_coin(coins: &[Vec2], from: Vec2, range: f32, blocked: impl Fn(Vec2) -> bool) -> Option<Vec2> {
    coins.iter().copied().filter(|c| c.distance(from) < range && !blocked(*c)).min_by(|a, b| effort(from, *a).total_cmp(&effort(from, *b)))
}

impl Loot {
    pub fn start(ctx: &mut Ctx) -> Self {
        let k = ctx.p.knight.pos;
        let range = ctx.cfg.stakes.loot_range;
        let pile = ctx.stakes.sense.coins.iter().filter(|c| c.distance(k) < range).count();
        if pile >= 8 {
            ctx.mind.excite(0.3);
        }
        Self { travel: None, target: Vec2::ZERO, elapsed: 0.0, picked: 0, pile, cheer: None }
    }

    fn style(&self, ctx: &Ctx) -> TravelStyle {
        let left = ctx.stakes.sense.coins.len();
        let bother = ctx.cfg.bother_level;
        if left >= 8 && bother > 0.0 && ctx.mind.needs.energy > 0.25 {
            TravelStyle::Hurry
        } else if left <= 2 || bother <= 0.0 {
            TravelStyle::Stroll
        } else {
            TravelStyle::Normal
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        if let Some(w) = &mut self.cheer {
            return if w.tick(ctx) { Status::Done } else { Status::Running };
        }
        self.elapsed += ctx.dt();
        if self.elapsed > 120.0 {
            ctx.stakes.loot_ready_at = ctx.now + 20.0;
            return self.wrap_up(ctx);
        }
        let k = ctx.p.knight.pos;
        let range = ctx.cfg.stakes.loot_range;
        let still_there = ctx.stakes.sense.coins.iter().any(|c| c.distance(self.target) < 12.0);
        if self.travel.is_none() || !still_there {
            if self.travel.is_some() && !still_there {
                self.picked += 1;
            }
            let interest = &*ctx.interest;
            let next = next_coin(&ctx.stakes.sense.coins, k, range, |c| interest.is_blocked(PoiTarget::Spot(c)));
            let Some(coin) = next else {
                ctx.nav.stop();
                return self.wrap_up(ctx);
            };
            self.target = coin;
            let style = self.style(ctx);
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
                    if k.distance(self.target) > 40.0 {
                        Self::leave(ctx, self.target);
                        self.travel = None;
                    }
                }
                TravelStep::Failed => {
                    // Unreachable: leave it (it fades), try the next one.
                    Self::leave(ctx, self.target);
                    self.travel = None;
                }
            }
        }
        Status::Running
    }

    /// Give up on a coin he can't get to: it fades, unless a Dream Gate could take him
    /// there (then it is only skipped for now, and kept for the gate).
    fn leave(ctx: &mut Ctx, coin: Vec2) {
        if super::dream_gate::may_take(ctx, coin) {
            log::debug!("loot: no way to the coin at ({:.0},{:.0}); keeping it for a dream gate", coin.x, coin.y);
            ctx.interest.block(PoiTarget::Spot(coin), ctx.now + 30.0);
            return;
        }
        ctx.interest.block(PoiTarget::Spot(coin), ctx.now + 120.0);
        ctx.out.stakes.push(StakeEvent::AbandonCoin { at: coin });
    }

    /// Done gathering: a happy act after a big haul.
    fn wrap_up(&mut self, ctx: &mut Ctx) -> Status {
        if self.picked >= BIG_HAUL && ctx.grounded() {
            ctx.mind.adjust(|n| &mut n.boredom, -0.2);
            ctx.mind.excite(0.2);
            self.cheer = ActWait::named(ctx, "collect_happy");
            if self.cheer.is_some() {
                return Status::Running;
            }
        }
        Status::Done
    }

    /// Playing the happy act after a big haul (keep at it).
    pub fn cheering(&self) -> bool {
        self.cheer.is_some()
    }

    pub fn reason(&self) -> String {
        if self.cheer.is_some() {
            return format!("loot: happy with {} coins", self.picked);
        }
        format!("loot: gathering geo ({}/{} picked)", self.picked, self.pile)
    }
}
