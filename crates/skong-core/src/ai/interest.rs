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
    /// A screen corner (bottom of a monitor's work area).
    Corner,
    /// A placed bench: somewhere to sit (bench acts) when tired or calm.
    Bench,
}

impl PoiKind {
    pub fn label(self) -> &'static str {
        match self {
            PoiKind::NewWindow => "new window",
            PoiKind::MovingWindow => "moving window",
            PoiKind::ActiveWindow => "active window",
            PoiKind::Unvisited => "unvisited window",
            PoiKind::HighLedge => "high ledge",
            PoiKind::Corner => "corner",
            PoiKind::Bench => "bench",
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
            PoiKind::Corner => 0.6 * curious,
            PoiKind::Bench => 0.7 * mind.tiredness() + 0.3 * (1.0 - mind.arousal()),
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
    visited: BTreeMap<PoiKey, f32>,
    /// When a window of each process was last visited.
    processes: BTreeMap<String, f32>,
    /// Unreachable targets, skipped until the given time.
    blocked: BTreeMap<PoiKey, f32>,
}

impl InterestMap {
    /// Fold this tick's desktop changes into the event list and decay novelty.
    pub(crate) fn observe(&mut self, now: f32, dt: f32, senses: &Senses, desk: &DesktopSnapshot, cfg: &BrainConfig) {
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
    }

    fn raise(&mut self, kind: PoiKind, target: PoiTarget, novelty: f32) {
        match self.events.iter_mut().find(|e| e.kind == kind && e.target == target) {
            Some(e) => e.novelty = e.novelty.max(novelty),
            None => self.events.push(Poi { kind, target, novelty }),
        }
    }

    /// 1 = a window of this process was just visited, 0 = never / long ago.
    fn process_familiarity(&self, process: &str, now: f32, cfg: &BrainConfig) -> f32 {
        self.processes.get(process).map_or(0.0, |&t| (1.0 - (now - t) / cfg.visit_memory_secs.max(1.0)).clamp(0.0, 1.0))
    }

    /// 1 = just visited, 0 = never / forgotten.
    pub fn familiarity(&self, key: PoiKey, now: f32, cfg: &BrainConfig) -> f32 {
        self.visited.get(&key).map_or(0.0, |&t| (1.0 - (now - t) / cfg.visit_memory_secs.max(1.0)).clamp(0.0, 1.0))
    }

    /// Everything currently worth considering, novelty already reduced by familiarity.
    pub(crate) fn candidates(&self, now: f32, knight: &Character, world: (&DesktopSnapshot, &Terrain, &[Vec2]), cfg: &BrainConfig) -> Vec<Poi> {
        let (desk, terrain, benches) = world;
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
        for m in &desk.monitors {
            let y = m.work.bottom as f32;
            for x in [m.work.left as f32 + 48.0, m.work.right as f32 - 48.0] {
                out.push(Poi { kind: PoiKind::Corner, target: PoiTarget::Spot(Vec2::new(x, y)), novelty: 0.2 });
            }
        }

        for &b in benches {
            if b.distance(knight.pos) > super::percept::BENCH_REACH {
                out.push(Poi { kind: PoiKind::Bench, target: PoiTarget::Spot(b), novelty: 0.6 });
            }
        }

        out.retain(|p| !self.blocked.contains_key(&p.target.key()));
        for p in &mut out {
            p.novelty *= 1.0 - self.familiarity(p.target.key(), now, cfg);
        }
        out.retain(|p| p.novelty >= MIN_NOVELTY);
        out
    }

    /// The knight arrived: remember the place (and its process) and drop its events.
    pub(crate) fn visit(&mut self, target: PoiTarget, now: f32, desk: &DesktopSnapshot) {
        let key = target.key();
        self.visited.insert(key, now);
        if let PoiTarget::Window(id) = target
            && let Some(w) = desk.window(id)
        {
            self.processes.insert(w.process.clone(), now);
        }
        self.events.retain(|e| e.target.key() != key);
    }

    /// Skip this target for a while (no route).
    pub(crate) fn block(&mut self, target: PoiTarget, until: f32) {
        self.blocked.insert(target.key(), until);
    }
}
