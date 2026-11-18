//! Screen-content surfaces: horizontal boundaries on screen the knight can stand on
//! (a line drawn in Paint, a table border, a toolbar edge...).
//!
//! Pure and platform-independent: the platform layer hands in BGRA pixels plus the
//! rectangles that changed, this module keeps a persistent per-output [`SurfaceMap`].
//!
//! # Detection
//!
//! Pixel `(x, y)` is a **top-edge pixel** when the row above looks like air and the pixel
//! contrasts with it:
//!
//! * `contrast = d(P[y][x], P[y-1][x]) >= edge_contrast`,
//! * `noise = max(d(P[y-1][x], P[y-1][x±1]), d(P[y-1][x], P[y-2][x])) <= air_tolerance`,
//! * `2 * noise <= contrast` (rejects gradients, tolerates anti-aliasing),
//!
//! where `d` is the largest per-channel (B, G, R) absolute difference, so colours of
//! equal luminance still separate. Edge pixels of one row are joined into runs
//! (gaps up to `max_gap` px), runs shorter than `min_run` are dropped (text), and a
//! run within `merge_rows` rows below an already kept run is suppressed where they
//! overlap, so a 1–5 px stroke yields one surface at its top.
//!
//! # Incremental updates and hysteresis
//!
//! [`SurfaceMap::rescan`] recomputes the edge mask only inside dirty rects (plus the two
//! rows below each, which depend on them) and re-joins runs only for touched rows.
//! [`SurfaceMap::update`] then applies hysteresis: a new surface (or a new stretch of an
//! existing one) is reported only after it was seen in two updates at least
//! `persist_secs` apart; anything that disappears is dropped immediately.
use crate::geom::RectI;

/// Detection tuning. See module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct VisionParams {
    /// Minimum colour step from the air row to the surface row (0–255).
    pub edge_contrast: u32,
    /// Maximum colour variation inside the air row (0–255).
    pub air_tolerance: u32,
    /// Minimum surface length in px.
    pub min_run: u32,
    /// Gaps up to this many px inside a run are bridged.
    pub max_gap: u32,
    /// A surface this many rows (or fewer) below a kept one is merged into it.
    pub merge_rows: u32,
    /// A new surface must persist this long (and over two updates) before being reported.
    pub persist_secs: f64,
}

impl Default for VisionParams {
    fn default() -> Self {
        Self { edge_contrast: 40, air_tolerance: 32, min_run: 48, max_gap: 2, merge_rows: 6, persist_secs: 0.06 }
    }
}

/// A horizontal surface: its top-edge pixel row `y`, columns `x0..x1` (half-open).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Surface {
    pub y: i32,
    pub x0: i32,
    pub x1: i32,
}

impl Surface {
    pub fn new(x0: i32, x1: i32, y: i32) -> Self {
        Self { y, x0, x1 }
    }
    pub fn len(&self) -> i32 {
        self.x1 - self.x0
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0
    }
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self { y: self.y + dy, x0: self.x0 + dx, x1: self.x1 + dx }
    }
}

/// Borrowed 32-bit BGRA image (alpha ignored). `stride` is in pixels.
#[derive(Clone, Copy, Debug)]
pub struct PixelView<'a> {
    pub px: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

impl<'a> PixelView<'a> {
    /// `None` when `px` is too short for the given geometry.
    pub fn new(px: &'a [u32], width: usize, height: usize, stride: usize) -> Option<Self> {
        let need = if height == 0 { 0 } else { (height - 1) * stride + width };
        (stride >= width && px.len() >= need).then_some(Self { px, width, height, stride })
    }

    fn row(&self, y: usize) -> &'a [u32] {
        &self.px[y * self.stride..y * self.stride + self.width]
    }
}

/// Largest per-channel absolute difference of two BGRA pixels (alpha ignored).
#[inline(always)]
fn cdiff(a: u32, b: u32) -> u32 {
    let d = |s: u32| ((a >> s) & 0xff).abs_diff((b >> s) & 0xff);
    d(0).max(d(8)).max(d(16))
}

