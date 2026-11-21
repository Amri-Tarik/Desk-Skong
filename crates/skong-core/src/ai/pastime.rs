//! Pastimes: what the knight does on his own when keeping the user company has gone on a
//! while and he's getting restless (the props are `crate::pastime`):
//!
//! * **Training dummy**: one stands on a settled ledge now and then. He walks over, squares
//!   up, and practises: three-slash combos, up slashes, pogos off its head, nail arts (great
//!   / cyclone / dash slash), a fireball. Each blow makes it swing ([`PastimeEvent::DummyHit`]).
//! * **Chasing a lumafly**: a glowing fly drifting from window to window. He follows it along
//!   the ledges under it (climbing where the route climbs), jumps at it when it's within
//!   reach and tries to catch it ([`PastimeEvent::Swipe`]); it darts off, or slips out of his
//!   hands and floats away while he watches.
//! * **Taking in the view**: up to the end of a high window edge, sitting there looking out
//!   and down over the desktop.
//! * **Stargazing** (night only): on a high ledge with open sky above, looking up a long while.
//!
//! All Ambient: the Shade, fights, visitors, Geo, shinies and deposits always come first (a
//! higher tier preempts at once), and never while the user types hard or a fullscreen app is
//! up. A dummy or a lumafly that's there, reachable and off its cooldown beats plain company
//! (and idling) at once, with a utility well above companion's; the quiet ones (the view, the
//! stars) are driven by boredom plus restlessness from a long stretch of plain company. Each
//! pastime has a cooldown so they stay treats, and doing one settles boredom so he goes back
//! to keeping company afterwards. Once started a pastime keeps its starting utility until
//! done (no dithering back to company).
use std::collections::BTreeMap;

use crate::controller::{Character, MoveInput, Side};
use crate::geom::{RectF, Vec2};
use crate::nav::{NavGoal, TravelStyle};
use crate::pastime::DummyHit;
use crate::terrain::LedgeKind;

use super::behaviours::{ActWait, Active, BehaviourKind, Ctx, SEAT_TOL, Status, Travel, TravelStep, on_point};
use super::candidates::Why;
use super::combat::{Mark, Stance, pogo_now};
use super::interest::PoiTarget;
use super::navigator::{Navigator, Reach};
use super::percept::Percepts;
use super::{Attack, AttackDir, Brain, Motion, spots};

/// The training dummy, as the app reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownDummy {
    pub id: u64,
    /// Bottom centre (the pole's foot, on its ledge).
    pub pos: Vec2,
    /// Its ledge (x0, x1).
    pub span: (f32, f32),
    /// The stuffed figure (what blows hit; screen px).
    pub body: RectF,
}

/// A lumafly, as the app reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownLumafly {
    pub id: u64,
    /// Where it glows (screen px).
    pub pos: Vec2,
    /// Caught (slipped away) or its time is up: floating off, not worth chasing.
    pub leaving: bool,
}

/// What the app should do for a pastime (see [`Brain::take_pastime_events`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PastimeEvent {
    /// A blow lands on the dummy after `delay` seconds.
    DummyHit { hit: DummyHit, delay: f32 },
    /// He jumped and cupped his hands at lumafly `fly` from `from` (his body centre).
    Swipe { fly: u64, from: Vec2 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PastimeKind {
    Dummy,
    Lumafly,
    View,
    Stargaze,
}

impl PastimeKind {
    pub fn label(self) -> &'static str {
        match self {
            PastimeKind::Dummy => "training dummy",
            PastimeKind::Lumafly => "lumafly",
            PastimeKind::View => "taking in the view",
            PastimeKind::Stargaze => "stargazing",
        }
    }

    /// A dummy or a lumafly (not the quiet ones).
    pub(crate) fn lively(self) -> bool {
        matches!(self, PastimeKind::Dummy | PastimeKind::Lumafly)
    }

    /// Seconds before the same pastime again (after one ends).
    fn cooldown(self) -> f32 {
        match self {
            PastimeKind::Dummy => 120.0,
            PastimeKind::Lumafly => 40.0,
            PastimeKind::View => 200.0,
            PastimeKind::Stargaze => 300.0,
        }
    }
}

/// The best pastime to take up right now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PastimeChoice {
    pub kind: PastimeKind,
    /// Where to go (stand spot / seat / ledge under the fly).
    pub at: Vec2,
    pub eta: f32,
    pub value: f32,
    /// The dummy or lumafly id.
    pub target: u64,
    /// Whose edge (process name), for the reason line.
    pub place: String,
    /// The travel style whose route was checked to land on `at` (he travels with it).
    pub style: TravelStyle,
}

/// What the brain knows and remembers about pastimes (lives in the interest map).
#[derive(Clone, Debug, Default)]
pub struct PastimeMemory {
    dummy: Option<KnownDummy>,
    flies: Vec<KnownLumafly>,
    events: Vec<PastimeEvent>,
    ready_at: BTreeMap<PastimeKind, f64>,
    tried_flies: BTreeMap<u64, f64>,
    pub(crate) best: Option<PastimeChoice>,
    /// Seat / ledge scans for the view and the stars (route planning): cached a while.
    scanned_at: Option<f64>,
    view: Option<PastimeChoice>,
    stars: Option<PastimeChoice>,
    /// Seconds of plain company / idling in a row (restlessness).
    company: f32,
    last_now: f64,
    last_kind: Option<PastimeKind>,
    /// The value the running pastime started with (it keeps it until done).
    running_value: f32,
    /// Test panel: take up this quiet pastime (the view, the stars) as soon as a spot is
    /// found, skipping the restlessness, the cooldown and (stars) the hour; with the brain
    /// time it was asked at.
    forced: Option<(PastimeKind, f64)>,
    /// What came of a forced pastime, for the panel (see [`Brain::take_test_notes`]).
    notes: Vec<String>,
    /// A dummy he set out for and couldn't get to: left alone until then (pastimes keep
    /// their own memory; the shared failure map is coarse and would hide it).
    dummy_skip: Option<(u64, f64)>,
    /// The dummy he would like to practise on but can't get to, and since when.
    dummy_unreached: Option<(u64, f64)>,
    /// What he last made of each dummy (logged when it changes, to diagnose live).
    dummy_view: Option<(u64, &'static str)>,
    /// The dummy that broke last (the app told him): practising on it, he stops with a look.
    dummy_broke: Option<u64>,
    /// The lumafly last route-checked and what came of it (diagnostics, `ai::candidates`).
    fly_view: Option<(u64, Why)>,
}

impl PastimeMemory {
    pub(crate) fn dummy(&self) -> Option<KnownDummy> {
        self.dummy
    }

    pub(crate) fn fly(&self, id: u64) -> Option<KnownLumafly> {
        self.flies.iter().find(|f| f.id == id).copied()
    }

    fn push(&mut self, ev: PastimeEvent) {
        if self.events.len() < 32 {
            self.events.push(ev);
        }
    }

    /// Log what he makes of dummy `id` whenever that changes (why he ignores it, or that he
    /// would practise on it).
    fn note_dummy(&mut self, id: u64, what: &'static str, now: f64) {
        if self.dummy_view != Some((id, what)) {
            self.dummy_view = Some((id, what));
            let at = self.dummy.map_or(Vec2::ZERO, |d| d.pos);
            log::info!("brain: training dummy #{id} at ({:.0},{:.0}): {what} (t {now:.0})", at.x, at.y);
        }
    }

