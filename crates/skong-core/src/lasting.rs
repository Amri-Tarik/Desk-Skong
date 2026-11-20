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

/// Window ledges must have lasted this long (s) before visitors arrive on them or the
/// knight sits on them: anything younger may be a window being dragged or content
/// scrolling past (things ended up floating in the air).
pub const SETTLED_SECS: f32 = 15.0;

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
    first: f64,
    last: f64,
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
    pub fn observe(&mut self, terrain: &Terrain, now: f64) {
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
        self.entries.retain(|e| ((now - e.last) as f32) <= FORGET_SECS);
    }

    /// Like [`LedgeAges::observe`], but anything new counts as already `age` seconds old (the
    /// first look at a desktop: what is there was there before anyone looked).
    pub fn observe_aged(&mut self, terrain: &Terrain, now: f64, age: f32) {
        self.observe(terrain, now);
        for e in self.entries.iter_mut().filter(|e| e.first == now) {
            e.first = now - age as f64;
        }
    }

    /// Has `l` been around long enough to stand things on ([`SETTLED_SECS`])? Ledges no
    /// window owns (monitor floors) always are.
    pub fn settled(&self, terrain: &Terrain, l: &Ledge, now: f64) -> bool {
        l.owner.is_none() || self.age(terrain, l, now) >= SETTLED_SECS
    }

    /// Seconds `l` has been on screen (0 if unknown).
    pub fn age(&self, terrain: &Terrain, l: &Ledge, now: f64) -> f32 {
        rel(terrain, l).and_then(|(o, x0, x1, dy)| self.find(o, x0, x1, dy)).map_or(0.0, |i| (now - self.entries[i].first) as f32)
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
            a.observe(&t, i as f64);
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

    #[test]
    fn only_settled_ledges_carry_things_and_the_first_look_counts_as_settled() {
        // What is on screen when the brain first looks counts as settled at once.
        let t = terrain(100.0, 100.0, false);
        let mut a = LedgeAges::default();
        a.observe_aged(&t, 0.0, SETTLED_SECS);
        assert!(a.settled(&t, &t.ledges[0], 0.0));
        // A content line that appears later must last SETTLED_SECS first.
        let line = terrain(100.0, 100.0, true);
        for i in 1..=10 {
            a.observe(&line, i as f64);
        }
        assert!(!a.settled(&line, &line.ledges[1], 10.0));
        for i in 11..=26 {
            a.observe(&line, i as f64);
        }
        assert!(a.settled(&line, &line.ledges[1], 26.0));
        // Floors (no owner) always are.
        let floor = Ledge { x0: 0.0, x1: 100.0, y: 1000.0, kind: LedgeKind::Ground, owner: None };
        assert!(LedgeAges::default().settled(&line, &floor, 0.0));
    }
}
