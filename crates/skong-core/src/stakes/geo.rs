//! Geo coins: they burst out of a defeated foe, fall with simple physics, bounce a couple
//! of times on ledges (window tops, content lines, the taskbar), settle, and are picked up
//! when the knight walks over them.
use crate::geom::{RectF, Vec2};
use crate::terrain::Terrain;

use super::{Xorshift, area_of, clamp_to_areas};
use super::sources::{GeoDrop, drop_point};

/// Coin denominations (the game's small / medium / large Geo).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoinSize {
    Small,
    Medium,
    Large,
}

impl CoinSize {
    pub fn value(self) -> u32 {
        match self {
            CoinSize::Small => 1,
            CoinSize::Medium => 5,
            CoinSize::Large => 25,
        }
    }

    /// Clip names in the `Geo` animation library: (resting, airborne).
    pub fn clips(self) -> (&'static str, &'static str) {
        match self {
            CoinSize::Small => ("Small Idle", "Small Air"),
            CoinSize::Medium => ("Med Idle", "Med Air"),
            CoinSize::Large => ("Large Idle", "Large Air"),
        }
    }

    /// Split `amount` into coins, largest first (at most `max` coins; the rest is lost in
    /// rounding only when `max` is tiny).
    pub fn split(amount: u32, max: usize) -> Vec<CoinSize> {
        let mut out = Vec::new();
        let mut left = amount;
        for size in [CoinSize::Large, CoinSize::Medium, CoinSize::Small] {
            // Keep a few small coins for the sparkle unless the sum is big.
            while left >= size.value() && out.len() < max {
                if size == CoinSize::Large && left < 40 {
                    break;
                }
                out.push(size);
                left -= size.value();
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coin {
    pub id: u32,
    pub size: CoinSize,
    /// Bottom centre (screen px).
    pub pos: Vec2,
    pub vel: Vec2,
    pub resting: bool,
    /// Seconds since it dropped.
    pub age: f32,
    bounces: u8,
}

/// A coin picked up this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pickup {
    pub pos: Vec2,
    pub value: u32,
}

/// All loose coins on the desktop.
#[derive(Clone, Debug)]
pub struct GeoField {
    coins: Vec<Coin>,
    next_id: u32,
    rng: Xorshift,
    /// Render scale (px per game unit / 64): physics constants scale with it.
    pub scale: f32,
    /// Coins vanish after lying around this long (s): the ones he can't reach fade out.
    pub lifetime: f32,
}

/// Physics at scale 1 (px, s).
const GRAVITY: f32 = 2400.0;
const MAX_FALL: f32 = 1400.0;
/// Speed below which a landing coin settles instead of bouncing.
const SETTLE_SPEED: f32 = 260.0;
const BOUNCE: f32 = 0.45;
const MAX_BOUNCES: u8 = 3;
/// Half width of a coin's pickup box.
const HALF: f32 = 9.0;
/// Default coin lifetime (s).
pub const COIN_LIFETIME: f32 = 4.0 * 60.0;
/// Coins blink this long before vanishing (s).
pub const FADE_SECS: f32 = 6.0;
pub const MAX_COINS: usize = 80;

impl GeoField {
    pub fn new(seed: u32) -> Self {
        Self { coins: Vec::new(), next_id: 1, rng: Xorshift::new(seed), scale: 1.0, lifetime: COIN_LIFETIME }
    }

    pub fn coins(&self) -> &[Coin] {
        &self.coins
    }

    /// Coins resting inside a claimed prop (bench, find, visitor…) hop out toward the
    /// nearer side of it, so loot never hides inside furniture.
    pub fn evict_from(&mut self, occ: &crate::occupancy::Occupancy) {
        let s = self.scale.max(0.1);
        for c in self.coins.iter_mut().filter(|c| c.resting) {
            if let Some(claim) = occ.covering(c.pos, HALF * s * 0.5) {
                let r = claim.rect;
                let left = c.pos.x - r.left < r.right - c.pos.x;
                let dist = if left { c.pos.x - r.left } else { r.right - c.pos.x } + HALF * s * 2.0;
                // A small hop: enough sideways speed to clear the prop before landing.
                let speed = (dist * 2.2).max(120.0 * s);
                c.vel = Vec2::new(if left { -speed } else { speed }, -380.0 * s);
                c.resting = false;
                c.bounces = MAX_BOUNCES - 1;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.coins.is_empty()
    }

    /// Value of every loose coin.
    pub fn total(&self) -> u32 {
        self.coins.iter().map(|c| c.size.value()).sum()
    }

    /// Resting coins (where the knight should walk to pick them up).
    pub fn resting(&self) -> impl Iterator<Item = &Coin> {
        self.coins.iter().filter(|c| c.resting)
    }

    /// Fling `amount` Geo out of `at`: up and sideways, like a defeated enemy's loot.
    pub fn burst(&mut self, at: Vec2, amount: u32) {
        let room = MAX_COINS.saturating_sub(self.coins.len()).max(1);
        for size in CoinSize::split(amount, room.min(20)) {
            let s = self.scale;
            let vel = Vec2::new(self.rng.range(-420.0, 420.0), self.rng.range(-900.0, -450.0)) * s;
            self.coins.push(Coin { id: self.next_id, size, pos: at, vel, resting: false, age: 0.0, bounces: 0 });
            self.next_id += 1;
        }
    }

    /// Spill a [`GeoDrop`]: a shattered window flings its coins outward from all over
    /// its rect; a shower lets them fall in through the top.
    pub fn spill(&mut self, d: &GeoDrop) {
        let room = MAX_COINS.saturating_sub(self.coins.len()).max(1);
        let s = self.scale;
        let centre = d.rect.center();
        for size in CoinSize::split(d.amount, room.min(30)) {
            let (u, v) = (self.rng.f32(), self.rng.f32());
            let pos = drop_point(d.rect, u, v);
            let vel = if d.shower {
                Vec2::new(self.rng.range(-120.0, 120.0), self.rng.range(0.0, 250.0)) * s
            } else {
                let out = (pos - centre) / (d.rect.width().max(d.rect.height()) * 0.5).max(1.0);
                Vec2::new(out.x * 500.0 + self.rng.range(-120.0, 120.0), self.rng.range(-700.0, -250.0)) * s
            };
            self.coins.push(Coin { id: self.next_id, size, pos, vel, resting: false, age: 0.0, bounces: 0 });
            self.next_id += 1;
        }
    }

    /// Is this coin about to vanish (blinking)?
    pub fn fading(&self, c: &Coin) -> bool {
        c.age >= self.lifetime - FADE_SECS
    }

    /// Advance the physics: fall, land on (one-way) ledges from above, bounce, settle.
    /// Resting coins whose ledge went away fall again.
    /// `areas` = monitor work areas: coins never leave them (no dead zones between
    /// monitors); empty = the terrain bounds.
    pub fn step(&mut self, dt: f32, terrain: &Terrain, areas: &[RectF]) {
        let s = self.scale.max(0.1);
        let bounds = [terrain.bounds];
        let areas = if areas.is_empty() { &bounds[..] } else { areas };
        let inside = |p: Vec2| areas.iter().any(|a| a.left <= p.x && p.x <= a.right && a.top <= p.y && p.y <= a.bottom);
        for c in &mut self.coins {
            c.age += dt;
            if c.resting {
                let supported = terrain.ledges.iter().any(|l| (l.y - c.pos.y).abs() < 1.5 && l.x0 <= c.pos.x && c.pos.x <= l.x1);
                if supported {
                    continue;
                }
                c.resting = false;
                c.vel = Vec2::ZERO;
            }
            c.vel.y = (c.vel.y + GRAVITY * s * dt).min(MAX_FALL * s);
            c.vel.x *= (1.0 - 0.6 * dt).max(0.0);
            if !inside(c.pos) {
                // Spawned off-screen (a shower above the top) or in a dead zone.
                c.pos = clamp_to_areas(c.pos, areas, HALF * s);
            }
            let from = c.pos;
            let mut to = c.pos + c.vel * dt;
            // Screen edges (and dead zones beside them) are walls.
            if !inside(Vec2::new(to.x, from.y)) {
                c.vel.x = -c.vel.x * BOUNCE;
                to.x = from.x;
            }
            // Landing: the first ledge crossed going down.
            if c.vel.y > 0.0
                && let Some(y) = terrain
                    .ledges
                    .iter()
                    .filter(|l| l.x0 <= to.x && to.x <= l.x1 && from.y <= l.y + 0.5 && to.y >= l.y)
                    .map(|l| l.y)
                    .min_by(f32::total_cmp)
            {
                to.y = y;
                if c.vel.y < SETTLE_SPEED * s || c.bounces >= MAX_BOUNCES {
                    c.vel = Vec2::ZERO;
                    c.resting = true;
                } else {
                    c.vel.y = -c.vel.y * BOUNCE;
                    c.vel.x *= 0.7;
                    c.bounces += 1;
                }
            }
            if !inside(to) {
                // Fell out of its work area (no ground under it): rest on its bottom.
                let a = area_of(from, areas).unwrap_or(terrain.bounds);
                to = Vec2::new(to.x.clamp(a.left, a.right), to.y.clamp(a.top, a.bottom));
                if to.y >= a.bottom - 0.5 {
                    c.vel = Vec2::ZERO;
                    c.resting = true;
                } else {
                    c.vel.y = c.vel.y.max(0.0);
                }
            }
            c.pos = to;
        }
        let life = self.lifetime;
        self.coins.retain(|c| c.age < life);
    }

    /// Coins under the knight's body (`body`, screen px) are collected.
    pub fn collect(&mut self, body: RectF) -> Vec<Pickup> {
        let half = HALF * self.scale;
        let mut got = Vec::new();
        self.coins.retain(|c| {
            let r = RectF::new(c.pos.x - half, c.pos.y - 2.0 * half, c.pos.x + half, c.pos.y);
            // Airborne coins are caught too, but only once they are on the way down.
            let catchable = c.resting || c.vel.y > 0.0;
            if catchable && r.overlaps(&body) {
                got.push(Pickup { pos: c.pos, value: c.size.value() });
                false
            } else {
                true
            }
        });
        got
    }

    /// Keep resting coins on their windows when the desktop moved them.
    pub fn follow(&mut self, old: &Terrain, new: &Terrain) {
        for c in self.coins.iter_mut().filter(|c| c.resting) {
            let owner = old.ledges.iter().find(|l| (l.y - c.pos.y).abs() < 1.5 && l.x0 <= c.pos.x && c.pos.x <= l.x1).and_then(|l| l.owner);
            if let Some(id) = owner
                && let (Some(a), Some(b)) = (old.window_rect(id), new.window_rect(id))
            {
                c.pos.x += b.left - a.left;
                c.pos.y += b.top - a.top;
            }
        }
    }

    pub fn clear(&mut self) {
        self.coins.clear();
    }
}

impl GeoField {
    /// The knight gave up on the coin at `at` (unreachable): let it fade out soon.
    pub fn abandon(&mut self, at: Vec2) {
        let fade = self.lifetime - FADE_SECS;
        for c in self.coins.iter_mut().filter(|c| c.pos.distance(at) < 12.0) {
            c.age = c.age.max(fade);
        }
    }
}