    /// Until when `kind` is on its cooldown (brain time), if it is.
    pub(crate) fn cooling(&self, kind: PastimeKind, now: f64) -> Option<f64> {
        self.ready_at.get(&kind).copied().filter(|&t| now < t)
    }

    /// What he last made of dummy `id` (when it isn't cooling or the chosen pastime).
    pub(crate) fn dummy_verdict(&self, id: u64) -> Why {
        match self.dummy_skip {
            Some((d, until)) if d == id => Why::GaveUp { until },
            _ => match self.dummy_view {
                Some((d, what)) if d == id => Why::Note(what),
                _ => Why::Note("not looked at yet"),
            },
        }
    }

    /// The verdict on each lumafly worth chasing (not floating off): cooling, given up on,
    /// the route check of the nearest, or a nearer one first. `Why::Planned`: chaseable.
    pub(crate) fn fly_verdicts(&self, now: f64) -> impl Iterator<Item = (u64, Why)> + '_ {
        let cooling = self.cooling(PastimeKind::Lumafly, now);
        self.flies.iter().filter(|f| !f.leaving).map(move |f| {
            let why = if let Some(until) = cooling {
                Why::Cooling { until }
            } else if let Some(&t) = self.tried_flies.get(&f.id) {
                Why::GaveUp { until: t + 60.0 }
            } else {
                match self.fly_view {
                    Some((id, why)) if id == f.id => why,
                    _ => Why::Outranked,
                }
            };
            (f.id, why)
        })
    }

    fn rest(&mut self, kind: PastimeKind, now: f64) {
        self.ready_at.insert(kind, now + kind.cooldown() as f64);
    }
}

/// Stand this far (units) from the dummy's figure edge to strike it.
const STRIKE_GAP: f32 = 0.55;
/// Arrival tolerance at the dummy stand spot (px).
const STAND_TOL: f32 = 10.0;
/// Travel styles tried for a dummy stand spot, gentlest first.
const DUMMY_STYLES: [TravelStyle; 2] = [TravelStyle::Stroll, TravelStyle::Normal];
/// A dummy he set out for and couldn't reach is left alone this long (s).
const DUMMY_SKIP_SECS: f64 = 90.0;
/// He jumps at a lumafly at most this far above his body centre (units; with the wings).
const FLY_JUMP_REACH: f32 = 4.0;
/// ... and at most this far to the side (units).
const FLY_JUMP_SIDE: f32 = 1.4;
/// Re-scan seats / high ledges for the view and the stars this often (s).
const SCAN_EVERY: f32 = 3.0;
/// The view: seats on window edges at least this high above the work area's bottom, as a
/// share of its height.
const VIEW_HEIGHT: f32 = 0.4;
/// Stargazing: open sky above the ledge this far (px at scale 1).
const OPEN_SKY: f32 = 220.0;
/// The quiet pastimes (view, stars) come up after this long of plain company / idling (s),
/// or when bored stiff. The dummy and lumaflies don't wait.
const MIN_COMPANY: f32 = 25.0;
/// Utility floor and cap of the lively pastimes (dummy, lumafly): above plain company (at
/// most ≈ 0.62) plus the hysteresis margin (0.12) even after the travel cost, and below the
/// scarlet flame the user summoned (`ai::scene`, 0.8) and the fun the user starts (battle,
/// pester invites) at full tilt. Whichever of dummy / flame starts first is finished first
/// (the hysteresis margin keeps the other from cutting in), so neither starves the other.
const LIVELY_MIN: f32 = 0.78;
const LIVELY_MAX: f32 = 0.79;
/// A forced pastime (test panel) that hasn't started after this long is dropped (s).
const FORCED_PATIENCE: f64 = 20.0;
/// Night hours (local): from .. to.
const NIGHT: (f32, f32) = (21.0, 5.0);

fn night(p: &Percepts) -> bool {
    p.local_hour.is_some_and(|h| h >= NIGHT.0 || h < NIGHT.1)
}

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where he stands to strike the dummy from `side` (+1 right of it, −1 left), on its ledge.
fn stand_spot(d: &KnownDummy, side: f32, ppu: f32) -> Vec2 {
    let half = 0.5 * d.body.width();
    let x = d.pos.x + side * (half + STRIKE_GAP * ppu);
    let (lo, hi) = (d.span.0 + 6.0, d.span.1 - 6.0);
    Vec2::new(if lo < hi { x.clamp(lo, hi) } else { d.pos.x }, d.pos.y)
}

/// Where and how he'd go to practise on a dummy (see [`dummy_stand`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DummyStand {
    pub at: Vec2,
    pub style: TravelStyle,
    pub eta: f32,
}

/// The brain's test for a training dummy, shared with the app's placement so the two always
/// agree: a stand spot beside it (the side he comes from first) that he stands on already,
/// or that a route with the gentlest style that does ([`DUMMY_STYLES`]) lands on within
/// the arrival tolerance. `reach` plans from where he is (`Navigator::reach`).
pub fn dummy_stand(d: &KnownDummy, k: &Character, reach: &mut dyn FnMut(Vec2, TravelStyle) -> Option<Reach>) -> Option<DummyStand> {
    let ppu = k.params.px_per_unit;
    let side = if k.pos.x >= d.pos.x { 1.0 } else { -1.0 };
    let spots = [stand_spot(d, side, ppu), stand_spot(d, -side, ppu)];
    if let Some(&at) = spots.iter().find(|&&at| on_point(k, at, STAND_TOL)) {
        return Some(DummyStand { at, style: TravelStyle::Stroll, eta: 0.0 });
    }
    spots.into_iter().find_map(|at| DUMMY_STYLES.into_iter().find_map(|style| reach(at, style).filter(|r| r.lands_on(at, STAND_TOL)).map(|r| DummyStand { at, style, eta: r.eta })))
}

impl Brain {
    /// The pastime props on screen (call before [`Brain::tick`]).
    pub fn set_pastimes(&mut self, dummy: Option<KnownDummy>, flies: &[KnownLumafly]) {
        let mem = &mut self.interest.pastimes;
        mem.dummy = dummy;
        mem.flies.clear();
        mem.flies.extend_from_slice(flies);
        let now = self.now;
        mem.tried_flies.retain(|id, t| now - *t < 60.0 && flies.iter().any(|f| f.id == *id));
    }

    /// Dummy blows and lumafly swipes since the last call.
    pub fn take_pastime_events(&mut self) -> Vec<PastimeEvent> {
        std::mem::take(&mut self.interest.pastimes.events)
    }

    /// Test panel: take in the view / stargaze now (the restlessness, the cooldown and the
    /// night hours are skipped; a settled, reachable spot is still needed). Lively pastimes
    /// come with their props (the app puts up a dummy / sends a lumafly instead).
    pub fn force_pastime(&mut self, kind: PastimeKind) {
        let mem = &mut self.interest.pastimes;
        mem.ready_at.remove(&kind);
        mem.scanned_at = None;
        mem.forced = Some((kind, self.now));
    }

