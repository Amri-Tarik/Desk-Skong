//! Things *found* inside windows. Each app is a "room" of Hallownest: the first time the
//! knight explores one of its windows, the room is rolled (deterministically from the
//! world seed and the app's name) for what was always there — a bench, a hot spring, a
//! geo deposit, or (most often) nothing. The record persists (the app saves [`World`] as
//! JSON), so the same things are in the same place whenever that app's window is open;
//! they ride along with the window and vanish when it closes.
//!
//! This module is pure: the app resolves positions every frame with [`World::shown`],
//! draws the props and tells the world when the knight explored, used or struck one.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ai::{KnownPlace, PlaceKind};
use crate::desktop::{DesktopSnapshot, WindowId, WindowInfo};
use crate::geom::{RectF, Vec2};
use crate::terrain::{LedgeKind, Terrain};

pub type FindId = u64;

/// What lives in a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindKind {
    /// A bench of one of the `props.json` styles.
    Bench { style: usize },
    /// A steaming hot spring: bathing restores energy faster than resting.
    Spring,
    /// A geo rock: slashed when playful; breaks after a few hits, regrows a day later.
    Deposit,
}

impl FindKind {
    pub fn place_kind(self) -> PlaceKind {
        match self {
            FindKind::Bench { .. } => PlaceKind::Bench,
            FindKind::Spring => PlaceKind::Spring,
            FindKind::Deposit => PlaceKind::Deposit,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FindKind::Bench { .. } => "bench",
            FindKind::Spring => "hot spring",
            FindKind::Deposit => "geo deposit",
        }
    }
}

/// Where a find sits on its window: relative, so it follows moves and resizes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Center as a fraction of the window width (0 = left edge, 1 = right edge).
    pub fx: f32,
    /// Its ledge, in px below the window top (0 = the title bar top).
    pub dy: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Find {
    pub id: FindId,
    pub kind: FindKind,
    /// `None` until the window had room for it (it is placed on the next visit).
    pub anchor: Option<Anchor>,
    /// Times the knight used it (sat, bathed, mined): favourite places get revisited.
    #[serde(default)]
    pub visits: u32,
    /// Deposit: hits taken since it (re)grew.
    #[serde(default)]
    pub hits: u32,
    /// Deposit: unix time it broke; hidden until it regrows.
    #[serde(default)]
    pub broken_at: Option<u64>,
}

impl Find {
    /// 0 = never used, → 1 for a favourite.
    pub fn favour(&self) -> f32 {
        self.visits as f32 / (self.visits as f32 + 3.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Room {
    pub finds: Vec<Find>,
}

/// Rarity and sizes. Widths are footprints in px at the current scale (the app knows the art).
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Weights of nothing / bench / spring / deposit for a newly explored app.
    pub nothing: f32,
    pub bench: f32,
    pub spring: f32,
    pub deposit: f32,
    /// Relative weight per bench style (index = `props.json` order).
    pub bench_styles: Vec<f32>,
    /// Hours until a broken deposit regrows.
    pub regrow_hours: f32,
    /// Hits that break a deposit.
    pub deposit_hp: u32,
}

impl Default for Rules {
    fn default() -> Self {
        Self { nothing: 0.55, bench: 0.2, spring: 0.1, deposit: 0.15, bench_styles: vec![3.0, 1.0, 1.5, 1.5, 1.5], regrow_hours: 24.0, deposit_hp: 3 }
    }
}

/// Keep finds this far from a ledge's ends (px).
const EDGE_MARGIN: f32 = 24.0;
/// Leave this much room above a find (steam, the knight standing on it): a maximized
/// window's title bar at the top of the screen does not qualify (px).
const HEADROOM: f32 = 48.0;
/// A content ledge may drift this much (re-detection jitter) and still carry its find (px).
const LEDGE_SLACK: f32 = 6.0;

/// Everything ever found, keyed by app (see [`app_key`]). Saved as `world.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct World {
    #[serde(default = "version")]
    pub version: u32,
    /// Rolls are a pure function of this and the app key.
    pub seed: u64,
    #[serde(default)]
    pub rooms: BTreeMap<String, Room>,
    #[serde(default)]
    next_id: FindId,
    /// Which window of an app hosts its finds right now (not saved).
    #[serde(skip)]
    hosts: BTreeMap<String, WindowId>,
}

fn version() -> u32 {
    1
}

/// A find on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub id: FindId,
    pub kind: FindKind,
    pub window: WindowId,
    /// Bottom center, resting on its ledge.
    pub pos: Vec2,
    pub favour: f32,
}

