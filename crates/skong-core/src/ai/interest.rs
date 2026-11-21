//! Points of interest: what is worth walking over to look at, how novel it still is,
//! and what the knight already knows (visited windows by id and process).
use std::collections::BTreeMap;

use crate::controller::Character;
use crate::desktop::{DesktopSnapshot, WindowId};
use crate::geom::Vec2;
use crate::nav::NavGoal;
use crate::terrain::Terrain;

use super::config::BrainConfig;
use super::needs::Mind;
use super::percept::Senses;
use super::places::PlaceMemory;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PoiKind {
    /// A window that just opened.
    NewWindow,
    /// A window being dragged / animating.
    MovingWindow,
    /// The foreground window (mischief: sit on its title bar).
    ActiveWindow,
    /// A window the knight has never stood on.
    Unvisited,
    /// The highest ledge around.
    HighLedge,
}

impl PoiKind {
    pub fn label(self) -> &'static str {
        match self {
            PoiKind::NewWindow => "new window",
            PoiKind::MovingWindow => "moving window",
            PoiKind::ActiveWindow => "active window",
            PoiKind::Unvisited => "unvisited window",
            PoiKind::HighLedge => "high ledge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoiTarget {
    Window(WindowId),
    Spot(Vec2),
}

/// Identity for memory: windows by id, spots quantized to a 64 px grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PoiKey {
    Window(WindowId),
    Spot(i32, i32),
}

impl PoiTarget {
    pub fn goal(self) -> NavGoal {
        match self {
            PoiTarget::Window(id) => NavGoal::OnWindow(id),
            PoiTarget::Spot(p) => NavGoal::Point(p),
        }
    }

    pub fn key(self) -> PoiKey {
        match self {
            PoiTarget::Window(id) => PoiKey::Window(id),
            PoiTarget::Spot(p) => PoiKey::Spot((p.x / 64.0).round() as i32, (p.y / 64.0).round() as i32),
        }
    }

