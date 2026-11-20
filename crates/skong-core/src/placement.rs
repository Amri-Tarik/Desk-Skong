//! Test-panel placement ("📍 Place…"): the user clicks anywhere on screen and the thing goes
//! on the nearest ledge at or below the click — a window's title bar, a content line, the
//! monitor floor. Never on a monitor running a fullscreen game. Pure: the app captures the
//! click (`skong_win::click_capture`) and does the rest.
use std::fmt;

use crate::desktop::{Monitor, WindowId};
use crate::geom::{RectI, Vec2};
use crate::terrain::{LedgeKind, Terrain};

/// A click just above a ledge (up to this many px) still lands on it, so clicking right on
/// a title bar's top edge works.
pub const ABOVE_SLACK: f32 = 6.0;
/// Keep a placed thing this far from its ledge's ends (px), when the ledge is wide enough.
pub const EDGE_MARGIN: f32 = 12.0;

/// Where a placed thing stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapped {
    /// Feet point on the ledge.
    pub at: Vec2,
    /// The window whose edge it is (`None`: the floor).
    pub owner: Option<WindowId>,
    /// The ledge's extent (x0, x1).
    pub span: (f32, f32),
    pub kind: LedgeKind,
}

/// Why a click can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapError {
    /// Not on any monitor.
    OffScreen,
    /// On a monitor running a fullscreen game (nothing of ours goes there).
    GameMonitor,
    /// No ledge under the click on its monitor (e.g. below the floor: the taskbar).
    NoLedge,
}

impl fmt::Display for SnapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SnapError::OffScreen => "that click wasn't on a monitor",
            SnapError::GameMonitor => "that monitor is running a fullscreen game",
            SnapError::NoLedge => "nothing to stand on under that click (try above the taskbar)",
        })
    }
}

/// The nearest ledge at or below `click` (within [`ABOVE_SLACK`] above it), on the click's
/// monitor, and the point on it under the click, kept `half_width` + [`EDGE_MARGIN`] from
/// its ends (centred when it's narrower than that).
pub fn snap_below(terrain: &Terrain, monitors: &[Monitor], game_monitors: &[RectI], click: Vec2, half_width: f32) -> Result<Snapped, SnapError> {
    let (cx, cy) = (click.x.floor() as i32, click.y.floor() as i32);
    let mon = monitors.iter().find(|m| m.rect.contains(cx, cy)).ok_or(SnapError::OffScreen)?;
    if game_monitors.iter().any(|g| g.contains(cx, cy) || *g == mon.rect) {
        return Err(SnapError::GameMonitor);
    }
    let r = mon.rect.to_f();
    let l = terrain
        .ledges
        .iter()
        .filter(|l| l.x0 <= click.x && click.x <= l.x1 && l.y >= click.y - ABOVE_SLACK && l.y > r.top && l.y <= r.bottom + 1.0)
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .ok_or(SnapError::NoLedge)?;
    let keep = half_width.max(0.0) + EDGE_MARGIN;
    let (lo, hi) = (l.x0.max(r.left) + keep, l.x1.min(r.right) - keep);
    let x = if lo <= hi { click.x.clamp(lo, hi) } else { 0.5 * (l.x0.max(r.left) + l.x1.min(r.right)) };
    Ok(Snapped { at: Vec2::new(x, l.y), owner: l.owner, span: (l.x0, l.x1), kind: l.kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectF;
    use crate::terrain::{Ledge, WindowBox};

    fn monitors() -> Vec<Monitor> {
        vec![
            Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 },
            Monitor { rect: RectI::new(1920, 0, 3840, 1080), work: RectI::new(1920, 0, 3840, 1040), primary: false, scale_milli: 1000 },
        ]
    }

    /// A window 7 (title bar at y 400, x 100..500) with a content line at y 600 in it, the
    /// floors of both monitors at 1040.
    fn terrain() -> Terrain {
        Terrain {
            windows: vec![WindowBox { id: 7, rect: RectF::new(100.0, 400.0, 500.0, 900.0) }],
            ledges: vec![
                Ledge { x0: 100.0, x1: 500.0, y: 400.0, kind: LedgeKind::OneWay, owner: Some(7) },
                Ledge { x0: 150.0, x1: 450.0, y: 600.0, kind: LedgeKind::Content, owner: Some(7) },
                Ledge { x0: 0.0, x1: 1920.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
                Ledge { x0: 1920.0, x1: 3840.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
            ],
            bounds: RectF::new(0.0, 0.0, 3840.0, 1080.0),
            ..Default::default()
        }
    }

    #[test]
    fn a_click_lands_on_the_first_ledge_below_it() {
        let (t, m) = (terrain(), monitors());
        // Above the window: its title bar.
        let s = snap_below(&t, &m, &[], Vec2::new(300.0, 120.0), 10.0).unwrap();
        assert_eq!((s.at, s.owner, s.kind), (Vec2::new(300.0, 400.0), Some(7), LedgeKind::OneWay));
        // Right on the title bar's top edge (a few px above) still counts.
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(300.0, 396.0), 10.0).unwrap().at.y, 400.0);
        // Inside the window, above the content line: the line.
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(300.0, 500.0), 10.0).unwrap().at.y, 600.0);
        // Beside the window: the floor.
        let s = snap_below(&t, &m, &[], Vec2::new(800.0, 300.0), 10.0).unwrap();
        assert_eq!((s.at, s.owner), (Vec2::new(800.0, 1040.0), None));
        // The other monitor: its own floor.
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(2500.0, 50.0), 10.0).unwrap().at, Vec2::new(2500.0, 1040.0));
    }

    #[test]
    fn it_keeps_clear_of_the_ledge_ends() {
        let (t, m) = (terrain(), monitors());
        // Near the title bar's left end: moved in by half its width + the margin.
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(102.0, 300.0), 30.0).unwrap().at.x, 100.0 + 30.0 + EDGE_MARGIN);
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(499.0, 300.0), 30.0).unwrap().at.x, 500.0 - 30.0 - EDGE_MARGIN);
        // Wider than the ledge: centred on it.
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(120.0, 300.0), 400.0).unwrap().at.x, 300.0);
        // A floor ending at the monitor seam stays on its monitor.
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(1919.0, 300.0), 20.0).unwrap().at.x, 1920.0 - 20.0 - EDGE_MARGIN);
    }

    #[test]
    fn never_on_a_game_monitor_off_screen_or_below_the_floor() {
        let (t, m) = (terrain(), monitors());
        assert_eq!(snap_below(&t, &m, &[RectI::new(1920, 0, 3840, 1080)], Vec2::new(2500.0, 50.0), 10.0), Err(SnapError::GameMonitor));
        // The other monitor is still fine.
        assert!(snap_below(&t, &m, &[RectI::new(1920, 0, 3840, 1080)], Vec2::new(300.0, 50.0), 10.0).is_ok());
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(-50.0, 50.0), 10.0), Err(SnapError::OffScreen));
        // On the taskbar (below the floor line).
        assert_eq!(snap_below(&t, &m, &[], Vec2::new(800.0, 1060.0), 10.0), Err(SnapError::NoLedge));
        assert!(SnapError::NoLedge.to_string().contains("taskbar"));
    }
}