/// Persistent surface detector for one screen (output).
pub struct SurfaceMap {
    params: VisionParams,
    width: usize,
    height: usize,
    /// 1 where a top-edge pixel was found, row-major.
    mask: Vec<u8>,
    /// Joined runs per row (`x0..x1`, length >= `min_run`).
    runs: Vec<Vec<(u32, u32)>>,
    /// Merged surfaces from the current mask, sorted. Before hysteresis.
    raw: Vec<Surface>,
    /// Reported surfaces, sorted.
    confirmed: Vec<Surface>,
    pending: Vec<Pending>,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    s: Surface,
    since: f64,
    seen: u32,
}

impl SurfaceMap {
    pub fn new(width: usize, height: usize, params: VisionParams) -> Self {
        Self {
            params,
            width,
            height,
            mask: vec![0; width * height],
            runs: vec![Vec::new(); height],
            raw: Vec::new(),
            confirmed: Vec::new(),
            pending: Vec::new(),
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn params(&self) -> &VisionParams {
        &self.params
    }

    /// Surfaces reported after hysteresis (local pixel coords), sorted by `(y, x0)`.
    pub fn surfaces(&self) -> &[Surface] {
        &self.confirmed
    }

    /// Surfaces in the current image, before hysteresis.
    pub fn raw_surfaces(&self) -> &[Surface] {
        &self.raw
    }

    /// When the next pending surface may be promoted (call [`Self::update`] then even if
    /// nothing changed on screen).
    pub fn next_deadline(&self) -> Option<f64> {
        self.pending.iter().map(|p| p.since + self.params.persist_secs).reduce(f64::min)
    }

    /// Re-detect inside `dirty` (local coords; clipped). `img` must be the whole current
    /// screen with this map's size; returns `false` (and does nothing) otherwise.
    pub fn rescan(&mut self, img: &PixelView, dirty: &[RectI]) -> bool {
        if img.width != self.width || img.height != self.height {
            return false;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let mut touched = vec![false; self.height];
        for r in dirty {
            // Edges on row y depend on rows y-2..=y and columns x-1..=x+1.
            let x0 = (r.left - 1).max(1);
            let x1 = (r.right + 1).min(w - 1);
            let y0 = r.top.max(2);
            let y1 = (r.bottom + 2).min(h);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            self.scan_rect(img, x0 as usize, x1 as usize, y0 as usize, y1 as usize);
            touched[y0 as usize..y1 as usize].iter_mut().for_each(|t| *t = true);
        }
        for (y, _) in touched.iter().enumerate().filter(|(_, t)| **t) {
            self.runs[y] = self.join_runs(y);
        }
        self.raw = self.merge_rows();
        true
    }

    fn scan_rect(&mut self, img: &PixelView, x0: usize, x1: usize, y0: usize, y1: usize) {
        let (t, a) = (self.params.edge_contrast, self.params.air_tolerance);
        let avx2 = has_avx2();
        for y in y0..y1 {
            let up_row = img.row(y - 1);
            let rows = Rows {
                cur: &img.row(y)[x0..x1],
                up: &up_row[x0..x1],
                ul: &up_row[x0 - 1..x1 - 1],
                ur: &up_row[x0 + 1..x1 + 1],
                up2: &img.row(y - 2)[x0..x1],
            };
            let m = &mut self.mask[y * self.width + x0..y * self.width + x1];
            #[cfg(target_arch = "x86_64")]
            if avx2 {
                // SAFETY: AVX2 support was verified at runtime by `has_avx2`.
                unsafe { edge_row_avx2(&rows, m, t, a) };
                continue;
            }
            let _ = avx2;
            edge_row(&rows, m, t, a);
        }
    }

    /// Runs of edge pixels on row `y`, gaps bridged, short ones dropped.
    fn join_runs(&self, y: usize) -> Vec<(u32, u32)> {
        let row = &self.mask[y * self.width..(y + 1) * self.width];
        let (gap, min) = (self.params.max_gap as usize, self.params.min_run as usize);
        let mut out = Vec::new();
        let mut cur: Option<(usize, usize)> = None;
        let mut x = 0;
        while x < row.len() {
            // Skip zero bytes 8 at a time.
            if let Some(chunk) = row.get(x..x + 8)
                && u64::from_ne_bytes(chunk.try_into().unwrap_or([1; 8])) == 0
            {
                x += 8;
                continue;
            }
            if row[x] != 0 {
                cur = match cur {
                    Some((s, e)) if x <= e + gap => Some((s, x + 1)),
                    Some((s, e)) => {
                        if e - s >= min {
                            out.push((s as u32, e as u32));
                        }
                        Some((x, x + 1))
                    }
                    None => Some((x, x + 1)),
                };
            }
            x += 1;
        }
        if let Some((s, e)) = cur
            && e - s >= min
        {
            out.push((s as u32, e as u32));
        }
        out
    }

    /// Keep the topmost of near-duplicate rows: a run is cut where a kept surface lies
    /// at most `merge_rows` rows above it.
    fn merge_rows(&self) -> Vec<Surface> {
        let merge = self.params.merge_rows as i32;
        let min = self.params.min_run as i32;
        let mut out: Vec<Surface> = Vec::new();
        // Index into `out` of the first surface that may still be within `merge` rows.
        let mut window_start = 0;
        for (y, runs) in self.runs.iter().enumerate() {
            if runs.is_empty() {
                continue;
            }
            let y = y as i32;
            while window_start < out.len() && out[window_start].y < y - merge {
                window_start += 1;
            }
            let row_start = out.len();
            for &(a, b) in runs {
                let mut pieces = vec![(a as i32, b as i32)];
                for k in &out[window_start..row_start] {
                    pieces = pieces.into_iter().flat_map(|p| subtract(p, (k.x0, k.x1))).collect();
                }
                out.extend(pieces.into_iter().filter(|(a, b)| b - a >= min).map(|(a, b)| Surface::new(a, b, y)));
            }
        }
        out
    }

    /// Hysteresis step at time `now` (seconds, monotonic). Returns `true` when
    /// [`Self::surfaces`] changed.
    pub fn update(&mut self, now: f64) -> bool {
        let mut confirmed: Vec<Surface> = Vec::new();
        let mut pending: Vec<Pending> = Vec::new();
        for c in &self.raw {
            let same_row = row_of(&self.confirmed, c.y, |k| k.y).iter();
            // Still-present parts of reported surfaces stay, immediately.
            confirmed.extend(same_row.clone().filter_map(|k| {
                let (a, b) = (k.x0.max(c.x0), k.x1.min(c.x1));
                (a < b).then_some(Surface::new(a, b, c.y))
            }));
            let mut fresh = vec![(c.x0, c.x1)];
            for k in same_row {
                fresh = fresh.into_iter().flat_map(|p| subtract(p, (k.x0, k.x1))).collect();
            }
            for (a, b) in fresh {
                let s = Surface::new(a, b, c.y);
                let prev = row_of(&self.pending, s.y, |p| p.s.y).iter().filter(|p| p.s.x0 < s.x1 && s.x0 < p.s.x1);
                let (since, seen) = prev.fold((now, 0), |(t, n), p| (t.min(p.since), n.max(p.seen)));
                let seen = seen + 1;
                if seen >= 2 && now - since >= self.params.persist_secs - 1e-9 {
                    confirmed.push(s);
                } else {
                    pending.push(Pending { s, since, seen });
                }
            }
        }
        confirmed.sort_unstable();
        // Re-join pieces of one surface that were confirmed at different times.
        let mut joined: Vec<Surface> = Vec::with_capacity(confirmed.len());
        for s in confirmed {
            match joined.last_mut() {
                Some(l) if l.y == s.y && s.x0 <= l.x1 + self.params.max_gap as i32 => l.x1 = l.x1.max(s.x1),
                _ => joined.push(s),
            }
        }
        self.pending = pending;
        let changed = joined != self.confirmed;
        self.confirmed = joined;
        changed
    }
}

/// The five pixel rows (same length) one mask row depends on.
struct Rows<'a> {
    cur: &'a [u32],
    up: &'a [u32],
    ul: &'a [u32],
    ur: &'a [u32],
    up2: &'a [u32],
}

/// Edge test for one row segment; written branch-free so it auto-vectorizes.
#[inline(always)]
fn edge_row(r: &Rows, m: &mut [u8], t: u32, a: u32) {
    let n = m.len();
    let (cur, up, ul, ur, up2) = (&r.cur[..n], &r.up[..n], &r.ul[..n], &r.ur[..n], &r.up2[..n]);
    for i in 0..n {
        let contrast = cdiff(cur[i], up[i]);
        let noise = cdiff(up[i], ul[i]).max(cdiff(up[i], ur[i])).max(cdiff(up[i], up2[i]));
        m[i] = ((contrast >= t) & (noise <= a) & (2 * noise <= contrast)) as u8;
    }
}

/// [`edge_row`] compiled for AVX2 (8 pixels per instruction, unsigned 32-bit max/min).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn edge_row_avx2(r: &Rows, m: &mut [u8], t: u32, a: u32) {
    edge_row(r, m, t, a);
}

fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// The items of `v` (sorted by `key`) whose key equals `y`.
fn row_of<T>(v: &[T], y: i32, key: impl Fn(&T) -> i32) -> &[T] {
    let lo = v.partition_point(|e| key(e) < y);
    let hi = lo + v[lo..].partition_point(|e| key(e) <= y);
    &v[lo..hi]
}

/// `s` minus `cut`: zero, one or two pieces (half-open spans).
fn subtract(s: (i32, i32), cut: (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    let before = (s.0, s.1.min(cut.0));
    let after = (s.0.max(cut.1), s.1);
    [before, after].into_iter().filter(|(a, b)| a < b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: u32 = 0xffff_ffff;
    const BLACK: u32 = 0xff00_0000;

    fn bgra(r: u8, g: u8, b: u8) -> u32 {
        0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
    }

    /// Mix of `a` and `b`, `t` in 0..=1 towards `b`.
    fn mix(a: u32, b: u32, t: f32) -> u32 {
        let ch = |s: u32| {
            let (x, y) = (((a >> s) & 0xff) as f32, ((b >> s) & 0xff) as f32);
            ((x + (y - x) * t).round() as u32) << s
        };
        0xff00_0000 | ch(16) | ch(8) | ch(0)
    }

    struct Img {
        w: usize,
        h: usize,
        px: Vec<u32>,
    }

    impl Img {
        fn new(w: usize, h: usize, bg: u32) -> Self {
            Self { w, h, px: vec![bg; w * h] }
        }
        fn fill(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, c: u32) {
            for y in y0..y1 {
                self.px[y * self.w + x0..y * self.w + x1].fill(c);
            }
        }
        fn view(&self) -> PixelView<'_> {
            PixelView::new(&self.px, self.w, self.h, self.w).expect("view")
        }
        fn full(&self) -> RectI {
            RectI::new(0, 0, self.w as i32, self.h as i32)
        }
    }

    /// Full scan + raw surfaces.
    fn detect(img: &Img) -> Vec<Surface> {
        let mut m = SurfaceMap::new(img.w, img.h, VisionParams::default());
        assert!(m.rescan(&img.view(), &[img.full()]));
        m.raw_surfaces().to_vec()
    }

    #[test]
    fn thin_line_one_surface_at_top() {
        for width in 1..=5 {
            let mut img = Img::new(400, 200, WHITE);
            img.fill(50, 100, 350, 100 + width, BLACK);
            assert_eq!(detect(&img), vec![Surface::new(50, 350, 100)], "stroke width {width}");
        }
    }

    #[test]
    fn thick_bar_has_top_surface() {
        let mut img = Img::new(400, 200, WHITE);
        img.fill(20, 60, 300, 90, bgra(40, 90, 200));
        let s = detect(&img);
        assert_eq!(s.first(), Some(&Surface::new(20, 300, 60)));
        // Its underside is a boundary too (air = uniform bar), far enough not to merge.
        assert!(s.iter().all(|s| s.y == 60 || s.y == 90), "{s:?}");
    }

    #[test]
    fn short_runs_and_text_like_noise_are_rejected() {
        let mut img = Img::new(600, 120, WHITE);
        // "Words": glyph blobs 6 px wide, 3 px apart, varying heights.
        let mut x = 10;
        let mut k = 0usize;
        while x + 6 < 590 {
            let top = 40 + (k * 7 % 5);
            img.fill(x, top, x + 6, 52, BLACK);
            x += 9 + (k % 3) * 4;
            k += 1;
        }
        // A short (30 px) dash.
        img.fill(100, 90, 130, 92, BLACK);
        assert_eq!(detect(&img), vec![]);
    }

    #[test]
    fn small_gaps_are_bridged_large_ones_split() {
        let mut img = Img::new(400, 100, WHITE);
        img.fill(10, 50, 100, 52, BLACK);
        img.fill(102, 50, 200, 52, BLACK); // 2 px gap: bridged
        img.fill(210, 50, 300, 52, BLACK); // 10 px gap: split
        assert_eq!(detect(&img), vec![Surface::new(10, 200, 50), Surface::new(210, 300, 50)]);
    }

    #[test]
    fn gradients_are_rejected() {
        // Vertical gradient, steep enough that each row step alone exceeds the contrast.
        let mut img = Img::new(300, 120, WHITE);
        for y in 0..120 {
            let v = (y * 60).min(255) as u8;
            img.fill(0, y, 300, y + 1, bgra(v, v, v));
        }
        assert_eq!(detect(&img), vec![], "steep vertical gradient");
        // Gentle vertical gradient.
        for y in 0..120 {
            let v = (y * 2) as u8;
            img.fill(0, y, 300, y + 1, bgra(v, v, v));
        }
        assert_eq!(detect(&img), vec![], "gentle vertical gradient");
        // Horizontal gradient: no vertical steps at all.
        let mut img = Img::new(256, 60, WHITE);
        for y in 0..60 {
            for x in 0..256 {
                img.px[y * 256 + x] = bgra(x as u8, x as u8, x as u8);
            }
        }
        assert_eq!(detect(&img), vec![]);
    }

    #[test]
    fn anti_aliased_line_gives_one_surface() {
        for blend in [0.1, 0.3, 0.5, 0.7] {
            let mut img = Img::new(400, 100, WHITE);
            img.fill(40, 49, 360, 50, mix(WHITE, BLACK, blend));
            img.fill(40, 50, 360, 52, BLACK);
            img.fill(40, 52, 360, 53, mix(WHITE, BLACK, blend));
            let s = detect(&img);
            assert_eq!(s.len(), 1, "blend {blend}: {s:?}");
            assert!((49..=50).contains(&s[0].y) && s[0].x0 == 40 && s[0].x1 == 360, "blend {blend}: {s:?}");
        }
    }

    #[test]
    fn equal_luma_colours_still_contrast() {
        // Pure red vs a green of about the same luminance.
        let red = bgra(255, 0, 0);
        let green = bgra(0, 130, 0);
        let mut img = Img::new(300, 100, green);
        img.fill(20, 40, 280, 43, red);
        assert_eq!(detect(&img), vec![Surface::new(20, 280, 40)]);
    }

    #[test]
    fn air_must_be_uniform() {
        // Busy texture (checkerboard) directly above the line: not air.
        let mut img = Img::new(300, 100, WHITE);
        for y in 30..50 {
            for x in 0..300 {
                if (x + y) % 2 == 0 {
                    img.px[y * 300 + x] = BLACK;
                }
            }
        }
        img.fill(0, 50, 300, 53, bgra(200, 30, 30));
        assert!(detect(&img).iter().all(|s| s.y != 50));
    }

    #[test]
    fn incremental_rescan_keeps_surfaces_outside_dirty_rects() {
        let mut img = Img::new(500, 300, WHITE);
        img.fill(50, 50, 450, 52, BLACK);
        img.fill(50, 200, 450, 202, BLACK);
        let mut m = SurfaceMap::new(500, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        assert_eq!(m.raw_surfaces(), &[Surface::new(50, 450, 50), Surface::new(50, 450, 200)]);

        // Erase the lower line and draw a new one; only that region is dirty.
        img.fill(50, 200, 450, 202, WHITE);
        img.fill(100, 250, 300, 253, BLACK);
        m.rescan(&img.view(), &[RectI::new(40, 190, 460, 260)]);
        assert_eq!(m.raw_surfaces(), &[Surface::new(50, 450, 50), Surface::new(100, 300, 250)]);

        // A dirty rect over the middle of the upper line keeps it whole.
        m.rescan(&img.view(), &[RectI::new(200, 40, 220, 60)]);
        assert_eq!(m.raw_surfaces()[0], Surface::new(50, 450, 50));

        // Shortening it through a small dirty rect is seen.
        img.fill(400, 50, 450, 52, WHITE);
        m.rescan(&img.view(), &[RectI::new(400, 50, 450, 52)]);
        assert_eq!(m.raw_surfaces()[0], Surface::new(50, 400, 50));
    }

    #[test]
    fn dirty_rect_above_line_updates_air_dependency() {
        // The line itself is unchanged, but its air row becomes busy.
        let mut img = Img::new(300, 100, WHITE);
        img.fill(20, 50, 280, 52, BLACK);
        let mut m = SurfaceMap::new(300, 100, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        assert_eq!(m.raw_surfaces().len(), 1);
        for x in (20..280).step_by(2) {
            img.px[49 * 300 + x] = BLACK;
        }
        m.rescan(&img.view(), &[RectI::new(20, 49, 280, 50)]);
        assert!(m.raw_surfaces().iter().all(|s| s.y != 50), "{:?}", m.raw_surfaces());
    }

    #[test]
    fn hysteresis_delays_new_and_drops_removed_immediately() {
        let p = VisionParams::default();
        let mut img = Img::new(300, 100, WHITE);
        let mut m = SurfaceMap::new(300, 100, p.clone());
        m.rescan(&img.view(), &[img.full()]);
        assert!(!m.update(0.0));

        img.fill(20, 50, 280, 52, BLACK);
        m.rescan(&img.view(), &[RectI::new(20, 50, 280, 52)]);
        assert!(!m.update(1.0), "first sighting is pending");
        assert!(m.surfaces().is_empty());
        assert_eq!(m.next_deadline(), Some(1.0 + p.persist_secs));
        assert!(!m.update(1.03), "seen twice but too young");
        assert!(m.update(1.07), "persisted");
        assert_eq!(m.surfaces(), &[Surface::new(20, 280, 50)]);
        assert_eq!(m.next_deadline(), None);

        // Extending it: the old part stays, the new part waits.
        img.fill(280, 50, 290, 52, BLACK);
        m.rescan(&img.view(), &[RectI::new(280, 50, 290, 52)]);
        assert!(!m.update(2.0));
        assert_eq!(m.surfaces(), &[Surface::new(20, 280, 50)]);
        assert!(m.update(2.1));
        assert_eq!(m.surfaces(), &[Surface::new(20, 290, 50)]);

        // Removal is immediate.
        img.fill(20, 50, 290, 52, WHITE);
        m.rescan(&img.view(), &[RectI::new(20, 50, 290, 52)]);
        assert!(m.update(2.11));
        assert!(m.surfaces().is_empty());
    }

    #[test]
    fn hysteresis_ignores_flicker() {
        let mut img = Img::new(300, 100, WHITE);
        let mut m = SurfaceMap::new(300, 100, VisionParams::default());
        let dirty = [RectI::new(0, 0, 300, 100)];
        // A line that moves every frame (scrolling) is never reported.
        for (i, y) in (20..80).step_by(3).enumerate() {
            img.fill(0, 0, 300, 100, WHITE);
            img.fill(20, y, 280, y + 2, BLACK);
            m.rescan(&img.view(), &dirty);
            m.update(i as f64 * 0.033);
            assert!(m.surfaces().is_empty(), "frame {i}");
        }
    }

    #[test]
    fn rescan_rejects_wrong_size() {
        let img = Img::new(10, 10, WHITE);
        let mut m = SurfaceMap::new(20, 10, VisionParams::default());
        assert!(!m.rescan(&img.view(), &[img.full()]));
        assert!(PixelView::new(&img.px, 10, 11, 10).is_none());
        assert!(PixelView::new(&img.px, 10, 10, 9).is_none());
    }

    /// Release-mode timing of a full-frame scan of a busy 1920x1080 image.
    #[test]
    fn full_hd_rescan_is_fast() {
        let (w, h) = (1920, 1080);
        let mut img = Img::new(w, h, WHITE);
        let mut seed = 12345u32;
        for y in (0..h).step_by(37) {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let x0 = (seed >> 8) as usize % (w / 2);
            img.fill(x0, y, (x0 + 200 + (seed as usize % 600)).min(w), (y + 3).min(h), BLACK);
        }
        let mut m = SurfaceMap::new(w, h, VisionParams::default());
        let start = std::time::Instant::now();
        let n = 5;
        for _ in 0..n {
            m.rescan(&img.view(), &[img.full()]);
        }
        let per = start.elapsed() / n;
        assert!(!m.raw_surfaces().is_empty());
        // Generous bound for debug builds; see the vision_bench example for release numbers.
        assert!(per.as_millis() < 1000, "full rescan took {per:?}");
    }
}
