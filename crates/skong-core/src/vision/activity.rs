//! Screen activity hotspots: where the screen keeps changing (a video, a build log,
//! someone typing), for the AI to look at.
//!
//! The platform layer feeds every captured frame's changed rects (already without our
//! own windows) into one [`ActivityGrid`] per monitor. Each [`ActivityParams::cell`]-px
//! cell accumulates the fraction of it that changed, with exponential decay
//! ([`ActivityParams::half_life`]). Frames that change most of a monitor at once (window
//! switches, scrolling a maximized window) are damped. Hot cells are clustered into
//! [`ActivitySpot`]s.
use crate::desktop::WindowId;
use crate::geom::{RectI, Vec2};

/// Where the screen is busy. Spots are sorted by decreasing intensity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    pub spots: Vec<ActivitySpot>,
}

/// A connected cluster of changing cells.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActivitySpot {
    /// Bounding box of the cluster, virtual-desktop px.
    pub rect: RectI,
    /// Activity-weighted centre, virtual-desktop px.
    pub center: Vec2,
    /// 0..1: ~0.5 for a region fully redrawn about 10 times a second for the last
    /// half-life, approaching 1 for continuous 30 Hz video.
    pub intensity: f32,
    /// How long (s) the cluster's oldest cell has been continuously hot.
    pub age_secs: f32,
    /// Front-most window containing `center` (`None`: wallpaper, or not yet assigned).
    pub owner: Option<WindowId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActivityParams {
    /// Cell size in px.
    pub cell: i32,
    /// Seconds for accumulated activity to halve.
    pub half_life: f32,
    /// A frame changing more than this fraction of the monitor is a "flash"...
    pub flash_fraction: f32,
    /// ...and counts only with this weight.
    pub flash_weight: f32,
    /// Accumulated activity at which intensity reaches 0.5.
    pub half_intensity: f32,
    /// Cells below this intensity are not part of any spot.
    pub hot: f32,
    /// At most this many spots per monitor.
    pub max_spots: usize,
}

impl Default for ActivityParams {
    fn default() -> Self {
        Self { cell: 32, half_life: 3.0, flash_fraction: 0.6, flash_weight: 0.02, half_intensity: 40.0, hot: 0.15, max_spots: 5 }
    }
}

/// Decaying per-cell activity for one monitor.
#[derive(Clone, Debug)]
pub struct ActivityGrid {
    params: ActivityParams,
    /// Monitor rect in virtual-desktop px.
    bounds: RectI,
    cols: i32,
    rows: i32,
    heat: Vec<f32>,
    /// When each cell became hot (`None` while cold).
    hot_since: Vec<Option<f64>>,
    /// Time `heat` was last decayed to.
    t: f64,
}

impl ActivityGrid {
    pub fn new(bounds: RectI, params: ActivityParams) -> Self {
        let c = params.cell.max(1);
        let (cols, rows) = ((bounds.width() + c - 1) / c, (bounds.height() + c - 1) / c);
        let n = (cols.max(0) * rows.max(0)) as usize;
        Self { params, bounds, cols, rows, heat: vec![0.0; n], hot_since: vec![None; n], t: 0.0 }
    }

    pub fn params(&self) -> &ActivityParams {
        &self.params
    }

    fn intensity(&self, heat: f32) -> f32 {
        heat / (heat + self.params.half_intensity)
    }

    /// Decay to time `now` (seconds, monotonic) and update hot-since times.
    pub fn decay_to(&mut self, now: f64) {
        let dt = (now - self.t).max(0.0) as f32;
        self.t = self.t.max(now);
        if dt > 0.0 {
            let k = 0.5f32.powf(dt / self.params.half_life);
            self.heat.iter_mut().for_each(|h| *h *= k);
        }
        let hot = self.params.hot;
        let h_int = self.params.half_intensity;
        for (h, since) in self.heat.iter().zip(&mut self.hot_since) {
            let is_hot = *h / (*h + h_int) >= hot;
            match (is_hot, *since) {
                (true, None) => *since = Some(now),
                (false, Some(_)) => *since = None,
                _ => {}
            }
        }
    }

    /// Nothing is hot or warming up.
    pub fn is_quiet(&self) -> bool {
        self.heat.iter().all(|&h| h < 0.01)
    }

    /// Add one frame's changed rects (virtual-desktop px, our own windows already
    /// removed) observed at `now`.
    pub fn add_frame(&mut self, rects: &[RectI], now: f64) {
        self.decay_to(now);
        let b = self.bounds;
        let area: i64 = rects.iter().map(|r| r.intersect(&b)).filter(|r| !r.is_empty()).map(|r| r.width() as i64 * r.height() as i64).sum();
        let total = (b.width() as i64 * b.height() as i64).max(1);
        let weight = if area as f32 > self.params.flash_fraction * total as f32 { self.params.flash_weight } else { 1.0 };
        let c = self.params.cell;
        let cell_area = (c * c) as f32;
        for r in rects {
            let r = r.intersect(&b);
            if r.is_empty() {
                continue;
            }
            let (l, t, rr, bb) = (r.left - b.left, r.top - b.top, r.right - b.left, r.bottom - b.top);
            for row in t / c..(bb + c - 1) / c {
                let (y0, y1) = ((row * c).max(t), ((row + 1) * c).min(bb));
                for col in l / c..(rr + c - 1) / c {
                    let (x0, x1) = ((col * c).max(l), ((col + 1) * c).min(rr));
                    let i = (row * self.cols + col) as usize;
                    self.heat[i] += weight * ((x1 - x0) * (y1 - y0)) as f32 / cell_area;
                }
            }
        }
        self.decay_to(now); // refresh hot-since for cells that just got hot
    }

