//! **Dream Gate** fallback travel (see [`crate::dreamgate`]): when something he really
//! wants (a visitor, the Shade, Geo, a deposit, a shiny) sits where no route gets him, he
//! plants a dream gate there and warps to it instead of giving up.
//!
//! Kept special: only for engagement-tier prizes, only when nothing of the kind is within
//! walking reach (those come first), only onto a settled ledge with room for him, and at
//! most once per [`DreamGateConfig::cooldown`]. The Shade is the exception: the hunt plays
//! a [`DreamGate`] itself, cooldown or not, whenever no route gets him under it
//! (`ShadeHunt::way`).
//!
//! The scan ([`Brain::scan_dream_gate`]) runs with the other scorings and leaves a
//! [`GateWish`]; the [`DreamGate`] behaviour plays it out: plant (`dreamgate_set`, the app
//! shows the gate on the target ledge), charge and vanish (`dreamgate_warp`; the app moves
//! him to the gate as it ends), reappear (`dreamgate_in`). Then the usual behaviour picks
//! the prize up from right beside it.
use serde::Deserialize;

use crate::dreamgate::{GateEvent, gate_spot};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Active, Ctx, SEAT_TOL, Status, on_point};
use super::greet::{VisitorState, stand_point};
use super::interest::PoiTarget;
use super::navigator::Navigator;
use super::percept::Percepts;
use super::places::{PlaceId, PlaceKind, want};
use super::shade_hunt::effort;
use super::shiny::{PICK_TOL, approach, stand_spot};
use super::{Brain, Intent, Tier};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct DreamGateConfig {
    pub enabled: bool,
    /// Seconds from one gate to the next (whether or not the warp worked out).
    pub cooldown: f32,
    /// Prizes closer than this (px) are not worth a warp.
    pub min_distance: f32,
    /// Look for an unreachable prize at most this often (s): each look plans a few routes.
    pub scan_every: f32,
    /// Utility of the warp (an engagement): above Geo (≤ 0.85) and found places, below a
    /// lesson or a conversation in progress.
    pub utility: f32,
}

impl Default for DreamGateConfig {
    fn default() -> Self {
        Self { enabled: true, cooldown: 90.0, min_distance: 160.0, scan_every: 1.0, utility: 0.9 }
    }
}

/// Route plans per scan at most.
const MAX_PLANS: usize = 6;
/// Candidates checked per kind of prize.
const PER_KIND: usize = 3;
/// He must reappear within this long after the warp act ends (s).
const ARRIVE_SECS: f32 = 3.0;

/// What he warps to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prize {
    Visitor(u32),
    Shade,
    Coin,
    Deposit(PlaceId),
    Shiny(u64),
}

impl Prize {
    pub fn label(self) -> &'static str {
        match self {
            Prize::Visitor(_) => "visitor",
            Prize::Shade => "shade",
            Prize::Coin => "geo",
            Prize::Deposit(_) => "geo deposit",
            Prize::Shiny(_) => "shiny",
        }
    }
}

/// An unreachable prize worth a warp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateWish {
    pub prize: Prize,
    /// Where the prize is (to notice it going away).
    pub target: Vec2,
    /// The gate: a feet point on the prize's ledge.
    pub at: Vec2,
}

/// Brain-side gate state (lives in the stakes mind).
#[derive(Clone, Copy, Debug)]
pub(crate) struct GateMind {
    /// No new gate before this brain time.
    pub ready_at: f64,
    pub wish: Option<GateWish>,
    scanned_at: f64,
    /// Warped to this prize; the brain tidies up what lives outside the behaviour.
    pub landed: Option<Prize>,
    /// Warps so far.
    pub warps: u32,
}

impl Default for GateMind {
    fn default() -> Self {
        Self { ready_at: 0.0, wish: None, scanned_at: f64::NEG_INFINITY, landed: None, warps: 0 }
    }
}

/// A prize candidate: where it is, where he would stand, how close counts as there.
struct Candidate {
    prize: Prize,
    target: Vec2,
    stand: Vec2,
    tol: f32,
}

/// The gate point for a prize standing at `stand` (feet level), if it makes a good one.
pub(crate) fn spot(ctx: &Ctx, stand: Vec2) -> Option<Vec2> {
    let (terrain, ages, now) = (ctx.p.terrain, &ctx.interest.ages, ctx.now);
    gate_spot(terrain, |l| ages.settled(terrain, l, now), stand, ctx.p.knight.size)
}

