//! Fighting enemies (and sparring with a visitor who challenged him): the knight's top
//! engagement. He hurries over, closes in, slashes (up / down at flyers), throws a spell or
//! a nail art now and then, takes the odd bump (recoil), and moves on to the next foe until
//! none is left. Losing all his masks makes him back off sore (or die, if `lethal`).
//!
//! The app owns the foes (`crate::enemy`, a sparring visitor) and reports them every tick
//! through [`super::Brain::see_foes`]; his blows come back as [`super::Intent::strikes`].
use std::collections::BTreeMap;

use serde::Deserialize;

use crate::controller::{Gait, MoveInput, Side};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};
use crate::props::ledge_below;
use crate::stakes::{DeathKind, Spell};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::stakes::{body_centre, try_spell};
use super::{Attack, AttackDir, Brain, Motion, Percepts};

/// Who the foe is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoeKind {
    /// A wandering enemy: fought for real (it dies and drops Geo).
    #[default]
    Enemy,
    /// A visitor sparring with him (Zote): blows land, nobody dies.
    Sparring,
}

/// Something to fight, as the brain perceives it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Foe {
    pub id: u32,
    pub kind: FoeKind,
    /// Body centre (screen px).
    pub pos: Vec2,
    /// Feet point (flyers: the bottom of their body).
    pub feet: Vec2,
    /// Half its body size (px).
    pub half: Vec2,
    pub flying: bool,
    /// Can be struck right now.
    pub vulnerable: bool,
    /// Coming at him (chasing, winding up, attacking).
    pub attacking: bool,
}

/// A blow that connects (nail or spell); the app applies it to the foe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoeStrike {
    pub id: u32,
    pub kind: FoeKind,
    pub damage: u32,
    /// The knight's body centre (knockback direction).
    pub from: Vec2,
    /// Where it lands (the foe's body centre): the hit marker goes there.
    pub at: Vec2,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct FightConfig {
    /// Bumps he takes in one fight before backing off.
    pub masks: u32,
    /// Losing every mask kills him (off by default: enemies only hurt).
    pub lethal: bool,
    /// Give up a fight after this long (s).
    pub max_secs: f32,
    /// After backing off, leave enemies alone this long (s).
    pub retry: f32,
    /// Give up on a foe that stays out of reach this long (s).
    pub unreachable_secs: f32,
    /// Utility of fighting an enemy (a spar is 0.98, like a lesson).
    pub utility: f32,
}

impl Default for FightConfig {
    fn default() -> Self {
        Self { masks: 4, lethal: false, max_secs: 60.0, retry: 45.0, unreachable_secs: 6.0, utility: 0.93 }
    }
}

/// Brain-side fight state (kept in the stakes mind: contact hits are stakes too).
#[derive(Clone, Debug, Default)]
pub(crate) struct FightMind {
    /// Bumps from foes not yet taken into account.
    pub hits: u32,
    /// No fighting enemies before this time (he backed off).
    pub ready_at: f32,
    /// Foes he gave up on (unreachable), until the given time.
    pub snubbed: BTreeMap<(FoeKind, u32), f32>,
}

/// Spar first, then the nearest enemy (not given up on, alive).
fn pick<'f>(foes: &'f [Foe], snubbed: &BTreeMap<(FoeKind, u32), f32>, knight: Vec2, now: f32) -> Option<&'f Foe> {
    foes.iter().filter(|f| !snubbed.get(&(f.kind, f.id)).is_some_and(|t| *t > now)).min_by(|a, b| {
        let key = |f: &Foe| (f.kind != FoeKind::Sparring, f.pos.distance(knight));
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    })
}

impl Brain {
    /// Foes on screen and the bumps they gave him since the last call (call before
    /// [`Brain::tick`]; empty when none).
    pub fn see_foes(&mut self, foes: Vec<Foe>, hits: u32) {
        if hits > 0 {
            // A bump wakes a sleeping knight.
            self.visitors.knock = true;
            self.mind.startle(0.25);
        }
        let new = foes.iter().any(|f| !self.senses.foes.iter().any(|o| o.id == f.id && o.kind == f.kind));
        if new {
            self.mind.excite(0.3);
        }
        self.stakes.fight.hits += hits;
        self.senses.foes = foes;
    }

