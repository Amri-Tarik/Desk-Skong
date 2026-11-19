//! Dream portals: something (a visitor, later other props) arrives at a random standable
//! spot anywhere on the virtual desktop through a rippling dream gate.
//!
//! * [`pick_spot`] chooses where: a standable surface the knight can actually reach,
//!   preferring places far from him and — sometimes — on another monitor, so he gets to
//!   cross the screen with his full movement kit.
//! * [`Portal`] is the timing: it opens (glow + ripple), lets its guest step out at
//!   [`Portal::EMERGE`], and closes after [`Portal::LIFETIME`]. The app renders it (dream
//!   gate glow and the white dream ripple) and asks [`Portal::take_emerge`] when to show
//!   the guest.
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

/// One dream gate's timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Portal {
    pub spot: PortalSpot,
    t: f32,
    emerged: bool,
    emerge_taken: bool,
}

impl Portal {
    /// Seconds from opening until the guest steps out.
    pub const EMERGE: f32 = 0.7;
    /// Seconds until the gate is gone.
    pub const LIFETIME: f32 = 2.2;

    pub fn open(spot: PortalSpot) -> Self {
        Self { spot, t: 0.0, emerged: false, emerge_taken: false }
    }

    pub fn tick(&mut self, dt: f32) -> PortalPhase {
        self.t += dt.max(0.0);
        self.emerged |= self.t >= Self::EMERGE;
        self.phase()
    }

    pub fn phase(&self) -> PortalPhase {
        if self.t >= Self::LIFETIME {
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
    fn portal_timing() {
        let mut p = Portal::open(PortalSpot { at: Vec2::new(10.0, 20.0), owner: None });
        assert_eq!(p.tick(0.3), PortalPhase::Opening);
        assert!(!p.take_emerge());
        assert_eq!(p.tick(0.5), PortalPhase::Open);
        assert!(p.take_emerge());
        assert!(!p.take_emerge());
        assert_eq!(p.tick(2.0), PortalPhase::Closed);
    }
}
