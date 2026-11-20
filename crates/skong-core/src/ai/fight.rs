//! Fighting enemies (and sparring with a visitor who challenged him): the knight's top
//! engagement. He hurries over, closes in, slashes (up / down at flyers), uses his whole kit
//! when the moment calls for it (pogo, dive, dodge, crystal dash: `super::combat`), throws a spell or
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
use crate::stakes::spells::SPELL_SOUL;
use crate::stakes::{DeathKind, Spell, SpellCues};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::combat::{DIVE_DAMAGE, DIVE_MAX_DROP, DODGE_RANGE, Kit, KitMove, Mark, POGO_SWING_CD, Stance, crowd, dive_hits};
use super::stakes::{body_centre, try_spell, try_spell_cued};
use super::{Attack, AttackDir, Brain, Motion, Percepts};

/// Who the foe is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoeKind {
    /// A wandering enemy: fought for real (it dies and drops Geo).
    #[default]
    Enemy,
    /// A visitor sparring with him (Zote): blows land, nobody dies.
    Sparring,
    /// The Troupe Master (`crate::enemy::grimm`): fought to the end; losing every mask
    /// kills him (a Shade is left).
    Boss,
    /// The scarlet flame that summons him: walked up to and struck once.
    Lantern,
    /// One of the boss's flames: never a target; dodged, jumped or stepped away from.
    Hazard,
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
    /// Winding up an attack (a Husk's anticipation, Zote's antic): a Great Slash cuts it short.
    pub winding_up: bool,
    /// Lunging / charging at him right now: a shadow dash through it dodges.
    pub lunging: bool,
    /// Where it is heading (px/s; flames in flight, a dashing boss), zero when unknown.
    pub vel: Vec2,
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
    /// Hits he can take from the boss before he falls (always lethal).
    pub boss_masks: u32,
}