    /// Utility of fighting the foes around (0 when there are none, or now is no time).
    pub(super) fn fight_utility(&self, p: &Percepts) -> f32 {
        let s = &self.senses;
        let Some(f) = pick(&s.foes, &self.stakes.fight.snubbed, p.knight.pos, self.now) else { return 0.0 };
        if self.asleep(p) || s.fullscreen || s.user_busy || self.stakes.pending_death.is_some() {
            return 0.0;
        }
        if f.kind == FoeKind::Sparring {
            return 0.98;
        }
        if self.now < self.stakes.fight.ready_at || self.mind.emotions.hurt > 0.6 {
            return 0.0;
        }
        let cfg = &self.cfg.fight;
        (cfg.utility + 0.04 * self.mind.personality.boldness).min(0.97)
    }
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Approach(Travel),
    Engage,
    /// An act between exchanges: recoil, a spell, the challenge bow.
    Act(Option<ActWait>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Fight {
    target: (FoeKind, u32),
    phase: Phase,
    /// Bumps he can still take.
    masks: u32,
    elapsed: f32,
    attack_cd: f32,
    dash_cd: f32,
    jump_hold: f32,
    double_jumped: bool,
    goal_at: Vec2,
    out_of_reach: f32,
    /// Foes beaten in this fight.
    pub beaten: u32,
    /// Blows landed on the current target.
    landed: u32,
}

/// Where to stand under a foe (the ledge below its feet).
fn ground_under(ctx: &Ctx, f: &Foe) -> Vec2 {
    ledge_below(ctx.p.terrain, f.feet - Vec2::new(0.0, 1.0)).map_or(f.feet, |(y, _)| Vec2::new(f.feet.x, y))
}

/// Close (or on the ledge he stands on): no route needed, he just goes at it.
fn within_reach(ctx: &Ctx, f: &Foe) -> bool {
    let k = ctx.p.knight;
    if f.pos.distance(body_centre(ctx)) < 5.0 * k.params.px_per_unit {
        return true;
    }
    let under = ground_under(ctx, f);
    k.support.is_some_and(|s| k.is_grounded() && (under.y - s.y).abs() < 4.0 && s.x0 <= under.x && under.x <= s.x1)
}

impl Fight {
    pub fn start(ctx: &mut Ctx) -> Self {
        ctx.mind.excite(0.3);
        ctx.mind.adjust(|n| &mut n.boredom, -0.2);
        let foe = pick(&ctx.senses.foes, &ctx.stakes.fight.snubbed, ctx.p.knight.pos, ctx.now).copied().unwrap_or_default();
        let mut f = Self {
            target: (foe.kind, foe.id),
            phase: Phase::Engage,
            masks: ctx.cfg.fight.masks.max(1),
            elapsed: 0.0,
            attack_cd: 0.3,
            dash_cd: 0.0,
            jump_hold: 0.0,
            double_jumped: false,
            goal_at: Vec2::ZERO,
            out_of_reach: 0.0,
            beaten: 0,
            landed: 0,
        };
        ctx.stakes.fight.hits = 0;
        f.aim(ctx, &foe);
        // A spar opens with a proper challenge.
        if foe.kind == FoeKind::Sparring && ctx.grounded() && matches!(f.phase, Phase::Engage) {
            ctx.face_toward(foe.pos.x);
            if let Some(w) = ActWait::named(ctx, "challenge") {
                f.phase = Phase::Act(Some(w));
            }
        }
        f
    }

    /// Head for `foe`: straight into the fight when close, else travel under it.
    fn aim(&mut self, ctx: &mut Ctx, foe: &Foe) {
        self.target = (foe.kind, foe.id);
        self.out_of_reach = 0.0;
        self.landed = 0;
        if within_reach(ctx, foe) {
            self.phase = Phase::Engage;
        } else {
            let goal = ground_under(ctx, foe);
            self.goal_at = goal;
            self.phase = Phase::Approach(Travel::to(ctx, goal, TravelStyle::Hurry));
        }
    }

    fn snub(ctx: &mut Ctx, target: (FoeKind, u32), secs: f32) {
        ctx.stakes.fight.snubbed.retain(|_, t| *t > ctx.now);
        ctx.stakes.fight.snubbed.insert(target, ctx.now + secs);
    }

    /// The current target gone (beaten or left): the next one, or done.
    fn next(&mut self, ctx: &mut Ctx) -> Status {
        ctx.nav.stop();
        if self.landed > 0 && self.target.0 == FoeKind::Enemy {
            self.beaten += 1;
            ctx.mind.excite(0.25);
            ctx.mind.adjust(|n| &mut n.boredom, -0.25);
        }
        match pick(&ctx.senses.foes, &ctx.stakes.fight.snubbed, ctx.p.knight.pos, ctx.now).copied() {
            Some(f) => {
                self.aim(ctx, &f);
                Status::Running
            }
            None => Status::Done,
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let dt = ctx.dt();
        self.elapsed += dt;
        self.attack_cd -= dt;
        self.dash_cd -= dt;
        let cfg = ctx.cfg.fight;
        if ctx.senses.user_busy || ctx.senses.fullscreen {
            // The user needs the screen: leave it.
            ctx.nav.stop();
            return Status::Done;
        }

        // Bumps from the foes.
        let hits = std::mem::take(&mut ctx.stakes.fight.hits);
        if hits > 0 {
            self.masks = self.masks.saturating_sub(hits);
            ctx.mind.hurt(0.1 * hits as f32);
            ctx.mind.startle(0.15);
            if self.masks == 0 {
                ctx.nav.stop();
                ctx.stakes.fight.ready_at = ctx.now + cfg.retry;
                if cfg.lethal {
                    ctx.stakes.pending_death = Some((DeathKind::Fight, false));
                } else {
                    // Beaten back: sore, he leaves them alone for a while.
                    ctx.mind.hurt(0.2);
                    ctx.mind.annoy(0.2);
                    log::info!("brain: knocked about by the enemies; backing off for {:.0} s", cfg.retry);
                }
                return Status::Done;
            }
            ctx.nav.stop();
            let off = self.foe(ctx).map(|f| f.pos - body_centre(ctx));
            let act = match try_spell(ctx, off, true, false) {
                Some((spell, w)) => {
                    self.strike_with(ctx, spell);
                    Some(w)
                }
                None => ActWait::named(ctx, "recoil"),
            };
            self.phase = Phase::Act(act);
        }

        let Some(foe) = self.foe(ctx) else { return self.next(ctx) };
        if self.elapsed > cfg.max_secs {
            ctx.nav.stop();
            ctx.stakes.fight.ready_at = ctx.now + cfg.retry;
            return Status::Done;
        }
        match &mut self.phase {
            Phase::Act(act) => {
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    self.phase = Phase::Engage;
                }
                Status::Running
            }
            Phase::Approach(t) => {
                if within_reach(ctx, &foe) {
                    ctx.nav.stop();
                    self.phase = Phase::Engage;
                    return Status::Running;
                }
                let goal = ground_under(ctx, &foe);
                if goal.distance(self.goal_at) > 120.0 {
                    t.retarget(NavGoal::Point(goal));
                    self.goal_at = goal;
                }
                match t.tick(ctx) {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Arrived => {
                        self.phase = Phase::Engage;
                        Status::Running
                    }
                    TravelStep::Failed if foe.pos.distance(ctx.p.knight.pos) < 600.0 => {
                        self.phase = Phase::Engage;
                        Status::Running
                    }
                    TravelStep::Failed => {
                        log::debug!("brain: can't get to {:?} {}; leaving it be", self.target.0, self.target.1);
                        Self::snub(ctx, self.target, 60.0);
                        self.landed = 0;
                        self.next(ctx)
                    }
                }
            }
            Phase::Engage if self.out_of_reach > cfg.unreachable_secs => {
                log::debug!("brain: {:?} {} stays out of reach; leaving it be", self.target.0, self.target.1);
                Self::snub(ctx, self.target, 30.0);
                self.landed = 0;
                self.next(ctx)
            }
            Phase::Engage => {
                self.engage(ctx, &foe);
                Status::Running
            }
        }
    }

    fn foe(&self, ctx: &Ctx) -> Option<Foe> {
        ctx.senses.foes.iter().find(|f| (f.kind, f.id) == self.target).copied()
    }

    /// A spell / nail art aimed at the target: it lands if it reaches.
    fn strike_with(&mut self, ctx: &mut Ctx, spell: Spell) {
        let from = body_centre(ctx);
        if let Some(f) = self.foe(ctx)
            && f.vulnerable
            && spell.damage() > 0
            && spell.reaches(f.pos - from, ctx.stakes.sense.scale.max(0.1))
        {
            self.landed += 1;
            ctx.out.strikes.push(FoeStrike { id: f.id, kind: f.kind, damage: spell.damage(), from, at: f.pos });
        }
    }

    /// Close in and slash; jump at flyers; now and then a spell or a nail art.
    fn engage(&mut self, ctx: &mut Ctx, foe: &Foe) {
        if !ctx.free_to_move() {
            return;
        }
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        let grounded = k.is_grounded();
        let centre = body_centre(ctx);
        let off = foe.pos - centre;
        let side = Side::of(off.x).unwrap_or(k.facing);
        // The nail reaches about 1.3 units past his body; stand just inside that.
        let reach_x = foe.half.x + 0.5 * k.body().width() + 1.3 * ppu;
        let reach_y = foe.half.y + 1.4 * ppu;
        let in_reach = off.x.abs() <= reach_x && off.y.abs() <= reach_y;
        // Out of reach: too high / low to hit, or far and not getting any closer.
        let stalled = off.x.abs() > 1.5 * reach_x && k.vel.x.abs() < 20.0;
        if stalled || off.y.abs() > reach_y + 2.5 * ppu {
            self.out_of_reach += ctx.dt();
        } else {
            self.out_of_reach = 0.0;
        }

        if self.attack_cd <= 0.0
            && let Some((spell, w)) = try_spell(ctx, Some(off), false, false)
        {
            ctx.nav.stop();
            self.strike_with(ctx, spell);
            self.attack_cd = ctx.cfg.battle.attack_cooldown;
            self.phase = Phase::Act(Some(w));
            return;
        }

        let mut input = MoveInput { gait: Gait::Run, ..Default::default() };
        if off.x.abs() > 0.8 * reach_x {
            input.x = side.sign();
        }
        if grounded {
            self.double_jumped = false;
            if off.y < -reach_y && off.x.abs() < 3.0 * ppu {
                input.jump_pressed = true;
                self.jump_hold = (-off.y / 500.0).clamp(0.08, 0.35);
            }
        } else if !self.double_jumped && k.vel.y > -60.0 && off.y < -reach_y && k.can_double_jump() {
            input.jump_pressed = true;
            self.double_jumped = true;
            self.jump_hold = 0.2;
        }
        if self.jump_hold > 0.0 {
            input.jump_held = true;
            self.jump_hold -= ctx.dt();
        }
        if off.x.abs() > 8.0 * ppu && off.y.abs() < 2.0 * ppu && self.dash_cd <= 0.0 && k.can_dash() {
            input.dash_pressed = true;
            self.dash_cd = 1.5;
        }
        if in_reach && self.attack_cd <= 0.0 {
            let dir = if off.y < -off.x.abs() {
                AttackDir::Up
            } else if !grounded && off.y > off.x.abs() {
                AttackDir::Down
            } else {
                AttackDir::Forward
            };
            let hit = foe.vulnerable;
            ctx.out.attack = Some(Attack { dir, side, hit });
            self.attack_cd = ctx.cfg.battle.attack_cooldown;
            if hit {
                self.landed += 1;
                ctx.out.strikes.push(FoeStrike { id: foe.id, kind: foe.kind, damage: 1, from: centre, at: foe.pos });
                ctx.stakes.spells.gain(1);
            }
        }
        ctx.out.face = Some(side);
        ctx.out.motion = Motion::Direct(input);
    }

    pub fn reason(&self) -> String {
        let who = match self.target.0 {
            FoeKind::Enemy => "enemy",
            FoeKind::Sparring => "sparring partner",
        };
        match self.phase {
            Phase::Approach(_) => format!("fight: going for the {who} #{}", self.target.1),
            Phase::Engage | Phase::Act(_) => format!("fight: {who} #{} ({} masks, {} beaten)", self.target.1, self.masks, self.beaten),
        }
    }
}