    /// Approximate position (window: top center).
    pub fn position(self, terrain: &Terrain) -> Option<Vec2> {
        match self {
            PoiTarget::Window(id) => terrain.window_rect(id).map(|r| Vec2::new((r.left + r.right) * 0.5, r.top)),
            PoiTarget::Spot(p) => Some(p),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Poi {
    pub kind: PoiKind,
    pub target: PoiTarget,
    /// 0..1, decays over time and with familiarity.
    pub novelty: f32,
}

impl Poi {
    /// How much this kind of thing matters given the current needs (before travel cost).
    pub fn relevance(&self, mind: &Mind) -> f32 {
        let n = mind.needs;
        let p = mind.personality;
        let curious = n.boredom * (0.5 + p.curiosity);
        match self.kind {
            PoiKind::NewWindow => 0.45 + curious,
            PoiKind::MovingWindow => 0.3 + 0.5 * curious + 0.5 * n.mischief,
            PoiKind::ActiveWindow => 1.2 * n.mischief + 0.3 * n.loneliness,
            PoiKind::Unvisited => 0.9 * curious,
            PoiKind::HighLedge => 0.5 * curious + 0.4 * p.boldness,
        }
    }

    pub fn interest(&self, mind: &Mind) -> f32 {
        self.novelty * self.relevance(mind)
    }
}

/// A point of interest with its route cost resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScoredPoi {
    pub poi: Poi,
    pub eta: f32,
    pub score: f32,
}

const MIN_NOVELTY: f32 = 0.05;

#[derive(Clone, Debug, Default)]
pub struct InterestMap {
    /// Event-driven POIs with decaying novelty.
    events: Vec<Poi>,
    /// When each place was last visited (brain clock).
    visited: BTreeMap<PoiKey, f64>,
    /// When a window of each process was last visited.
    processes: BTreeMap<String, f64>,
    /// Unreachable targets, skipped until the given time.
    blocked: BTreeMap<PoiKey, f64>,
    /// Coins he gave up on for now (no route, kept for a Dream Gate), by where they rest,
    /// until the given time. Kept apart from `blocked`: a seat, a cursor spot or a fight
    /// goal that failed nearby (same 64 px cell) must never hide fresh Geo.
    skipped_coins: Vec<(Vec2, f64)>,
    /// The exact spots behind `blocked`'s spot cells, until the same time. Found places and
    /// shinies are judged by these (see [`InterestMap::failed_at`]): a failure *at them*
    /// keeps them from being retried, but an unrelated seat or goal that failed in the same
    /// 64 px cell no longer hides a deposit, a grub or a shiny for a minute.
    failed_spots: Vec<(Vec2, f64)>,
    /// Things found in windows (see [`super::places`]).
    pub(crate) places: PlaceMemory,
    /// How long each window ledge has been on screen (seats must have settled).
    pub(crate) ages: crate::lasting::LedgeAges,
    /// Brain clock and terrain generation of the last age update (`None`: never).
    pub(crate) ages_seen: Option<(f64, u64)>,
    /// Shinies on the ground (see [`super::shiny`]).
    pub(crate) shinies: super::shiny::ShinyMemory,
    /// Where the user's highlighted text is (see [`super::highlight`]).
    pub(crate) highlight: super::highlight::HighlightMemory,
    /// The training dummy, lumaflies, pastime cooldowns (see [`super::pastime`]).
    pub(crate) pastimes: super::pastime::PastimeMemory,
}

impl InterestMap {
    /// Fold this tick's desktop changes into the event list and decay novelty.
    pub(crate) fn observe(&mut self, now: f64, dt: f32, senses: &Senses, desk: &DesktopSnapshot, cfg: &BrainConfig) {
        let decay = 0.5f32.powf(dt / cfg.novelty_half_life.max(1.0));
        for e in &mut self.events {
            e.novelty *= decay;
        }
        self.events.retain(|e| {
            e.novelty >= MIN_NOVELTY
                && match e.target {
                    PoiTarget::Window(id) => desk.window(id).is_some(),
                    PoiTarget::Spot(_) => true,
                }
        });

        let ch = &senses.changes;
        for &id in &ch.opened {
            let Some(w) = desk.window(id) else { continue };
            if w.fullscreen {
                continue;
            }
            let familiar = self.process_familiarity(&w.process, now, cfg);
            self.raise(PoiKind::NewWindow, PoiTarget::Window(id), 1.0 - 0.5 * familiar);
        }
        for &(id, _) in &ch.moved {
            if Some(id) != senses.support_owner {
                self.raise(PoiKind::MovingWindow, PoiTarget::Window(id), 0.7);
            }
        }
        if let Some(id) = ch.focused {
            self.raise(PoiKind::ActiveWindow, PoiTarget::Window(id), 0.6);
        }
        self.blocked.retain(|_, until| *until > now);
        self.skipped_coins.retain(|(_, until)| *until > now);
        self.failed_spots.retain(|(_, until)| *until > now);
    }

    fn raise(&mut self, kind: PoiKind, target: PoiTarget, novelty: f32) {
        match self.events.iter_mut().find(|e| e.kind == kind && e.target == target) {
            Some(e) => e.novelty = e.novelty.max(novelty),
            None => self.events.push(Poi { kind, target, novelty }),
        }
    }

    /// 1 = a window of this process was just visited, 0 = never / long ago.
    fn process_familiarity(&self, process: &str, now: f64, cfg: &BrainConfig) -> f32 {
        self.processes.get(process).map_or(0.0, |&t| (1.0 - (now - t) as f32 / cfg.visit_memory_secs.max(1.0)).clamp(0.0, 1.0))
    }

    /// 1 = just visited, 0 = never / forgotten.
    pub fn familiarity(&self, key: PoiKey, now: f64, cfg: &BrainConfig) -> f32 {
        self.visited.get(&key).map_or(0.0, |&t| (1.0 - (now - t) as f32 / cfg.visit_memory_secs.max(1.0)).clamp(0.0, 1.0))
    }

    /// Everything currently worth considering, novelty already reduced by familiarity.
    pub(crate) fn candidates(&self, now: f64, knight: &Character, world: (&DesktopSnapshot, &Terrain), cfg: &BrainConfig) -> Vec<Poi> {
        let (desk, terrain) = world;
        let support = knight.support.and_then(|s| s.owner);
        let mut out: Vec<Poi> = self.events.clone();
        let has = |out: &[Poi], t: PoiTarget| out.iter().any(|p| p.target == t);

        for w in &desk.windows {
            let t = PoiTarget::Window(w.id);
            if Some(w.id) == support || w.fullscreen || terrain.window_rect(w.id).is_none() || has(&out, t) {
                continue;
            }
            if w.foreground {
                out.push(Poi { kind: PoiKind::ActiveWindow, target: t, novelty: 0.35 });
            } else if !self.visited.contains_key(&t.key()) {
                let familiar = self.process_familiarity(&w.process, now, cfg);
                out.push(Poi { kind: PoiKind::Unvisited, target: t, novelty: 0.35 * (1.0 - 0.5 * familiar) });
            }
        }

        let high = terrain
            .ledges
            .iter()
            .filter(|l| l.x1 - l.x0 >= 80.0 && l.owner.is_some() && l.owner != support)
            .min_by(|a, b| a.y.total_cmp(&b.y));
        if let Some(l) = high {
            out.push(Poi { kind: PoiKind::HighLedge, target: PoiTarget::Spot(Vec2::new((l.x0 + l.x1) * 0.5, l.y)), novelty: 0.3 });
        }
        out.retain(|p| !self.blocked.contains_key(&p.target.key()));
        for p in &mut out {
            p.novelty *= 1.0 - self.familiarity(p.target.key(), now, cfg);
        }
        out.retain(|p| p.novelty >= MIN_NOVELTY);
        out
    }

    /// The knight arrived: remember the place (and its process) and drop its events.
    pub(crate) fn visit(&mut self, target: PoiTarget, now: f64, desk: &DesktopSnapshot) {
        let key = target.key();
        self.visited.insert(key, now);
        if let PoiTarget::Window(id) = target
            && let Some(w) = desk.window(id)
        {
            self.processes.insert(w.process.clone(), now);
        }
        self.events.retain(|e| e.target.key() != key);
    }

    /// Is this target (or a spot within 64 px of it) temporarily unreachable?
    pub fn is_blocked(&self, target: PoiTarget) -> bool {
        self.blocked.contains_key(&target.key())
    }

    /// Skip this target for a while (no route).
    pub(crate) fn block(&mut self, target: PoiTarget, until: f64) {
        self.blocked.insert(target.key(), until);
        if let PoiTarget::Spot(at) = target {
            self.failed_spots.retain(|(p, _)| p.distance(at) >= SPOT_SAME);
            self.failed_spots.push((at, until));
        }
    }

    /// Reachable again (he got there by other means: a Dream Gate).
    pub(crate) fn unblock(&mut self, target: PoiTarget) {
        self.blocked.remove(&target.key());
        if let PoiTarget::Spot(at) = target {
            self.failed_spots.retain(|(p, _)| p.distance(at) >= SPOT_SAME);
        }
    }

    /// Did getting to (or onto) `at` itself fail recently? Unlike [`Self::is_blocked`],
    /// only a failure within a few px counts, not anything in the same 64 px cell: found
    /// places and shinies use this, so a nearby seat or goal that failed doesn't hide them.
    pub fn failed_at(&self, at: Vec2) -> bool {
        self.failed_spots.iter().any(|(p, _)| p.distance(at) < SPOT_SAME)
    }

    /// Is the coin resting at `at` skipped for now (only coins Loot gave up on count)?
    pub fn coin_skipped(&self, at: Vec2) -> bool {
        self.skipped_coins.iter().any(|(c, _)| c.distance(at) < COIN_SAME)
    }

    /// Leave the coin at `at` alone until `until` (no route to it).
    pub(crate) fn skip_coin(&mut self, at: Vec2, until: f64) {
        self.skipped_coins.retain(|(c, _)| c.distance(at) >= COIN_SAME);
        self.skipped_coins.push((at, until));
    }

    /// The coin at `at` is fair game again (he got there by a Dream Gate).
    pub(crate) fn unskip_coin(&mut self, at: Vec2) {
        self.skipped_coins.retain(|(c, _)| c.distance(at) >= COIN_SAME);
    }
}

/// Resting coins this close (px) are the same coin for skipping.
const COIN_SAME: f32 = 12.0;
/// Failed goals this close (px) are the same spot for [`InterestMap::failed_at`]: travel
/// re-targets a drifting place only past 6 px, so its last goal is within that of it.
const SPOT_SAME: f32 = 8.0;
/// Route plans a candidate scoring (places, shinies, lumaflies, watch spots) may spend per
/// rescore. Candidates are checked in want order until enough reachable ones are found:
/// checking only the top two let two unreachable favourites hide a reachable third (Visit
/// scored 0 and he idled beside a bench, a deposit or a grub he could have walked to).
/// Plans cost a few ms each and scoring runs up to 4 times a second, hence the cap.
pub(crate) const CANDIDATE_PLANS: usize = 5;
/// A candidate no route reaches is left out of the next rescores this long (s), so they
/// spend the plan budget on the others instead of re-planning the same unreachable
/// favourites every time. Short: a moved window can open a route at any moment.
pub(crate) const UNREACHABLE_SECS: f64 = 5.0;
/// The passed-over verdict on a candidate left unchecked once the plan budget was spent.
pub(crate) const UNPLANNED: &str = "not route-checked yet (plan budget spent on better ones)";