    /// What came of the test panel's requests since the last call (one line each).
    pub fn take_test_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.interest.pastimes.notes)
    }

    /// The training dummy he'd like to practise on but can't get to (no stand spot a route
    /// lands on, or he set out and failed), and for how many seconds: the app moves it.
    pub fn dummy_unreachable(&self) -> Option<(u64, f32)> {
        if matches!(self.active, Active::Pastime(_)) {
            return None;
        }
        self.interest.pastimes.dummy_unreached.map(|(id, since)| (id, (self.now - since) as f32))
    }

    /// The user moved dummy `id` (dragged it): whether he can get to it is judged afresh,
    /// and his verdict logged again.
    pub fn dummy_moved(&mut self, id: u64) {
        let mem = &mut self.interest.pastimes;
        if mem.dummy_skip.is_some_and(|(d, _)| d == id) {
            mem.dummy_skip = None;
        }
        if mem.dummy_unreached.is_some_and(|(d, _)| d == id) {
            mem.dummy_unreached = None;
        }
        if mem.dummy_view.is_some_and(|(d, _)| d == id) {
            mem.dummy_view = None;
        }
        // Lifted from under his nail mid-drill: no cooldown for that (he may go on).
        mem.ready_at.remove(&PastimeKind::Dummy);
    }

    /// Dummy `id` broke (worn out, under his nail or the user's clicks): if he was practising
    /// on it he stops gracefully (the last swing finishes, a look down at the pieces, then
    /// back to whatever's next). Call when it breaks; it's gone from the percepts from then.
    pub fn dummy_broken(&mut self, id: u64) {
        self.interest.pastimes.dummy_broke = Some(id);
    }

    /// The pastime under way, if any.
    pub fn pastime(&self) -> Option<PastimeKind> {
        match &self.active {
            Active::Pastime(b) => Some(b.kind),
            _ => None,
        }
    }

    /// Find the best pastime and where to do it (route-planned).
    pub(crate) fn score_pastimes(&mut self, p: &Percepts, nav: &mut dyn Navigator) {
        let now = self.now;
        let kind = self.active.kind();
        let running = match &self.active {
            Active::Pastime(b) => Some(b.kind),
            _ => None,
        };
        let mem = &mut self.interest.pastimes;
        if let Some((fk, asked)) = mem.forced {
            if running == Some(fk) {
                mem.notes.push(format!("{}: started", fk.label()));
                mem.forced = None;
            } else if now - asked > FORCED_PATIENCE {
                mem.notes.push(format!("{}: didn't start within {FORCED_PATIENCE:.0} s (busy with {kind:?})", fk.label()));
                mem.forced = None;
            }
        }
        let dt = (now - mem.last_now).max(0.0) as f32;
        mem.last_now = now;
        mem.company = match kind {
            BehaviourKind::Companion | BehaviourKind::Idle => mem.company + dt,
            // Sitting with him counts too, but slower.
            BehaviourKind::Rest => mem.company + 0.5 * dt,
            _ => 0.0,
        };
        if kind == BehaviourKind::Pastime {
            return; // committed: the choice is the running one
        }
        // The quiet ones (the view, the stars) only once he's been keeping company a while,
        // or is bored stiff. A dummy or a lumafly right there is reason enough on its own.
        let restless_enough = mem.company >= MIN_COMPANY || self.mind.needs.boredom >= self.cfg.boredom.bored;
        let k = p.knight;
        let ppu = k.params.px_per_unit;
        let style = self.mind.travel_style();
        let ready = |m: &PastimeMemory, kind: PastimeKind| m.ready_at.get(&kind).is_none_or(|&t| now >= t);
        let mut options: Vec<PastimeChoice> = Vec::new();

        // The dummy: stand beside it, on the side he comes from (its own skip memory, not the
        // shared failure map: that one is coarse and kept dummies out of mind for good).
        let mem = &mut self.interest.pastimes;
        mem.dummy_skip = mem.dummy_skip.filter(|&(id, until)| now < until && mem.dummy.is_some_and(|d| d.id == id));
        match mem.dummy {
            Some(d) if ready(mem, PastimeKind::Dummy) => {
                let stand = if mem.dummy_skip.is_some() { None } else { dummy_stand(&d, k, &mut |at, st| nav.reach(k, NavGoal::Point(at), st)) };
                match stand {
                    Some(s) => {
                        mem.dummy_unreached = None;
                        mem.note_dummy(d.id, "practisable", now);
                        options.push(PastimeChoice { kind: PastimeKind::Dummy, at: s.at, eta: s.eta, value: 0.0, target: d.id, place: String::new(), style: s.style });
                    }
                    None => {
                        let why = if mem.dummy_skip.is_some() { "skipped: he set out for it and couldn't get there" } else { "unreachable: no stand spot beside it that a stroll or a run lands on" };
                        mem.note_dummy(d.id, why, now);
                        if mem.dummy_unreached.is_none_or(|(id, _)| id != d.id) {
                            mem.dummy_unreached = Some((d.id, now));
                        }
                    }
                }
            }
            Some(d) => {
                mem.dummy_unreached = None;
                mem.note_dummy(d.id, "on its cooldown (he practised a little while ago)", now);
            }
            None => mem.dummy_unreached = None,
        }

        // A lumafly: the ledge under it.
        if ready(&self.interest.pastimes, PastimeKind::Lumafly) {
            let mem = &self.interest.pastimes;
            let fly = mem.flies.iter().filter(|f| !f.leaving && !mem.tried_flies.contains_key(&f.id)).min_by(|a, b| a.pos.distance(k.pos).total_cmp(&b.pos.distance(k.pos))).copied();
            // The same checks as one chain, each outcome noted for the passed-over list.
            let mut view = None;
            if let Some(f) = fly {
                view = Some((f.id, Why::Note("no ledge under it")));
                if let Some(under) = spots::project(p.terrain, f.pos) {
                    view = Some((f.id, Why::NoRoute(style)));
                    if let Some(r) = nav.reach(k, NavGoal::Point(under), style) {
                        view = Some((f.id, Why::Misses));
                        if r.end.is_none_or(|e| e.distance(under) < 3.0 * ppu) {
                            view = Some((f.id, Why::Planned));
                            options.push(PastimeChoice { kind: PastimeKind::Lumafly, at: under, eta: r.eta, value: 0.0, target: f.id, place: String::new(), style });
                        }
                    }
                }
            }
            self.interest.pastimes.fly_view = view;
        } else {
            self.interest.pastimes.fly_view = None;
        }

        // The view and the stars: re-scanned every few seconds.
        if !restless_enough {
            self.interest.pastimes.view = None;
            self.interest.pastimes.stars = None;
        } else if self.interest.pastimes.scanned_at.is_none_or(|t| (now - t) as f32 >= SCAN_EVERY) {
            self.interest.pastimes.scanned_at = Some(now);
            let view = if ready(&self.interest.pastimes, PastimeKind::View) { self.find_view(p, nav) } else { None };
            let stars = if night(p) && ready(&self.interest.pastimes, PastimeKind::Stargaze) { self.find_sky(p, nav) } else { None };
            self.interest.pastimes.view = view;
            self.interest.pastimes.stars = stars;
        }
        let mem = &self.interest.pastimes;
        if ready(mem, PastimeKind::View) {
            options.extend(mem.view.clone());
        }
        if night(p) && ready(mem, PastimeKind::Stargaze) {
            options.extend(mem.stars.clone());
        }

        let m = &self.mind;
        let (n, pers, e) = (m.needs, m.personality, m.emotions);
        let restless = smooth(MIN_COMPANY, 120.0, mem.company);
        let base = 0.28 + 0.4 * n.boredom + 0.22 * restless;
        let last = mem.last_kind;
        for o in &mut options {
            let (bonus, ok) = match o.kind {
                PastimeKind::Dummy => (0.1 + 0.12 * n.mischief + 0.06 * pers.vigour, n.energy > 0.35 && e.hurt < 0.3),
                PastimeKind::Lumafly => (0.14 + 0.1 * pers.curiosity + 0.05 * e.excitement, n.energy > 0.2),
                PastimeKind::View => (0.04 + 0.08 * (1.0 - m.arousal()) + if m.tiredness() > 0.4 { 0.05 } else { 0.0 }, true),
                PastimeKind::Stargaze => (0.12, true),
            };
            let again = if Some(o.kind) == last { 0.05 } else { 0.0 };
            o.value = match o.kind {
                // A dummy or a lumafly is more fun than plain company, right away: well
                // above companion's utility (≈ 0.42–0.62) plus the switching margin, still
                // far below every engagement (a higher tier preempts it anyway).
                PastimeKind::Dummy | PastimeKind::Lumafly if ok => ((base + bonus).max(LIVELY_MIN) - (0.01 * o.eta).min(0.03)).min(LIVELY_MAX),
                _ if ok => (base + bonus - again - 0.01 * o.eta).clamp(0.0, 0.8),
                _ => 0.0,
            };
        }
        if let Some(o) = options.iter().find(|o| o.kind == PastimeKind::Dummy && o.value <= 0.0) {
            self.interest.pastimes.note_dummy(o.target, "not up to it: too tired or too sore", now);
        }
        if let Some(o) = options.iter().find(|o| o.kind == PastimeKind::Lumafly && o.value <= 0.0) {
            self.interest.pastimes.fly_view = Some((o.target, Why::Note("not up to it: worn out")));
        }
        options.retain(|o| o.value > 0.0);
        // Test panel: the view / the stars now, if there is a spot for them.
        if let Some((fk @ (PastimeKind::View | PastimeKind::Stargaze), _)) = self.interest.pastimes.forced {
            let found = if fk == PastimeKind::View { self.find_view(p, nav) } else { self.find_sky(p, nav) };
            match found {
                Some(c) => options.push(PastimeChoice { value: LIVELY_MAX, ..c }),
                None => {
                    let why = match fk {
                        PastimeKind::View => "no settled window edge high on his monitor (upper 60 %, 120 px wide, room for his legs) that he can reach",
                        _ => "no settled ledge with open sky above it that he can reach on his monitor",
                    };
                    let mem = &mut self.interest.pastimes;
                    mem.notes.push(format!("can't start {}: {why}", fk.label()));
                    mem.forced = None;
                }
            }
        }
        let best = options.into_iter().max_by(|a, b| a.value.total_cmp(&b.value));
        self.interest.pastimes.best = best;
    }

    /// The end of a high window edge to sit on (the end with room to dangle his legs),
    /// reachable; on his monitor.
    fn find_view(&self, p: &Percepts, nav: &mut dyn Navigator) -> Option<PastimeChoice> {
        let k = p.knight;
        let mon = p.desktop.monitor_at(k.pos.x as i32, k.pos.y as i32 - 1).or(p.desktop.primary())?;
        let work = mon.work.to_f();
        let (ages, now) = (&self.interest.ages, self.now);
        let legs = k.params.px(28.0 / 64.0);
        let mut seats: Vec<(Vec2, String)> = Vec::new();
        for l in &p.terrain.ledges {
            let Some(owner) = l.owner else { continue };
            let high = work.bottom - l.y >= VIEW_HEIGHT * work.height();
            let on_mon = l.y >= work.top && l.y <= work.bottom && l.x1 > work.left && l.x0 < work.right;
            if l.kind == LedgeKind::Ground || !high || !on_mon || l.x1 - l.x0 < 120.0 || !ages.settled(p.terrain, l, now) {
                continue;
            }
            // The end nearer the monitor's middle looks out over the desktop.
            let mid = work.center().x;
            let ends = [l.x0 + 24.0, l.x1 - 24.0];
            let mut ends = ends.to_vec();
            ends.sort_by(|a, b| (a - mid).abs().total_cmp(&(b - mid).abs()));
            for x in ends {
                if x < work.left + 30.0 || x > work.right - 30.0 || !spots::perchable(p.terrain, l, x, legs) {
                    continue;
                }
                let at = Vec2::new(x, l.y);
                if !self.interest.is_blocked(PoiTarget::Spot(at)) {
                    let name = p.desktop.window(owner).map_or_else(|| "a window".to_string(), |w| w.process.clone());
                    seats.push((at, name));
                    break;
                }
            }
        }
        seats.sort_by(|a, b| a.0.distance(k.pos).total_cmp(&b.0.distance(k.pos)));
        for (at, place) in seats.into_iter().take(3) {
            let reach = [TravelStyle::Stroll, TravelStyle::Normal].into_iter().find_map(|st| nav.reach(k, NavGoal::Point(at), st).filter(|r| r.lands_on(at, SEAT_TOL)).map(|r| (r, st)));
            if let Some((r, style)) = reach {
                return Some(PastimeChoice { kind: PastimeKind::View, at, eta: r.eta, value: 0.0, target: 0, place, style });
            }
        }
        None
    }

    /// A high ledge with open sky above it, on his monitor (the floor if nothing better).
    fn find_sky(&self, p: &Percepts, nav: &mut dyn Navigator) -> Option<PastimeChoice> {
        let k = p.knight;
        let mon = p.desktop.monitor_at(k.pos.x as i32, k.pos.y as i32 - 1).or(p.desktop.primary())?;
        let work = mon.work.to_f();
        let (ages, now) = (&self.interest.ages, self.now);
        let sky = OPEN_SKY * k.params.px_per_unit / 64.0;
        let mut spots_: Vec<(Vec2, String)> = p
            .terrain
            .ledges
            .iter()
            .filter(|l| l.y >= work.top + sky * 0.5 && l.y <= work.bottom + 1.0 && l.x1 - l.x0 >= 80.0 && l.x1 > work.left && l.x0 < work.right && ages.settled(p.terrain, l, now))
            .filter_map(|l| {
                let x = ((l.x0 + l.x1) * 0.5).clamp(work.left + 40.0, work.right - 40.0);
                let open = !p.terrain.ledges.iter().any(|o| o.y < l.y - 1.0 && o.y > l.y - sky && o.x0 < x + 40.0 && o.x1 > x - 40.0);
                let name = l.owner.and_then(|id| p.desktop.window(id)).map_or_else(|| "the floor".to_string(), |w| w.process.clone());
                open.then_some((Vec2::new(x, l.y), name))
            })
            .filter(|(at, _)| !self.interest.is_blocked(PoiTarget::Spot(*at)))
            .collect();
        // Highest first.
        spots_.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
        for (at, place) in spots_.into_iter().take(3) {
            if on_point(k, at, 40.0) {
                return Some(PastimeChoice { kind: PastimeKind::Stargaze, at: k.pos, eta: 0.0, value: 0.0, target: 0, place, style: TravelStyle::Normal });
            }
            if let Some(r) = nav.reach(k, NavGoal::Point(at), TravelStyle::Normal).filter(|r| r.lands_on(at, 40.0)) {
                return Some(PastimeChoice { kind: PastimeKind::Stargaze, at, eta: r.eta, value: 0.0, target: 0, place, style: TravelStyle::Normal });
            }
        }
        None
    }

    /// Utility of a pastime (Ambient): none while hidden, asleep, held or fullscreen; the
    /// lively ones not while the user types hard.
    pub(crate) fn pastime_utility(&self, p: &Percepts) -> f32 {
        let s = &self.senses;
        if s.fullscreen || self.asleep(p) || matches!(self.active, Active::Nap(_)) {
            return 0.0;
        }
        if let Active::Pastime(b) = &self.active {
            return if b.over { 0.0 } else { self.interest.pastimes.running_value };
        }
        let Some(c) = &self.interest.pastimes.best else { return 0.0 };
        let lively = matches!(c.kind, PastimeKind::Dummy | PastimeKind::Lumafly);
        if (lively && s.user_busy) || !p.knight.is_grounded() {
            return 0.0;
        }
        c.value
    }
}

