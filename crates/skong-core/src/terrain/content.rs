//! Window-anchored store of on-screen content surfaces.
//!
//! Vision reports surfaces in screen pixels, a frame or two late, and with hysteresis
//! that hides anything that keeps moving. [`ContentLayer`] bridges that to terrain:
//!
//! * each surface piece is **owned** by the front-most window containing it (or `None`
//!   for the wallpaper) and anchored to that window's rect at ingest time;
//! * [`ContentLayer::ledges`] translates pieces by their owner's movement since, so
//!   content rides along with a dragged window between vision updates;
//! * while a window is moving (its rect changed less than `settle` seconds before the
//!   frame was captured), fresh vision for it is ignored and its anchored pieces are
//!   kept: vision would otherwise drop them until the window rests again.
use std::collections::{HashMap, HashSet};

use super::build::{Span, subtract_all};
use crate::desktop::{DesktopSnapshot, WindowId};
use crate::geom::RectI;
use crate::vision::Surface;

/// A content surface in virtual-desktop pixels: top row `y`, columns `x0..x1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentLedge {
    pub x0: i32,
    pub x1: i32,
    pub y: i32,
    /// Front-most window showing it, `None` on the wallpaper.
    pub owner: Option<WindowId>,
}

#[derive(Clone, Copy, Debug)]
struct Anchored {
    s: Surface,
    owner: Option<WindowId>,
    /// Owner rect when the piece was observed.
    anchor: RectI,
    /// Which output (vision instance) reported it.
    source: u32,
}

#[derive(Clone, Debug)]
pub struct ContentLayer {
    /// Seconds a window must be still before its fresh vision replaces anchored pieces.
    settle: f64,
    pieces: Vec<Anchored>,
    /// Last seen rect of each window and when it last changed.
    seen: HashMap<WindowId, (RectI, f64)>,
}

impl Default for ContentLayer {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SETTLE)
    }
}

impl ContentLayer {
    /// Longer than vision's capture interval + hysteresis, so a window's content never
    /// blinks out when it comes to rest.
    pub const DEFAULT_SETTLE: f64 = 0.25;