/// What a broken deposit drops (the app bursts it into Geo coins).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepositDrop {
    pub at: Vec2,
    pub geo: u32,
}

/// Identity of an app for discoveries: its executable name, lowercased, without `.exe`.
/// `None` for windows that are not rooms (the pet's own windows, no process name).
pub fn app_key(w: &WindowInfo) -> Option<String> {
    let p = w.process.trim().to_ascii_lowercase();
    let p = p.strip_suffix(".exe").unwrap_or(&p).to_string();
    (!p.is_empty() && p != "skong").then_some(p)
}

/// splitmix64: tiny, deterministic, good enough for rolls.
struct Mix(u64);

impl Mix {
    fn new(seed: u64, key: &str) -> Self {
        // FNV-1a over the key, folded into the seed.
        let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        Mix(seed ^ h)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn pick(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
        let mut r = self.f32() * total;
        for (i, w) in weights.iter().enumerate() {
            r -= w.max(0.0);
            if r < 0.0 {
                return i;
            }
        }
        weights.len().saturating_sub(1)
    }
}

/// The deterministic roll for one app: what its room holds (before placement).
pub fn roll(seed: u64, key: &str, rules: &Rules) -> Vec<FindKind> {
    let mut m = Mix::new(seed, key);
    match m.pick(&[rules.nothing, rules.bench, rules.spring, rules.deposit]) {
        1 => vec![FindKind::Bench { style: m.pick(&rules.bench_styles) }],
        2 => vec![FindKind::Spring],
        3 => vec![FindKind::Deposit],
        _ => vec![],
    }
}

/// Choose a spot for something `width` px wide on window `id`: its top ledge (title bar)
/// if there is room, else the widest content ledge inside it, clear of the finds already
/// there (`taken`: bottom center and width). Deterministic in `salt`.
pub fn place(terrain: &Terrain, id: WindowId, width: f32, salt: u64, taken: &[(Vec2, f32)]) -> Option<Anchor> {
    let rect = terrain.window_rect(id)?;
    let need = width + 2.0 * EDGE_MARGIN;
    let fits = terrain.ledges.iter().filter(|l| l.owner == Some(id) && l.kind != LedgeKind::Ground && l.x1 - l.x0 >= need && l.y >= terrain.bounds.top + HEADROOM);
    let top = fits.clone().filter(|l| (l.y - rect.top).abs() < 2.0).max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)));
    let ledge = top.or_else(|| fits.max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0))))?;
    let lo = ledge.x0 + EDGE_MARGIN + width * 0.5;
    let hi = ledge.x1 - EDGE_MARGIN - width * 0.5;
    let clear = |x: f32| !taken.iter().any(|(p, w)| (p.y - ledge.y).abs() < 4.0 && (p.x - x).abs() < (w + width) * 0.5 + 12.0);
    let mut mix = Mix(salt);
    // Stay out of the right end of a title bar (window buttons) when there is room.
    let first = (0..8).map(|_| lo + (hi - lo) * (0.15 + 0.6 * mix.f32())).find(|&x| clear(x));
    let x = first.or_else(|| (0..=20).map(|i| lo + (hi - lo) * i as f32 / 20.0).find(|&x| clear(x)))?;
    Some(Anchor { fx: (x - rect.left) / rect.width().max(1.0), dy: ledge.y - rect.top })
}