// --- the behaviour ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drill {
    /// Three forward slashes.
    Combo,
    UpSlash,
    /// Jump at it and bounce off its head (down slashes).
    Pogo,
    GreatSlash,
    Cyclone,
    DashSlash,
    Fireball,
}

impl Drill {
    fn label(self) -> &'static str {
        match self {
            Drill::Combo => "three-slash combo",
            Drill::UpSlash => "up slash",
            Drill::Pogo => "pogo off its head",
            Drill::GreatSlash => "great slash",
            Drill::Cyclone => "cyclone slash",
            Drill::DashSlash => "dash slash",
            Drill::Fireball => "fireball",
        }
    }

    fn act(self) -> Option<&'static str> {
        match self {
            Drill::GreatSlash => Some("great_slash"),
            Drill::Cyclone => Some("cyclone_slash"),
            Drill::DashSlash => Some("dash_slash"),
            Drill::Fireball => Some("fireball"),
            _ => None,
        }
    }

    /// When the act's blow lands (s after it starts), and how many.
    fn act_hits(self) -> &'static [(f32, bool)] {
        match self {
            Drill::GreatSlash => &[(0.55, true)],
            Drill::Cyclone => &[(0.35, false), (0.6, false), (0.85, true)],
            Drill::DashSlash => &[(0.45, true)],
            Drill::Fireball => &[(0.6, true)],
            _ => &[],
        }
    }
}