    pub fn new(settle: f64) -> Self {
        Self { settle, pieces: Vec::new(), seen: HashMap::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// Record window movement. Call with every new desktop snapshot (`now` in the same
    /// clock as `captured_at` of [`Self::ingest`]).
    pub fn observe_desktop(&mut self, desk: &DesktopSnapshot, now: f64) {
        let mut seen = HashMap::with_capacity(desk.windows.len());
        for w in &desk.windows {
            let since = match self.seen.get(&w.id) {
                Some(&(r, t)) if r == w.rect => t,
                Some(_) => now,
                None => f64::NEG_INFINITY, // new window: nothing to carry
            };
            seen.insert(w.id, (w.rect, since));
        }
        self.seen = seen;
    }

    /// Replace everything `source` reported with `surfaces` (virtual-desktop pixels),
    /// observed in a frame captured at `captured_at`, assigning owners from `desk`.
    pub fn ingest(&mut self, source: u32, surfaces: &[Surface], desk: &DesktopSnapshot, captured_at: f64) {
        let moving: HashSet<WindowId> = self
            .seen
            .iter()
            .filter(|(_, (_, since))| *since + self.settle > captured_at)
            .map(|(id, _)| *id)
            .collect();
        self.pieces.retain(|p| p.source != source || p.owner.is_some_and(|id| moving.contains(&id)));
        for s in surfaces {
            let mut rest: Vec<Span> = vec![(s.x0, s.x1)];
            for w in &desk.windows {
                if rest.is_empty() {
                    break;
                }
                let r = w.rect;
                if !(r.top <= s.y && s.y < r.bottom) {
                    continue;
                }
                if !moving.contains(&w.id) {
                    for &(a, b) in &rest {
                        let (a, b) = (a.max(r.left), b.min(r.right));
                        if a < b {
                            self.pieces.push(Anchored { s: Surface::new(a, b, s.y), owner: Some(w.id), anchor: r, source });
                        }
                    }
                }
                rest = subtract_all(rest, std::iter::once((r.left, r.right)));
            }
            for (a, b) in rest {
                self.pieces.push(Anchored { s: Surface::new(a, b, s.y), owner: None, anchor: RectI::default(), source });
            }
        }
    }

    /// Forget everything `source` reported (its capture stopped).
    pub fn clear_source(&mut self, source: u32) {
        self.pieces.retain(|p| p.source != source);
    }

    /// Current content ledges: pieces follow their owner's movement since they were
    /// observed and are clipped to its current rect; pieces of closed windows vanish.
    pub fn ledges(&self, desk: &DesktopSnapshot) -> Vec<ContentLedge> {
        self.pieces
            .iter()
            .filter_map(|p| {
                let Some(id) = p.owner else {
                    return Some(ContentLedge { x0: p.s.x0, x1: p.s.x1, y: p.s.y, owner: None });
                };
                let r = desk.window(id)?.rect;
                let s = p.s.translated(r.left - p.anchor.left, r.top - p.anchor.top);
                let (x0, x1) = (s.x0.max(r.left), s.x1.min(r.right));
                (x0 < x1 && r.top <= s.y && s.y < r.bottom).then_some(ContentLedge { x0, x1, y: s.y, owner: p.owner })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{Monitor, WindowInfo};

    fn win(id: u64, l: i32, t: i32, r: i32, b: i32) -> WindowInfo {
        WindowInfo {
            id,
            rect: RectI::new(l, t, r, b),
            title: String::new(),
            class: String::new(),
            process: String::new(),
            pid: 0,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        }
    }

    fn desk(windows: Vec<WindowInfo>) -> DesktopSnapshot {
        let m = Monitor { rect: RectI::new(0, 0, 1000, 800), work: RectI::new(0, 0, 1000, 760), primary: true, scale_milli: 1000 };
        DesktopSnapshot { generation: 1, monitors: vec![m], windows }
    }

    fn cl(x0: i32, x1: i32, y: i32, owner: Option<u64>) -> ContentLedge {
        ContentLedge { x0, x1, y, owner }
    }

    #[test]
    fn owners_are_front_most_windows() {
        // Window 1 in front of window 2; a surface crossing both and the wallpaper.
        let d = desk(vec![win(1, 300, 100, 500, 400), win(2, 100, 200, 800, 600)]);
        let mut c = ContentLayer::default();
        c.observe_desktop(&d, 0.0);
        c.ingest(0, &[Surface::new(50, 900, 300)], &d, 1.0);
        let mut got = c.ledges(&d);
        got.sort_by_key(|l| l.x0);
        assert_eq!(
            got,
            vec![cl(50, 100, 300, None), cl(100, 300, 300, Some(2)), cl(300, 500, 300, Some(1)), cl(500, 800, 300, Some(2)), cl(800, 900, 300, None)]
        );
    }

    #[test]
    fn content_follows_owner_and_vanishes_with_it() {
        let d0 = desk(vec![win(1, 100, 100, 500, 400)]);
        let mut c = ContentLayer::default();
        c.observe_desktop(&d0, 0.0);
        c.ingest(0, &[Surface::new(150, 450, 200)], &d0, 1.0);

        let d1 = desk(vec![win(1, 130, 80, 530, 380)]);
        c.observe_desktop(&d1, 2.0);
        assert_eq!(c.ledges(&d1), vec![cl(180, 480, 180, Some(1))]);

        // Shrunk from the right: clipped to the window.
        let d2 = desk(vec![win(1, 130, 80, 300, 380)]);
        c.observe_desktop(&d2, 2.1);
        assert_eq!(c.ledges(&d2), vec![cl(180, 300, 180, Some(1))]);

        let d3 = desk(vec![]);
        c.observe_desktop(&d3, 2.2);
        assert!(c.ledges(&d3).is_empty());
    }

    #[test]
    fn moving_window_keeps_anchored_content_until_it_settles() {
        let d0 = desk(vec![win(1, 100, 100, 500, 400)]);
        let mut c = ContentLayer::new(0.25);
        c.observe_desktop(&d0, 0.0);
        c.ingest(0, &[Surface::new(150, 450, 200)], &d0, 1.0);

        // Window dragged at t=2.0; the next vision frame (t=2.03) no longer sees the
        // stroke (hysteresis hides moving content) but the anchored piece survives.
        let d1 = desk(vec![win(1, 200, 100, 600, 400)]);
        c.observe_desktop(&d1, 2.0);
        c.ingest(0, &[], &d1, 2.03);
        assert_eq!(c.ledges(&d1), vec![cl(250, 550, 200, Some(1))]);
        // Fresh pieces inside a moving window are ignored too (they may be stale).
        c.ingest(0, &[Surface::new(210, 260, 300)], &d1, 2.1);
        assert_eq!(c.ledges(&d1), vec![cl(250, 550, 200, Some(1))]);

        // At rest long enough: vision is authoritative again.
        c.ingest(0, &[Surface::new(250, 550, 200), Surface::new(210, 260, 300)], &d1, 2.3);
        let mut got = c.ledges(&d1);
        got.sort_by_key(|l| l.y);
        assert_eq!(got, vec![cl(250, 550, 200, Some(1)), cl(210, 260, 300, Some(1))]);
        c.ingest(0, &[], &d1, 2.4);
        assert!(c.ledges(&d1).is_empty());
    }

    #[test]
    fn sources_are_independent() {
        let d = desk(vec![]);
        let mut c = ContentLayer::default();
        c.observe_desktop(&d, 0.0);
        c.ingest(0, &[Surface::new(0, 100, 50)], &d, 1.0);
        c.ingest(1, &[Surface::new(1000, 1100, 50)], &d, 1.0);
        c.ingest(0, &[], &d, 1.1);
        assert_eq!(c.ledges(&d), vec![cl(1000, 1100, 50, None)]);
        c.clear_source(1);
        assert!(c.is_empty());
    }
}
