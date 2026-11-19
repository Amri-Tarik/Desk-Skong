//! Dream portals: something (a visitor, later other props) arrives at a random standable
//! spot anywhere on the virtual desktop through a portal (a puff of dust, or a dream gate).
//!
//! * [`pick_spot`] chooses where: a standable surface the knight can actually reach,
//!   preferring places far from him and — sometimes — on another monitor, so he gets to
//!   cross the screen with his full movement kit.
//! * [`Portal`] is the timing: it opens, lets its guest step out, and closes. Its
//!   [`PortalStyle`] picks the look: subtle dust puffs and a quick fade-in (default), or
//!   the dream gate (glow + white dream ripple) for special moments. The app renders it
//!   and asks [`Portal::take_emerge`] when to show the guest.
use serde::Deserialize;

use crate::desktop::{Monitor, WindowId};
use crate::geom::Vec2;

/// A horizontal place to stand (nav surface or terrain ledge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footing {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub owner: Option<WindowId>,
}

/// Where a portal opens: the feet point of whatever steps out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalSpot {
    pub at: Vec2,
    pub owner: Option<WindowId>,
}

/// Placement tuning.
#[derive(Clone, Copy, Debug)]
pub struct SpotOptions {
    /// Surfaces narrower than this are skipped (px).
    pub min_width: f32,
    /// Keep this far from either end of the surface (px).
    pub margin: f32,
    /// Don't open closer than this to the knight (px).
    pub min_distance: f32,
    /// Bonus for a spot on another monitor than the knight's (0..1).
    pub other_monitor_bonus: f32,
    /// Weight of the random jitter in the score (keeps it organic).
    pub jitter: f32,
    /// Route-check at most this many of the best candidates.
    pub max_checks: usize,
}

impl Default for SpotOptions {
    fn default() -> Self {
        Self { min_width: 140.0, margin: 50.0, min_distance: 350.0, other_monitor_bonus: 0.35, jitter: 0.6, max_checks: 6 }
    }
}

/// Is there `need` px of room above `f` before the top of its monitor? Arrivals that open
/// higher than that would clip off the screen. Footings outside every monitor pass (the
/// spot picker rejects them anyway).
pub fn has_headroom(f: &Footing, monitors: &[Monitor], need: f32) -> bool {
    let x = (0.5 * (f.x0 + f.x1)) as i32;
    monitors.iter().find(|m| m.rect.contains(x, f.y as i32 - 1)).is_none_or(|m| f.y - need >= m.rect.top as f32)
}

/// The footings something may arrive on: `settled` ones (floors, or window ledges that
/// lasted [`crate::lasting::SETTLED_SECS`]) with `need` px of headroom.
pub fn arrival_footings(footings: impl IntoIterator<Item = Footing>, monitors: &[Monitor], need: f32, settled: &dyn Fn(&Footing) -> bool) -> Vec<Footing> {
    footings.into_iter().filter(|f| settled(f) && has_headroom(f, monitors, need)).collect()
}

