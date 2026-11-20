//! Things *found* inside windows. Each app is a "room" of Hallownest: the first time the
//! knight explores one of its windows, the room is rolled (deterministically from the
//! world seed and the app's name) for what was always there — a bench, a geo deposit, or
//! (most often) nothing. The record persists (the app saves [`World`] as JSON), so the same
//! things are in the same place whenever that app's window is open; they ride along with
//! the window and vanish when it closes.
//!
//! Finds sit on **lasting** surfaces of their window — its title bar or a content line (a
//! table border, a divider) that has been on screen for a while ([`Site::min_age`],
//! tracked by [`LedgeAges`]) — anchored relative to the window. When that content goes
//! away (scrolled off), the find hides until the line is back.
//!
//! Nothing overlaps: placement asks the shared [`Occupancy`], and [`settle`] resolves
//! collisions every frame (a find slides along its ledge to make room, or hides while it
//! can't).
//!
//! This module is pure: the app resolves positions every frame with [`World::shown`] and
//! [`settle`], draws the props and tells the world when the knight explored, used or
//! struck one.
use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::ai::{KnownPlace, PlaceKind};
use crate::desktop::{DesktopSnapshot, WindowId, WindowInfo};
use crate::geom::{RectF, Vec2};
use crate::lasting::LedgeAges;
use crate::occupancy::{ClaimKey, ClaimKind, Occupancy};
use crate::terrain::{Ledge, LedgeKind, Terrain};

pub type FindId = u64;

/// What lives in a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindKind {
    /// A bench of one of the `props.json` styles.
    Bench { style: usize },
    /// A geo rock: slashed for Geo; breaks after a few hits and is gone, and another one
    /// turns up in some other app's window a little later (something new to travel to).
    Deposit,
}

impl FindKind {
    pub fn place_kind(self) -> PlaceKind {
        match self {
            FindKind::Bench { .. } => PlaceKind::Bench,
            FindKind::Deposit => PlaceKind::Deposit,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FindKind::Bench { .. } => "bench",
            FindKind::Deposit => "geo deposit",
        }
    }

    /// Who keeps its spot when two props collide (higher wins).
    fn priority(self) -> i32 {
        match self {
            FindKind::Bench { .. } => 50,
            FindKind::Deposit => 40,
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
    /// `None` until the window had room for it (it is placed on a later visit).
    pub anchor: Option<Anchor>,
    /// Times the knight used it (sat, mined): favourite places get revisited.
    #[serde(default)]
    pub visits: u32,
    /// Deposit: hits taken since it (re)grew.
    #[serde(default)]
    pub hits: u32,
    /// Deposit: unix time it broke (old saves only: broken rocks are removed now).
    #[serde(default)]
    pub broken_at: Option<u64>,
    /// The user dragged it onto a monitor floor: its feet point there (it stays in its
    /// app's room, and shows whether that app's windows are open or not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<[f32; 2]>,
}

/// Where the user dropped a find (see [`World::move_find`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FindSpot {
    /// On a ledge of window `id` at feet point `at`.
    Window { id: WindowId, at: Vec2 },
    /// On a monitor floor at `at`.
    Floor { at: Vec2 },
}

/// [`Shown::window`] of a find on the floor (no window holds it).
pub const FLOOR_WINDOW: WindowId = 0;

impl Find {
    /// 0 = never used, → 1 for a favourite.
    pub fn favour(&self) -> f32 {
        self.visits as f32 / (self.visits as f32 + 3.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Room {
    /// Finds of kinds this version no longer knows (e.g. the retired hot springs) are
    /// dropped on load.
    #[serde(deserialize_with = "known_finds")]
    pub finds: Vec<Find>,
}

fn known_finds<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Find>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(raw.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

/// Rarity and upkeep.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Weights of nothing / bench / deposit for a newly explored app.
    pub nothing: f32,
    pub bench: f32,
    pub deposit: f32,
    /// Relative weight per bench style (index = `props.json` order).
    pub bench_styles: Vec<f32>,
    /// Seconds (range) until a broken deposit turns up again elsewhere.
    pub relocate_secs: [u64; 2],
    /// Hits that break a deposit.
    pub deposit_hp: u32,
}

impl Default for Rules {
    fn default() -> Self {
        Self { nothing: 0.6, bench: 0.25, deposit: 0.15, bench_styles: vec![3.0, 1.0, 1.5, 1.5, 1.5], relocate_secs: [60, 180], deposit_hp: 3 }
    }
}

/// Keep finds this far from a ledge's ends (px).
pub(crate) const EDGE_MARGIN: f32 = 24.0;
/// Leave this much room above a find (the knight standing on it): a maximized window's
/// title bar at the very top of the screen does not qualify (px).
pub(crate) const HEADROOM: f32 = 48.0;
/// A content ledge may drift this much (re-detection jitter) and still carry its find (px).
pub(crate) const LEDGE_SLACK: f32 = 6.0;
/// Surfaces must have been on screen this long to carry a find (s).
pub const LASTING_SECS: f32 = 30.0;

/// Size of a find on screen, and how much free ledge it needs (e.g. room to stand beside
/// a deposit to mine it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub w: f32,
    pub h: f32,
    /// Ledge width needed (>= `w`).
    pub room: f32,
}

impl Footprint {
    /// Screen rect of a find whose bottom center is `pos`.
    pub fn rect(&self, pos: Vec2) -> RectF {
        RectF::new(pos.x - self.w * 0.5, pos.y - self.h, pos.x + self.w * 0.5, pos.y)
    }
}

/// Where finds can go right now: the terrain, which ledges have lasted, and what is
/// already taken.
pub struct Site<'a> {
    pub terrain: &'a Terrain,
    pub ages: &'a LedgeAges,
    /// Clock of `ages`.
    pub now: f64,
    /// Ledges younger than this (s) carry nothing ([`LASTING_SECS`]; 0 for debug reveals).
    pub min_age: f32,
    pub occupancy: &'a Occupancy,
    pub footprint: &'a dyn Fn(FindKind) -> Footprint,
}

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
    /// Broken deposits waiting to turn up elsewhere: (app they broke in, unix time due).
    /// A rock never regrows in place: the next one is somewhere new to travel to.
    #[serde(default)]
    relocations: Vec<(String, u64)>,
    /// Which window of an app hosts its finds right now (not saved).
    #[serde(skip)]
    hosts: BTreeMap<String, WindowId>,
}

fn version() -> u32 {
    2
}

/// A find on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub id: FindId,
    pub kind: FindKind,
    pub window: WindowId,
    /// Bottom center, resting on its ledge.
    pub pos: Vec2,
    /// The ledge it rests on (x0, x1).
    pub span: (f32, f32),
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
#[derive(Clone, Debug)]
pub(crate) struct Mix(pub(crate) u64);

impl Mix {
    pub(crate) fn new(seed: u64, key: &str) -> Self {
        // FNV-1a over the key, folded into the seed.
        let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        Mix(seed ^ h)
    }
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub(crate) fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub(crate) fn pick(&mut self, weights: &[f32]) -> usize {
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
    match m.pick(&[rules.nothing, rules.bench, rules.deposit]) {
        1 => vec![FindKind::Bench { style: m.pick(&rules.bench_styles) }],
        2 => vec![FindKind::Deposit],
        _ => vec![],
    }
}

/// Window-owned ledges of `id` that could carry a find needing `room` px: lasting, wide
/// enough, not at the very top of the screen.
fn candidate_ledges<'t>(site: &'t Site, id: WindowId, room: f32) -> Vec<&'t Ledge> {
    let t = site.terrain;
    t.ledges
        .iter()
        .filter(|l| {
            l.owner == Some(id)
                && l.kind != LedgeKind::Ground
                && l.x1 - l.x0 >= room + 2.0 * EDGE_MARGIN
                && l.y >= t.bounds.top + HEADROOM
                && site.ages.age(t, l, site.now) >= site.min_age
        })
        .collect()
}