/// Could a gate take him to the prize at `stand` right now (enabled, off cooldown, a good
/// spot)? Behaviours check this before giving a prize up for good.
pub(crate) fn may_take(ctx: &Ctx, stand: Vec2) -> bool {
    let cfg = ctx.cfg.dream_gate;
    cfg.enabled && ctx.now >= ctx.stakes.gate.ready_at && stand.distance(ctx.p.knight.pos) >= cfg.min_distance && spot(ctx, stand).is_some()
}

/// Does a route get him onto `stand`? Out of plans counts as yes (no warp on a guess).
fn reachable(ctx: &mut Ctx, stand: Vec2, tol: f32, plans: &mut usize) -> bool {
    let k = ctx.p.knight;
    if on_point(k, stand, tol) {
        return true;
    }
    if *plans == 0 {
        return true;
    }
    *plans -= 1;
    // Hurry is the most capable style: if it can't, nothing can.
    ctx.nav.reach(k, NavGoal::Point(stand), TravelStyle::Hurry).is_some_and(|r| r.lands_on(stand, tol))
}

/// Visitors standing around, nearest first.
fn visitors(ctx: &Ctx) -> Vec<Candidate> {
    let k = ctx.p.knight.pos;
    let mut v: Vec<Candidate> = ctx
        .senses
        .visitors
        .iter()
        .filter(|v| !v.greeted && v.state == VisitorState::Waiting)
        .map(|v| Candidate { prize: Prize::Visitor(v.id), target: v.pos, stand: stand_point(ctx, v), tol: 40.0 })
        .collect();
    v.sort_by(|a, b| a.target.distance(k).total_cmp(&b.target.distance(k)));
    v
}

/// Loose Geo, cheapest first (like Loot).
fn coins(ctx: &Ctx) -> Vec<Candidate> {
    let k = ctx.p.knight.pos;
    let range = ctx.cfg.stakes.loot_range;
    let mut c: Vec<Vec2> = ctx.stakes.sense.coins.iter().copied().filter(|c| c.distance(k) < range).collect();
    c.sort_by(|a, b| effort(k, *a).total_cmp(&effort(k, *b)));
    c.into_iter().map(|c| Candidate { prize: Prize::Coin, target: c, stand: c, tol: 12.0 }).collect()
}

/// Geo deposits he'd like to mine, nearest first.
fn deposits(ctx: &Ctx) -> Vec<Candidate> {
    let k = ctx.p.knight.pos;
    let mut d: Vec<Candidate> = ctx
        .interest
        .places
        .all()
        .iter()
        .filter(|p| p.kind == PlaceKind::Deposit && want(p, ctx.mind) > 0.05)
        .map(|p| Candidate { prize: Prize::Deposit(p.id), target: p.center, stand: p.at, tol: SEAT_TOL + 2.0 })
        .collect();
    d.sort_by(|a, b| a.stand.distance(k).total_cmp(&b.stand.distance(k)));
    d
}

/// Shinies, cheapest first (not while he's typing-busy: they wait).
fn shinies(ctx: &Ctx) -> Vec<Candidate> {
    if ctx.senses.user_busy {
        return vec![];
    }
    let k = ctx.p.knight.pos;
    let mut s: Vec<Candidate> = ctx
        .interest
        .shinies
        .all()
        .iter()
        .map(|s| Candidate { prize: Prize::Shiny(s.id), target: s.pos, stand: stand_spot(s, approach(s, k)), tol: PICK_TOL })
        .collect();
    s.sort_by(|a, b| effort(k, a.stand).total_cmp(&effort(k, b.stand)));
    s
}

/// The most precious prize no route reaches. Kinds are tried in order of value; a kind
/// with anything reachable ends the search (walk to that first, the gate can wait). The
/// Shade isn't one: the hunt takes its own gate (`ShadeHunt::way`), cooldown or not.
fn find_wish(ctx: &mut Ctx) -> Option<GateWish> {
    let min = ctx.cfg.dream_gate.min_distance;
    let kinds: [fn(&Ctx) -> Vec<Candidate>; 4] = [visitors, coins, deposits, shinies];
    let mut plans = MAX_PLANS;
    for kind in kinds {
        let mut wish = None;
        for c in kind(ctx).into_iter().take(PER_KIND) {
            if reachable(ctx, c.stand, c.tol, &mut plans) {
                log::trace!("dream gate: a {} at ({:.0},{:.0}) is within reach", c.prize.label(), c.stand.x, c.stand.y);
                return None;
            }
            if wish.is_some() || c.stand.distance(ctx.p.knight.pos) < min {
                continue;
            }
            match spot(ctx, c.stand) {
                Some(at) => wish = Some(GateWish { prize: c.prize, target: c.target, at }),
                None => log::trace!("dream gate: no gate spot by the {} at ({:.0},{:.0})", c.prize.label(), c.stand.x, c.stand.y),
            }
        }
        if wish.is_some() {
            return wish;
        }
    }
    None
}

