//! How long each window-owned ledge has been on screen. Things are only found on surfaces
//! that last (a title bar, a table border, a divider that stayed put), not on a line that
//! flashed by while scrolling.
//!
//! Identity across terrain rebuilds: same owner window, about the same height relative to
//! the window top, overlapping horizontally (window-relative, so moving a window keeps
//! its ledges' ages). A ledge unseen for [`FORGET_SECS`] is forgotten; if it comes back it
//! starts aging from zero.
use crate::desktop::WindowId;
use crate::terrain::{Ledge, Terrain};

/// A ledge missing this long is forgotten (s): rides out re-detection flicker.
pub const FORGET_SECS: f32 = 3.0;
/// Same ledge if within this many px vertically (window-relative).
const DY_TOL: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    owner: WindowId,
    /// Window-relative geometry (px from the window's left / top).
    dy: f32,
    x0: f32,
    x1: f32,
    first: f32,
    last: f32,
}

#[derive(Clone, Debug, Default)]
pub struct LedgeAges {
    entries: Vec<Entry>,
}

/// Window-relative (x0, x1, dy) of a window-owned ledge.
fn rel(terrain: &Terrain, l: &Ledge) -> Option<(WindowId, f32, f32, f32)> {
    let id = l.owner?;
    let r = terrain.window_rect(id)?;
    Some((id, l.x0 - r.left, l.x1 - r.left, l.y - r.top))
}

impl LedgeAges {
    fn find(&self, owner: WindowId, x0: f32, x1: f32, dy: f32) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.owner == owner && (e.dy - dy).abs() <= DY_TOL)
            .map(|(i, e)| (i, e.x1.min(x1) - e.x0.max(x0), (e.x1 - e.x0).min(x1 - x0)))
            .filter(|&(_, overlap, narrow)| overlap >= 0.5 * narrow.max(1.0))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _, _)| i)
    }

    /// Note every window-owned ledge of `terrain` as seen at `now` (call on each terrain
    /// rebuild and every so often in between).
    pub fn observe(&mut self, terrain: &Terrain, now: f32) {
        for l in &terrain.ledges {
            let Some((owner, x0, x1, dy)) = rel(terrain, l) else { continue };
            match self.find(owner, x0, x1, dy) {
                Some(i) => {
                    let e = &mut self.entries[i];
                    (e.x0, e.x1, e.dy, e.last) = (x0, x1, dy, now);
                }
                None => self.entries.push(Entry { owner, dy, x0, x1, first: now, last: now }),
            }
        }
        self.entries.retain(|e| now - e.last <= FORGET_SECS);
    }

    /// Seconds `l` has been on screen (0 if unknown).
    pub fn age(&self, terrain: &Terrain, l: &Ledge, now: f32) -> f32 {
        rel(terrain, l).and_then(|(o, x0, x1, dy)| self.find(o, x0, x1, dy)).map_or(0.0, |i| now - self.entries[i].first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectF;
    use crate::terrain::{LedgeKind, WindowBox};

    fn terrain(left: f32, top: f32, with_line: bool) -> Terrain {
        let mut ledges = vec![Ledge { x0: left, x1: left + 600.0, y: top, kind: LedgeKind::OneWay, owner: Some(1) }];
        if with_line {
            ledges.push(Ledge { x0: left + 50.0, x1: left + 400.0, y: top + 200.0, kind: LedgeKind::Content, owner: Some(1) });
        }
        Terrain { windows: vec![WindowBox { id: 1, rect: RectF::new(left, top, left + 600.0, top + 500.0) }], ledges, ..Default::default() }
    }

    #[test]
    fn ledges_age_while_on_screen_and_follow_their_window() {
        let mut a = LedgeAges::default();
        let t = terrain(100.0, 100.0, true);
        for i in 0..=40 {
            a.observe(&t, i as f32);
        }
        assert_eq!(a.age(&t, &t.ledges[1], 40.0), 40.0);
        // The window moves: same ledges, same ages.
        let moved = terrain(500.0, 300.0, true);
        a.observe(&moved, 41.0);
        assert_eq!(a.age(&moved, &moved.ledges[0], 41.0), 41.0);
        assert_eq!(a.age(&moved, &moved.ledges[1], 41.0), 41.0);
    }

    #[test]
    fn a_line_that_goes_away_starts_over() {
        let mut a = LedgeAges::default();
        let with = terrain(100.0, 100.0, true);
        a.observe(&with, 0.0);
        a.observe(&with, 20.0);
        // A brief flicker is ridden out...
        a.observe(&terrain(100.0, 100.0, false), 21.0);
        a.observe(&with, 22.0);
        assert_eq!(a.age(&with, &with.ledges[1], 22.0), 22.0);
        // ... a real disappearance is not.
        a.observe(&terrain(100.0, 100.0, false), 30.0);
        a.observe(&with, 31.0);
        assert_eq!(a.age(&with, &with.ledges[1], 31.0), 0.0);
    }
}
