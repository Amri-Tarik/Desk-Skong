//! Where Geo comes from besides won fights:
//!
//! * **Shattered windows** — closing or minimizing a window breaks it into Geo like a
//!   breakable wall in the game: more for bigger windows, capped, and rate-limited so
//!   closing a whole app (or "show desktop") at once doesn't flood the desktop. Each window
//!   pays at most once per `repay_after` seconds, so minimizing and restoring the same
//!   window over and over is no Geo farm.
//! * **Geo showers** — every few minutes while the user is around (never over a
//!   fullscreen app or while typing fast), a handful of coins rains in through the top
//!   of the screen somewhere.
//!
//! Pure bookkeeping: the app reports closed / minimized windows and presence, and spawns the bursts
//! this returns into its [`super::GeoField`].
use serde::Deserialize;

use std::collections::HashMap;

use crate::desktop::WindowId;
use crate::geom::{RectF, RectI, Vec2};

use super::Xorshift;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeoSourceConfig {
    /// Closing windows drops Geo.
    pub shatter_windows: bool,
    /// Minimizing windows drops Geo too (same size rules and rate limit; needs
    /// `shatter_windows`).
    pub shatter_minimized: bool,
    /// A window that paid out pays nothing again for this many seconds (minimize-restore
    /// farming).
    pub repay_after: f32,
    /// Geo per million px² of a closed window (a 1600×900 window ≈ 1.44 M px²).
    pub geo_per_mpx: f32,
    /// Geo for a closed window: [min, max].
    pub shatter_geo: [u32; 2],
    /// Windows smaller than this (either side, px) don't shatter (tooltips, popups).
    pub min_window_side: i32,
    /// At most this many shatters per `shatter_window` seconds.
    pub max_shatters: usize,
    pub shatter_window: f32,
    /// Seconds of user presence between showers: [min, max]. Empty range = never.
    pub shower_every: [f32; 2],
    /// Geo per shower: [min, max].
    pub shower_geo: [u32; 2],
    /// No new Geo while this much is already lying around.
    pub max_loose: u32,
}

impl Default for GeoSourceConfig {
    fn default() -> Self {
        Self {
            shatter_windows: true,
            shatter_minimized: true,
            repay_after: 600.0,
            geo_per_mpx: 25.0,
            shatter_geo: [6, 45],
            min_window_side: 160,
            max_shatters: 2,
            shatter_window: 20.0,
            shower_every: [180.0, 480.0],
            shower_geo: [8, 30],
            max_loose: 250,
        }
    }
}

/// A burst to spawn: coins fly out of `rect` (a shattered window) or fall in at its top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoDrop {
    pub rect: RectF,
    pub amount: u32,
    pub shower: bool,
}

#[derive(Clone, Debug)]
pub struct GeoSources {
    pub cfg: GeoSourceConfig,
    /// Seconds of presence until the next shower.
    shower_in: f32,
    /// Times (on our clock) of recent shatters.
    shatters: Vec<f64>,
    /// When each window last paid out (on our clock).
    paid: HashMap<WindowId, f64>,
    clock: f64,
    rng: Xorshift,
}

impl GeoSources {
    pub fn new(cfg: GeoSourceConfig, seed: u32) -> Self {
        let mut s = Self { cfg, shower_in: 0.0, shatters: Vec::new(), paid: HashMap::new(), clock: 0.0, rng: Xorshift::new(seed) };
        s.shower_in = s.next_gap();
        s
    }

    fn next_gap(&mut self) -> f32 {
        let [lo, hi] = self.cfg.shower_every;
        if hi <= 0.0 { f32::INFINITY } else { self.rng.range(lo.max(1.0), hi.max(lo.max(1.0))) }
    }

    fn amount(&mut self, [lo, hi]: [u32; 2]) -> u32 {
        self.rng.range(lo as f32, hi.max(lo) as f32 + 0.999) as u32
    }

    /// Seconds until the next shower (while present).
    pub fn shower_in(&self) -> f32 {
        self.shower_in
    }

    /// Advance the clock. `present` = the user is around; `quiet` = a fullscreen app or
    /// fast typing (the timer pauses). Returns a shower when one is due.
    pub fn tick(&mut self, dt: f32, present: bool, quiet: bool, loose: u32, bounds: RectF) -> Option<GeoDrop> {
        self.clock += dt as f64;
        if !present || quiet {
            return None;
        }
        self.shower_in -= dt;
        if self.shower_in > 0.0 {
            return None;
        }
        self.shower_in = self.next_gap();
        if loose >= self.cfg.max_loose {
            return None;
        }
        Some(self.shower(bounds))
    }

    /// A shower right now (debug "Spill geo"), somewhere along the top of `bounds`.
    pub fn shower(&mut self, bounds: RectF) -> GeoDrop {
        let w = (bounds.width() * 0.15).max(80.0);
        let x = self.rng.range(bounds.left + w, (bounds.right - w).max(bounds.left + w + 1.0));
        let amount = self.amount(self.cfg.shower_geo);
        // The app moves showers to a reachable portal spot (`npcs::Visitors::shower_portal`).
        GeoDrop { rect: RectF::new(x - w * 0.5, bounds.top - 10.0, x + w * 0.5, bounds.top), amount, shower: true }
    }

    /// A window really closed (not just hidden by a virtual desktop switch). Returns its
    /// burst unless it's too small, it paid recently, the limit is reached, or enough Geo
    /// lies around.
    pub fn on_closed(&mut self, id: WindowId, rect: RectI, loose: u32) -> Option<GeoDrop> {
        if !self.cfg.shatter_windows {
            return None;
        }
        self.shatter(id, rect, loose)
    }

    /// A window was minimized: shatters like a closed one (same rules, shared rate limit),
    /// but each window pays at most once per [`GeoSourceConfig::repay_after`].
    pub fn on_minimized(&mut self, id: WindowId, rect: RectI, loose: u32) -> Option<GeoDrop> {
        if !self.cfg.shatter_windows || !self.cfg.shatter_minimized {
            return None;
        }
        self.shatter(id, rect, loose)
    }

    fn shatter(&mut self, id: WindowId, rect: RectI, loose: u32) -> Option<GeoDrop> {
        let c = self.cfg;
        let clock = self.clock;
        self.paid.retain(|_, t| ((clock - *t) as f32) < c.repay_after);
        if self.paid.contains_key(&id) {
            return None;
        }
        if rect.width() < c.min_window_side || rect.height() < c.min_window_side || loose >= c.max_loose {
            return None;
        }
        self.shatters.retain(|t| ((self.clock - t) as f32) < c.shatter_window);
        if self.shatters.len() >= c.max_shatters {
            return None;
        }
        self.shatters.push(self.clock);
        self.paid.insert(id, self.clock);
        let area = rect.width() as f32 * rect.height() as f32 / 1.0e6;
        let [lo, hi] = c.shatter_geo;
        let jitter = self.rng.range(0.8, 1.2);
        let amount = ((area * c.geo_per_mpx * jitter) as u32).clamp(lo, hi.max(lo));
        Some(GeoDrop { rect: rect.to_f(), amount, shower: false })
    }
}

/// Spawn point helpers for a drop (deterministic given the field's RNG).
pub(crate) fn drop_point(rect: RectF, u: f32, v: f32) -> Vec2 {
    Vec2::new(rect.left + rect.width() * u, rect.top + rect.height() * v)
}