impl Brain {
    /// Look for a prize worth a warp (call with the other scorings, before the utilities).
    pub(super) fn scan_dream_gate(&mut self, p: &Percepts, nav: &mut dyn Navigator) {
        let cfg = self.cfg.dream_gate;
        if matches!(self.active, Active::DreamGate(_)) {
            return;
        }
        if !cfg.enabled || self.now < self.stakes.gate.ready_at {
            self.stakes.gate.wish = None;
            return;
        }
        if ((self.now - self.stakes.gate.scanned_at) as f32) < cfg.scan_every {
            return;
        }
        self.stakes.gate.scanned_at = self.now;
        let s = &self.senses;
        let able = !s.fullscreen
            && p.knight.is_grounded()
            && !self.asleep(p)
            && self.mind.needs.energy >= self.cfg.tiers.critical_energy
            && self.active.kind().tier() != Tier::Urgent
            && self.stakes.pending_death.is_none();
        let wish = if able {
            let mut scratch = Intent::default();
            let mut ctx = self.ctx(p, nav, &mut scratch);
            find_wish(&mut ctx)
        } else {
            None
        };
        if let Some(w) = wish
            && self.stakes.gate.wish.map(|o| o.prize) != Some(w.prize)
        {
            log::debug!("dream gate: no route to the {} at ({:.0},{:.0}); a gate would do", w.prize.label(), w.target.x, w.target.y);
        }
        self.stakes.gate.wish = wish;
    }

    /// Utility of the DreamGate behaviour.
    pub(super) fn dream_gate_utility(&self) -> f32 {
        let running = matches!(self.active, Active::DreamGate(_));
        if running || self.stakes.gate.wish.is_some() { self.cfg.dream_gate.utility } else { 0.0 }
    }

    /// After a warp: tidy up what the behaviour can't reach (call after running it).
    pub(super) fn dream_gate_landed(&mut self) {
        if let Some(Prize::Visitor(id)) = self.stakes.gate.landed.take() {
            self.visitors.unsnub(id);
        }
    }

    /// Test panel: no cooldown, look again now (the next prize no route reaches gets a
    /// gate at once; the rest of the rules still apply).
    pub fn ready_dream_gate(&mut self) {
        self.stakes.gate.ready_at = self.now;
        self.stakes.gate.scanned_at = f64::NEG_INFINITY;
    }

    /// Warps so far.
    pub fn dream_gates(&self) -> u32 {
        self.stakes.gate.warps
    }
}

// --- the behaviour ------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Facing the prize, planting the gate there (`dreamgate_set`).
    Plant { act: Option<ActWait>, planted: bool },
    /// Charging and vanishing (`dreamgate_warp`); the app moves him as it ends.
    Warp(ActWait),
    /// Reappearing at the gate (`dreamgate_in`, started by the app).
    Arrive { waited: f32 },
}

/// Warp to an unreachable prize.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DreamGate {
    pub wish: GateWish,
    phase: Phase,
    /// The gate is planted and not yet used or closed.
    open: bool,
}

impl DreamGate {
    pub fn start(ctx: &mut Ctx, wish: GateWish) -> Self {
        ctx.nav.stop();
        let gate = &mut ctx.stakes.gate;
        gate.ready_at = ctx.now + ctx.cfg.dream_gate.cooldown as f64;
        gate.wish = None;
        log::info!("brain: no way to the {} at ({:.0},{:.0}); dream gate to ({:.0},{:.0})", wish.prize.label(), wish.target.x, wish.target.y, wish.at.x, wish.at.y);
        ctx.face_toward(wish.target.x);
        Self { wish, phase: Phase::Plant { act: ActWait::named(ctx, "dreamgate_set"), planted: false }, open: false }
    }

    /// Is the gate still open (to be closed if something else takes over)?
    pub fn open(&self) -> bool {
        self.open
    }