/// Choose a spot for a find of `kind` on window `id`: any lasting ledge of it (title bar or
/// content line, picked by width so finds spread out), where its rect is free (`free`).
/// Deterministic in `salt`.
pub fn place(site: &Site, id: WindowId, kind: FindKind, salt: u64, free: &dyn Fn(RectF) -> bool) -> Option<Anchor> {
    let rect = site.terrain.window_rect(id)?;
    let fp = (site.footprint)(kind);
    let ledges = candidate_ledges(site, id, fp.room);
    if ledges.is_empty() {
        return None;
    }
    let ok = |l: &Ledge, x: f32| free(fp.rect(Vec2::new(x, l.y)));
    let range = |l: &Ledge| (l.x0 + EDGE_MARGIN + fp.room * 0.5, l.x1 - EDGE_MARGIN - fp.room * 0.5);
    let mut mix = Mix(salt);
    let weights: Vec<f32> = ledges.iter().map(|l| l.x1 - l.x0).collect();
    let random = (0..16).find_map(|_| {
        let l = ledges[mix.pick(&weights)];
        let (lo, hi) = range(l);
        let x = lo + (hi - lo) * mix.f32();
        ok(l, x).then_some((l, x))
    });
    let (l, x) = random.or_else(|| ledges.iter().find_map(|l| {
        let (lo, hi) = range(l);
        (0..=20).map(|i| lo + (hi - lo) * i as f32 / 20.0).find(|&x| ok(l, x)).map(|x| (*l, x))
    }))?;
    Some(Anchor { fx: (x - rect.left) / rect.width().max(1.0), dy: l.y - rect.top })
}

