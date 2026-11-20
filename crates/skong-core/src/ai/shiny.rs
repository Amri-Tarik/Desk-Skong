//! Shinies on the ground (see [`crate::shiny`]) and the **Shiny** behaviour: the knight
//! spots a glint, looks at it ("?"), walks over, faces it, kneels and picks it up
//! (`pick_up_trinket`). The app removes the shiny at the act's "Collect Normal 2" step.
//!
//! An engagement like Loot, a little below it: Geo lying around and visitors come first
//! (he leaves the shiny for later), and it never wakes him, interrupts a nap, or happens
//! while the user types hard or a fullscreen app is up.
use std::collections::BTreeMap;

use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Active, Ctx, Status, Travel, TravelStep, on_point, style_for};
use super::interest::PoiTarget;
use super::navigator::Navigator;
use super::percept::Percepts;
use super::places::DiscoveryEvent;
use super::shade_hunt::effort;
use super::Brain;

/// A shiny on screen, as the app reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownShiny {
    pub id: u64,
    /// Bottom centre of the glint, on its ledge.
    pub pos: Vec2,
    /// The ledge it lies on (x0, x1).
    pub span: (f32, f32),
    /// Stand this far beside it to pick it up (px).
    pub reach: f32,
}

/// Arrival tolerance at the pick-up spot (px).
pub(crate) const PICK_TOL: f32 = 8.0;
/// A shiny he went for and didn't get is left alone this long (s).
const RETRY_SECS: f32 = 90.0;
/// Utility ceiling: below Geo (Loot starts at 0.6 and climbs) and visitors (0.86+).
const MAX_UTILITY: f32 = 0.72;

/// The best shiny to go for right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShinyChoice {
    pub shiny: KnownShiny,
    /// Where to stand.
    pub at: Vec2,
    pub eta: f32,
}

/// Shinies on screen and what the knight did about them (lives in the interest map).
#[derive(Clone, Debug, Default)]
pub struct ShinyMemory {
    shinies: Vec<KnownShiny>,
    tried: BTreeMap<u64, f64>,
    pub(crate) best: Option<ShinyChoice>,
}

impl ShinyMemory {
    pub fn all(&self) -> &[KnownShiny] {
        &self.shinies
    }

    pub fn get(&self, id: u64) -> Option<KnownShiny> {
        self.shinies.iter().find(|s| s.id == id).copied()
    }

    /// Forget a failed attempt (he got there by other means: a Dream Gate).
    pub(crate) fn forget_try(&mut self, id: u64) {
        self.tried.remove(&id);
    }
}

/// Where to stand to pick `s` up from `side` (+1: to its right, −1: to its left), kept on
/// its ledge.
pub(crate) fn stand_spot(s: &KnownShiny, side: f32) -> Vec2 {
    let x = s.pos.x + side * s.reach;
    let (lo, hi) = (s.span.0 + 4.0, s.span.1 - 4.0);
    Vec2::new(if lo < hi { x.clamp(lo, hi) } else { s.pos.x }, s.pos.y)
}

/// The side he comes from.
pub(crate) fn approach(s: &KnownShiny, from: Vec2) -> f32 {
    if from.x >= s.pos.x { 1.0 } else { -1.0 }
}

impl Brain {
    /// The shinies on screen (call before [`Brain::tick`], every tick or on change).
    pub fn set_shinies(&mut self, shinies: &[KnownShiny]) {
        let mem = &mut self.interest.shinies;
        mem.shinies.clear();
        mem.shinies.extend_from_slice(shinies);
        let now = self.now;
        mem.tried.retain(|id, t| ((now - *t) as f32) < RETRY_SECS && shinies.iter().any(|s| s.id == *id));
    }

    /// The shiny he is going for, if any.
    pub fn shiny_target(&self) -> Option<u64> {
        match &self.active {
            Active::Shiny(b) => Some(b.shiny.id),
            _ => None,
        }
    }

    /// Rank shinies: closest by effort first, only spots a route really gets him to.
    pub(crate) fn score_shinies(&mut self, p: &Percepts, nav: &mut dyn Navigator) {
        let k = p.knight;
        let style = self.mind.travel_style();
        let current = self.shiny_target();
        let mem = &self.interest.shinies;
        let mut ranked: Vec<(KnownShiny, Vec2)> = mem
            .shinies
            .iter()
            .filter(|s| Some(s.id) == current || !mem.tried.contains_key(&s.id))
            .map(|s| (*s, stand_spot(s, approach(s, k.pos))))
            .filter(|(_, at)| !self.interest.is_blocked(PoiTarget::Spot(*at)))
            .collect();
        ranked.sort_by(|a, b| effort(k.pos, a.1).total_cmp(&effort(k.pos, b.1)));
        self.interest.shinies.best = ranked
            .into_iter()
            .take(2)
            .filter_map(|(shiny, at)| {
                let eta = if on_point(k, at, PICK_TOL) {
                    0.0
                } else {
                    let r = nav.reach(k, NavGoal::Point(at), style)?;
                    if !r.lands_on(at, PICK_TOL) {
                        return None;
                    }
                    r.eta
                };
                Some(ShinyChoice { shiny, at, eta })
            })
            .min_by(|a, b| a.eta.total_cmp(&b.eta));
    }

