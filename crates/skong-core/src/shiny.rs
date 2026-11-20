//! Shinies: small glints that turn up on the ground now and then (every few minutes, or
//! when the bored knight asks the world for something to happen). The knight notices one,
//! walks over and picks it up (`pick_up_trinket`), and it is gone. Short-lived and never
//! saved: a shiny left alone fades away after a while.
//!
//! Placement follows the discoveries' rules ([`crate::discovery`]): only on surfaces that
//! have lasted ([`LASTING_SECS`]; the monitor floor always counts), never where the shared
//! [`Occupancy`] already has something (benches, finds, visitors, the Shade), never on a
//! game monitor, and not right under the knight's feet (he should *notice* it). On a
//! window it is anchored relative to the window, so it rides along and goes when the
//! window closes.
//!
//! This module is pure: the app ticks [`Shinies`], places, resolves ([`Shinies::shown`],
//! [`settle`]) and draws each frame with [`glint`].
use serde::Deserialize;

use crate::canvas::{Image, premul};
use crate::desktop::{DesktopSnapshot, WindowId};
use crate::discovery::{self, Anchor, EDGE_MARGIN, Footprint, HEADROOM, LEDGE_SLACK, Mix};
use crate::geom::{RectF, RectI, Vec2};
use crate::lasting::LedgeAges;
use crate::occupancy::{ClaimKey, ClaimKind, Occupancy};
use crate::terrain::{Ledge, LedgeKind, Terrain};

pub use crate::discovery::LASTING_SECS;

pub type ShinyId = u64;

/// Rates and rewards (the app reads them from `data/props.json` → `shinies`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShinyRules {
    /// Seconds between shinies turning up by themselves, [min, max]. Only counts while the
    /// user is around, not typing hard, and nothing is fullscreen.
    pub every_secs: [f32; 2],
    /// At most this many on screen at once.
    pub max: usize,
    /// One left alone this long fades away (s).
    pub lifetime_secs: f32,
    /// Chance that picking one up also pops out a little Geo, and how much.
    pub geo_chance: f32,
    pub geo: [u32; 2],
    /// Never closer to the knight than this (px at scale 1).
    pub min_distance: f32,
}

impl Default for ShinyRules {
    fn default() -> Self {
        Self { every_secs: [240.0, 540.0], max: 2, lifetime_secs: 900.0, geo_chance: 0.2, geo: [1, 3], min_distance: 160.0 }
    }
}

/// Where a shiny lies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    /// On a ledge of a window (title bar or a lasting content line), relative to it.
    Window { id: WindowId, anchor: Anchor },
    /// On a monitor's floor.
    Ground { pos: Vec2 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shiny {
    pub id: ShinyId,
    pub spot: Spot,
    /// [`Shinies::clock`] when it turned up.
    pub born: f64,
}

/// A shiny on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownShiny {
    pub id: ShinyId,
    /// Bottom centre, resting on its ledge.
    pub pos: Vec2,
    /// The ledge it rests on (x0, x1).
    pub span: (f32, f32),
    /// Seconds since it turned up.
    pub age: f32,
}

/// Where a shiny may go right now.
pub struct ShinySite<'a> {
    pub terrain: &'a Terrain,
    pub ages: &'a LedgeAges,
    /// Clock of `ages`.
    pub now: f64,
    /// Window ledges younger than this (s) carry nothing ([`LASTING_SECS`]; 0 for debug).
    pub min_age: f32,
    pub occupancy: &'a Occupancy,
    pub footprint: Footprint,
    /// The knight's feet: not too close to him.
    pub knight: Vec2,
    pub min_distance: f32,
    /// Keep to this area (the knight's monitor work area), if given.
    pub area: Option<RectF>,
    /// Monitors running a fullscreen game.
    pub avoid: &'a [RectI],
    /// Can the knight get there (a route that ends on the spot)? Checked last.
    pub reachable: &'a dyn Fn(Vec2) -> bool,
}