/// Where an anchored find of `width` px is on window `rect` right now: on a ledge of that
/// window that carries its whole footprint. `None` while covered / out of room.
pub fn resolve(terrain: &Terrain, id: WindowId, rect: RectF, a: Anchor, width: f32) -> Option<Vec2> {
    let x = rect.left + a.fx * rect.width();
    let y = rect.top + a.dy;
    let half = width * 0.5;
    terrain
        .ledges
        .iter()
        .filter(|l| l.owner == Some(id) && (l.y - y).abs() <= LEDGE_SLACK && l.x0 <= x - half && x + half <= l.x1)
        .min_by(|p, q| (p.y - y).abs().total_cmp(&(q.y - y).abs()))
        .map(|l| Vec2::new(x, l.y))
}

impl World {
    pub fn new(seed: u64) -> Self {
        Self { version: version(), seed, rooms: BTreeMap::new(), next_id: 1, hosts: BTreeMap::new() }
    }

    pub fn from_json(s: &str) -> serde_json::Result<World> {
        let mut w: World = serde_json::from_str(s)?;
        let max = w.rooms.values().flat_map(|r| r.finds.iter().map(|f| f.id)).max().unwrap_or(0);
        w.next_id = w.next_id.max(max + 1);
        Ok(w)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn find(&self, id: FindId) -> Option<&Find> {
        self.rooms.values().flat_map(|r| r.finds.iter()).find(|f| f.id == id)
    }

    fn find_mut(&mut self, id: FindId) -> Option<&mut Find> {
        self.rooms.values_mut().flat_map(|r| r.finds.iter_mut()).find(|f| f.id == id)
    }

    fn add(&mut self, key: &str, kind: FindKind) -> FindId {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.rooms.entry(key.to_string()).or_default().finds.push(Find { id, kind, anchor: None, visits: 0, hits: 0, broken_at: None });
        id
    }

    /// The knight explored window `id` (a window he never stood on). Rolls the app's room
    /// the first time; places anything not yet placed. Returns the finds revealed now.
    pub fn explored(&mut self, id: WindowId, desk: &DesktopSnapshot, terrain: &Terrain, rules: &Rules, width: impl Fn(FindKind) -> f32) -> Vec<FindId> {
        // Fullscreen windows (games, videos) are not rooms: nothing is found there.
        let Some(key) = desk.window(id).filter(|w| !w.fullscreen).and_then(app_key) else { return vec![] };
        if !self.rooms.contains_key(&key) {
            let kinds = roll(self.seed, &key, rules);
            self.rooms.insert(key.clone(), Room::default());
            for k in kinds {
                self.add(&key, k);
            }
        }
        self.hosts.insert(key.clone(), id);
        self.place_pending(&key, id, terrain, width)
    }

    /// Debug / gift: put a `kind` find into window `id`'s room right now.
    pub fn force(&mut self, kind: FindKind, id: WindowId, desk: &DesktopSnapshot, terrain: &Terrain, width: impl Fn(FindKind) -> f32) -> Option<FindId> {
        let key = desk.window(id).filter(|w| !w.fullscreen).and_then(app_key)?;
        let taken = self.taken(&key, id, terrain, &width);
        let anchor = place(terrain, id, width(kind), self.seed ^ self.next_id, &taken)?;
        let fid = self.add(&key, kind);
        if let Some(f) = self.find_mut(fid) {
            f.anchor = Some(anchor);
        }
        self.hosts.insert(key, id);
        Some(fid)
    }

    fn place_pending(&mut self, key: &str, id: WindowId, terrain: &Terrain, width: impl Fn(FindKind) -> f32) -> Vec<FindId> {
        let seed = self.seed;
        let mut taken = self.taken(key, id, terrain, &width);
        let Some(room) = self.rooms.get_mut(key) else { return vec![] };
        let mut placed = vec![];
        for f in room.finds.iter_mut().filter(|f| f.anchor.is_none()) {
            let w = width(f.kind);
            f.anchor = place(terrain, id, w, seed ^ f.id.wrapping_mul(0x9E37_79B9), &taken);
            if let (Some(a), Some(rect)) = (f.anchor, terrain.window_rect(id)) {
                placed.push(f.id);
                taken.push((Vec2::new(rect.left + a.fx * rect.width(), rect.top + a.dy), w));
            }
        }
        placed
    }

    /// Where the anchored finds of a room sit on window `id` (bottom center, width).
    fn taken(&self, key: &str, id: WindowId, terrain: &Terrain, width: &impl Fn(FindKind) -> f32) -> Vec<(Vec2, f32)> {
        let Some(rect) = terrain.window_rect(id) else { return vec![] };
        let finds = self.rooms.get(key).map_or(&[][..], |r| r.finds.as_slice());
        finds.iter().filter_map(|f| f.anchor.map(|a| (Vec2::new(rect.left + a.fx * rect.width(), rect.top + a.dy), width(f.kind)))).collect()
    }

    /// Finds on screen: for every app with a room, on its hosting window (the one last
    /// explored if still open, else its first visible window that has room).
    pub fn shown(&mut self, desk: &DesktopSnapshot, terrain: &Terrain, now_unix: u64, rules: &Rules, width: impl Fn(FindKind) -> f32) -> Vec<Shown> {
        let regrow = (rules.regrow_hours * 3600.0) as u64;
        for f in self.rooms.values_mut().flat_map(|r| r.finds.iter_mut()) {
            if f.broken_at.is_some_and(|t| now_unix >= t + regrow) {
                f.broken_at = None;
                f.hits = 0;
            }
        }
        let mut out = vec![];
        let mut hosts = BTreeMap::new();
        for (key, room) in &self.rooms {
            let live: Vec<&Find> = room.finds.iter().filter(|f| f.anchor.is_some() && f.broken_at.is_none()).collect();
            if live.is_empty() {
                continue;
            }
            let prefer = self.hosts.get(key).copied().filter(|id| desk.window(*id).is_some_and(|w| app_key(w).as_deref() == Some(key)));
            let candidates = prefer.into_iter().chain(desk.windows.iter().filter(|w| !w.fullscreen && app_key(w).as_deref() == Some(key)).map(|w| w.id));
            for wid in candidates {
                let Some(rect) = terrain.window_rect(wid) else { continue };
                let here: Vec<Shown> = live
                    .iter()
                    .filter_map(|f| {
                        let pos = resolve(terrain, wid, rect, f.anchor?, width(f.kind))?;
                        Some(Shown { id: f.id, kind: f.kind, window: wid, pos, favour: f.favour() })
                    })
                    .collect();
                if !here.is_empty() {
                    hosts.insert(key.clone(), wid);
                    out.extend(here);
                    break;
                }
            }
        }
        for (k, v) in hosts {
            self.hosts.insert(k, v);
        }
        out
    }

    /// The knight used a find (sat, bathed, mined).
    pub fn used(&mut self, id: FindId) {
        if let Some(f) = self.find_mut(id) {
            f.visits += 1;
        }
    }

    /// A slash landed on deposit `id` at `at`. Returns the geo drop when it breaks.
    pub fn strike(&mut self, id: FindId, at: Vec2, now_unix: u64, rules: &Rules) -> Option<DepositDrop> {
        let seed = self.seed;
        let f = self.find_mut(id).filter(|f| f.kind == FindKind::Deposit && f.broken_at.is_none())?;
        f.hits += 1;
        if f.hits < rules.deposit_hp {
            return None;
        }
        f.broken_at = Some(now_unix);
        f.visits += 1;
        let geo = 5 + (Mix(seed ^ id ^ (now_unix / 86_400)).next() % 11) as u32;
        Some(DepositDrop { at, geo })
    }

    /// Forget everything (debug).
    pub fn clear(&mut self) {
        self.rooms.clear();
        self.hosts.clear();
    }
}

/// What the brain knows about the finds on screen. `stand` gives where to stand to use a
/// find of each kind (benches / springs: on it; deposits: beside it, facing it).
pub fn known_places(shown: &[Shown], fresh: impl Fn(FindId) -> bool, stand: impl Fn(&Shown) -> Vec2) -> Vec<KnownPlace> {
    shown.iter().map(|s| KnownPlace { id: s.id, kind: s.kind.place_kind(), at: stand(s), center: s.pos, favour: s.favour, fresh: fresh(s.id) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Monitor;
    use crate::geom::RectI;
    use crate::terrain::TerrainOptions;

    fn win(id: WindowId, rect: RectI, process: &str) -> WindowInfo {
        WindowInfo {
            id,
            rect,
            title: String::new(),
            class: String::new(),
            process: process.into(),
            pid: 1,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        }
    }

    fn desk(windows: Vec<WindowInfo>) -> (DesktopSnapshot, Terrain) {
        let d = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows,
        };
        let t = Terrain::from_desktop(&d, &TerrainOptions::default());
        (d, t)
    }

    fn w(_: FindKind) -> f32 {
        150.0
    }

    /// Rules that always find something of `kind`.
    fn always(kind: FindKind) -> Rules {
        let one = |k: FindKind| if std::mem::discriminant(&k) == std::mem::discriminant(&kind) { 1.0 } else { 0.0 };
        Rules { nothing: 0.0, bench: one(FindKind::Bench { style: 0 }), spring: one(FindKind::Spring), deposit: one(FindKind::Deposit), ..Rules::default() }
    }

    #[test]
    fn rolls_are_deterministic_per_app_and_vary_across_apps() {
        let r = Rules::default();
        assert_eq!(roll(7, "paint", &r), roll(7, "paint", &r));
        let apps: Vec<String> = (0..200).map(|i| format!("app{i}")).collect();
        let found = apps.iter().filter(|a| !roll(7, a, &r).is_empty()).count();
        assert!((60..120).contains(&found), "about 45% of rooms hold something: {found}");
        let springs = apps.iter().filter(|a| roll(7, a, &r) == vec![FindKind::Spring]).count();
        assert!((8..40).contains(&springs), "springs are rare: {springs}");
        let other_seed = apps.iter().filter(|a| roll(7, a, &r) != roll(8, a, &r)).count();
        assert!(other_seed > 40, "another world, other finds");
    }

    #[test]
    fn app_key_is_the_lowercased_exe() {
        assert_eq!(app_key(&win(1, RectI::new(0, 0, 10, 10), "MSPaint.EXE")).as_deref(), Some("mspaint"));
        assert_eq!(app_key(&win(1, RectI::new(0, 0, 10, 10), "")), None);
        assert_eq!(app_key(&win(1, RectI::new(0, 0, 10, 10), "skong.exe")), None);
    }

    #[test]
    fn first_visit_reveals_and_only_once() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe")]);
        let mut world = World::new(3);
        let rules = always(FindKind::Spring);
        let found = world.explored(1, &d, &t, &rules, w);
        assert_eq!(found.len(), 1);
        assert_eq!(world.find(found[0]).map(|f| f.kind), Some(FindKind::Spring));
        assert!(world.explored(1, &d, &t, &rules, w).is_empty(), "already there, nothing new");
    }

    #[test]
    fn placed_on_the_title_bar_following_moves_and_vanishing_with_the_window() {
        let rules = always(FindKind::Deposit);
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe")]);
        let mut world = World::new(3);
        world.explored(1, &d, &t, &rules, w);
        let s = world.shown(&d, &t, 0, &rules, w);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos.y, 400.0, "on the title bar");
        assert!(s[0].pos.x - 75.0 >= 200.0 + EDGE_MARGIN && s[0].pos.x + 75.0 <= 900.0 - EDGE_MARGIN);
        let x0 = s[0].pos.x;

        let (d2, t2) = desk(vec![win(1, RectI::new(500, 300, 1200, 800), "paint.exe")]);
        let s2 = world.shown(&d2, &t2, 0, &rules, w);
        assert_eq!(s2[0].pos, Vec2::new(x0 + 300.0, 300.0), "rides the window");

        let (d3, t3) = desk(vec![win(2, RectI::new(500, 300, 1200, 800), "notepad.exe")]);
        assert!(world.shown(&d3, &t3, 0, &rules, w).is_empty(), "closed with its window");

        // Reopened later (new window id): still there, same relative spot.
        let (d4, t4) = desk(vec![win(9, RectI::new(200, 400, 900, 900), "paint.exe")]);
        let s4 = world.shown(&d4, &t4, 0, &rules, w);
        assert_eq!(s4.len(), 1);
        assert_eq!(s4[0].pos, Vec2::new(x0, 400.0));
    }