/// Pick a reachable spot. `random()` returns uniform [0,1); `reachable(p)` route-checks
/// (e.g. `NavGraph::plan`). Spots must lie inside a monitor's work area.
pub fn pick_spot(
    footings: &[Footing],
    knight: Vec2,
    monitors: &[Monitor],
    opts: &SpotOptions,
    random: &mut dyn FnMut() -> f32,
    reachable: &mut dyn FnMut(Vec2) -> bool,
) -> Option<PortalSpot> {
    let mon_of = |p: Vec2| monitors.iter().position(|m| m.work.contains(p.x as i32, (p.y - 1.0) as i32));
    let knight_mon = mon_of(knight);
    let diag = monitors
        .iter()
        .map(|m| m.rect)
        .reduce(|a, b| crate::geom::RectI::new(a.left.min(b.left), a.top.min(b.top), a.right.max(b.right), a.bottom.max(b.bottom)))
        .map_or(2000.0, |r| ((r.width() as f32).powi(2) + (r.height() as f32).powi(2)).sqrt().max(1.0));
    let mut cands: Vec<(PortalSpot, f32)> = footings
        .iter()
        .filter(|f| f.x1 - f.x0 >= opts.min_width.max(2.0 * opts.margin + 1.0))
        .filter_map(|f| {
            let x = f.x0 + opts.margin + random() * (f.x1 - f.x0 - 2.0 * opts.margin);
            let at = Vec2::new(x, f.y);
            let mon = mon_of(at)?;
            let d = at.distance(knight);
            if d < opts.min_distance {
                return None;
            }
            let other = if knight_mon.is_some_and(|k| k != mon) { opts.other_monitor_bonus } else { 0.0 };
            let score = d / diag + other + opts.jitter * random();
            Some((PortalSpot { at, owner: f.owner }, score))
        })
        .collect();
    cands.sort_by(|a, b| b.1.total_cmp(&a.1));
    cands.into_iter().take(opts.max_checks).map(|(s, _)| s).find(|s| reachable(s.at))
}

/// Where in its life a portal is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalPhase {
    /// Glow gathering, ripple spreading.
    Opening,
    /// The guest has stepped out; the gate lingers.
    Open,
    Closed,
}

/// How arrivals look.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortalStyle {
    /// A few small dust puffs at the feet and a quick fade-in. Unobtrusive.
    #[default]
    Dust,
    /// The dream gate: glow and the white dream ripple. Flashy; for special moments.
    Dream,
}

impl PortalStyle {
    /// Seconds from opening until the guest steps out.
    pub fn emerge(self) -> f32 {
        match self {
            PortalStyle::Dust => 0.1,
            PortalStyle::Dream => 0.7,
        }
    }

    /// Seconds until the portal is gone.
    pub fn lifetime(self) -> f32 {
        match self {
            PortalStyle::Dust => 0.6,
            PortalStyle::Dream => 2.2,
        }
    }
}

/// One portal's timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Portal {
    pub spot: PortalSpot,
    pub style: PortalStyle,
    t: f32,
    emerged: bool,
    emerge_taken: bool,
}

impl Portal {
    pub fn open(spot: PortalSpot, style: PortalStyle) -> Self {
        Self { spot, style, t: 0.0, emerged: false, emerge_taken: false }
    }

    pub fn tick(&mut self, dt: f32) -> PortalPhase {
        self.t += dt.max(0.0);
        self.emerged |= self.t >= self.style.emerge();
        self.phase()
    }

    pub fn phase(&self) -> PortalPhase {
        if self.t >= self.style.lifetime() {
            PortalPhase::Closed
        } else if self.emerged {
            PortalPhase::Open
        } else {
            PortalPhase::Opening
        }
    }

    /// `true` exactly once, when the guest should appear.
    pub fn take_emerge(&mut self) -> bool {
        if self.emerged && !self.emerge_taken {
            self.emerge_taken = true;
            return true;
        }
        false
    }

    /// Keep the gate on a window that moves (delta of its owner's top-left).
    pub fn shift(&mut self, d: Vec2) {
        self.spot.at += d;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;

    fn mons() -> Vec<Monitor> {
        vec![
            Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 },
            Monitor { rect: RectI::new(1920, 0, 3840, 1080), work: RectI::new(1920, 0, 3840, 1040), primary: false, scale_milli: 1000 },
        ]
    }