/// Where an anchored find of `width` px is on window `rect` right now: on a ledge of that
/// window that carries its whole footprint. Returns the bottom center and the ledge span;
/// `None` while covered / scrolled away / out of room.
/// `settled`: the ledge has been on screen long enough to carry it (a line that just
/// appeared where the old one was — new text, a scrolled page — doesn't count yet; see
/// [`crate::lasting::SETTLED_SECS`]).
pub fn resolve(terrain: &Terrain, id: WindowId, rect: RectF, a: Anchor, width: f32, settled: &dyn Fn(&Ledge) -> bool) -> Option<(Vec2, (f32, f32))> {
    let x = rect.left + a.fx * rect.width();
    let y = rect.top + a.dy;
    let half = width * 0.5;
    terrain
        .ledges
        .iter()
        .filter(|l| l.owner == Some(id) && (l.y - y).abs() <= LEDGE_SLACK && l.x0 <= x - half && x + half <= l.x1 && settled(l))
        .min_by(|p, q| (p.y - y).abs().total_cmp(&(q.y - y).abs()))
        .map(|l| (Vec2::new(x, l.y), (l.x0, l.x1)))
}

/// Collision pass over this frame's finds (after the higher-priority claims — placed
/// benches, NPCs — are in `occ`): each find, highest priority first, keeps its spot if it
/// is free, else slides along its ledge to the nearest free spot, else hides while it
/// can't fit. Survivors are claimed as [`ClaimKind::Find`].
pub fn settle(mut shown: Vec<Shown>, occ: &mut Occupancy, footprint: &dyn Fn(FindKind) -> Footprint) -> Vec<Shown> {
    occ.release_kind(ClaimKind::Find);
    let prio = |s: &Shown| s.kind.priority() + (s.favour * 5.0) as i32;
    shown.sort_by(|a, b| prio(b).cmp(&prio(a)).then(a.id.cmp(&b.id)));
    let mut out = Vec::with_capacity(shown.len());
    for mut s in shown {
        let fp = footprint(s.kind);
        let rect = fp.rect(s.pos);
        let key = ClaimKey::new(ClaimKind::Find, s.id);
        let Some(dx) = occ.nudge(rect, s.span.0, s.span.1, Some(key), 4.0) else { continue };
        s.pos.x += dx;
        occ.claim(key, fp.rect(s.pos), prio(&s));
        out.push(s);
    }
    out
}

impl World {
    pub fn new(seed: u64) -> Self {
        Self { version: version(), seed, rooms: BTreeMap::new(), next_id: 1, relocations: Vec::new(), hosts: BTreeMap::new() }
    }

    pub fn from_json(s: &str) -> serde_json::Result<World> {
        let mut w: World = serde_json::from_str(s)?;
        let max = w.rooms.values().flat_map(|r| r.finds.iter().map(|f| f.id)).max().unwrap_or(0);
        w.next_id = w.next_id.max(max + 1);
        w.version = version();
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
        self.rooms.entry(key.to_string()).or_default().finds.push(Find { id, kind, anchor: None, visits: 0, hits: 0, broken_at: None, floor: None });
        id
    }

    /// The knight explored window `id` (a window he never stood on). Rolls the app's room
    /// the first time; places anything not yet placed. Returns the finds revealed now.
    pub fn explored(&mut self, id: WindowId, desk: &DesktopSnapshot, site: &Site, rules: &Rules) -> Vec<FindId> {
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
        self.place_pending(&key, id, site)
    }

    /// The knight came onto window `id` (any way): finds of its app that had no lasting
    /// room before are placed now, while he is there to find them.
    pub fn arrived(&mut self, id: WindowId, desk: &DesktopSnapshot, site: &Site) -> Vec<FindId> {
        let Some(key) = desk.window(id).filter(|w| !w.fullscreen).and_then(app_key) else { return vec![] };
        let pending = self.rooms.get(&key).is_some_and(|r| r.finds.iter().any(|f| f.anchor.is_none()));
        if !pending {
            return vec![];
        }
        self.hosts.insert(key.clone(), id);
        self.place_pending(&key, id, site)
    }

    /// Debug / gift: put a `kind` find into window `id`'s room right now.
    pub fn force(&mut self, kind: FindKind, id: WindowId, desk: &DesktopSnapshot, site: &Site) -> Option<FindId> {
        let key = desk.window(id).filter(|w| !w.fullscreen).and_then(app_key)?;
        let taken = self.taken(&key, id, site);
        let free = |r: RectF| site.occupancy.is_free(r, None) && !taken.iter().any(|t| t.overlaps(&r));
        let anchor = place(site, id, kind, self.seed ^ self.next_id, &free)?;
        let fid = self.add(&key, kind);
        if let Some(f) = self.find_mut(fid) {
            f.anchor = Some(anchor);
        }
        self.hosts.insert(key, id);
        Some(fid)
    }

