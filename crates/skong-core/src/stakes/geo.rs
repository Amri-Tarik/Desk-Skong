//! Geo coins: they burst out of a defeated foe, fall with simple physics, bounce a couple
//! of times on ledges (window tops, content lines, the taskbar), settle, and are picked up
//! when the knight walks over them.
use crate::geom::{RectF, Vec2};
use crate::terrain::Terrain;

use super::Xorshift;

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
/// Coins disappear after lying around this long (s).
pub const COIN_LIFETIME: f32 = 15.0 * 60.0;
pub const MAX_COINS: usize = 48;

impl GeoField {
    pub fn new(seed: u32) -> Self {
        Self { coins: Vec::new(), next_id: 1, rng: Xorshift::new(seed), scale: 1.0 }
    }

    pub fn coins(&self) -> &[Coin] {
        &self.coins
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

    /// Advance the physics: fall, land on (one-way) ledges from above, bounce, settle.
    /// Resting coins whose ledge went away fall again.
    pub fn step(&mut self, dt: f32, terrain: &Terrain) {
        let s = self.scale.max(0.1);
        let b = terrain.bounds;
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
            let from = c.pos;
            let mut to = c.pos + c.vel * dt;
            // Screen edges are walls.
            if to.x < b.left + HALF * s || to.x > b.right - HALF * s {
                c.vel.x = -c.vel.x * BOUNCE;
                to.x = to.x.clamp(b.left + HALF * s, b.right - HALF * s);
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
            if to.y > b.bottom {
                // Fell past everything (no ground under it): rest on the screen bottom.
                to.y = b.bottom;
                c.vel = Vec2::ZERO;
                c.resting = true;
            }
            c.pos = to;
        }
        self.coins.retain(|c| c.age < COIN_LIFETIME);
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