    fn close(&mut self, ctx: &mut Ctx) {
        if self.open {
            ctx.out.gate = Some(GateEvent::Close);
            self.open = false;
        }
    }

    fn give_up(&mut self, ctx: &mut Ctx, why: &str) -> Status {
        log::debug!("dream gate: {why}; gate closed");
        self.close(ctx);
        Status::Failed
    }

    /// The prize is still there.
    fn prize_here(&self, ctx: &Ctx) -> bool {
        match self.wish.prize {
            Prize::Visitor(id) => ctx.senses.visitors.iter().any(|v| v.id == id && !v.greeted && v.state != VisitorState::Leaving),
            Prize::Shade => ctx.stakes.sense.shade.is_some(),
            Prize::Coin => ctx.stakes.sense.coins.iter().any(|c| c.distance(self.wish.target) < 12.0),
            Prize::Deposit(id) => ctx.interest.places.get(id).is_some(),
            Prize::Shiny(id) => ctx.interest.shinies.get(id).is_some(),
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let at = self.wish.at;
        let here = self.prize_here(ctx);
        ctx.nav.stop();
        match self.phase {
            Phase::Plant { mut act, mut planted } => {
                if !here {
                    return self.give_up(ctx, "the prize went away");
                }
                ctx.face_toward(self.wish.target.x);
                // The gate appears as he reaches for it (or at once without the act).
                let done = act.as_mut().is_none_or(|w| w.tick(ctx));
                if !planted && (act.is_none_or(|w| w.started()) || done) {
                    planted = true;
                    self.open = true;
                    ctx.out.gate = Some(GateEvent::Set { at });
                }
                self.phase = Phase::Plant { act, planted };
                if done {
                    if spot(ctx, at).is_none_or(|s| s.distance(at) > 4.0) {
                        return self.give_up(ctx, "the gate's ledge went away");
                    }
                    let Some(w) = ActWait::named(ctx, crate::dreamgate::WARP_ACT) else {
                        return self.give_up(ctx, "no warp act");
                    };
                    self.phase = Phase::Warp(w);
                }
                Status::Running
            }
            Phase::Warp(mut w) => {
                if !w.started() && !here {
                    return self.give_up(ctx, "the prize went away");
                }
                let done = w.tick(ctx);
                self.phase = Phase::Warp(w);
                if !done {
                    return Status::Running;
                }
                if !w.started() {
                    return self.give_up(ctx, "the warp never started");
                }
                // The app used the gate up (moved him there, or let him reappear in place).
                self.open = false;
                self.phase = Phase::Arrive { waited: 0.0 };
                Status::Running
            }
            Phase::Arrive { waited } => {
                let waited = waited + ctx.dt();
                self.phase = Phase::Arrive { waited };
                let there = ctx.p.knight.pos.distance(at) < 24.0 && ctx.grounded();
                if there && ctx.p.act.is_none() {
                    self.land(ctx);
                    return Status::Done;
                }
                if waited > ARRIVE_SECS {
                    return self.give_up(ctx, if there { "stuck mid-arrival" } else { "reappeared where he stood" });
                }
                Status::Running
            }
        }
    }

    /// At the gate: the prize is reachable again for the behaviour that wants it.
    fn land(&mut self, ctx: &mut Ctx) {
        let w = self.wish;
        log::info!("brain: dream-gated to the {} at ({:.0},{:.0})", w.prize.label(), w.at.x, w.at.y);
        ctx.interest.unblock(PoiTarget::Spot(w.at));
        ctx.interest.unblock(PoiTarget::Spot(w.target));
        match w.prize {
            Prize::Shade => ctx.stakes.shade_ready_at = ctx.now,
            Prize::Coin => {
                ctx.interest.unskip_coin(w.target);
                ctx.stakes.loot_ready_at = ctx.now;
            }
            Prize::Shiny(id) => ctx.interest.shinies.forget_try(id),
            Prize::Deposit(_) | Prize::Visitor(_) => {}
        }
        ctx.interest.places.rescore();
        ctx.stakes.gate.landed = Some(w.prize);
        ctx.stakes.gate.warps += 1;
        ctx.mind.excite(0.15);
    }

    pub fn reason(&self) -> String {
        let what = self.wish.prize.label();
        match self.phase {
            Phase::Plant { .. } => format!("dream gate: no route to the {what}; planting a gate by it"),
            Phase::Warp(_) => format!("dream gate: warping to the {what}"),
            Phase::Arrive { .. } => format!("dream gate: arriving by the {what}"),
        }
    }
}