    /// Test panel: put a `kind` find into window `id`'s room right at `at` (a feet point on
    /// one of its ledges, picked by the user), if its rect is free there.
    pub fn force_at(&mut self, kind: FindKind, id: WindowId, at: Vec2, desk: &DesktopSnapshot, site: &Site) -> Result<FindId, &'static str> {
        let key = desk.window(id).filter(|w| !w.fullscreen).and_then(app_key).ok_or("not a window things can be found in")?;
        let rect = site.terrain.window_rect(id).ok_or("that window is gone")?;
        if !site.terrain.ledges.iter().any(|l| l.owner == Some(id) && (l.y - at.y).abs() <= LEDGE_SLACK && l.x0 <= at.x && at.x <= l.x1) {
            return Err("no ledge of that window there");
        }
        let r = (site.footprint)(kind).rect(at);
        let taken = self.taken(&key, id, site);
        if !site.occupancy.is_free(r, None) || taken.iter().any(|t| t.overlaps(&r)) {
            return Err("something already stands there");
        }
        let fid = self.add(&key, kind);
        if let Some(f) = self.find_mut(fid) {
            f.anchor = Some(Anchor { fx: (at.x - rect.left) / rect.width().max(1.0), dy: at.y - rect.top });
        }
        self.hosts.insert(key, id);
        Ok(fid)
    }

    fn place_pending(&mut self, key: &str, id: WindowId, site: &Site) -> Vec<FindId> {
        let seed = self.seed;
        let mut taken = self.taken(key, id, site);
        let Some(room) = self.rooms.get_mut(key) else { return vec![] };
        let Some(rect) = site.terrain.window_rect(id) else { return vec![] };
        let mut placed = vec![];
        for f in room.finds.iter_mut().filter(|f| f.anchor.is_none()) {
            let free = |r: RectF| site.occupancy.is_free(r, None) && !taken.iter().any(|t| t.overlaps(&r));
            f.anchor = place(site, id, f.kind, seed ^ f.id.wrapping_mul(0x9E37_79B9), &free);
            if let Some(a) = f.anchor {
                placed.push(f.id);
                taken.push((site.footprint)(f.kind).rect(Vec2::new(rect.left + a.fx * rect.width(), rect.top + a.dy)));
            }
        }
        placed
    }

    /// Rects of the anchored finds of a room on window `id` (shown or not).
    fn taken(&self, key: &str, id: WindowId, site: &Site) -> Vec<RectF> {
        let Some(rect) = site.terrain.window_rect(id) else { return vec![] };
        let finds = self.rooms.get(key).map_or(&[][..], |r| r.finds.as_slice());
        finds
            .iter()
            .filter(|f| f.floor.is_none())
            .filter_map(|f| f.anchor.map(|a| (site.footprint)(f.kind).rect(Vec2::new(rect.left + a.fx * rect.width(), rect.top + a.dy))))
            .map(|r| RectF::new(r.left - 8.0, r.top, r.right + 8.0, r.bottom))
            .collect()
    }

    /// Finds on screen (before collisions, see [`settle`]): for every app with a room, on
    /// its hosting window (the one last explored if still open, else its first visible
    /// window where something resolves).
    pub fn shown(&mut self, desk: &DesktopSnapshot, terrain: &Terrain, now_unix: u64, rules: &Rules, footprint: &dyn Fn(FindKind) -> Footprint, settled: &dyn Fn(&Ledge) -> bool) -> Vec<Shown> {
        // Worlds saved before deposits moved on keep broken rocks to regrow: they go now,
        // and turn up elsewhere instead.
        let _ = rules;
        for (key, room) in &mut self.rooms {
            let broken = room.finds.iter().filter(|f| f.broken_at.is_some()).count();
            room.finds.retain(|f| f.broken_at.is_none());
            self.relocations.extend(std::iter::repeat_n((key.clone(), now_unix), broken));
        }
        let mut out = vec![];
        let mut hosts = BTreeMap::new();
        for (key, room) in &self.rooms {
            // Dragged onto a floor: there, on its ground ledge (hidden while that's gone).
            for f in room.finds.iter().filter(|f| f.broken_at.is_none()) {
                let Some([x, y]) = f.floor else { continue };
                let half = footprint(f.kind).w * 0.5;
                let ground = terrain.ledges.iter().find(|l| l.kind == LedgeKind::Ground && (l.y - y).abs() <= LEDGE_SLACK && l.x0 <= x - half && x + half <= l.x1);
                if let Some(l) = ground {
                    out.push(Shown { id: f.id, kind: f.kind, window: FLOOR_WINDOW, pos: Vec2::new(x, l.y), span: (l.x0, l.x1), favour: f.favour() });
                }
            }
            let live: Vec<&Find> = room.finds.iter().filter(|f| f.anchor.is_some() && f.floor.is_none() && f.broken_at.is_none()).collect();
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
                        let (pos, span) = resolve(terrain, wid, rect, f.anchor?, footprint(f.kind).w, settled)?;
                        Some(Shown { id: f.id, kind: f.kind, window: wid, pos, span, favour: f.favour() })
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

    /// The app (room) find `id` belongs to.
    pub fn room_of(&self, id: FindId) -> Option<&str> {
        self.rooms.iter().find(|(_, r)| r.finds.iter().any(|f| f.id == id)).map(|(k, _)| k.as_str())
    }

    /// Can a find be dropped on window `id`'s edges? Only windows that are rooms (an app's,
    /// not fullscreen, not ours).
    pub fn is_room(desk: &DesktopSnapshot, id: WindowId) -> bool {
        desk.window(id).filter(|w| !w.fullscreen).and_then(app_key).is_some()
    }

    /// The user dragged find `id` to `spot`. Its visits and hits (a half-mined rock) go
    /// with it. Dropped on a window, it moves into that app's room, anchored there (it
    /// rides that window from now on and, a deposit, takes that app's region's look);
    /// dropped on a floor, it stays in its own room and lies there. `false`: no such find,
    /// or that window isn't a room.
    pub fn move_find(&mut self, id: FindId, spot: FindSpot, desk: &DesktopSnapshot, terrain: &Terrain, rules: &Rules) -> bool {
        let Some(from) = self.room_of(id).map(str::to_string) else { return false };
        match spot {
            FindSpot::Floor { at } => {
                let Some(f) = self.find_mut(id) else { return false };
                f.floor = Some([at.x, at.y]);
                true
            }
            FindSpot::Window { id: wid, at } => {
                let Some(key) = desk.window(wid).filter(|w| !w.fullscreen).and_then(app_key) else { return false };
                let Some(rect) = terrain.window_rect(wid) else { return false };
                let Some(i) = self.rooms.get(&from).and_then(|r| r.finds.iter().position(|f| f.id == id)) else { return false };
                let Some(mut f) = self.rooms.get_mut(&from).map(|r| r.finds.remove(i)) else { return false };
                f.floor = None;
                f.anchor = Some(Anchor { fx: (at.x - rect.left) / rect.width().max(1.0), dy: at.y - rect.top });
                if !self.rooms.contains_key(&key) {
                    // A room never explored: rolled now (what it always held), plus this.
                    let kinds = roll(self.seed, &key, rules);
                    self.rooms.insert(key.clone(), Room::default());
                    for k in kinds {
                        self.add(&key, k);
                    }
                }
                self.rooms.entry(key.clone()).or_default().finds.push(f);
                self.hosts.insert(key, wid);
                true
            }
        }
    }

    /// The knight used a find (sat, mined).
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
        let geo = 5 + (Mix(seed ^ id ^ (now_unix / 86_400)).next() % 11) as u32;
        // Gone for good; another one turns up in some other app's window in a while.
        let key = self.rooms.iter().find(|(_, r)| r.finds.iter().any(|f| f.id == id)).map(|(k, _)| k.clone());
        if let Some(k) = key {
            if let Some(room) = self.rooms.get_mut(&k) {
                room.finds.retain(|f| f.id != id);
            }
            let wait = rules.relocate_secs[0] + Mix(seed ^ id ^ now_unix).next() % (rules.relocate_secs[1] - rules.relocate_secs[0]).max(1);
            self.relocations.push((k, now_unix + wait));
        }
        Some(DepositDrop { at, geo })
    }

    /// Broken deposits due to turn up again: place each in a visible window of *another*
    /// app (trying windows in a seed-shuffled order), on a ledge that lasted. Returns what
    /// was placed, with its window, to reveal; anything that found no spot waits.
    pub fn relocate_due(&mut self, desk: &DesktopSnapshot, site: &Site, now_unix: u64) -> Vec<(FindId, WindowId)> {
        let mut placed = vec![];
        let due: Vec<(String, u64)> = self.relocations.iter().filter(|(_, t)| now_unix >= *t).cloned().collect();
        for (from, t) in due {
            let mut wins: Vec<&WindowInfo> = desk.windows.iter().filter(|w| !w.fullscreen && app_key(w).is_some_and(|k| k != from)).collect();
            let mut mix = Mix(self.seed ^ t);
            for i in (1..wins.len()).rev() {
                wins.swap(i, (mix.next() % (i as u64 + 1)) as usize);
            }
            let ids: Vec<WindowId> = wins.iter().map(|w| w.id).collect();
            if let Some((fid, wid)) = ids.into_iter().find_map(|wid| self.force(FindKind::Deposit, wid, desk, site).map(|f| (f, wid))) {
                self.relocations.retain(|r| r != &(from.clone(), t));
                placed.push((fid, wid));
            }
        }
        placed
    }

    /// Forget everything (debug).
    pub fn clear(&mut self) {
        self.rooms.clear();
        self.hosts.clear();
    }
}