/// Seconds between the slashes of a combo.
const COMBO_GAP: f32 = 0.3;
/// Breather between drills (s).
const BREATHER: [f32; 2] = [0.7, 1.6];
/// Longest a drill may take (s).
const DRILL_SECS: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
struct DrillRun {
    drill: Drill,
    t: f32,
    reps: u32,
    cooldown: f32,
    jumped: bool,
    airborne: bool,
    bounces: u32,
    double: bool,
    act: Option<ActWait>,
}

impl DrillRun {
    fn new(drill: Drill) -> Self {
        Self { drill, t: 0.0, reps: 0, cooldown: 0.15, jumped: false, airborne: false, bounces: 0, double: false, act: None }
    }
}

#[derive(Clone, Copy, Debug)]
enum ChasePhase {
    Watch { act: Option<ActWait>, left: f32 },
    Follow,
    Jump { t: f32, swiped: bool, airborne: bool },
    /// It got away (caught and slipped off, or its time was up): watch it go.
    Farewell { act: Option<ActWait>, left: f32 },
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Notice { act: Option<ActWait>, left: f32 },
    Travel(Travel),
    /// At the dummy: between drills.
    Breather { left: f32 },
    Drill(DrillRun),
    Chase(ChasePhase),
    /// At the spot: the pastime's act (the view, the stars).
    Settle { turn: f32, act: Option<ActWait> },
    /// The dummy broke: the last swing / landing finishes (`wait` at most), then a look
    /// down at the pieces (`look`, started), and the pastime ends.
    Broken { wait: f32, look: Option<ActWait>, looked: bool },
}

/// Longest he waits for the last swing / landing after the dummy breaks (s).
const BROKEN_WAIT: f32 = 2.0;

/// Doing a pastime.
#[derive(Clone, Debug)]
pub(crate) struct Pastime {
    pub kind: PastimeKind,
    target: u64,
    at: Vec2,
    place: String,
    /// The style checked to land on `at`.
    style: TravelStyle,
    phase: Phase,
    started: f64,
    drills_left: u32,
    drills_done: u32,
    last_drill: Option<Drill>,
    /// Chase: where his travel goes, when it was set, jumps tried.
    goal_set_at: f64,
    jumps: u32,
    /// Walking under the fly along his own ledge (until close; see `follow`).
    walking: bool,
    /// Finished (the brain drops its utility to 0 this think).
    pub over: bool,
}

impl Pastime {
    pub fn start(ctx: &mut Ctx, c: PastimeChoice) -> Self {
        let k = ctx.p.knight;
        let mem = &mut ctx.interest.pastimes;
        mem.running_value = c.value;
        mem.last_kind = Some(c.kind);
        mem.rest(c.kind, ctx.now); // a cooldown already: failing doesn't retry at once
        let eye = k.body().center();
        let look = |ctx: &Ctx, y: f32| {
            if y < eye.y - 120.0 {
                ActWait::named(ctx, "look_up")
            } else if y > eye.y + 140.0 {
                ActWait::named(ctx, "look_down")
            } else {
                None
            }
        };
        let phase = match c.kind {
            PastimeKind::Dummy => {
                let y = ctx.interest.pastimes.dummy().map_or(eye.y, |d| d.body.center().y);
                Phase::Notice { act: look(ctx, y), left: 0.6 }
            }
            PastimeKind::Lumafly => {
                let y = ctx.interest.pastimes.fly(c.target).map_or(eye.y, |f| f.pos.y);
                Phase::Chase(ChasePhase::Watch { act: look(ctx, y), left: 0.8 })
            }
            PastimeKind::View | PastimeKind::Stargaze => Phase::Notice { act: None, left: 0.2 },
        };
        let drills = 4 + (ctx.rng.f32() * 3.0) as u32;
        log::debug!("brain: pastime {} ({:.2}) at ({:.0},{:.0})", c.kind.label(), c.value, c.at.x, c.at.y);
        Self { kind: c.kind, target: c.target, at: c.at, place: c.place, style: c.style, phase, started: ctx.now, drills_left: drills, drills_done: 0, last_drill: None, goal_set_at: f64::NEG_INFINITY, jumps: 0, walking: false, over: false }
    }