/// Ledges that could carry a shiny, each with the x range its centre may take.
pub(crate) fn candidates<'t>(site: &'t ShinySite) -> Vec<(&'t Ledge, f32, f32)> {
    let t = site.terrain;
    let fp = site.footprint;
    t.ledges
        .iter()
        .filter(|l| {
            let lasting = l.kind == LedgeKind::Ground || site.ages.age(t, l, site.now) >= site.min_age;
            (l.owner.is_some() || l.kind == LedgeKind::Ground) && l.y >= t.bounds.top + HEADROOM && lasting
        })
        .filter_map(|l| {
            let (mut x0, mut x1) = (l.x0, l.x1);
            if let Some(a) = site.area {
                if l.y < a.top || l.y > a.bottom + 1.0 {
                    return None;
                }
                (x0, x1) = (x0.max(a.left), x1.min(a.right));
            }
            let (lo, hi) = (x0 + EDGE_MARGIN + fp.room * 0.5, x1 - EDGE_MARGIN - fp.room * 0.5);
            (hi > lo).then_some((l, lo, hi))
        })
        .collect()
}

/// Choose a spot for a new shiny (deterministic in `salt`), avoiding `taken` (other shinies).
pub fn place(site: &ShinySite, taken: &[RectF], salt: u64) -> Option<(Spot, Vec2)> {
    let fp = site.footprint;
    let ledges = candidates(site);
    if ledges.is_empty() {
        return None;
    }
    let ok = |l: &Ledge, x: f32| {
        let p = Vec2::new(x, l.y);
        let r = fp.rect(p);
        p.distance(site.knight) >= site.min_distance
            && site.occupancy.is_free(r, None)
            && !taken.iter().any(|t| t.overlaps(&r))
            && !site.avoid.iter().any(|m| m.contains(x as i32, l.y as i32 - 1))
            && (site.reachable)(p)
    };
    let mut mix = Mix(salt);
    let weights: Vec<f32> = ledges.iter().map(|(_, lo, hi)| hi - lo + 1.0).collect();
    let random = (0..24).find_map(|_| {
        let (l, lo, hi) = ledges[mix.pick(&weights)];
        let x = lo + (hi - lo) * mix.f32();
        ok(l, x).then_some((l, x))
    });
    let (l, x) = random.or_else(|| ledges.iter().find_map(|&(l, lo, hi)| (0..=20).map(|i| lo + (hi - lo) * i as f32 / 20.0).find(|&x| ok(l, x)).map(|x| (l, x))))?;
    spot_on(site.terrain, l, x)
}

/// The [`Spot`] at `x` on ledge `l` (window-relative on a window), with its position.
pub(crate) fn spot_on(terrain: &Terrain, l: &Ledge, x: f32) -> Option<(Spot, Vec2)> {
    let pos = Vec2::new(x, l.y);
    let spot = match l.owner {
        Some(id) => {
            let rect = terrain.window_rect(id)?;
            Spot::Window { id, anchor: Anchor { fx: (x - rect.left) / rect.width().max(1.0), dy: l.y - rect.top } }
        }
        None => Spot::Ground { pos },
    };
    Some((spot, pos))
}

/// The shinies on screen and when the next one turns up by itself.
#[derive(Clone, Debug)]
pub struct Shinies {
    list: Vec<Shiny>,
    next_id: ShinyId,
    clock: f64,
    next_at: f64,
    mix: Mix,
}

impl Shinies {
    pub fn new(seed: u64, rules: &ShinyRules) -> Self {
        let mut s = Self { list: Vec::new(), next_id: 1, clock: 0.0, next_at: 0.0, mix: Mix(seed ^ SEED_TAG) };
        s.schedule(rules);
        s
    }

    fn schedule(&mut self, rules: &ShinyRules) {
        let [lo, hi] = rules.every_secs;
        self.next_at = self.clock + (lo + (hi - lo).max(0.0) * self.mix.f32()) as f64;
    }