    /// Hot clusters at time `now` (after decaying), strongest first, at most
    /// `max_spots`. Owners are left `None` (the caller knows the windows).
    pub fn spots(&mut self, now: f64) -> Vec<ActivitySpot> {
        self.decay_to(now);
        let (cols, rows) = (self.cols, self.rows);
        let hot: Vec<bool> = self.heat.iter().map(|&h| self.intensity(h) >= self.params.hot).collect();
        let mut seen = vec![false; hot.len()];
        let mut out = Vec::new();
        let c = self.params.cell;
        for start in 0..hot.len() {
            if !hot[start] || seen[start] {
                continue;
            }
            // Flood fill (8-connected).
            let mut stack = vec![start];
            seen[start] = true;
            let (mut c0, mut r0, mut c1, mut r1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            let (mut wsum, mut cx, mut cy, mut peak, mut since) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, now);
            while let Some(i) = stack.pop() {
                let (col, row) = (i as i32 % cols, i as i32 / cols);
                (c0, r0, c1, r1) = (c0.min(col), r0.min(row), c1.max(col), r1.max(row));
                let h = self.heat[i];
                wsum += h;
                cx += h * (col as f32 + 0.5);
                cy += h * (row as f32 + 0.5);
                peak = peak.max(self.intensity(h));
                if let Some(s) = self.hot_since[i] {
                    since = since.min(s);
                }
                for dr in -1..=1 {
                    for dc in -1..=1 {
                        let (nc, nr) = (col + dc, row + dr);
                        if (0..cols).contains(&nc) && (0..rows).contains(&nr) {
                            let j = (nr * cols + nc) as usize;
                            if hot[j] && !seen[j] {
                                seen[j] = true;
                                stack.push(j);
                            }
                        }
                    }
                }
            }
            let b = self.bounds;
            let rect = RectI::new(b.left + c0 * c, b.top + r0 * c, b.left + (c1 + 1) * c, b.top + (r1 + 1) * c).intersect(&b);
            let center = Vec2::new(b.left as f32 + cx / wsum * c as f32, b.top as f32 + cy / wsum * c as f32);
            out.push(ActivitySpot { rect, center, intensity: peak, age_secs: (now - since).max(0.0) as f32, owner: None });
        }
        out.sort_by(|a, b| b.intensity.total_cmp(&a.intensity));
        out.truncate(self.params.max_spots);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RectI = RectI::new(0, 0, 1920, 1080);

    #[test]
    fn continuously_updating_region_becomes_the_top_spot() {
        let mut g = ActivityGrid::new(SCREEN, ActivityParams::default());
        let video = RectI::new(600, 300, 900, 500);
        let caret = RectI::new(100, 900, 102, 918);
        let mut t = 0.0;
        for i in 0..60 {
            let mut rects = vec![video];
            if i % 15 == 0 {
                rects.push(caret); // blinks twice a second
            }
            g.add_frame(&rects, t);
            t += 1.0 / 30.0;
        }
        let spots = g.spots(t);
        assert_eq!(spots.len(), 1, "{spots:?}");
        let s = spots[0];
        // Within a cell of the region (barely covered edge cells stay cold).
        assert!(s.rect.left <= 600 && s.rect.right >= 900 - 32 && s.rect.top <= 300 && s.rect.bottom >= 500, "{s:?}");
        assert!(s.rect.width() <= 300 + 64 && s.rect.height() <= 200 + 64, "{s:?}");
        assert!((s.center.x - 750.0).abs() < 20.0 && (s.center.y - 400.0).abs() < 20.0, "{s:?}");
        assert!(s.intensity > 0.5, "{s:?}");
        assert!(s.age_secs > 1.5, "{s:?}");
    }

    #[test]
    fn one_off_full_screen_change_is_not_a_spot() {
        let mut g = ActivityGrid::new(SCREEN, ActivityParams::default());
        g.add_frame(&[SCREEN], 0.0);
        assert!(g.spots(0.1).is_empty());
        // Even repeated flashes (window switching) stay faint.
        for i in 0..10 {
            g.add_frame(&[SCREEN], 0.2 + i as f64 * 0.5);
        }
        assert!(g.spots(5.5).is_empty());
    }

    #[test]
    fn activity_decays_and_clusters_split() {
        let mut g = ActivityGrid::new(SCREEN, ActivityParams::default());
        let (a, b) = (RectI::new(0, 0, 200, 200), RectI::new(1500, 800, 1700, 1000));
        for i in 0..90 {
            let t = i as f64 / 30.0;
            g.add_frame(&[a], t);
            if i % 2 == 0 {
                g.add_frame(&[b], t);
            }
        }
        let spots = g.spots(3.0);
        assert_eq!(spots.len(), 2, "{spots:?}");
        assert!(spots[0].rect.left == 0 && spots[1].rect.left >= 1472, "strongest first: {spots:?}");
        assert!(spots[0].intensity > spots[1].intensity);
        // 4 half-lives of silence: heat / 16, below the hot threshold.
        assert!(g.spots(3.0 + 12.0).is_empty());
        assert!(!g.is_quiet());
        g.decay_to(60.0);
        assert!(g.is_quiet());
    }

    #[test]
    fn grid_respects_monitor_offset() {
        let mon = RectI::new(-1920, 0, 0, 1080);
        let mut g = ActivityGrid::new(mon, ActivityParams::default());
        for i in 0..60 {
            g.add_frame(&[RectI::new(-1000, 100, -800, 300)], i as f64 / 30.0);
        }
        let s = g.spots(2.0);
        assert_eq!(s.len(), 1);
        assert!(s[0].rect.left <= -1000 && s[0].rect.right >= -800 && s[0].center.x < -800.0, "{s:?}");
    }
}