    fn finish(&mut self, ctx: &mut Ctx, satisfied: bool) -> Status {
        self.over = true;
        ctx.nav.stop();
        ctx.interest.pastimes.rest(self.kind, ctx.now);
        if satisfied {
            let (boredom, excite) = match self.kind {
                PastimeKind::Dummy => (-0.45, 0.15),
                PastimeKind::Lumafly => (-0.35, 0.2),
                PastimeKind::View => (-0.4, 0.0),
                PastimeKind::Stargaze => (-0.45, 0.0),
            };
            ctx.mind.adjust(|n| &mut n.boredom, boredom);
            if self.kind == PastimeKind::Dummy {
                ctx.mind.adjust(|n| &mut n.mischief, -0.25);
            }
            ctx.mind.excite(excite);
            Status::Done
        } else {
            Status::Failed
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        if self.over {
            return Status::Done;
        }
        let s = ctx.senses;
        let lively = matches!(self.kind, PastimeKind::Dummy | PastimeKind::Lumafly);
        if s.fullscreen || (lively && s.user_busy) {
            // The screen is needed: put it off.
            return self.finish(ctx, false);
        }
        match self.kind {
            PastimeKind::Dummy => self.tick_dummy(ctx),
            PastimeKind::Lumafly => self.tick_chase(ctx),
            PastimeKind::View | PastimeKind::Stargaze => self.tick_spot(ctx),
        }
    }

    fn travel_to(&mut self, ctx: &mut Ctx, at: Vec2, tol: f32) {
        let style = match self.kind {
            // The style whose route was checked to land there (a stroll may not get up to a
            // spot a run does: it stops underneath).
            PastimeKind::Dummy | PastimeKind::View | PastimeKind::Stargaze => self.style,
            PastimeKind::Lumafly => match super::behaviours::style_for(ctx, at) {
                TravelStyle::Hurry => TravelStyle::Normal,
                s => s,
            },
        };
        self.phase = Phase::Travel(Travel::exact(ctx, at, style, tol));
    }

    // --- the dummy -----------------------------------------------------------------

    fn tick_dummy(&mut self, ctx: &mut Ctx) -> Status {
        if let Phase::Broken { .. } = self.phase {
            return self.tick_broken(ctx);
        }
        let Some(d) = ctx.interest.pastimes.dummy().filter(|d| d.id == self.target) else {
            if ctx.interest.pastimes.dummy_broke == Some(self.target) {
                log::debug!("brain: training dummy #{} broke; he stops", self.target);
                ctx.nav.stop();
                self.phase = Phase::Broken { wait: BROKEN_WAIT, look: None, looked: false };
                return self.tick_broken(ctx);
            }
            // Gone (faded out, its window closed).
            let ok = self.drills_done > 0;
            return self.finish(ctx, ok);
        };
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        match &mut self.phase {
            Phase::Notice { act, left } => {
                ctx.nav.stop();
                ctx.face_toward(d.pos.x);
                *left -= ctx.dt();
                if act.as_mut().is_none_or(|w| w.tick(ctx)) && *left <= 0.0 {
                    if on_point(k, self.at, STAND_TOL) {
                        self.phase = Phase::Breather { left: 0.4 };
                    } else {
                        let at = self.at;
                        self.travel_to(ctx, at, STAND_TOL);
                    }
                }
                Status::Running
            }
            Phase::Travel(t) => match t.tick(ctx) {
                TravelStep::Moving => Status::Running,
                TravelStep::Failed if self.drills_done == 0 => {
                    // Couldn't get to it after all: leave this one be (his own memory; the app
                    // moves a dummy he can't reach), not the whole pastime.
                    log::debug!("brain: couldn't get to training dummy #{}; leaving it be", d.id);
                    let st = self.finish(ctx, false);
                    let mem = &mut ctx.interest.pastimes;
                    mem.dummy_skip = Some((d.id, ctx.now + DUMMY_SKIP_SECS));
                    mem.ready_at.insert(PastimeKind::Dummy, ctx.now);
                    st
                }
                TravelStep::Failed => self.finish(ctx, true),
                TravelStep::Arrived => {
                    self.phase = Phase::Breather { left: 0.3 };
                    Status::Running
                }
            },
            Phase::Breather { left } => {
                ctx.nav.stop();
                if ctx.p.act.is_some() {
                    return Status::Running; // the last swing / nail art finishing
                }
                ctx.face_toward(d.pos.x);
                *left -= ctx.dt();
                if *left > 0.0 || !ctx.grounded() {
                    ctx.out.waiting = true;
                    return Status::Running;
                }
                if self.drills_left == 0 {
                    return self.finish(ctx, true);
                }
                // Drifted off his spot (a dash slash, a pogo)? Back to it first.
                let side = if k.pos.x >= d.pos.x { 1.0 } else { -1.0 };
                let spot = stand_spot(&d, side, ppu);
                let far = (k.pos.x - d.pos.x).abs() > 0.5 * d.body.width() + (STRIKE_GAP + 1.2) * ppu || (k.pos.y - d.pos.y).abs() > 4.0;
                if far {
                    self.at = spot;
                    self.travel_to(ctx, spot, STAND_TOL);
                    return Status::Running;
                }
                let drill = self.pick_drill(ctx, &d);
                self.drills_left -= 1;
                self.last_drill = Some(drill);
                self.phase = Phase::Drill(DrillRun::new(drill));
                Status::Running
            }
            Phase::Drill(run) => {
                let done = drill_tick(run, ctx, &d);
                if done {
                    self.drills_done += 1;
                    ctx.mind.adjust(|n| &mut n.boredom, -0.04);
                    let gap = ctx.range(BREATHER);
                    self.phase = Phase::Breather { left: gap };
                }
                Status::Running
            }
            Phase::Chase(_) | Phase::Settle { .. } | Phase::Broken { .. } => Status::Running,
        }
    }

    /// The next drill: varied (never the same twice in a row), nail arts and spells less
    /// often, a dash slash only with room past the dummy.
    fn pick_drill(&self, ctx: &mut Ctx, d: &KnownDummy) -> Drill {
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        let toward = if d.pos.x >= k.pos.x { 1.0 } else { -1.0 };
        let beyond = if toward > 0.0 { d.span.1 - d.pos.x } else { d.pos.x - d.span.0 };
        let room = beyond > 4.0 * ppu;
        let has = |ctx: &Ctx, n: &str| ctx.act_id(n).is_some();
        let options = [
            (Drill::Combo, 3.0),
            (Drill::UpSlash, 1.2),
            (Drill::Pogo, 1.6),
            (Drill::GreatSlash, if has(ctx, "great_slash") { 1.0 } else { 0.0 }),
            (Drill::Cyclone, if has(ctx, "cyclone_slash") { 0.8 } else { 0.0 }),
            (Drill::DashSlash, if room && has(ctx, "dash_slash") { 0.7 } else { 0.0 }),
            (Drill::Fireball, if has(ctx, "fireball") { 0.8 } else { 0.0 }),
        ];
        let options: Vec<(Drill, f32)> = options.into_iter().map(|(dr, w)| (dr, if Some(dr) == self.last_drill { 0.0 } else { w })).collect();
        ctx.rng.pick_weighted(&options).unwrap_or(Drill::Combo)
    }

    /// The dummy broke: no more drills. The swing or landing under way finishes, he looks
    /// down at the pieces a moment, and that's the end of it (practised: satisfied).
    fn tick_broken(&mut self, ctx: &mut Ctx) -> Status {
        let dt = ctx.dt();
        let Phase::Broken { wait, look, looked } = &mut self.phase else { return Status::Running };
        ctx.nav.stop();
        if !*looked {
            *wait -= dt;
            if (!ctx.grounded() || ctx.p.act.is_some()) && *wait > 0.0 {
                ctx.out.waiting = true;
                return Status::Running;
            }
            *looked = true;
            *look = ActWait::named(ctx, "look_down");
        }
        if look.as_mut().is_none_or(|w| w.tick(ctx)) {
            let ok = self.drills_done > 0;
            return self.finish(ctx, ok);
        }
        Status::Running
    }

    // --- the lumafly ---------------------------------------------------------------

    fn tick_chase(&mut self, ctx: &mut Ctx) -> Status {
        let fly = ctx.interest.pastimes.fly(self.target);
        let Phase::Chase(phase) = &mut self.phase else {
            // Travelling (Follow uses its own travel below).
            return self.follow(ctx, fly);
        };
        match phase {
            ChasePhase::Watch { act, left } => {
                ctx.nav.stop();
                if let Some(f) = fly {
                    ctx.face_toward(f.pos.x);
                }
                *left -= ctx.dt();
                if act.as_mut().is_none_or(|w| w.tick(ctx)) && *left <= 0.0 {
                    *phase = ChasePhase::Follow;
                }
                Status::Running
            }
            ChasePhase::Farewell { act, left } => {
                ctx.nav.stop();
                if let Some(f) = fly {
                    ctx.face_toward(f.pos.x);
                }
                *left -= ctx.dt();
                ctx.out.waiting = false;
                if act.as_mut().is_none_or(|w| w.tick(ctx)) && *left <= 0.0 {
                    return self.finish(ctx, true);
                }
                Status::Running
            }
            ChasePhase::Follow => self.follow(ctx, fly),
            ChasePhase::Jump { t, swiped, airborne } => {
                let k = ctx.p.knight;
                *t += ctx.dt();
                *airborne |= !k.is_grounded();
                let Some(f) = fly else {
                    self.phase = Phase::Chase(ChasePhase::Follow);
                    return Status::Running;
                };
                let centre = k.body().center();
                let mut input = MoveInput { x: (f.pos.x - k.pos.x).clamp(-60.0, 60.0) / 60.0, ..Default::default() };
                if *t < 0.05 {
                    input.jump_pressed = true;
                }
                // Hold the jump while it's still above him.
                input.jump_held = *t < 0.6 && f.pos.y < centre.y - 10.0;
                // Wings near the top if it's still above.
                if *airborne && !*swiped && k.vel.y > -80.0 && f.pos.y < centre.y - 30.0 && k.can_double_jump() {
                    input.jump_pressed = true;
                    input.jump_held = true;
                }
                let close = f.pos.distance(centre) <= 1.1 * k.params.px_per_unit;
                let apex = *airborne && k.vel.y >= 0.0;
                if !*swiped && (close || apex) && *airborne {
                    *swiped = true;
                    self.jumps += 1;
                    ctx.interest.pastimes.push(PastimeEvent::Swipe { fly: f.id, from: centre });
                }
                ctx.out.motion = Motion::Direct(input);
                ctx.out.waiting = false;
                let landed = *airborne && k.is_grounded() && *t > 0.2;
                let never_left = !*airborne && *t > 0.5;
                if landed || never_left || *t > 3.0 {
                    if never_left {
                        self.jumps += 1; // couldn't jump here: counts as a try
                    }
                    self.goal_set_at = f64::NEG_INFINITY;
                    self.phase = Phase::Chase(ChasePhase::Follow);
                }
                Status::Running
            }
        }
    }

    /// Follow the lumafly along the ledges under it; jump at it within reach.
    fn follow(&mut self, ctx: &mut Ctx, fly: Option<KnownLumafly>) -> Status {
        let Some(f) = fly else { return self.finish(ctx, self.jumps > 0) };
        if f.leaving {
            // Slipped out of his hands (or its time is up): watch it float away.
            let act = ActWait::named(ctx, "look_up");
            ctx.face_toward(f.pos.x);
            self.phase = Phase::Chase(ChasePhase::Farewell { act, left: 2.5 });
            return Status::Running;
        }
        if self.jumps >= 4 || ctx.now - self.started > 60.0 {
            ctx.interest.pastimes.tried_flies.insert(f.id, ctx.now);
            let act = ActWait::named(ctx, "look_up");
            self.phase = Phase::Chase(ChasePhase::Farewell { act, left: 1.5 });
            return Status::Running;
        }
        let k = ctx.p.knight;
        let ppu = k.params.px_per_unit;
        let centre = k.body().center();
        let (dx, dy) = (f.pos.x - k.pos.x, centre.y - f.pos.y);
        let in_reach = dx.abs() <= FLY_JUMP_SIDE * ppu && dy >= -0.5 * ppu && dy <= FLY_JUMP_REACH * ppu;
        if in_reach && k.is_grounded() && ctx.p.act.is_none_or(|a| a.interruptible) {
            if ctx.p.act.is_some() {
                ctx.out.cancel_act = true;
            }
            ctx.nav.stop();
            self.phase = Phase::Chase(ChasePhase::Jump { t: 0.0, swiped: false, airborne: false });
            return Status::Running;
        }
        let under = spots::project(ctx.p.terrain, f.pos).unwrap_or(Vec2::new(f.pos.x, k.pos.y));
        // Under it on the ledge he stands on: walk there directly, with slack. Routes
        // re-planned to the drifting spot started behind him or dashed past it (live: back,
        // forth, a dash, turning on the spot six times a second).
        let same_ledge = k.is_grounded() && (k.pos.y - under.y).abs() < 4.0 && k.support.is_some_and(|s| (s.x0..=s.x1).contains(&under.x));
        if same_ledge {
            if matches!(self.phase, Phase::Travel(_)) {
                ctx.nav.stop();
                self.phase = Phase::Chase(ChasePhase::Follow);
            }
            let dx = under.x - k.pos.x;
            if dx.abs() > 0.8 * ppu {
                self.walking = true;
            } else if dx.abs() < 0.25 * ppu {
                self.walking = false;
            }
            if self.walking && ctx.free_to_move() {
                let x = dx.signum() * (dx.abs() / (1.5 * ppu)).clamp(0.35, 1.0);
                ctx.out.motion = Motion::Direct(MoveInput { x, ..MoveInput::default() });
            } else {
                ctx.nav.stop();
                ctx.face_toward(f.pos.x);
                ctx.out.waiting = false; // eyes on it
            }
            return Status::Running;
        }
        self.walking = false;
        // Elsewhere: travel under it, re-aimed as it drifts.
        let travelling = matches!(self.phase, Phase::Travel(_));
        let stale = (under.x - self.at.x).abs() > 50.0 || (under.y - self.at.y).abs() > 4.0;
        if (!travelling || stale) && ctx.now - self.goal_set_at > 0.6 {
            let here = (k.pos.x - under.x).abs() < 0.6 * ppu && (k.pos.y - under.y).abs() < 4.0;
            self.at = under;
            self.goal_set_at = ctx.now;
            if here {
                self.phase = Phase::Chase(ChasePhase::Follow);
            } else {
                let style = if k.pos.distance(under) > 500.0 { TravelStyle::Hurry } else { TravelStyle::Normal };
                self.phase = Phase::Travel(Travel::to(ctx, under, style));
            }
        }
        if let Phase::Travel(t) = &mut self.phase {
            match t.tick(ctx) {
                TravelStep::Moving => {}
                TravelStep::Arrived => self.phase = Phase::Chase(ChasePhase::Follow),
                TravelStep::Failed => {
                    // Out of reach for now: watch it from here a moment, then try again.
                    self.phase = Phase::Chase(ChasePhase::Follow);
                    self.goal_set_at = ctx.now + 2.0;
                    self.jumps += 1;
                }
            }
        } else {
            ctx.nav.stop();
            ctx.face_toward(f.pos.x);
            ctx.out.waiting = false; // eyes on it
        }
        Status::Running
    }

    // --- the view, the stars --------------------------------------------------------

    fn tick_spot(&mut self, ctx: &mut Ctx) -> Status {
        let (act_name, tol) = match self.kind {
            PastimeKind::View => ("take_in_view", SEAT_TOL),
            _ => ("stargaze", 40.0),
        };
        match &mut self.phase {
            Phase::Notice { left, .. } => {
                *left -= ctx.dt();
                if *left <= 0.0 {
                    if on_point(ctx.p.knight, self.at, tol) {
                        self.phase = Phase::Settle { turn: 0.2, act: ActWait::named(ctx, act_name) };
                    } else {
                        let at = self.at;
                        self.travel_to(ctx, at, tol);
                    }
                }
                Status::Running
            }
            Phase::Travel(t) => match t.tick(ctx) {
                TravelStep::Moving => Status::Running,
                TravelStep::Failed => self.finish(ctx, false),
                TravelStep::Arrived => {
                    self.phase = Phase::Settle { turn: 0.2, act: ActWait::named(ctx, act_name) };
                    Status::Running
                }
            },
            Phase::Settle { turn, act } => {
                ctx.nav.stop();
                if *turn > 0.0 {
                    // Face out over the edge (the view), or anywhere (the stars are behind the screen).
                    *turn -= ctx.dt();
                    if self.kind == PastimeKind::View
                        && let Some(sup) = ctx.p.knight.support
                    {
                        let out = if self.at.x - sup.x0 < sup.x1 - self.at.x { Side::Left } else { Side::Right };
                        ctx.out.face = Some(out);
                    }
                    return Status::Running;
                }
                let Some(w) = act else { return self.finish(ctx, false) };
                if !w.tick(ctx) {
                    ctx.out.waiting = false;
                    return Status::Running;
                }
                let ok = w.started();
                self.finish(ctx, ok)
            }
            _ => Status::Running,
        }
    }

    pub fn reason(&self) -> String {
        let what = match (&self.kind, &self.phase) {
            (PastimeKind::Dummy, Phase::Notice { .. }) => "a training dummy over there; time to practise".into(),
            (PastimeKind::Dummy, Phase::Travel(_)) => "walking over to the training dummy".into(),
            (PastimeKind::Dummy, Phase::Breather { .. }) => format!("training dummy: catching his breath ({} drills done)", self.drills_done),
            (PastimeKind::Dummy, Phase::Drill(r)) => format!("training dummy: {}", r.drill.label()),
            (PastimeKind::Dummy, Phase::Broken { .. }) => format!("the training dummy broke ({} drills done); a look at the pieces", self.drills_done),
            (PastimeKind::Lumafly, Phase::Chase(ChasePhase::Watch { .. })) => "a lumafly drifting by!".into(),
            (PastimeKind::Lumafly, Phase::Chase(ChasePhase::Jump { .. })) => "jumping to catch the lumafly".into(),
            (PastimeKind::Lumafly, Phase::Chase(ChasePhase::Farewell { .. })) => "watching the lumafly float away".into(),
            (PastimeKind::Lumafly, _) => format!("following the lumafly ({} tries)", self.jumps),
            (PastimeKind::View, Phase::Settle { .. }) => format!("sitting on {}'s edge, taking in the view", self.place),
            (PastimeKind::View, _) => format!("climbing up to {}'s edge for the view", self.place),
            (PastimeKind::Stargaze, Phase::Settle { .. }) => format!("night: stargazing from {}", self.place),
            (PastimeKind::Stargaze, _) => format!("night: up to {} to look at the stars", self.place),
            _ => self.kind.label().into(),
        };
        let why = match self.kind {
            PastimeKind::Dummy | PastimeKind::Lumafly => "more fun than standing around",
            PastimeKind::View | PastimeKind::Stargaze => "restless after a long while of company",
        };
        format!("pastime: {what} ({why})")
    }
}

/// One drill at the dummy; `true` when it's over.
fn drill_tick(run: &mut DrillRun, ctx: &mut Ctx, d: &KnownDummy) -> bool {
    let dt = ctx.dt();
    run.t += dt;
    run.cooldown -= dt;
    let k = ctx.p.knight;
    let side = Side::of(d.pos.x - k.pos.x).unwrap_or(k.facing);
    let from_right = k.pos.x > d.pos.x;
    let hit = |heavy| DummyHit { heavy, from_right };
    if run.t > DRILL_SECS {
        return k.is_grounded() && ctx.p.act.is_none();
    }
    match run.drill {
        Drill::Combo | Drill::UpSlash => {
            ctx.nav.stop();
            ctx.out.face = Some(side);
            let want = if run.drill == Drill::Combo { 3 } else { 1 };
            if run.reps >= want {
                return run.cooldown <= 0.0 && ctx.p.act.is_none();
            }
            if run.cooldown <= 0.0 && k.is_grounded() && ctx.p.act.is_none() {
                let dir = if run.drill == Drill::Combo { AttackDir::Forward } else { AttackDir::Up };
                ctx.out.attack = Some(Attack { dir, side, hit: true });
                ctx.interest.pastimes.push(PastimeEvent::DummyHit { hit: hit(false), delay: 0.08 });
                run.reps += 1;
                run.cooldown = if run.reps >= want { 0.45 } else { COMBO_GAP };
            }
            false
        }
        Drill::Pogo => {
            ctx.nav.stop();
            let mut input = MoveInput::default();
            let grounded = k.is_grounded();
            if !run.jumped {
                if grounded && ctx.p.act.is_none() && run.cooldown <= 0.0 {
                    input.jump_pressed = true;
                    input.jump_held = true;
                    input.x = side.sign();
                    run.jumped = true;
                    run.t = 0.0;
                }
                ctx.out.face = Some(side);
                ctx.out.motion = Motion::Direct(input);
                return run.t > 1.5; // couldn't jump here
            }
            run.airborne |= !grounded;
            let centre = k.body().center();
            let mark = Mark { off: d.body.center() - centre, half: Vec2::new(0.5 * d.body.width(), 0.5 * d.body.height()), ..Default::default() };
            let stance = Stance::of(k);
            // Up and over it: steer above its head.
            input.jump_held = run.t < 0.45;
            input.x = ((d.pos.x - k.pos.x) / (0.8 * k.params.px_per_unit)).clamp(-1.0, 1.0);
            if run.airborne && !run.double && k.vel.y > -60.0 && mark.off.y - mark.half.y < 0.0 && k.can_double_jump() {
                // Not above it yet at the top of the jump: wings.
                input.jump_pressed = true;
                input.jump_held = true;
                run.double = true;
            } else if run.cooldown <= 0.0 && run.bounces < 2 && pogo_now(&mark, &stance) {
                input.pogo = true;
                ctx.out.attack = Some(Attack { dir: AttackDir::Down, side, hit: true });
                ctx.interest.pastimes.push(PastimeEvent::DummyHit { hit: hit(false), delay: 0.05 });
                run.bounces += 1;
                run.cooldown = 0.3;
            }
            ctx.out.motion = Motion::Direct(input);
            run.airborne && grounded && run.t > 0.25
        }
        Drill::GreatSlash | Drill::Cyclone | Drill::DashSlash | Drill::Fireball => {
            ctx.nav.stop();
            if run.act.is_none() && run.reps == 0 {
                run.act = drill_act(ctx, run.drill);
                if run.act.is_none() {
                    return true; // no such act
                }
            }
            let Some(w) = &mut run.act else { return true };
            if !w.started() {
                ctx.out.face = Some(side);
            }
            let finished = w.tick(ctx);
            if w.started() && run.reps == 0 {
                run.reps = 1;
                for &(delay, heavy) in run.drill.act_hits() {
                    ctx.interest.pastimes.push(PastimeEvent::DummyHit { hit: hit(heavy), delay });
                }
            }
            finished
        }
    }
}

fn drill_act(ctx: &Ctx, drill: Drill) -> Option<ActWait> {
    drill.act().and_then(|n| ActWait::named(ctx, n))
}
