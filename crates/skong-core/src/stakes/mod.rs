//! Stakes for the knight's fights: dying, the Hollow Shade left behind, respawning, Geo
//! (coins that drop, bounce and get picked up, plus a persistent wallet), and the spells
//! and nail arts he throws in now and then.
//!
//! ```text
//!   cursor battle lost badly ─► death act ─► Shade spawns where he died (carries the Geo)
//!            │                                     │ persists across restarts
//!            ▼                                     ▼
//!   respawn at nearest bench / safe ledge ─► (recovered, bold) travel to the Shade, fight it
//!                                                  │ Shade bursts ─► Geo back in the wallet
//!   cursor battle won ─► Geo coins burst from the cursor ─► bounce onto ledges ─► picked up
//! ```
//!
//! Everything here is pure (no OS, no clocks): the app steps [`Shade`] and [`GeoField`],
//! renders them, and stores [`StakesSave`] in its settings. The brain side (behaviours
//! that decide to die, respawn, hunt the Shade, collect Geo and cast spells) lives in
//! `ai::stakes`.
pub mod geo;
pub mod save;
pub mod shade;
pub mod sources;
pub mod spells;
#[cfg(test)]
mod tests;

pub use geo::{Coin, CoinSize, GeoField, Pickup};
pub use save::{ShadeSave, StakesSave};
pub use sources::{GeoDrop, GeoSourceConfig, GeoSources};
pub use shade::{Shade, ShadePhase, ShadeTick};
pub use spells::{Spell, SpellBook, SpellConfig, SpellSituation};

use crate::geom::{RectF, Vec2};
use crate::terrain::{LedgeKind, Terrain};

/// How the knight died (picks the death act; hazards are for later: spikes, acid pools).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DeathKind {
    /// Beaten in a fight: `Death` (+ the mask shell dropping).
    #[default]
    Fight,
    /// Fell on spikes: `Spike Death`.
    Spikes,
    /// Fell into acid: `Acid Death`.
    Acid,
    /// A long fall without a dive or down-dash to break it: `Death`, and a Shade to win
    /// the Geo back from.
    Fall,
}

impl DeathKind {
    /// Act in `data/acts/stakes.json`.
    pub fn act_name(self) -> &'static str {
        match self {
            DeathKind::Fight | DeathKind::Fall => "death",
            DeathKind::Spikes => "spike_death",
            DeathKind::Acid => "acid_death",
        }
    }

    /// Hazard deaths leave no Shade in the game (hazard respawn); fights and fatal falls do.
    pub fn leaves_shade(self) -> bool {
        matches!(self, DeathKind::Fight | DeathKind::Fall)
    }
}

/// Where the knight comes back after dying: the nearest placed bench (sitting spot), else
/// the nearest point on a wide, solid ledge (the taskbar/screen bottom first), else `near`.
pub fn respawn_point(terrain: &Terrain, benches: &[Vec2], near: Vec2) -> Vec2 {
    if let Some(b) = benches.iter().min_by(|a, b| a.distance(near).total_cmp(&b.distance(near))) {
        return *b;
    }
    let pick = |ground_only: bool| {
        terrain
            .ledges
            .iter()
            .filter(|l| l.x1 - l.x0 >= 160.0 && (!ground_only || l.kind == LedgeKind::Ground))
            .map(|l| {
                let x = near.x.clamp(l.x0 + 40.0, l.x1 - 40.0);
                Vec2::new(x, l.y)
            })
            .min_by(|a, b| a.distance(near).total_cmp(&b.distance(near)))
    };
    pick(true).or_else(|| pick(false)).unwrap_or(near)
}

/// Tiny deterministic xorshift (the brain's own RNG is private to `ai`).
#[derive(Clone, Debug)]
pub(crate) struct Xorshift(u32);

impl Xorshift {
    pub fn new(seed: u32) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9).rotate_left(11) | 1)
    }

    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

/// Keep `p` inside the union of `areas` (monitor work areas), `margin` px from their
/// edges: a point in a dead zone between monitors (or off-screen) moves to the nearest
/// point of the nearest area. Empty `areas` = no clamping.
pub fn clamp_to_areas(p: Vec2, areas: &[RectF], margin: f32) -> Vec2 {
    let inner = |a: &RectF| {
        let mx = margin.min(a.width() * 0.5);
        let my = margin.min(a.height() * 0.5);
        RectF::new(a.left + mx, a.top + my, a.right - mx, a.bottom - my)
    };
    areas
        .iter()
        .map(|a| {
            let r = inner(a);
            Vec2::new(p.x.clamp(r.left, r.right), p.y.clamp(r.top, r.bottom))
        })
        .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
        .unwrap_or(p)
}

/// The work area containing `p` (or the nearest one).
pub fn area_of(p: Vec2, areas: &[RectF]) -> Option<RectF> {
    areas.iter().copied().min_by(|a, b| clamp_to_areas(p, &[*a], 0.0).distance(p).total_cmp(&clamp_to_areas(p, &[*b], 0.0).distance(p)))
}