    pub fn clock(&self) -> f64 {
        self.clock
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get(&self, id: ShinyId) -> Option<&Shiny> {
        self.list.iter().find(|s| s.id == id)
    }

    /// Advance the clock. `true` when one should turn up by itself now (the caller then
    /// [`place`]s it, or calls [`Self::retry_soon`] when there is no room). The timer
    /// only runs while `allowed`.
    pub fn due(&mut self, dt: f32, allowed: bool, rules: &ShinyRules) -> bool {
        self.clock += dt as f64;
        if !allowed {
            self.next_at += dt as f64;
            return false;
        }
        if self.clock < self.next_at {
            return false;
        }
        self.schedule(rules);
        self.list.len() < rules.max
    }

    /// No room just now: try again in a little while.
    pub fn retry_soon(&mut self) {
        self.next_at = self.clock + 20.0;
    }

    /// A new shiny at `spot`.
    pub fn add(&mut self, spot: Spot) -> ShinyId {
        let id = self.next_id;
        self.next_id += 1;
        self.list.push(Shiny { id, spot, born: self.clock });
        id
    }

    /// Picked up (or debug-cleared): gone.
    pub fn take(&mut self, id: ShinyId) -> Option<Shiny> {
        let i = self.list.iter().position(|s| s.id == id)?;
        Some(self.list.remove(i))
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// Geo popping out of one just picked up (usually none).
    pub fn reward(&mut self, rules: &ShinyRules) -> u32 {
        if self.mix.f32() >= rules.geo_chance {
            return 0;
        }
        let [lo, hi] = rules.geo;
        lo + (self.mix.next() % (hi.saturating_sub(lo) as u64 + 1)) as u32
    }

    /// Rects of the shinies (where they were placed), so new ones keep apart.
    pub fn rects(&self, terrain: &Terrain, fp: Footprint) -> Vec<RectF> {
        self.list.iter().filter_map(|s| spot_pos(terrain, s.spot)).map(|p| fp.rect(p)).collect()
    }

    /// Shinies on screen (before collisions, see [`settle`]). Old ones and those whose
    /// window closed are dropped; one whose ledge is covered or scrolled away hides.
    pub fn shown(&mut self, desk: &DesktopSnapshot, terrain: &Terrain, rules: &ShinyRules, width: f32, settled: &dyn Fn(&crate::terrain::Ledge) -> bool) -> Vec<ShownShiny> {
        let clock = self.clock;
        self.list.retain(|s| {
            let alive = ((clock - s.born) as f32) < rules.lifetime_secs;
            let open = match s.spot {
                Spot::Window { id, .. } => desk.window(id).is_some_and(|w| !w.fullscreen),
                Spot::Ground { .. } => true,
            };
            alive && open
        });
        self.list
            .iter()
            .filter_map(|s| {
                let (pos, span) = resolve(terrain, s.spot, width, settled)?;
                Some(ShownShiny { id: s.id, pos, span, age: (clock - s.born) as f32 })
            })
            .collect()
    }
}

/// The spot for something standing at `at` on a ledge of window `owner` (`None`: the floor),
/// relative to its window so it rides along (test-panel placement).
pub fn spot_at(terrain: &Terrain, at: Vec2, owner: Option<WindowId>) -> Option<Spot> {
    match owner {
        Some(id) => {
            let rect = terrain.window_rect(id)?;
            Some(Spot::Window { id, anchor: Anchor { fx: (at.x - rect.left) / rect.width().max(1.0), dy: at.y - rect.top } })
        }
        None => Some(Spot::Ground { pos: at }),
    }
}

/// Seed tag for shinies' own rolls ("shin").
const SEED_TAG: u64 = 0x7368_696e;

/// Where a spot is (as placed), ignoring whether its ledge is there right now.
fn spot_pos(terrain: &Terrain, spot: Spot) -> Option<Vec2> {
    match spot {
        Spot::Window { id, anchor } => terrain.window_rect(id).map(|r| Vec2::new(r.left + anchor.fx * r.width(), r.top + anchor.dy)),
        Spot::Ground { pos } => Some(pos),
    }
}

/// Where a shiny `width` px wide rests right now, with its ledge span.
pub fn resolve(terrain: &Terrain, spot: Spot, width: f32, settled: &dyn Fn(&crate::terrain::Ledge) -> bool) -> Option<(Vec2, (f32, f32))> {
    match spot {
        Spot::Window { id, anchor } => discovery::resolve(terrain, id, terrain.window_rect(id)?, anchor, width, settled),
        Spot::Ground { pos } => {
            let half = width * 0.5;
            terrain
                .ledges
                .iter()
                .find(|l| l.kind == LedgeKind::Ground && (l.y - pos.y).abs() <= LEDGE_SLACK && l.x0 <= pos.x - half && pos.x + half <= l.x1)
                .map(|l| (Vec2::new(pos.x, l.y), (l.x0, l.x1)))
        }
    }
}

/// Collision pass (after benches, finds and visitors have claimed their rects): each
/// shiny keeps its spot if free, else slides along its ledge, else hides while it can't
/// fit. Survivors are claimed as [`ClaimKind::Shiny`] (coins don't rest on them).
pub fn settle(shown: Vec<ShownShiny>, occ: &mut Occupancy, fp: Footprint) -> Vec<ShownShiny> {
    occ.release_kind(ClaimKind::Shiny);
    let mut out = Vec::with_capacity(shown.len());
    for mut s in shown {
        let key = ClaimKey::new(ClaimKind::Shiny, s.id);
        let Some(dx) = occ.nudge(fp.rect(s.pos), s.span.0, s.span.1, Some(key), 4.0) else { continue };
        s.pos.x += dx;
        occ.claim(key, fp.rect(s.pos), 20);
        out.push(s);
    }
    out
}

// --- the glint ------------------------------------------------------------------------

/// Seconds per slow pulse of the glint.
pub const PULSE_SECS: f32 = 2.2;

/// A soft, twinkling glint on the ground, `size` px square: a pale warm core with a thin
/// four-point star that breathes slowly, and two tiny satellite sparks that wink now and
/// then. Mostly dim with a brief brighter twinkle (subtle, never a flash). `phase` (0..1)
/// keeps neighbouring shinies out of step; `alpha` fades the whole thing.
pub fn glint(size: u32, t: f64, phase: f32, alpha: f32) -> Image {
    use std::f32::consts::TAU;
    let size = size.max(5);
    let s = size as f32;
    let mut img = Image::new(size, size);
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return img;
    }
    // 0..1, lingering low with a short rise: sin² sharpened.
    let wave = 0.5 + 0.5 * (TAU * ((t / PULSE_SECS as f64).fract() as f32 + phase)).sin();
    let pulse = wave * wave * wave;
    let glow = 0.4 + 0.6 * pulse;
    let (cx, cy) = (s * 0.5, s * 0.5);
    let core_r = (s * 0.085).max(0.8);
    let arm = s * 0.5 * (0.45 + 0.5 * pulse);
    let thin = (s * 0.035).max(0.55);
    let sparks = [(-0.27, 0.16, 1.7, 0.3), (0.24, -0.2, 1.3, 0.75)];
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = dx.hypot(dy);
            let mut shape = (-(d / core_r).powi(2)).exp();
            // The star: long axis arms, short faint diagonals.
            let ray = |along: f32, across: f32, len: f32| if along.abs() < len { (1.0 - along.abs() / len).powf(1.5) * (-(across / thin).powi(2)).exp() } else { 0.0 };
            shape += 0.75 * (ray(dx, dy, arm) + ray(dy, dx, arm));
            let (u, v) = ((dx + dy) * std::f32::consts::FRAC_1_SQRT_2, (dx - dy) * std::f32::consts::FRAC_1_SQRT_2);
            shape += 0.3 * (ray(u, v, arm * 0.45) + ray(v, u, arm * 0.45));
            let mut a = glow * shape.min(1.0);
            for &(fx, fy, period, off) in &sparks {
                let wink = (TAU * ((t / period).fract() as f32 + off + phase)).sin().max(0.0).powi(6);
                let r = (s * 0.05).max(0.6);
                let (sx, sy) = (cx + fx * s, cy + fy * s);
                a += 0.7 * wink * (-((x as f32 + 0.5 - sx).hypot(y as f32 + 0.5 - sy) / r).powi(2)).exp();
            }
            let a = (a.min(1.0) * 0.92 * alpha * 255.0) as u8;
            if a > 0 {
                img.px[(y * size + x) as usize] = premul(255, 249, 226, a);
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{Monitor, WindowInfo};
    use crate::geom::RectI;
    use crate::terrain::TerrainOptions;

    fn win(id: WindowId, rect: RectI) -> WindowInfo {
        WindowInfo { id, rect, title: String::new(), class: String::new(), process: "paint.exe".into(), pid: 1, maximized: false, fullscreen: false, foreground: false, topmost: false }
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

    fn aged(t: &Terrain, secs: f32) -> LedgeAges {
        let mut a = LedgeAges::default();
        let mut now = 0.0;
        while now <= secs as f64 {
            a.observe(t, now);
            now += 1.0;
        }
        a
    }

    const FP: Footprint = Footprint { w: 20.0, h: 18.0, room: 60.0 };

    fn site<'a>(t: &'a Terrain, ages: &'a LedgeAges, now: f64, occ: &'a Occupancy) -> ShinySite<'a> {
        ShinySite { terrain: t, ages, now, min_age: LASTING_SECS, occupancy: occ, footprint: FP, knight: Vec2::new(100.0, 1040.0), min_distance: 160.0, area: None, avoid: &[], reachable: &|_| true }
    }

    #[test]
    fn young_window_ledges_carry_nothing_but_the_floor_always_does() {
        let (_, t) = desk(vec![win(1, RectI::new(300, 400, 1200, 900))]);
        let ages = aged(&t, 10.0);
        let occ = Occupancy::default();
        for salt in 0..40 {
            let (spot, pos) = place(&site(&t, &ages, 10.0, &occ), &[], salt).expect("the floor");
            assert!(matches!(spot, Spot::Ground { .. }), "window ledge is only 10 s old: {spot:?}");
            assert_eq!(pos.y, 1040.0);
        }
        let ages = aged(&t, 40.0);
        let on_window = (0..40).filter(|&salt| matches!(place(&site(&t, &ages, 40.0, &occ), &[], salt), Some((Spot::Window { id: 1, .. }, _)))).count();
        assert!(on_window > 0, "a lasting title bar carries shinies too");
    }

    #[test]
    fn placement_avoids_claims_other_shinies_games_and_the_knight() {
        let (_, t) = desk(vec![]);
        let ages = LedgeAges::default();
        let mut occ = Occupancy::default();
        // A bench and a visitor on the floor.
        let bench = RectF::new(500.0, 980.0, 650.0, 1040.0);
        let npc = RectF::new(900.0, 900.0, 1000.0, 1040.0);
        occ.claim(ClaimKey::new(ClaimKind::Bench, 1), bench, 100);
        occ.claim(ClaimKey::new(ClaimKind::Npc, 2), npc, 90);
        let other = FP.rect(Vec2::new(1300.0, 1040.0));
        let game = [RectI::new(1500, 0, 1920, 1080)];
        let mut s = site(&t, &ages, 0.0, &occ);
        s.avoid = &game;
        for salt in 0..200 {
            let (_, p) = place(&s, &[other], salt).expect("room");
            let r = FP.rect(p);
            assert!(!r.overlaps(&bench) && !r.overlaps(&npc) && !r.overlaps(&other), "{p:?}");
            assert!(p.x < 1500.0, "not on the game monitor: {p:?}");
            assert!(p.distance(s.knight) >= 160.0, "not at his feet: {p:?}");
        }
    }

    #[test]
    fn only_where_he_can_get_to() {
        let (_, t) = desk(vec![]);
        let ages = LedgeAges::default();
        let occ = Occupancy::default();
        let reachable = |p: Vec2| p.x > 1200.0;
        let s = ShinySite { reachable: &reachable, ..site(&t, &ages, 0.0, &occ) };
        for salt in 0..50 {
            assert!(place(&s, &[], salt).expect("room").1.x > 1200.0);
        }
        let nowhere = |_: Vec2| false;
        assert!(place(&ShinySite { reachable: &nowhere, ..site(&t, &ages, 0.0, &occ) }, &[], 1).is_none());
    }

    #[test]
    fn rides_its_window_and_goes_with_it() {
        let (d, t) = desk(vec![win(1, RectI::new(300, 400, 1200, 900))]);
        let rules = ShinyRules::default();
        let mut sh = Shinies::new(1, &rules);
        let id = sh.add(Spot::Window { id: 1, anchor: Anchor { fx: 0.5, dy: 0.0 } });
        let s = sh.shown(&d, &t, &rules, 20.0, &|_| true);
        assert_eq!(s[0].pos, Vec2::new(750.0, 400.0));
        let (d2, t2) = desk(vec![win(1, RectI::new(400, 300, 1300, 800))]);
        assert_eq!(sh.shown(&d2, &t2, &rules, 20.0, &|_| true)[0].pos, Vec2::new(850.0, 300.0), "rides along");
        let (d3, t3) = desk(vec![]);
        assert!(sh.shown(&d3, &t3, &rules, 20.0, &|_| true).is_empty());
        assert!(sh.get(id).is_none(), "gone with its window");
    }

    #[test]
    fn timer_paces_spawns_caps_them_and_old_ones_fade() {
        let rules = ShinyRules { every_secs: [100.0, 100.0], max: 2, lifetime_secs: 300.0, ..ShinyRules::default() };
        let (d, t) = desk(vec![]);
        let mut sh = Shinies::new(3, &rules);
        let mut spawned = 0;
        let step = |sh: &mut Shinies, secs: f32, allowed: bool| {
            let mut n = 0;
            for _ in 0..(secs as usize) {
                if sh.due(1.0, allowed, &rules) {
                    sh.add(Spot::Ground { pos: Vec2::new(500.0 + 100.0 * sh.len() as f32, 1040.0) });
                    n += 1;
                }
            }
            n
        };
        spawned += step(&mut sh, 150.0, false);
        assert_eq!(spawned, 0, "not while the user is busy or away");
        spawned += step(&mut sh, 105.0, true);
        assert_eq!(spawned, 1, "one every 100 s of allowed time");
        spawned += step(&mut sh, 100.0, true);
        spawned += step(&mut sh, 100.0, true);
        assert_eq!(spawned, 2, "at most two at once");
        assert_eq!(sh.shown(&d, &t, &rules, 20.0, &|_| true).len(), 2);
        step(&mut sh, 300.0, false);
        assert!(sh.shown(&d, &t, &rules, 20.0, &|_| true).is_empty(), "left alone, they fade away");
    }

    #[test]
    fn settle_keeps_shinies_out_of_props() {
        let mut occ = Occupancy::default();
        occ.claim(ClaimKey::new(ClaimKind::Find, 7), RectF::new(480.0, 980.0, 540.0, 1040.0), 40);
        let s = ShownShiny { id: 1, pos: Vec2::new(500.0, 1040.0), span: (0.0, 1920.0), age: 1.0 };
        let out = settle(vec![s], &mut occ, FP);
        assert_eq!(out.len(), 1);
        assert!(!FP.rect(out[0].pos).overlaps(&RectF::new(480.0, 980.0, 540.0, 1040.0)), "slid aside: {:?}", out[0].pos);
        assert!(occ.get(ClaimKey::new(ClaimKind::Shiny, 1)).is_some(), "claimed");
        // No room on a short ledge: hidden while it can't fit.
        let s = ShownShiny { span: (470.0, 550.0), ..s };
        assert!(settle(vec![s], &mut occ, FP).is_empty());
    }

    #[test]
    fn rewards_are_rare_and_small() {
        let rules = ShinyRules::default();
        let mut sh = Shinies::new(9, &rules);
        let r: Vec<u32> = (0..1000).map(|_| sh.reward(&rules)).collect();
        let lucky = r.iter().filter(|&&g| g > 0).count();
        assert!((120..280).contains(&lucky), "about one in five: {lucky}");
        assert!(r.iter().all(|&g| g <= 3));
    }

    fn alpha(img: &Image, x: u32, y: u32) -> u32 {
        img.px[(y * img.width + x) as usize] >> 24
    }

    #[test]
    fn the_glint_twinkles_softly() {
        let size = 24;
        let c = size / 2;
        let at = |t: f32| glint(size, t as f64, 0.0, 1.0);
        // Over one pulse: brightest and dimmest centre.
        let centres: Vec<u32> = (0..44).map(|i| alpha(&at(i as f32 * PULSE_SECS / 44.0), c, c)).collect();
        let (lo, hi) = (*centres.iter().min().unwrap(), *centres.iter().max().unwrap());
        assert!(hi > 180, "a visible twinkle: {hi}");
        assert!(lo > 60 && lo < hi - 60, "never gone, but it breathes: {lo}..{hi}");
        // Small and soft: the corners stay clear, most of the square is empty.
        let img = at(PULSE_SECS * 0.25);
        for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
            assert_eq!(alpha(&img, x, y), 0, "corner ({x},{y})");
        }
        let lit = img.px.iter().filter(|p| **p >> 24 > 40).count();
        assert!(lit < (size * size / 4) as usize, "a glint, not a blob: {lit} px");
        // Fades with alpha.
        assert!(alpha(&glint(size, 0.5, 0.0, 0.3), c, c) < alpha(&glint(size, 0.5, 0.0, 1.0), c, c));
        assert!(glint(size, 0.5, 0.0, 0.0).px.iter().all(|p| *p == 0));
    }
}
