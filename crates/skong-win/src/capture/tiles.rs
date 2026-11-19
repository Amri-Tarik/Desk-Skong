//! Which parts of one screen to analyse, and when.
//!
//! The screen is split into [`TILE`]-px tiles. Frame dirty rects mark tiles *touched*;
//! every scheduling [`Tiles::tick`] turns touched tiles into *pending* work and
//! returns the pending tiles worth analysing now, as merged rects:
//!
//! * a tile touched in [`HOT_STREAK`] consecutive ticks is **hot** (animated: a video,
//!   a blinking caret, streaming text). Hot tiles are analysed at most every
//!   [`SLOW_INTERVAL`]; content that keeps moving is never reported anyway. A tile
//!   cools down (and its pending work runs at once) on the first tick it is untouched;
//! * tiles under **our own windows** (the knight's sprite window is excluded from
//!   capture, but moving it still produces dirty rects) are treated as hot;
//! * tiles inside **excluded** regions (fullscreen windows) are never analysed; when
//!   an exclusion goes away its tiles are analysed again.
use std::time::{Duration, Instant};

use skong_core::geom::RectI;

/// Tile size in px.
pub const TILE: i32 = 32;
/// Consecutive touched ticks after which a tile counts as animated.
pub const HOT_STREAK: u8 = 3;
/// Hot tiles are analysed at most this often.
pub const SLOW_INTERVAL: Duration = Duration::from_millis(250);

pub struct Tiles {
    width: i32,
    height: i32,
    cols: i32,
    rows: i32,
    touched: Vec<bool>,
    pending: Vec<bool>,
    streak: Vec<u8>,
    own: Vec<bool>,
    excluded: Vec<bool>,
    last_slow: Option<Instant>,
}

impl Tiles {
    pub fn new(width: i32, height: i32) -> Self {
        let (cols, rows) = ((width + TILE - 1) / TILE, (height + TILE - 1) / TILE);
        let n = (cols * rows).max(0) as usize;
        Self {
            width,
            height,
            cols,
            rows,
            touched: vec![false; n],
            pending: vec![false; n],
            streak: vec![0; n],
            own: vec![false; n],
            excluded: vec![false; n],
            last_slow: None,
        }
    }

    /// Tile index range `(c0..c1, r0..r1)` covering `r` (clipped).
    fn span(&self, r: &RectI) -> Option<(i32, i32, i32, i32)> {
        let r = r.intersect(&RectI::new(0, 0, self.width, self.height));
        (!r.is_empty()).then(|| (r.left / TILE, (r.right + TILE - 1) / TILE, r.top / TILE, (r.bottom + TILE - 1) / TILE))
    }

    fn for_tiles(&self, r: &RectI, mut f: impl FnMut(usize)) {
        if let Some((c0, c1, r0, r1)) = self.span(r) {
            for row in r0..r1 {
                for col in c0..c1 {
                    f((row * self.cols + col) as usize);
                }
            }
        }
    }

    /// Record a changed rect. Returns whether it touched any tile that is neither ours
    /// nor excluded (i.e. real work).
    pub fn mark(&mut self, r: &RectI) -> bool {
        let mut real = false;
        let span = self.span(r);
        let (own, excluded) = (&self.own, &self.excluded);
        let touched = &mut self.touched;
        let cols = self.cols;
        if let Some((c0, c1, r0, r1)) = span {
            for row in r0..r1 {
                for col in c0..c1 {
                    let i = (row * cols + col) as usize;
                    touched[i] = true;
                    real |= !own[i] && !excluded[i];
                }
            }
        }
        real
    }

    pub fn mark_all(&mut self) {
        self.touched.fill(true);
    }

    /// Replace the set of regions covered by our own windows (local coords).
    pub fn set_own(&mut self, rects: &[RectI]) {
        let mut own = vec![false; self.own.len()];
        for r in rects {
            self.for_tiles(r, |i| own[i] = true);
        }
        self.own = own;
    }

    /// Replace the excluded regions (local coords); tiles leaving the exclusion are
    /// marked touched so they get analysed again.
    pub fn set_excluded(&mut self, rects: &[RectI]) {
        let mut ex = vec![false; self.excluded.len()];
        for r in rects {
            self.for_tiles(r, |i| ex[i] = true);
        }
        for (i, (&was, &is)) in self.excluded.iter().zip(&ex).enumerate() {
            if was && !is {
                self.touched[i] = true;
            }
        }
        self.excluded = ex;
    }

    /// Hot pending work is waiting (wake up to cool it down or flush it).
    pub fn has_pending(&self) -> bool {
        self.pending.iter().any(|&p| p)
    }