/// What the brain knows about the finds on screen. `stand` gives where to stand to use a
/// find of each kind (benches: on it; deposits: beside it, facing it).
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

    /// Adds a content line to window `id` (a table border, say).
    fn with_line(mut t: Terrain, id: WindowId, x0: f32, x1: f32, y: f32) -> Terrain {
        t.ledges.push(Ledge { x0, x1, y, kind: LedgeKind::Content, owner: Some(id) });
        t
    }

    fn fp(_: FindKind) -> Footprint {
        Footprint { w: 150.0, h: 60.0, room: 150.0 }
    }

    /// Ages after `secs` of seeing `t`.
    fn aged(t: &Terrain, secs: f32) -> LedgeAges {
        let mut a = LedgeAges::default();
        let mut now = 0.0;
        while now <= secs as f64 {
            a.observe(t, now);
            now += 1.0;
        }
        a
    }

    struct Env {
        terrain: Terrain,
        ages: LedgeAges,
        occ: Occupancy,
        now: f64,
        min_age: f32,
    }

    impl Env {
        fn new(terrain: Terrain) -> Self {
            let ages = aged(&terrain, 40.0);
            Self { terrain, ages, occ: Occupancy::default(), now: 40.0, min_age: LASTING_SECS }
        }
        fn site(&self) -> Site<'_> {
            Site { terrain: &self.terrain, ages: &self.ages, now: self.now, min_age: self.min_age, occupancy: &self.occ, footprint: &fp }
        }
    }

    /// Rules that always find something of `kind`.
    fn always(kind: FindKind) -> Rules {
        let one = |k: FindKind| if std::mem::discriminant(&k) == std::mem::discriminant(&kind) { 1.0 } else { 0.0 };
        Rules { nothing: 0.0, bench: one(FindKind::Bench { style: 0 }), deposit: one(FindKind::Deposit), ..Rules::default() }
    }

    #[test]
    fn rolls_are_deterministic_per_app_and_vary_across_apps() {
        let r = Rules::default();
        assert_eq!(roll(7, "paint", &r), roll(7, "paint", &r));
        let apps: Vec<String> = (0..200).map(|i| format!("app{i}")).collect();
        let found = apps.iter().filter(|a| !roll(7, a, &r).is_empty()).count();
        assert!((55..110).contains(&found), "about 40% of rooms hold something: {found}");
        let deposits = apps.iter().filter(|a| roll(7, a, &r) == vec![FindKind::Deposit]).count();
        assert!((12..50).contains(&deposits), "deposits are rarer than benches: {deposits}");
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
        let env = Env::new(t);
        let mut world = World::new(3);
        let rules = always(FindKind::Deposit);
        let found = world.explored(1, &d, &env.site(), &rules);
        assert_eq!(found.len(), 1);
        assert_eq!(world.find(found[0]).map(|f| f.kind), Some(FindKind::Deposit));
        assert!(world.explored(1, &d, &env.site(), &rules).is_empty(), "already there, nothing new");
    }

    #[test]
    fn placed_relative_to_the_window_following_moves_and_vanishing_with_it() {
        let rules = always(FindKind::Deposit);
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe")]);
        let env = Env::new(t);
        let mut world = World::new(3);
        world.explored(1, &d, &env.site(), &rules);
        let s = world.shown(&d, &env.terrain, 0, &rules, &fp, &|_| true);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pos.y, 400.0, "on the title bar (the only lasting ledge)");
        assert!(s[0].pos.x - 75.0 >= 200.0 + EDGE_MARGIN && s[0].pos.x + 75.0 <= 900.0 - EDGE_MARGIN);
        let x0 = s[0].pos.x;

        let (d2, t2) = desk(vec![win(1, RectI::new(500, 300, 1200, 800), "paint.exe")]);
        let s2 = world.shown(&d2, &t2, 0, &rules, &fp, &|_| true);
        assert_eq!(s2[0].pos, Vec2::new(x0 + 300.0, 300.0), "rides the window");

        let (d3, t3) = desk(vec![win(2, RectI::new(500, 300, 1200, 800), "notepad.exe")]);
        assert!(world.shown(&d3, &t3, 0, &rules, &fp, &|_| true).is_empty(), "closed with its window");

        // Reopened later (new window id): still there, same relative spot.
        let (d4, t4) = desk(vec![win(9, RectI::new(200, 400, 900, 900), "paint.exe")]);
        let s4 = world.shown(&d4, &t4, 0, &rules, &fp, &|_| true);
        assert_eq!(s4.len(), 1);
        assert_eq!(s4[0].pos, Vec2::new(x0, 400.0));
    }

    #[test]
    fn finds_go_on_lasting_content_lines_and_hide_when_they_scroll_away() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 1400, 900), "excel.exe")]);
        let t = with_line(t, 1, 210.0, 1390.0, 650.0);
        let env = Env::new(t);
        // Many rooms: finds spread over the title bar and the content line.
        let mut ys = std::collections::BTreeSet::new();
        for seed in 0..30 {
            let mut world = World::new(seed);
            world.force(FindKind::Deposit, 1, &d, &env.site()).expect("room");
            let s = world.shown(&d, &env.terrain, 0, &Rules::default(), &fp, &|_| true);
            ys.insert(s[0].pos.y as i32);
        }
        assert_eq!(ys.into_iter().collect::<Vec<_>>(), vec![400, 650], "not only title bars");

        // A find on the line hides while the line is gone and comes back with it.
        let mut world = World::new(1);
        let id = loop {
            let id = world.force(FindKind::Bench { style: 0 }, 1, &d, &env.site()).expect("room");
            if world.shown(&d, &env.terrain, 0, &Rules::default(), &fp, &|_| true).iter().any(|s| s.id == id && s.pos.y == 650.0) {
                break id;
            }
        };
        let (_, bare) = desk(vec![win(1, RectI::new(200, 400, 1400, 900), "excel.exe")]);
        assert!(!world.shown(&d, &bare, 0, &Rules::default(), &fp, &|_| true).iter().any(|s| s.id == id), "scrolled away");
        assert!(world.shown(&d, &env.terrain, 0, &Rules::default(), &fp, &|_| true).iter().any(|s| s.id == id), "back");
    }

    #[test]
    fn fresh_surfaces_carry_nothing_until_they_last() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 1400, 900), "excel.exe")]);
        let young = with_line(t, 1, 210.0, 1390.0, 650.0);
        let mut env = Env::new(young.clone());
        env.ages = aged(&young, 10.0);
        env.now = 10.0;
        let mut world = World::new(5);
        assert!(world.explored(1, &d, &env.site(), &always(FindKind::Bench { style: 1 })).is_empty(), "nothing has lasted yet");
        env.ages = aged(&young, 35.0);
        env.now = 35.0;
        assert_eq!(world.arrived(1, &d, &env.site()).len(), 1, "placed when he comes by later");
    }

    #[test]
    fn covered_finds_hide_and_narrow_windows_wait_for_room() {
        let rules = always(FindKind::Bench { style: 0 });
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 360, 900), "tiny.exe")]);
        let mut world = World::new(3);
        assert!(world.explored(1, &d, &Env::new(t).site(), &rules).is_empty(), "no room on a 160 px window");
        let (d2, t2) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "tiny.exe")]);
        assert_eq!(world.arrived(1, &d2, &Env::new(t2).site()).len(), 1, "placed once there is room");
        // A window on top covering the whole title bar hides it.
        let (d3, t3) = desk(vec![win(2, RectI::new(100, 300, 1000, 600), "other.exe"), win(1, RectI::new(200, 400, 900, 900), "tiny.exe")]);
        assert!(world.shown(&d3, &t3, 0, &rules, &fp, &|_| true).is_empty());
    }

    #[test]
    fn persistence_round_trip_and_old_springs_are_ignored() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe"), win(2, RectI::new(1000, 200, 1800, 700), "code.exe")]);
        let env = Env::new(t);
        let mut world = World::new(11);
        world.explored(1, &d, &env.site(), &always(FindKind::Bench { style: 2 }));
        world.explored(2, &d, &env.site(), &always(FindKind::Deposit));
        let ids: Vec<FindId> = world.rooms.values().flat_map(|r| r.finds.iter().map(|f| f.id)).collect();
        world.used(ids[0]);
        let back = World::from_json(&world.to_json()).expect("parse");
        assert_eq!(back.rooms, world.rooms);
        assert_eq!(back.seed, 11);
        let mut back = back;
        let fresh = back.add("x", FindKind::Deposit);
        assert!(!ids.contains(&fresh), "ids keep counting after a reload");

        // A v1 world with a hot spring: the spring is dropped, the rest kept.
        let v1 = r#"{"version":1,"seed":5,"rooms":{"paint":{"finds":[
            {"id":1,"kind":{"kind":"spring"},"anchor":{"fx":0.5,"dy":0.0},"visits":2},
            {"id":2,"kind":{"kind":"bench","style":0},"anchor":{"fx":0.3,"dy":0.0}}]}},"next_id":3}"#;
        let old = World::from_json(v1).expect("old worlds load");
        let finds = &old.rooms["paint"].finds;
        assert_eq!(finds.len(), 1);
        assert_eq!(finds[0].kind, FindKind::Bench { style: 0 });
        assert_eq!(old.next_id, 3);
    }

    #[test]
    fn broken_deposits_are_gone_and_turn_up_in_another_app() {
        let rules = always(FindKind::Deposit);
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe"), win(2, RectI::new(1000, 300, 1800, 900), "notepad.exe")]);
        let env = Env::new(t);
        let mut world = World::new(3);
        let id = world.explored(1, &d, &env.site(), &rules)[0];
        let at = Vec2::new(500.0, 400.0);
        for _ in 0..rules.deposit_hp - 1 {
            assert_eq!(world.strike(id, at, 1000, &rules), None);
        }
        let drop = world.strike(id, at, 1000, &rules).expect("broke");
        assert!((5..=15).contains(&drop.geo));
        assert!(world.shown(&d, &env.terrain, 1001, &rules, &fp, &|_| true).is_empty(), "gone");
        assert!(world.relocate_due(&d, &env.site(), 1001).is_empty(), "not right away");
        let placed = world.relocate_due(&d, &env.site(), 1000 + 200);
        assert_eq!(placed.len(), 1, "turned up again");
        assert_eq!(placed[0].1, 2, "in another app's window, not where it broke");
        let shown = world.shown(&d, &env.terrain, 1200, &rules, &fp, &|_| true);
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].kind, shown[0].window), (FindKind::Deposit, 2));
        // It never regrows in paint.
        assert!(world.shown(&d, &env.terrain, 1000 + 25 * 3600, &rules, &fp, &|_| true).iter().all(|s| s.window != 1));
    }

    #[test]
    fn placement_avoids_claims_and_other_finds() {
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 1000, 900), "paint.exe")]);
        let mut env = Env::new(t);
        // A placed bench in the middle of the title bar.
        env.occ.claim(ClaimKey::new(ClaimKind::Bench, 1), RectF::new(500.0, 340.0, 650.0, 400.0), 100);
        let mut world = World::new(3);
        for k in [FindKind::Deposit, FindKind::Bench { style: 0 }] {
            world.force(k, 1, &d, &env.site()).expect("room for two");
        }
        let s = settle(world.shown(&d, &env.terrain, 0, &Rules::default(), &fp, &|_| true), &mut env.occ, &fp);
        assert_eq!(s.len(), 2);
        let rects: Vec<RectF> = s.iter().map(|s| fp(s.kind).rect(s.pos)).collect();
        assert!(!rects[0].overlaps(&rects[1]));
        for r in rects {
            assert!(!r.overlaps(&RectF::new(500.0, 340.0, 650.0, 400.0)), "{r:?} sits in the bench");
        }
    }

    fn shown_at(id: FindId, kind: FindKind, x: f32, span: (f32, f32)) -> Shown {
        Shown { id, kind, window: 1, pos: Vec2::new(x, 400.0), span, favour: 0.0 }
    }

    #[test]
    fn colliding_finds_slide_apart_or_the_lesser_one_hides() {
        let mut occ = Occupancy::default();
        // Two windows' finds now on top of each other; the bench keeps its spot.
        let bench = shown_at(1, FindKind::Bench { style: 0 }, 500.0, (200.0, 1000.0));
        let rock = shown_at(2, FindKind::Deposit, 520.0, (200.0, 1000.0));
        let s = settle(vec![rock, bench], &mut occ, &fp);
        assert_eq!(s.len(), 2);
        let b = s.iter().find(|s| s.id == 1).unwrap();
        let r = s.iter().find(|s| s.id == 2).unwrap();
        assert_eq!(b.pos.x, 500.0, "the bench stays");
        assert!((r.pos.x - b.pos.x).abs() >= 150.0, "the rock slid aside: {r:?}");
        assert_eq!(occ.claims().len(), 2);

        // No room to slide on a short ledge: the rock hides while they overlap.
        let rock = shown_at(2, FindKind::Deposit, 520.0, (420.0, 600.0));
        let s = settle(vec![rock, bench], &mut occ, &fp);
        assert_eq!(s.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1]);

        // A placed bench (higher-priority claim) beats a found bench.
        let mut occ = Occupancy::default();
        occ.claim(ClaimKey::new(ClaimKind::Bench, 9), RectF::new(430.0, 340.0, 570.0, 400.0), 100);
        let s = settle(vec![shown_at(1, FindKind::Bench { style: 0 }, 500.0, (420.0, 600.0))], &mut occ, &fp);
        assert!(s.is_empty());
    }

    #[test]
    fn a_dragged_find_moves_rooms_or_lies_on_the_floor_keeping_its_hits() {
        let rules = always(FindKind::Deposit);
        let (d, t) = desk(vec![win(1, RectI::new(200, 400, 900, 900), "paint.exe"), win(2, RectI::new(1000, 300, 1800, 700), "notes.exe")]);
        let env = Env::new(t);
        let mut world = World::new(3);
        let id = world.explored(1, &d, &env.site(), &rules)[0];
        let at = world.shown(&d, &env.terrain, 0, &rules, &fp, &|_| true)[0].pos;
        assert_eq!(world.strike(id, at, 1000, &rules), None, "half mined");
        // Onto the other app's title bar: its room now, riding that window.
        assert!(world.move_find(id, FindSpot::Window { id: 2, at: Vec2::new(1400.0, 300.0) }, &d, &env.terrain, &rules));
        assert_eq!(world.room_of(id), Some("notes"));
        let s = world.shown(&d, &env.terrain, 0, &rules, &fp, &|_| true);
        let s = s.iter().find(|s| s.id == id).expect("shown");
        assert_eq!((s.window, s.pos), (2, Vec2::new(1400.0, 300.0)));
        assert_eq!(world.find(id).map(|f| f.hits), Some(1), "its hits go with it");
        // The room it came from was rolled already; the new one was rolled on the way in
        // (what it always held, placed once he gets there).
        assert!(world.rooms["notes"].finds.len() >= 2);
        // Onto the floor: it stays in its room and shows there, the window or not.
        assert!(world.move_find(id, FindSpot::Floor { at: Vec2::new(150.0, 1040.0) }, &d, &env.terrain, &rules));
        let (d2, t2) = desk(vec![]);
        let s = world.shown(&d2, &t2, 0, &rules, &fp, &|_| true);
        assert_eq!(s.iter().find(|s| s.id == id).map(|s| (s.window, s.pos)), Some((FLOOR_WINDOW, Vec2::new(150.0, 1040.0))));
        // It round-trips through the save.
        let back = World::from_json(&world.to_json()).expect("json");
        assert_eq!(back.find(id).and_then(|f| f.floor), Some([150.0, 1040.0]));
        // Not onto a window that isn't a room (ours, fullscreen), nor a find that isn't.
        let (d3, t3) = desk(vec![win(5, RectI::new(200, 100, 600, 300), "skong.exe")]);
        assert!(!World::is_room(&d3, 5));
        assert!(!world.move_find(id, FindSpot::Window { id: 5, at: Vec2::new(300.0, 100.0) }, &d3, &t3, &rules));
        assert!(!world.move_find(999, FindSpot::Floor { at: Vec2::ZERO }, &d, &env.terrain, &rules));
    }
}