    #[test]
    fn covered_finds_hide_and_narrow_windows_wait_for_room() {
        let rules = always(FindKind::Bench { style: 0 });
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 360, 900), "tiny.exe")]);
        let mut world = World::new(3);
        assert!(world.explored(1, &d, &t, &rules, w).is_empty(), "no room on a 160 px window");
        let (d2, t2) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "tiny.exe")]);
        assert_eq!(world.explored(1, &d2, &t2, &rules, w).len(), 1, "placed once there is room");
        // A window on top covering the whole title bar hides it.
        let (d3, t3) = desk(vec![win(2, RectI::new(100, 300, 1000, 600), "other.exe"), win(1, RectI::new(200, 400, 900, 900), "tiny.exe")]);
        assert!(world.shown(&d3, &t3, 0, &rules, w).is_empty());
    }

    #[test]
    fn persistence_round_trip() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe"), win(2, RectI::new(1000, 200, 1800, 700), "code.exe")]);
        let mut world = World::new(11);
        world.explored(1, &d, &t, &always(FindKind::Bench { style: 2 }), w);
        world.explored(2, &d, &t, &always(FindKind::Deposit), w);
        let ids: Vec<FindId> = world.rooms.values().flat_map(|r| r.finds.iter().map(|f| f.id)).collect();
        world.used(ids[0]);
        let back = World::from_json(&world.to_json()).expect("parse");
        assert_eq!(back.rooms, world.rooms);
        assert_eq!(back.seed, 11);
        let mut back = back;
        let fresh = back.add("x", FindKind::Spring);
        assert!(!ids.contains(&fresh), "ids keep counting after a reload");
    }

    #[test]
    fn deposits_break_drop_geo_and_regrow() {
        let rules = always(FindKind::Deposit);
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe")]);
        let mut world = World::new(3);
        let id = world.explored(1, &d, &t, &rules, w)[0];
        let at = Vec2::new(500.0, 400.0);
        for _ in 0..rules.deposit_hp - 1 {
            assert_eq!(world.strike(id, at, 1000, &rules), None);
        }
        let drop = world.strike(id, at, 1000, &rules).expect("broke");
        assert!((5..=15).contains(&drop.geo));
        assert!(world.shown(&d, &t, 2000, &rules, w).is_empty(), "broken");
        assert_eq!(world.shown(&d, &t, 1000 + 25 * 3600, &rules, w).len(), 1, "regrown a day later");
    }

    #[test]
    fn finds_in_one_window_do_not_overlap() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 1000, 900), "paint.exe")]);
        let mut world = World::new(3);
        for k in [FindKind::Deposit, FindKind::Bench { style: 0 }, FindKind::Spring] {
            world.force(k, 1, &d, &t, w).expect("room for three");
        }
        let s = world.shown(&d, &t, 0, &Rules::default(), w);
        assert_eq!(s.len(), 3);
        for (i, a) in s.iter().enumerate() {
            for b in &s[i + 1..] {
                assert!((a.pos.x - b.pos.x).abs() >= 150.0, "{a:?} overlaps {b:?}");
            }
        }
    }
}