    /// One scheduling step. Returns the rects (tile-aligned, local coords) to analyse now.
    pub fn tick(&mut self, now: Instant) -> Vec<RectI> {
        let slow_due = self.last_slow.is_none_or(|t| now >= t + SLOW_INTERVAL);
        let mut flushed_hot = false;
        let mut scan = vec![false; self.pending.len()];
        for (i, scan_i) in scan.iter_mut().enumerate() {
            if std::mem::take(&mut self.touched[i]) {
                self.streak[i] = self.streak[i].saturating_add(1);
                self.pending[i] = true;
            } else {
                self.streak[i] = 0;
            }
            if !self.pending[i] {
                continue;
            }
            if self.excluded[i] {
                self.pending[i] = false;
                continue;
            }
            let hot = self.streak[i] >= HOT_STREAK || self.own[i];
            if hot && !slow_due {
                continue;
            }
            flushed_hot |= hot;
            *scan_i = true;
            self.pending[i] = false;
        }
        if flushed_hot {
            self.last_slow = Some(now);
        }
        self.rects(&scan)
    }

    /// Merge selected tiles into rects: horizontal runs per tile row, then identical
    /// runs on consecutive rows.
    fn rects(&self, sel: &[bool]) -> Vec<RectI> {
        let mut out: Vec<RectI> = Vec::new();
        let mut prev_row: Vec<usize> = Vec::new(); // indices into `out` from the previous tile row
        for row in 0..self.rows {
            let mut this_row = Vec::new();
            let mut col = 0;
            while col < self.cols {
                if !sel[(row * self.cols + col) as usize] {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < self.cols && sel[(row * self.cols + col) as usize] {
                    col += 1;
                }
                let r = RectI::new(start * TILE, row * TILE, (col * TILE).min(self.width), ((row + 1) * TILE).min(self.height));
                match prev_row.iter().copied().find(|&k| out[k].left == r.left && out[k].right == r.right && out[k].bottom == r.top) {
                    Some(k) => {
                        out[k].bottom = r.bottom;
                        this_row.push(k);
                    }
                    None => {
                        this_row.push(out.len());
                        out.push(r);
                    }
                }
            }
            prev_row = this_row;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(v: &[RectI]) -> i32 {
        v.iter().map(|r| r.width() * r.height()).sum()
    }

    #[test]
    fn changes_are_analysed_once_and_merged() {
        let mut t = Tiles::new(320, 200); // 10 x 7 tiles, last row 8 px tall
        let t0 = Instant::now();
        assert!(t.tick(t0).is_empty());
        assert!(t.mark(&RectI::new(5, 5, 70, 40)));
        let r = t.tick(t0);
        assert_eq!(r, vec![RectI::new(0, 0, 96, 64)]);
        assert!(t.tick(t0).is_empty(), "nothing new");
        t.mark_all();
        assert_eq!(t.tick(t0), vec![RectI::new(0, 0, 320, 200)]);
    }

    #[test]
    fn animated_tiles_go_to_the_slow_lane_and_flush_when_quiet() {
        let mut t = Tiles::new(320, 320);
        let anim = RectI::new(0, 0, 32, 32);
        let mut now = Instant::now();
        let step = Duration::from_millis(33);
        // First ticks analyse it normally; from the 3rd consecutive change on it is hot.
        for i in 0..2 {
            t.mark(&anim);
            assert_eq!(t.tick(now), vec![anim], "tick {i}");
            now += step;
        }
        t.mark(&anim);
        // Hot, but the slow lane was never used: flushed right away.
        assert_eq!(t.tick(now), vec![anim]);
        now += step;
        let mut analysed = 0;
        for i in 0..30 {
            t.mark(&anim);
            // A one-off change elsewhere (a different tile each time).
            let x = 64 + 32 * (i % 8);
            t.mark(&RectI::new(x + 4, 164, x + 20, 180));
            let r = t.tick(now);
            assert!(r.contains(&RectI::new(x, 160, x + 32, 192)), "one-off changes stay fast: {r:?}");
            analysed += r.contains(&anim) as i32;
            now += step;
        }
        // ~1 s of animation at 30 Hz: ~4 slow-lane passes instead of 30.
        assert!((3..=5).contains(&analysed), "{analysed}");
        // Animation stops: the next tick cools it and flushes pending work at once.
        assert!(t.has_pending());
        assert_eq!(t.tick(now), vec![anim]);
        assert!(!t.has_pending());
    }

    #[test]
    fn own_windows_are_slow_and_excluded_regions_skipped() {
        let mut t = Tiles::new(320, 320);
        let mut now = Instant::now();
        t.set_own(&[RectI::new(40, 40, 90, 90)]);
        assert!(!t.mark(&RectI::new(40, 40, 90, 90)), "only our own window");
        assert_eq!(t.tick(now), vec![RectI::new(32, 32, 96, 96)], "first slow pass");
        now += Duration::from_millis(40);
        t.mark(&RectI::new(40, 40, 90, 90));
        assert!(t.tick(now).is_empty(), "within the slow interval");
        now += SLOW_INTERVAL;
        t.mark(&RectI::new(40, 40, 90, 90));
        assert_eq!(area(&t.tick(now)), 64 * 64);

        t.set_excluded(&[RectI::new(160, 0, 320, 320)]);
        assert!(!t.mark(&RectI::new(200, 10, 300, 300)));
        assert!(t.tick(now).is_empty());
        assert!(!t.has_pending(), "excluded work is dropped");
        t.set_excluded(&[]);
        assert_eq!(t.tick(now), vec![RectI::new(160, 0, 320, 320)], "re-analysed after exclusion ends");
    }
}