    /// Utility of the Shiny behaviour: curiosity, a bit more when bored, never while
    /// asleep / napping, typing-busy, fullscreen, or (to start) in mid-air.
    pub(crate) fn shiny_utility(&self, p: &Percepts) -> f32 {
        let s = &self.senses;
        let Some(c) = self.interest.shinies.best else { return 0.0 };
        let running = matches!(self.active, Active::Shiny(_));
        if s.fullscreen || s.user_busy || self.asleep(p) || matches!(self.active, Active::Nap(_)) || (!running && !p.knight.is_grounded()) {
            return 0.0;
        }
        let m = &self.mind;
        let want = 0.5 + 0.15 * m.personality.curiosity + 0.1 * m.needs.boredom + 0.05 * m.emotions.excitement;
        (want - 0.01 * c.eta).clamp(0.0, MAX_UTILITY)
    }
}

// --- the behaviour ------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Spotted: look at it a moment.
    Notice { act: Option<ActWait>, left: f32 },
    Travel(Travel),
    /// Beside it: turn to it, then kneel and pick it up.
    Pick { turn: f32, act: Option<ActWait> },
}

/// Go and pick up a shiny.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PickShiny {
    pub shiny: KnownShiny,
    /// The side he picks it up from (+1 right of it, −1 left).
    side: f32,
    phase: Phase,
    /// Seconds it has been missing (hidden a moment, or gone for good).
    missing: f32,
}

impl PickShiny {
    pub fn start(ctx: &mut Ctx, choice: ShinyChoice) -> Self {
        let s = choice.shiny;
        ctx.mind.excite(0.15);
        ctx.mind.adjust(|n| &mut n.boredom, -0.05);
        let eye = ctx.p.knight.body().center();
        let dy = s.pos.y - eye.y;
        let act = if dy < -100.0 {
            ActWait::named(ctx, "look_up")
        } else if dy > 120.0 {
            ActWait::named(ctx, "look_down")
        } else {
            None
        };
        log::debug!("brain: spotted a shiny at ({:.0},{:.0})", s.pos.x, s.pos.y);
        Self { shiny: s, side: approach(&s, ctx.p.knight.pos), phase: Phase::Notice { act, left: 0.7 }, missing: 0.0 }
    }

    fn at(&self) -> Vec2 {
        stand_spot(&self.shiny, self.side)
    }

    /// Geo lying around within reach: that comes first (the shiny waits).
    fn geo_first(ctx: &Ctx) -> bool {
        let k = ctx.p.knight.pos;
        let range = ctx.cfg.stakes.loot_range;
        ctx.now >= ctx.stakes.loot_ready_at && ctx.stakes.sense.coins.iter().any(|c| c.distance(k) < range)
    }

    fn give_up(&self, ctx: &mut Ctx) {
        ctx.interest.shinies.tried.insert(self.shiny.id, ctx.now);
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let picking = matches!(self.phase, Phase::Pick { act: Some(w), .. } if w.started());
        match ctx.interest.shinies.get(self.shiny.id) {
            Some(s) => {
                self.shiny = s;
                self.missing = 0.0;
            }
            // Picked up mid-act: let the act finish.
            None if picking => {}
            None => {
                self.missing += ctx.dt();
                if self.missing > 1.0 {
                    ctx.nav.stop();
                    return Status::Done;
                }
            }
        }
        if !matches!(self.phase, Phase::Pick { .. }) && (ctx.senses.user_busy || Self::geo_first(ctx)) {
            ctx.nav.stop();
            return Status::Done;
        }
        let (s, at) = (self.shiny, self.at());
        match &mut self.phase {
            Phase::Notice { act, left } => {
                ctx.nav.stop();
                ctx.face_toward(s.pos.x);
                *left -= ctx.dt();
                if act.as_mut().is_none_or(|w| w.tick(ctx)) && *left <= 0.0 {
                    self.phase = if on_point(ctx.p.knight, at, PICK_TOL) {
                        Phase::Pick { turn: 0.15, act: ActWait::named(ctx, "pick_up_trinket") }
                    } else {
                        let style = match style_for(ctx, at) {
                            TravelStyle::Stroll if ctx.mind.needs.energy > 0.4 => TravelStyle::Normal,
                            st => st,
                        };
                        Phase::Travel(Travel::exact(ctx, at, style, PICK_TOL))
                    };
                }
                Status::Running
            }
            Phase::Travel(t) => {
                if let NavGoal::Point(g) = t.goal
                    && g.distance(at) > 6.0
                {
                    t.retarget(NavGoal::Point(at));
                }
                match t.tick(ctx) {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Failed => {
                        self.give_up(ctx);
                        Status::Failed
                    }
                    TravelStep::Arrived => {
                        ctx.face_toward(s.pos.x);
                        self.phase = Phase::Pick { turn: 0.15, act: ActWait::named(ctx, "pick_up_trinket") };
                        Status::Running
                    }
                }
            }
            Phase::Pick { turn, act } => {
                ctx.nav.stop();
                // Face it first: facing only applies while no act plays.
                if *turn > 0.0 {
                    *turn -= ctx.dt();
                    ctx.face_toward(s.pos.x);
                    return Status::Running;
                }
                let Some(w) = act else {
                    self.give_up(ctx);
                    return Status::Done;
                };
                if !w.tick(ctx) {
                    if !w.started() {
                        ctx.face_toward(s.pos.x);
                    }
                    ctx.out.discovery = Some(DiscoveryEvent::PickUp(s.id));
                    return Status::Running;
                }
                if w.started() {
                    ctx.mind.adjust(|n| &mut n.boredom, -0.25);
                    ctx.mind.excite(0.1);
                }
                self.give_up(ctx); // got it (or couldn't): don't go for this one again
                Status::Done
            }
        }
    }

    pub fn reason(&self) -> String {
        match self.phase {
            Phase::Notice { .. } => "shiny: something glinting over there?".into(),
            Phase::Travel(t) => format!("shiny: walking over to it ({:?})", t.style),
            Phase::Pick { .. } => "shiny: picking it up".into(),
        }
    }
}