impl Default for FightConfig {
    fn default() -> Self {
        Self { masks: 4, lethal: false, max_secs: 60.0, retry: 45.0, unreachable_secs: 6.0, utility: 0.93, boss_masks: 7 }
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

/// Spar first, then the boss, then the nearest enemy (not given up on, alive), then a
/// lantern to strike. Flames are never targets.
fn pick<'f>(foes: &'f [Foe], snubbed: &BTreeMap<(FoeKind, u32), f32>, knight: Vec2, now: f32) -> Option<&'f Foe> {
    let rank = |k: FoeKind| match k {
        FoeKind::Sparring => 0,
        FoeKind::Boss => 1,
        FoeKind::Enemy => 2,
        FoeKind::Lantern | FoeKind::Hazard => 3,
    };
    foes.iter().filter(|f| f.kind != FoeKind::Hazard && !snubbed.get(&(f.kind, f.id)).is_some_and(|t| *t > now)).min_by(|a, b| {
        let key = |f: &Foe| (rank(f.kind), f.pos.distance(knight));
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
        if s.fullscreen || self.stakes.pending_death.is_some() {
            return 0.0;
        }
        // The boss is fought to the end, sore or not (only the Shade and dying come first).
        if f.kind == FoeKind::Boss {
            return 0.98;
        }
        // Strict: enemies on screen are always fought (typing doesn't matter: he fights on
        // his ledges, not on your cursor). Only a fullscreen app, a pending death, or a nap
        // while nobody's around (a bump wakes him anyway) hold it back.
        if self.asleep(p) && s.user_away {
            return 0.0;
        }
        if f.kind == FoeKind::Sparring {
            return 0.98;
        }
        if f.kind == FoeKind::Lantern {
            // A scarlet flame on his desktop: curious, he goes and strikes it.
            return 0.9;
        }
        // Too sore (or backing off) to *start* one. A fight under way ends itself (out of
        // masks, too long, nobody left): it used to lose its tier the moment a bump took him
        // past the soreness line, so Geo or Recover cut in, then the fight took over again
        // as he healed — walk, stop, turn, walk, at the foes.
        let running = matches!(self.active, super::behaviours::Active::Fight(_));
        if self.now < self.stakes.fight.ready_at || (!running && self.mind.emotions.hurt > 0.6) {
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
    /// Every foe is gone: land (from a last pogo or dive), then maybe focus.
    Landing(f32),
    /// Focusing to heal after the fight.
    Heal(Option<ActWait>),
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
    /// Pogo, dive, dodge, crystal dash opener, wall leap (see [`super::combat`]).
    kit: Kit,
    /// Facing the boss (his masks are the boss's, losing them is lethal).
    boss: bool,
    /// Answered the boss's bow.
    bowed: bool,
}

/// Where to stand under a foe (the ledge below its feet).
fn ground_under(ctx: &Ctx, f: &Foe) -> Vec2 {
    ledge_below(ctx.p.terrain, f.feet - Vec2::new(0.0, 1.0)).map_or(f.feet, |(y, _)| Vec2::new(f.feet.x, y))
}

/// He stands on a window top over a ground foe, with the soul for a dive: he can dive onto
/// it from here (walk along the ledge until it is right below).
fn dive_spot(ctx: &Ctx, f: &Foe) -> bool {
    let k = ctx.p.knight;
    let s = Stance::of(k);
    let drop = f.pos.y - k.pos.y;
    s.dive
        && s.on_one_way
        && !f.flying
        && ctx.stakes.spells.soul + 1e-4 >= SPELL_SOUL
        && drop > 2.0 * s.ppu
        && drop < DIVE_MAX_DROP * s.ppu
        && k.support.is_some_and(|l| l.x0 <= f.pos.x && f.pos.x <= l.x1)
}

/// Close (or on the ledge he stands on, or right above it): no route needed, he just goes
/// at it.
fn within_reach(ctx: &Ctx, f: &Foe) -> bool {
    let k = ctx.p.knight;
    if f.pos.distance(body_centre(ctx)) < 5.0 * k.params.px_per_unit || dive_spot(ctx, f) {
        return true;
    }
    let under = ground_under(ctx, f);
    k.support.is_some_and(|s| k.is_grounded() && (under.y - s.y).abs() < 4.0 && s.x0 <= under.x && under.x <= s.x1)
}

/// An enemy right in front of him, on his level, between him and his target (an enemy
/// too): it gets in the way, so it's fought first.
fn in_the_way(ctx: &Ctx, target: &Foe) -> Option<Foe> {
    if target.kind != FoeKind::Enemy {
        return None;
    }
    let c = body_centre(ctx);
    let ppu = ctx.p.knight.params.px_per_unit;
    let to_target = target.pos.x - c.x;
    let snubbed = &ctx.stakes.fight.snubbed;
    ctx.senses
        .foes
        .iter()
        .filter(|f| f.kind == FoeKind::Enemy && f.id != target.id && !snubbed.get(&(f.kind, f.id)).is_some_and(|t| *t > ctx.now))
        .filter(|f| {
            let dx = f.pos.x - c.x;
            dx * to_target > 0.0 && dx.abs() + ppu < to_target.abs() && dx.abs() < 4.0 * ppu && (f.pos.y - c.y).abs() < f.half.y + 1.5 * ppu
        })
        .min_by(|a, b| (a.pos.x - c.x).abs().total_cmp(&(b.pos.x - c.x).abs()))
        .copied()
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
            kit: Kit::new(ctx.now),
            boss: false,
            bowed: false,
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
        if foe.kind == FoeKind::Boss && !self.boss {
            // The real thing: his masks count against the boss now, and they are his life.
            self.boss = true;
            self.masks = ctx.cfg.fight.boss_masks.max(1);
            ctx.mind.excite(0.4);
            log::info!("brain: the troupe master! {} masks", self.masks);
        }
        // The boss comes to him (he teleports about): no trips, just the fight where he is.
        if foe.kind == FoeKind::Boss || within_reach(ctx, foe) {
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
            None => {
                // Nobody left: come down, then focus if he got hurt.
                self.landed = 0;
                self.phase = Phase::Landing(0.0);
                Status::Running
            }
        }
    }

    /// After the fight: land, then a Focus if he is hurt and has the soul for it.
    fn wrap_up(&mut self, ctx: &mut Ctx) -> Status {
        match &mut self.phase {
            Phase::Landing(t) => {
                *t += ctx.dt();
                if !ctx.grounded() {
                    return if *t > 3.0 { Status::Done } else { Status::Running };
                }
                match try_spell(ctx, None, false, true) {
                    Some((_, w)) => {
                        log::debug!("brain: focusing after the fight");
                        self.phase = Phase::Heal(Some(w));
                        Status::Running
                    }
                    None => Status::Done,
                }
            }
            Phase::Heal(w) => {
                if w.as_mut().is_none_or(|w| w.tick(ctx)) {
                    ctx.mind.emotions.hurt *= 0.3;
                    Status::Done
                } else {
                    Status::Running
                }
            }
            _ => Status::Done,
        }
    }

    /// The dive slammed down at `land`: it hits every foe near the impact.
    fn dive_strikes(&mut self, ctx: &mut Ctx, land: Vec2) {
        let k = ctx.p.knight;
        let dark = matches!(k.state, crate::controller::MoveState::Dive { dark: true, .. });
        let from = body_centre(ctx);
        let ppu = k.params.px_per_unit;
        let damage = DIVE_DAMAGE + u32::from(dark);
        for f in ctx.senses.foes.iter().filter(|f| f.vulnerable && dive_hits(land, f, ppu)) {
            if (f.kind, f.id) == self.target {
                self.landed += 1;
            }
            ctx.out.strikes.push(FoeStrike { id: f.id, kind: f.kind, damage, from, at: f.pos });
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let dt = ctx.dt();
        self.elapsed += dt;
        self.attack_cd -= dt;
        self.dash_cd -= dt;
        let cfg = ctx.cfg.fight;
        if ctx.senses.fullscreen {
            // The user needs the screen: leave it.
            ctx.nav.stop();
            return Status::Done;
        }

        if matches!(self.phase, Phase::Landing(_) | Phase::Heal(_)) {
            ctx.stakes.fight.hits = 0;
            return self.wrap_up(ctx);
        }
        if let Some(land) = self.kit.dive_landed(ctx) {
            self.dive_strikes(ctx, land);
        }

        // Bumps from the foes (none while shadow dashing through one).
        let mut hits = std::mem::take(&mut ctx.stakes.fight.hits);
        if hits > 0 && !self.kit.touchable(ctx.now) {
            log::debug!("brain: dashed through it untouched");
            hits = 0;
        }
        if hits > 0 {
            self.masks = self.masks.saturating_sub(hits);
            ctx.mind.hurt(0.1 * hits as f32);
            ctx.mind.startle(0.15);
            if self.masks == 0 {
                ctx.nav.stop();
                ctx.stakes.fight.ready_at = ctx.now + cfg.retry;
                if self.boss {
                    log::info!("brain: out of masks against the troupe master");
                    ctx.stakes.pending_death = Some((DeathKind::Fight, false));
                } else if cfg.lethal {
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
            if !self.boss {
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
            // Against the boss there's no time to reel: the next flame is already coming.
        }

        let Some(foe) = self.foe(ctx) else { return self.next(ctx) };
        let boss = foe.kind == FoeKind::Boss;
        if self.elapsed > cfg.max_secs && !boss {
            ctx.nav.stop();
            ctx.stakes.fight.ready_at = ctx.now + cfg.retry;
            return Status::Done;
        }
        if matches!(self.phase, Phase::Approach(_) | Phase::Engage)
            && let Some(f) = in_the_way(ctx, &foe)
        {
            // Another one stands between him and his target: that one first (walking past it
            // got him bumped and stopped at every step).
            log::debug!("brain: {:?} {} is in the way; that one first", f.kind, f.id);
            ctx.nav.stop();
            self.aim(ctx, &f);
            return Status::Running;
        }
        match &mut self.phase {
            Phase::Act(act) => {
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    self.phase = Phase::Engage;
                }
                Status::Running
            }
            Phase::Approach(t) => {
                if boss || within_reach(ctx, &foe) {
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
                    // The boss comes to him anyway (he teleports): stand and face him.
                    TravelStep::Failed if boss || foe.pos.distance(ctx.p.knight.pos) < 600.0 => {
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
            Phase::Engage if self.out_of_reach > cfg.unreachable_secs && !boss => {
                log::debug!("brain: {:?} {} stays out of reach; leaving it be", self.target.0, self.target.1);
                Self::snub(ctx, self.target, 30.0);
                self.landed = 0;
                self.next(ctx)
            }
            Phase::Engage => {
                self.engage(ctx, &foe);
                Status::Running
            }
            Phase::Landing(_) | Phase::Heal(_) => self.wrap_up(ctx),
        }
    }

    fn foe(&self, ctx: &Ctx) -> Option<Foe> {
        ctx.senses.foes.iter().find(|f| (f.kind, f.id) == self.target).copied()
    }

    /// A spell / nail art: it lands on every foe it reaches (the wraiths, a cyclone and a
    /// fireball's flight catch more than one).
    fn strike_with(&mut self, ctx: &mut Ctx, spell: Spell) {
        let from = body_centre(ctx);
        let scale = ctx.stakes.sense.scale.max(0.1);
        if spell.damage() == 0 {
            return;
        }
        for f in ctx.senses.foes.iter().filter(|f| f.vulnerable && spell.reaches(f.pos - from, scale)) {
            if (f.kind, f.id) == self.target {
                self.landed += 1;
            }
            ctx.out.strikes.push(FoeStrike { id: f.id, kind: f.kind, damage: spell.damage(), from, at: f.pos });
        }
    }

    /// The foe stands on the ledge he stands on (a crystal dash along it reaches it).
    fn same_ledge(ctx: &Ctx, foe: &Foe) -> bool {
        let k = ctx.p.knight;
        !foe.flying && k.support.is_some_and(|s| (foe.feet.y - s.y).abs() < 4.0 && s.x0 <= foe.feet.x && foe.feet.x <= s.x1)
    }

    /// Close in and slash; jump at flyers; pogo, dive, dodge, crystal dash, a spell or a nail
    /// art when the moment calls for one (see [`super::combat`]).
    fn engage(&mut self, ctx: &mut Ctx, foe: &Foe) {
        if !ctx.free_to_move() {
            return;
        }
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        let centre = body_centre(ctx);
        let off = foe.pos - centre;
        let side = Side::of(off.x).unwrap_or(k.facing);
        if foe.kind == FoeKind::Boss && !foe.attacking {
            // His entrance: answer the bow, then wait for the show to begin (no cheap shots).
            ctx.out.face = Some(side);
            ctx.out.motion = Motion::Hold;
            if !self.bowed && ctx.grounded() {
                self.bowed = true;
                if let Some(w) = ActWait::named(ctx, "challenge") {
                    self.phase = Phase::Act(Some(w));
                }
            }
            return;
        }
        let mut input = MoveInput { gait: Gait::Run, ..Default::default() };
        if self.evade(ctx, &mut input) {
            self.kit.pogo_run = false;
            ctx.out.motion = Motion::Direct(input);
            return;
        }
        // Untouchable right now (the boss mid-teleport or ballooned): keep clear, no swings.
        let futile = foe.kind == FoeKind::Boss && !foe.vulnerable;
        // The nail reaches about 1.3 units past his body; stand just inside that.
        let reach_x = foe.half.x + 0.5 * k.body().width() + 1.3 * ppu;
        let reach_y = foe.half.y + 1.4 * ppu;
        let in_reach = off.x.abs() <= reach_x && off.y.abs() <= reach_y;
        // Out of reach: too high / low to hit, or far and not getting any closer (a dive
        // from a ledge above still reaches it).
        let stalled = off.x.abs() > 1.5 * reach_x && k.vel.x.abs() < 20.0;
        if (stalled || off.y.abs() > reach_y + 2.5 * ppu) && !self.dive_in_reach(ctx, foe) {
            self.out_of_reach += ctx.dt();
        } else {
            self.out_of_reach = 0.0;
        }

        // A spell or nail art (not into a lunge: that one gets dodged; not with flames about
        // or the boss winding up: he'd be stuck in the cast).
        if self.attack_cd <= 0.0 && !foe.lunging && !futile && !Self::danger(ctx) {
            let scale = ctx.stakes.sense.scale.max(0.1);
            let cues = SpellCues { flying: foe.flying, winding_up: foe.winding_up, crowd: crowd(&ctx.senses.foes, centre, scale) };
            if let Some((spell, w)) = try_spell_cued(ctx, Some(off), false, false, cues) {
                ctx.nav.stop();
                self.kit.pogo_run = false;
                self.strike_with(ctx, spell);
                self.attack_cd = ctx.cfg.battle.attack_cooldown;
                self.phase = Phase::Act(Some(w));
                return;
            }
        }

        let mark = Mark::foe(foe, centre);
        // (No crystal dash opener into flames.)
        let same_ledge = Self::same_ledge(ctx, foe) && !Self::danger(ctx);
        match self.kit.step(ctx, &mark, self.attack_cd <= 0.0 && !futile, same_ledge, !futile, &mut input) {
            Some(KitMove::PogoSwing) => {
                ctx.out.attack = Some(Attack { dir: AttackDir::Down, side, hit: foe.vulnerable });
                self.attack_cd = POGO_SWING_CD;
                if foe.vulnerable {
                    self.landed += 1;
                    ctx.out.strikes.push(FoeStrike { id: foe.id, kind: foe.kind, damage: 1, from: centre, at: foe.pos });
                    ctx.stakes.spells.gain(1);
                    self.kit.bounce(ctx.now, &mut input);
                }
            }
            Some(KitMove::PogoJump) => self.jump_hold = 0.35,
            Some(KitMove::PogoSteer) => {
                if self.jump_hold > 0.0 {
                    input.jump_held = true;
                    self.jump_hold -= ctx.dt();
                }
            }
            Some(KitMove::Dive { .. } | KitMove::Dodge(_) | KitMove::Opening | KitMove::WallLeap) => {
                self.jump_hold = 0.0;
                self.double_jumped = false;
            }
            Some(KitMove::Busy) => {}
            None => self.close_in(ctx, foe, off, side, reach_x, reach_y, in_reach, &mut input),
        }
        ctx.out.face = Some(side);
        ctx.out.motion = Motion::Direct(input);
    }

    /// The boss and his flames, each read from its tell: a pillar glowing under his feet →
    /// out from over it (a dash if he can, toward the roomier side near a ledge's end);
    /// spikes about to rise along his ledge → jump them; a firebat, a fireball or the boss
    /// himself coming at him at his height → shadow dash through it, or jump; the boss
    /// ballooned overhead → keep well away, where the rings of fire have gaps.
    /// Returns whether it took over this tick's input.
    fn evade(&mut self, ctx: &mut Ctx, input: &mut MoveInput) -> bool {
        let k = ctx.p.knight;
        let s = Stance::of(k);
        let ppu = s.ppu;
        let body = k.body();
        let centre = body.center();
        let dt = ctx.dt();
        let threats: Vec<Foe> = ctx.senses.foes.iter().filter(|f| f.kind == FoeKind::Hazard || (f.kind == FoeKind::Boss && f.lunging)).copied().collect();
        // Room to run on his ledge, each way.
        let (room_l, room_r) = k.support.map_or((f32::INFINITY, f32::INFINITY), |l| (k.pos.x - l.x0, l.x1 - k.pos.x));
        for h in &threats {
            let off = h.pos - centre;
            let over = off.x.abs() <= h.half.x + 0.5 * body.width() && h.pos.y + h.half.y >= body.bottom - 0.6 * ppu && h.pos.y - h.half.y <= body.bottom + 0.5 * ppu;
            if h.kind == FoeKind::Hazard && h.winding_up && !h.flying && over {
                if h.half.x > 2.0 * ppu {
                    // A row of spikes: up and over (and hold it).
                    if s.grounded {
                        input.jump_pressed = true;
                        self.jump_hold = 0.4;
                        self.double_jumped = false;
                    } else if !self.double_jumped && k.vel.y > -60.0 && k.can_double_jump() {
                        input.jump_pressed = true;
                        self.double_jumped = true;
                        self.jump_hold = 0.25;
                    }
                    if self.jump_hold > 0.0 {
                        input.jump_held = true;
                        self.jump_hold -= dt;
                    }
                    log::trace!("brain: jumping the spikes");
                } else {
                    // A pillar about to erupt under him: out from over it (not into a ledge's
                    // end: then through to the other side).
                    let mut away = if off.x.abs() < 1.0 { if room_r >= room_l { 1.0 } else { -1.0 } } else { -off.x.signum() };
                    let room = if away > 0.0 { room_r } else { room_l };
                    if room < 2.0 * ppu {
                        away = -away;
                    }
                    input.x = away;
                    if s.grounded && k.can_dash() && self.dash_cd <= 0.0 {
                        input.dash_pressed = true;
                        self.dash_cd = 1.0;
                    }
                    ctx.out.face = Side::of(away);
                }
                return true;
            }
            let level = off.y.abs() <= h.half.y + s.half_h + 0.2 * ppu;
            let close = off.x.abs() <= h.half.x + DODGE_RANGE * ppu;
            let coming = h.vel.dot(-off) > 0.0;
            if h.kind == FoeKind::Boss && h.lunging && coming && h.vel.y > 0.4 * h.vel.x.abs() && off.x.abs() < 8.0 * ppu && off.y < 0.0 {
                // The boss diving at him from above: run on along his line, out from under
                // it (never up into it).
                input.x = if h.vel.x.abs() > 1.0 { h.vel.x.signum() } else { -off.x.signum() };
                if self.jump_hold > 0.0 {
                    self.jump_hold = 0.0;
                }
                return true;
            }
            if h.lunging && (h.flying || h.kind == FoeKind::Boss) && level && close && coming {
                let m = Mark::foe(h, centre);
                if let Some(mv) = self.kit.dodge(ctx, &m, input) {
                    ctx.out.face = match mv {
                        KitMove::Dodge(side) => Some(side),
                        _ => None,
                    };
                    return true;
                }
                if h.vel.y > 0.4 * h.vel.x.abs() {
                    // Diving down at him: don't hop into it; run on, out from under it.
                    let along = if h.vel.x.abs() > 1.0 { h.vel.x.signum() } else { -off.x.signum() };
                    input.x = along;
                    return true;
                }
                if s.grounded {
                    // No shadow dash ready: hop over it.
                    input.jump_pressed = true;
                    input.jump_held = true;
                    self.jump_hold = 0.35;
                    self.double_jumped = false;
                    return true;
                }
            }
        }
        // The boss winding up (a slash coming along the ledge, an uppercut, a dive, a cast):
        // out of his reach if close, else hold and watch until it's spent.
        if let Some(b) = ctx.senses.foes.iter().find(|f| f.kind == FoeKind::Boss && f.winding_up) {
            let dx = centre.x - b.pos.x;
            ctx.out.face = Side::of(-dx);
            if dx.abs() < 5.5 * ppu && b.pos.y - centre.y < 3.0 * ppu {
                let mut away = if dx.abs() < 1.0 { if room_r >= room_l { 1.0 } else { -1.0 } } else { dx.signum() };
                if (if away > 0.0 { room_r } else { room_l }) < 1.0 * ppu {
                    away = -away;
                }
                input.x = away;
            }
            return true;
        }
        // The boss ballooned up above (untouchable, spraying rings of fire): keep away.
        if let Some(b) = ctx.senses.foes.iter().find(|f| f.kind == FoeKind::Boss && f.flying && f.attacking && !f.vulnerable && !f.lunging) {
            let dx = centre.x - b.pos.x;
            if dx.abs() < 9.0 * ppu && b.pos.y < centre.y - 2.0 * ppu {
                let mut away = if dx.abs() < 1.0 { if room_r >= room_l { 1.0 } else { -1.0 } } else { dx.signum() };
                if (if away > 0.0 { room_r } else { room_l }) < 1.0 * ppu {
                    away = -away;
                }
                input.x = away;
                ctx.out.face = Side::of(-dx);
                return true;
            }
        }
        false
    }

    /// The boss's flames nearby or the boss winding up / lunging: no time for a long cast.
    fn danger(ctx: &Ctx) -> bool {
        let centre = body_centre(ctx);
        let near = 8.0 * ctx.p.knight.params.px_per_unit;
        ctx.senses.foes.iter().any(|f| match f.kind {
            FoeKind::Hazard => f.pos.distance(centre) < near + f.half.x,
            FoeKind::Boss => f.winding_up || f.lunging,
            _ => false,
        })
    }

    /// He could dive onto it from where he stands (a window top right above it).
    fn dive_in_reach(&self, ctx: &Ctx, foe: &Foe) -> bool {
        ctx.now >= self.kit.dive_ready_at && dive_spot(ctx, foe)
    }

    /// Plain fighting: walk up, jump (and double jump) at flyers, dash at far foes, slash.
    #[allow(clippy::too_many_arguments)]
    fn close_in(&mut self, ctx: &mut Ctx, foe: &Foe, off: Vec2, side: Side, reach_x: f32, reach_y: f32, in_reach: bool, input: &mut MoveInput) {
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        let grounded = k.is_grounded();
        let centre = body_centre(ctx);
        if self.dive_in_reach(ctx, foe) && off.y > reach_y {
            // Right above it on a window top: line up for the dive.
            input.x = (off.x / ppu).clamp(-1.0, 1.0);
            return;
        }
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
        // Something under him while he may not bounce (resting after a pogo run): no swing.
        let resting_down = !grounded && off.y > off.x.abs() && !self.kit.may_pogo(ctx.now);
        let futile = foe.kind == FoeKind::Boss && !foe.vulnerable;
        if in_reach && self.attack_cd <= 0.0 && !resting_down && !futile {
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
                if dir == AttackDir::Down && !grounded {
                    // Every mid-air down slash that connects bounces him, as in the game.
                    self.kit.bounce(ctx.now, input);
                }
            }
        }
    }

    pub fn reason(&self) -> String {
        let who = match self.target.0 {
            FoeKind::Enemy | FoeKind::Hazard => "enemy",
            FoeKind::Sparring => "sparring partner",
            FoeKind::Boss => "troupe master",
            FoeKind::Lantern => "scarlet flame",
        };
        match self.phase {
            Phase::Approach(_) => format!("fight: going for the {who} #{}", self.target.1),
            Phase::Engage | Phase::Act(_) => format!("fight: {who} #{} ({} masks, {} beaten)", self.target.1, self.masks, self.beaten),
            Phase::Landing(_) => format!("fight: over ({} beaten), landing", self.beaten),
            Phase::Heal(_) => "fight: over, focusing to heal".into(),
        }
    }
}