    fn seq(vals: &[f32]) -> impl FnMut() -> f32 + '_ {
        let mut i = 0;
        move || {
            i += 1;
            vals[(i - 1) % vals.len()]
        }
    }

    #[test]
    fn prefers_far_reachable_spots() {
        let footings = [
            Footing { x0: 0.0, x1: 1920.0, y: 1040.0, owner: None },
            Footing { x0: 2000.0, x1: 2600.0, y: 500.0, owner: Some(9) },
            Footing { x0: 900.0, x1: 950.0, y: 300.0, owner: Some(3) }, // too narrow
        ];
        let knight = Vec2::new(300.0, 1040.0);
        let opts = SpotOptions { jitter: 0.0, ..Default::default() };
        let mut random = seq(&[0.5]);
        let s = pick_spot(&footings, knight, &mons(), &opts, &mut random, &mut |_| true).expect("spot");
        assert_eq!(s.owner, Some(9));
        assert_eq!(s.at, Vec2::new(2300.0, 500.0));
        // The far one is unreachable: fall back to the floor (far enough from the knight).
        let mut random = seq(&[0.5]);
        let s = pick_spot(&footings, knight, &mons(), &opts, &mut random, &mut |p| p.x < 1920.0).expect("spot");
        assert_eq!((s.owner, s.at.y), (None, 1040.0));
        assert!(s.at.distance(knight) >= opts.min_distance);
    }

    #[test]
    fn nothing_when_all_too_close_or_offscreen() {
        let footings = [Footing { x0: 200.0, x1: 400.0, y: 1040.0, owner: None }, Footing { x0: 5000.0, x1: 6000.0, y: 500.0, owner: None }];
        let mut random = seq(&[0.5]);
        assert!(pick_spot(&footings, Vec2::new(300.0, 1040.0), &mons(), &SpotOptions::default(), &mut random, &mut |_| true).is_none());
    }

    #[test]
    fn arrivals_only_on_settled_footings_with_headroom() {
        let fresh = Footing { x0: 2000.0, x1: 2600.0, y: 500.0, owner: Some(9) };
        let lasting = Footing { x0: 100.0, x1: 700.0, y: 600.0, owner: Some(4) };
        let floor = Footing { x0: 0.0, x1: 1920.0, y: 1040.0, owner: None };
        let high = Footing { x0: 2000.0, x1: 2600.0, y: 120.0, owner: None }; // near the top
        let settled = |f: &Footing| f.owner != Some(9);
        let need = 3.6 * 64.0;
        let ok = arrival_footings([fresh, lasting, floor, high], &mons(), need, &settled);
        assert_eq!(ok, vec![lasting, floor]);
        assert!(!has_headroom(&high, &mons(), need));
        assert!(has_headroom(&Footing { y: need + 1.0, ..high }, &mons(), need));
        // The picker never lands on the far, fresh window ledge it would otherwise prefer.
        let knight = Vec2::new(1800.0, 1040.0);
        let opts = SpotOptions { jitter: 0.0, ..Default::default() };
        for r in [0.0, 0.5, 0.99] {
            let vals = [r];
            let mut random = seq(&vals);
            let s = pick_spot(&ok, knight, &mons(), &opts, &mut random, &mut |_| true).expect("spot");
            assert!(s.owner == Some(4) || s.owner.is_none(), "{s:?}");
            assert!(s.at.y >= need);
        }
        // Nothing settled with room: no spot at all.
        let none = arrival_footings([fresh, high], &mons(), need, &settled);
        let mut random = seq(&[0.5]);
        assert!(pick_spot(&none, knight, &mons(), &opts, &mut random, &mut |_| true).is_none());
    }

    #[test]
    fn portal_timing() {
        let mut p = Portal::open(PortalSpot { at: Vec2::new(10.0, 20.0), owner: None }, PortalStyle::Dream);
        assert_eq!(p.tick(0.3), PortalPhase::Opening);
        assert!(!p.take_emerge());
        assert_eq!(p.tick(0.5), PortalPhase::Open);
        assert!(p.take_emerge());
        assert!(!p.take_emerge());
        assert_eq!(p.tick(2.0), PortalPhase::Closed);
        // Dust arrivals are quick.
        let mut d = Portal::open(PortalSpot { at: Vec2::ZERO, owner: None }, PortalStyle::Dust);
        assert_eq!(d.tick(0.15), PortalPhase::Open);
        assert!(d.take_emerge());
        assert_eq!(d.tick(0.5), PortalPhase::Closed);
    }
}
