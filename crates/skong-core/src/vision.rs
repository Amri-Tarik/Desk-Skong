//! Screen-content terrain: horizontal boundaries the knight can stand on (a line drawn
//! in Paint, a table separator, a toolbar edge...) and vertical ones that block him
//! (column borders, panel dividers, a vertical stroke).
//!
//! Pure and platform-independent: the platform layer hands in BGRA pixels plus the
//! rectangles that changed, this module keeps a persistent per-output [`SurfaceMap`].
//!
//! # Detection
//!
//! `d(a, b)` is the largest per-channel (B, G, R) absolute difference, so colours of
//! equal luminance still separate. Let `H[y][x] = d(P[y][x], P[y][x+1])` and
//! `V[y][x] = d(P[y][x], P[y-1][x])`. A boundary pixel needs a step `c >= edge_contrast`
//! into it from a two-pixel-deep **air** band on one side, whose own variation `noise`
//! (along the band and across its two rows/columns) satisfies `noise <= air_tolerance`
//! and `2 * noise <= c`. The relative test is what makes dark themes work (a `#30363d`
//! separator on `#0d1117` is a step of ~20 on a perfectly flat background) while
//! gradients and anti-aliasing fringes are rejected. The boundary must also be smooth
//! along its own length (variation to at least one neighbour along it at most half the
//! step, so line ends still count), which a drawn line
//! or a border is and the tops of a word are not.
//!
//! * **Ledge** (top edge) at `(x, y)`: `c = V[y][x]`, air = rows `y-1..=y-3` (three rows
//!   deep, so underlines and strike-throughs right under text don't count).
//! * **Left-facing wall** (solid starts at column `x`, air at `x-1`, `x-2`) and
//!   **right-facing wall** (solid ends at `x`, air at `x+1`, `x+2`; the wall line is at
//!   `x + 1`). A 1 px vertical line therefore yields both, blocking from both sides. A
//!   boundary between two flat areas (a panel edge) has air on both sides and blocks
//!   from both sides too.
//!
//! Boundary pixels are joined into runs (gaps up to `max_gap` / `max_wall_gap` px); runs shorter than
//! `min_run` (ledges) / `min_wall_run` (walls), or less than `min_fill` solid, are
//! dropped, which removes text, icons and avatars. Near-duplicates are merged: a ledge within `merge_rows` rows below a
//! kept one is cut where they overlap (a 1–5 px stroke gives one ledge, at its top),
//! and a wall within `merge_cols` columns *inside* a kept one of the same facing too.
//!
//! # Incremental updates and hysteresis
//!
//! [`SurfaceMap::scan`] recomputes the boundary masks only around dirty rects (grow
//! them by [`MARGIN`] before reading pixels) and [`SurfaceMap::finish`] re-joins runs
//! only for touched rows / columns. [`SurfaceMap::update`] then applies hysteresis: a
//! new ledge or wall (or a new stretch of one) is reported only after it was seen in two
//! updates at least `persist_secs` apart; anything that disappears is dropped
//! immediately.
use crate::geom::RectI;
use crate::terrain::Facing;

mod activity;

pub use activity::{Activity, ActivityGrid, ActivityParams, ActivitySpot};

/// Pixels needed around a dirty rect to re-detect everything it can affect.
pub const MARGIN: i32 = 4;

/// Detection tuning. See module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct VisionParams {
    /// Minimum colour step from the air band into the boundary (0–255).
    pub edge_contrast: u8,
    /// Maximum colour variation inside the air band (0–255); also at most half the step.
    pub air_tolerance: u8,
    /// Minimum ledge length in px.
    pub min_run: u32,
    /// Minimum wall length in px.
    pub min_wall_run: u32,
    /// Minimum fraction of a run's pixels that must be boundary pixels: real lines are
    /// solid, the bridged tops of a word are not.
    pub min_fill: f32,
    /// Gaps up to this many px inside a ledge run are bridged.
    pub max_gap: u32,
    /// Gaps up to this many px inside a wall run are bridged (a line crossing a wall
    /// spoils its air band for 3 rows).
    pub max_wall_gap: u32,
    /// A ledge this many rows (or fewer) below a kept one is merged into it.
    pub merge_rows: u32,
    /// A wall this many columns (or fewer) inside a kept one of the same facing is merged.
    pub merge_cols: u32,
    /// A new ledge / wall must persist this long (and over two updates) before being
    /// reported.
    pub persist_secs: f64,
}

impl Default for VisionParams {
    fn default() -> Self {
        Self {
            edge_contrast: 14,
            air_tolerance: 32,
            min_run: 48,
            min_wall_run: 48,
            max_gap: 2,
            max_wall_gap: 4,
            min_fill: 0.85,
            merge_rows: 6,
            merge_cols: 3,
            persist_secs: 0.06,
        }
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

/// A vertical content boundary: wall line at `x`, rows `y0..y1`, solid on the side
/// opposite to `face` (same convention as [`crate::terrain::Wall`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentWall {
    pub x: i32,
    pub y0: i32,
    pub y1: i32,
    pub face: Facing,
}

impl ContentWall {
    pub fn new(x: i32, y0: i32, y1: i32, face: Facing) -> Self {
        Self { x, y0, y1, face }
    }
    pub fn len(&self) -> i32 {
        self.y1 - self.y0
    }
    pub fn is_empty(&self) -> bool {
        self.y1 <= self.y0
    }
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self { x: self.x + dx, y0: self.y0 + dy, y1: self.y1 + dy, face: self.face }
    }
}

/// Borrowed 32-bit BGRA image (alpha ignored). `stride` is in pixels.
#[derive(Clone, Copy, Debug)]
pub struct PixelView<'a> {
    pub px: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    /// Map coordinates of the view's top-left pixel (a view may cover only a region).
    pub x0: usize,
    pub y0: usize,
}

impl<'a> PixelView<'a> {
    /// `None` when `px` is too short for the given geometry.
    pub fn new(px: &'a [u32], width: usize, height: usize, stride: usize) -> Option<Self> {
        let need = if height == 0 { 0 } else { (height - 1) * stride + width };
        (stride >= width && px.len() >= need).then_some(Self { px, width, height, stride, x0: 0, y0: 0 })
    }

    /// The same pixels placed at map position `(x0, y0)`.
    pub fn at(self, x0: usize, y0: usize) -> Self {
        Self { x0, y0, ..self }
    }

    /// Map row `y`, map columns `x0..x1` (must lie inside the view).
    fn seg(&self, y: usize, x0: usize, x1: usize) -> &'a [u32] {
        let r = (y - self.y0) * self.stride;
        &self.px[r + x0 - self.x0..r + x1 - self.x0]
    }
}

/// Largest per-channel absolute difference of two BGRA pixels (alpha ignored).
#[inline(always)]
fn cdiff(a: u32, b: u32) -> u8 {
    let d = |s: u32| ((a >> s) & 0xff).abs_diff((b >> s) & 0xff);
    d(0).max(d(8)).max(d(16)) as u8
}

/// One bit per pixel, row-major, rows padded to whole bytes (bit `x % 8` of byte `x / 8`).
#[derive(Clone, Debug)]
struct BitPlane {
    row_bytes: usize,
    bits: Vec<u8>,
}

impl BitPlane {
    fn new(width: usize, height: usize) -> Self {
        let row_bytes = width.div_ceil(8);
        Self { row_bytes, bits: vec![0; row_bytes * height] }
    }

    fn row(&self, y: usize) -> &[u8] {
        &self.bits[y * self.row_bytes..(y + 1) * self.row_bytes]
    }

    /// Store `vals` (0/1 bytes; `vals[i]` is column `ax0 + i`, `ax0 = x0 & !7`) for
    /// columns `x0..x1` of row `y`, leaving the other bits alone.
    fn store(&mut self, y: usize, x0: usize, x1: usize, vals: &[u8]) {
        let ax0 = x0 & !7;
        let row = &mut self.bits[y * self.row_bytes..(y + 1) * self.row_bytes];
        for (bi, byte) in row.iter_mut().enumerate().take(x1.div_ceil(8)).skip(ax0 / 8) {
            let base = bi * 8 - ax0;
            let mut chunk = [0u8; 8];
            let avail = vals.len().saturating_sub(base).min(8);
            chunk[..avail].copy_from_slice(&vals[base..base + avail]);
            // Eight 0/1 bytes → eight bits, byte i → bit i.
            let packed = (u64::from_le_bytes(chunk).wrapping_mul(0x0102_0408_1020_4080) >> 56) as u8;
            let lo = x0.max(bi * 8) - bi * 8;
            let hi = x1.min(bi * 8 + 8) - bi * 8;
            let sel = ((1u16 << hi) - (1u16 << lo)) as u8;
            *byte = (*byte & !sel) | (packed & sel);
        }
    }
}

/// Ring of per-row difference buffers used while scanning one rect.
struct Diffs {
    /// `H` of rows `r` at index `r % 8`, `V` likewise; columns `cx0..cx0 + n`.
    h: [Vec<u8>; 8],
    v: [Vec<u8>; 8],
    cx0: usize,
    n: usize,
}

impl Diffs {
    fn new(cx0: usize, n: usize) -> Self {
        Self { h: std::array::from_fn(|_| vec![0; n]), v: std::array::from_fn(|_| vec![0; n]), cx0, n }
    }

    #[inline(always)]
    fn compute_h(&mut self, img: &PixelView, y: usize) {
        let p = img.seg(y, self.cx0, self.cx0 + self.n);
        let n = self.n;
        let h = &mut self.h[y % 8][..n];
        let (a, b) = (&p[..n - 1], &p[1..n]);
        for i in 0..n - 1 {
            h[i] = cdiff(a[i], b[i]);
        }
        h[n - 1] = 0;
    }

    #[inline(always)]
    fn compute_v(&mut self, img: &PixelView, y: usize) {
        let (p, q) = (img.seg(y, self.cx0, self.cx0 + self.n), img.seg(y - 1, self.cx0, self.cx0 + self.n));
        let n = self.n;
        let (v, p, q) = (&mut self.v[y % 8][..n], &p[..n], &q[..n]);
        for i in 0..n {
            v[i] = cdiff(p[i], q[i]);
        }
    }
}

/// Boundary tests for one row: writes 0/1 into `ledge`, `left`, `right` (`len` = pixels,
/// index `i` ↔ difference index `i + 2`).
struct RowTest<'a> {
    h0: &'a [u8], // H[y]
    h1: &'a [u8], // H[y-1]
    h2: &'a [u8], // H[y-2]
    h3: &'a [u8], // H[y-3]
    v0: &'a [u8], // V[y]
    v1: &'a [u8], // V[y-1]
    v2: &'a [u8], // V[y-2]
    vn: &'a [u8], // V[y+1]
}

/// One rect to scan: rows `y0..y1`, columns `x0..x1` (map coords).
struct ScanJob<'a, 'b> {
    img: &'a PixelView<'b>,
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    t: u8,
    a: u8,
}

/// Difference rows, boundary tests and bit packing for one rect; everything inlined
/// so the AVX2 wrapper compiles all of it with AVX2.
#[inline(always)]
fn scan_rows(job: &ScanJob, planes: [&mut BitPlane; 3], scratch: &mut [Vec<u8>; 3]) {
    let ScanJob { img, x0, x1, y0, y1, t, a } = *job;
    let n = x1 - x0;
    let ax0 = x0 & !7;
    let mut d = Diffs::new(x0 - 2, n + 4);
    for y in y0 - 3..y0 {
        d.compute_h(img, y);
    }
    for y in y0 - 2..=y0 {
        d.compute_v(img, y);
    }
    let [ledge, left, right] = planes;
    for y in y0..y1 {
        d.compute_h(img, y);
        d.compute_v(img, y + 1);
        let r = RowTest {
            h0: &d.h[y % 8],
            h1: &d.h[(y + 7) % 8],
            h2: &d.h[(y + 6) % 8],
            h3: &d.h[(y + 5) % 8],
            v0: &d.v[y % 8],
            v1: &d.v[(y + 7) % 8],
            v2: &d.v[(y + 6) % 8],
            vn: &d.v[(y + 1) % 8],
        };
        let off = x0 - ax0;
        let [s0, s1, s2] = scratch;
        row_test(&r, (&mut s0[off..off + n], &mut s1[off..off + n], &mut s2[off..off + n]), t, a);
        ledge.store(y, x0, x1, s0);
        left.store(y, x0, x1, s1);
        right.store(y, x0, x1, s2);
    }
}

/// [`scan_rows`] compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn scan_rows_avx2(job: &ScanJob, planes: [&mut BitPlane; 3], scratch: &mut [Vec<u8>; 3]) {
    scan_rows(job, planes, scratch);
}

/// Boundary tests for one row: writes 0/1 into `ledge`, `left`, `right` (`len` = pixels,
/// index `i` ↔ difference index `i + 2`). Every neighbour is pre-sliced so the loop is
/// free of bounds checks and vectorizes (32 pixels per AVX2 instruction).
#[inline(always)]
fn row_test(r: &RowTest, out: (&mut [u8], &mut [u8], &mut [u8]), t: u8, a: u8) {
    let (ledge, left, right) = out;
    let n = ledge.len();
    let (left, right) = (&mut left[..n], &mut right[..n]);
    // `at(s, k)[i]` is `s[i + k]`, i.e. column offset `k - 2`.
    fn sl(s: &[u8], k: usize, n: usize) -> &[u8] {
        &s[k..k + n]
    }
    let at = |s, k| sl(s, k, n);
    let (h0m2, h0m1, h0c, h0p1) = (at(r.h0, 0), at(r.h0, 1), at(r.h0, 2), at(r.h0, 3));
    let (h1m1, h1c, h2m1, h2c, h3m1, h3c) = (at(r.h1, 1), at(r.h1, 2), at(r.h2, 1), at(r.h2, 2), at(r.h3, 1), at(r.h3, 2));
    let (v0m2, v0m1, v0c, v0p1, v0p2) = (at(r.v0, 0), at(r.v0, 1), at(r.v0, 2), at(r.v0, 3), at(r.v0, 4));
    let (v1c, v2c) = (at(r.v1, 2), at(r.v2, 2));
    let (vnm2, vnm1, vnc, vnp1, vnp2) = (at(r.vn, 0), at(r.vn, 1), at(r.vn, 2), at(r.vn, 3), at(r.vn, 4));
    let ok = |c: u8, noise: u8| ((c >= t) & (noise <= a) & (noise <= c / 2)) as u8;
    for i in 0..n {
        let ln = h1m1[i].max(h1c[i]).max(v1c[i]).max(h2m1[i]).max(h2c[i]).max(v2c[i]).max(h3m1[i]).max(h3c[i]);
        // The boundary itself must be smooth along its length (on at least one side, so
        // ends count; a line is, the tops of a word mostly aren't).
        ledge[i] = ok(v0c[i], ln) & (h0m1[i].min(h0c[i]) <= v0c[i] / 2) as u8;
        let along = v0c[i].min(vnc[i]);
        let lw = v0m1[i].max(vnm1[i]).max(h0m2[i]).max(v0m2[i]).max(vnm2[i]);
        left[i] = ok(h0m1[i], lw) & (along <= h0m1[i] / 2) as u8;
        let rw = v0p1[i].max(vnp1[i]).max(h0p1[i]).max(v0p2[i]).max(vnp2[i]);
        right[i] = ok(h0c[i], rw) & (along <= h0c[i] / 2) as u8;
    }
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

/// A segment on a lane (a row for ledges; a column + facing for walls).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Seg {
    lane: i64,
    a: i32,
    b: i32,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    s: Seg,
    since: f64,
    seen: u32,
}

/// Report-after-persisting, drop-immediately filter over lane segments.
#[derive(Clone, Debug, Default)]
struct Hysteresis {
    confirmed: Vec<Seg>,
    pending: Vec<Pending>,
}

impl Hysteresis {
    fn next_deadline(&self, persist: f64) -> Option<f64> {
        self.pending.iter().map(|p| p.since + persist).reduce(f64::min)
    }

    /// `raw` sorted by `(lane, a)`. Returns whether `confirmed` changed.
    fn update(&mut self, raw: &[Seg], now: f64, persist: f64, gap: i32) -> bool {
        let mut confirmed: Vec<Seg> = Vec::new();
        let mut pending: Vec<Pending> = Vec::new();
        for c in raw {
            let same = lane_of(&self.confirmed, c.lane, |k| k.lane);
            // Still-present parts of reported segments stay, immediately.
            confirmed.extend(same.iter().filter_map(|k| {
                let (a, b) = (k.a.max(c.a), k.b.min(c.b));
                (a < b).then_some(Seg { lane: c.lane, a, b })
            }));
            let mut fresh = vec![(c.a, c.b)];
            for k in same {
                fresh = fresh.into_iter().flat_map(|p| subtract(p, (k.a, k.b))).collect();
            }
            for (a, b) in fresh {
                let s = Seg { lane: c.lane, a, b };
                let prev = lane_of(&self.pending, s.lane, |p| p.s.lane).iter().filter(|p| p.s.a < s.b && s.a < p.s.b);
                let (since, seen) = prev.fold((now, 0), |(t, n), p| (t.min(p.since), n.max(p.seen)));
                let seen = seen + 1;
                if seen >= 2 && now - since >= persist - 1e-9 {
                    confirmed.push(s);
                } else {
                    pending.push(Pending { s, since, seen });
                }
            }
        }
        confirmed.sort_unstable();
        // Re-join pieces of one segment that were confirmed at different times.
        let mut joined: Vec<Seg> = Vec::with_capacity(confirmed.len());
        for s in confirmed {
            match joined.last_mut() {
                Some(l) if l.lane == s.lane && s.a <= l.b + gap => l.b = l.b.max(s.b),
                _ => joined.push(s),
            }
        }
        self.pending = pending;
        let changed = joined != self.confirmed;
        self.confirmed = joined;
        changed
    }
}

fn wall_seg(w: &ContentWall) -> Seg {
    Seg { lane: w.x as i64 * 2 + (w.face == Facing::Right) as i64, a: w.y0, b: w.y1 }
}

/// Persistent detector for one screen (output).
pub struct SurfaceMap {
    params: VisionParams,
    width: usize,
    height: usize,
    /// Ledge, left-facing and right-facing wall boundary pixels.
    ledge: BitPlane,
    left: BitPlane,
    right: BitPlane,
    /// Rows / columns scanned since the last [`Self::finish`].
    touched_rows: Vec<bool>,
    /// Rects (columns x0..x1, rows y0..y1) scanned since the last finish.
    scanned: Vec<(usize, usize, usize, usize)>,
    /// Regions whose masks are never rewritten (covered by our own windows).
    frozen: Vec<RectI>,
    /// Joined runs per row (`x0..x1`, length >= `min_run`).
    runs: Vec<Vec<(u32, u32)>>,
    /// Joined runs per column (`y0..y1`, length >= `min_wall_run`), left / right facing.
    col_runs: [Vec<Vec<(u32, u32)>>; 2],
    /// Before hysteresis, sorted.
    raw: Vec<Surface>,
    raw_walls: Vec<ContentWall>,
    /// Reported, sorted.
    surfaces: Vec<Surface>,
    walls: Vec<ContentWall>,
    hyst: Hysteresis,
    hyst_walls: Hysteresis,
    /// Scratch rows for one scanned row: ledge / left / right test results.
    scratch: [Vec<u8>; 3],
}

impl SurfaceMap {
    pub fn new(width: usize, height: usize, params: VisionParams) -> Self {
        Self {
            params,
            width,
            height,
            ledge: BitPlane::new(width, height),
            left: BitPlane::new(width, height),
            right: BitPlane::new(width, height),
            touched_rows: vec![false; height],
            scanned: Vec::new(),
            frozen: Vec::new(),
            runs: vec![Vec::new(); height],
            col_runs: [vec![Vec::new(); width], vec![Vec::new(); width]],
            raw: Vec::new(),
            raw_walls: Vec::new(),
            surfaces: Vec::new(),
            walls: Vec::new(),
            hyst: Hysteresis::default(),
            hyst_walls: Hysteresis::default(),
            scratch: std::array::from_fn(|_| vec![0; width + 16]),
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

    /// Ledges reported after hysteresis (local pixel coords), sorted by `(y, x0)`.
    pub fn surfaces(&self) -> &[Surface] {
        &self.surfaces
    }

    /// Walls reported after hysteresis (local pixel coords).
    pub fn walls(&self) -> &[ContentWall] {
        &self.walls
    }

    /// Ledges in the current image, before hysteresis.
    pub fn raw_surfaces(&self) -> &[Surface] {
        &self.raw
    }

    /// Walls in the current image, before hysteresis.
    pub fn raw_walls(&self) -> &[ContentWall] {
        &self.raw_walls
    }

    /// When the next pending ledge / wall may be promoted (call [`Self::update`] then
    /// even if nothing changed on screen).
    pub fn next_deadline(&self) -> Option<f64> {
        let p = self.params.persist_secs;
        match (self.hyst.next_deadline(p), self.hyst_walls.next_deadline(p)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Freeze the boundary masks inside `rects` (map coords): [`Self::scan`] leaves them
    /// untouched, so whatever is detected there stays exactly as it was while, e.g., one
    /// of our own windows covers it. Returns the previously frozen rects: rescan them so
    /// areas that were just uncovered are brought up to date.
    pub fn set_frozen(&mut self, rects: &[RectI]) -> Vec<RectI> {
        std::mem::replace(&mut self.frozen, rects.to_vec())
    }

    pub fn frozen(&self) -> &[RectI] {
        &self.frozen
    }

    /// [`Self::scan`] + [`Self::finish`]: re-detect around `dirty` (map coords; clipped).
    /// Returns `false` (and does nothing) if `img` does not fit in the map.
    pub fn rescan(&mut self, img: &PixelView, dirty: &[RectI]) -> bool {
        if !self.scan(img, dirty) {
            return false;
        }
        self.finish();
        true
    }

    /// Recompute the boundary masks that pixel changes inside `dirty` (map coords) can
    /// affect, as far as `img` provides the pixels: to cover a rect completely the view
    /// must extend [`MARGIN`] px beyond it on every side. Returns `false` (and does
    /// nothing) if the view lies outside the map. Call [`Self::finish`] afterwards.
    pub fn scan(&mut self, img: &PixelView, dirty: &[RectI]) -> bool {
        if img.x0 + img.width > self.width || img.y0 + img.height > self.height {
            return false;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let (vx0, vy0) = (img.x0 as i32, img.y0 as i32);
        let (vx1, vy1) = (vx0 + img.width as i32, vy0 + img.height as i32);
        for r in dirty {
            // A changed pixel affects ledges up to 3 rows below / 1 column aside and
            // walls up to 1 row above or below / 2 columns aside. Testing (x, y) reads
            // rows y-3..=y+1 and columns x-2..=x+2.
            let x0 = (r.left - 2).max(2).max(vx0 + 2);
            let x1 = (r.right + 2).min(w - 2).min(vx1 - 2);
            let y0 = (r.top - 1).max(3).max(vy0 + 3);
            let y1 = (r.bottom + 3).min(h - 1).min(vy1 - 1);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let mut parts = vec![RectI::new(x0, y0, x1, y1)];
            for f in &self.frozen {
                parts = parts.into_iter().flat_map(|p| rect_minus(p, f)).collect();
            }
            for p in parts {
                self.scan_rect(img, p.left as usize, p.right as usize, p.top as usize, p.bottom as usize);
            }
            self.touched_rows[y0 as usize..y1 as usize].iter_mut().for_each(|t| *t = true);
            self.scanned.push((x0 as usize, x1 as usize, y0 as usize, y1 as usize));
        }
        true
    }

    fn scan_rect(&mut self, img: &PixelView, x0: usize, x1: usize, y0: usize, y1: usize) {
        let job = ScanJob { img, x0, x1, y0, y1, t: self.params.edge_contrast, a: self.params.air_tolerance };
        let planes = [&mut self.ledge, &mut self.left, &mut self.right];
        #[cfg(target_arch = "x86_64")]
        if has_avx2() {
            // SAFETY: AVX2 support was verified at runtime by `has_avx2`.
            unsafe { scan_rows_avx2(&job, planes, &mut self.scratch) };
            return;
        }
        scan_rows(&job, planes, &mut self.scratch);
    }

    /// Re-join runs on rows / columns touched by [`Self::scan`] and re-merge.
    pub fn finish(&mut self) {
        for y in 0..self.height {
            if std::mem::take(&mut self.touched_rows[y]) {
                self.runs[y] = self.join_row(y);
            }
        }
        for (x0, x1, y0, y1) in std::mem::take(&mut self.scanned) {
            for side in 0..2 {
                if (y1 - y0) * 4 >= self.height {
                    // Tall rect: a byte-wise pass over the full height is cheaper.
                    for (i, r) in self.join_cols(side, x0, x1).into_iter().enumerate() {
                        self.col_runs[side][x0 + i] = r;
                    }
                } else {
                    for c in x0..x1 {
                        self.rejoin_col(side, c, y0, y1);
                    }
                }
            }
        }
        let rows = self.runs.iter().enumerate().map(|(y, r)| (y as i32, r.as_slice()));
        self.raw = self.merge_rows(rows);
        let left = self.col_runs[0].iter().enumerate().map(|(x, r)| (x as i32, r.as_slice()));
        let right = self.col_runs[1].iter().enumerate().map(|(x, r)| (x as i32, r.as_slice()));
        self.raw_walls = self.merge_cols(left, right);
    }

    /// Runs of ledge pixels on row `y`, gaps bridged, short ones dropped.
    fn join_row(&self, y: usize) -> Vec<(u32, u32)> {
        let row = self.ledge.row(y);
        let (gap, min) = (self.params.max_gap as usize, self.params.min_run as usize);
        let mut j = Joiner::new(gap, min, self.params.min_fill);
        let mut bi = 0;
        while bi < row.len() {
            // Skip 64 empty pixels at a time.
            if let Some(chunk) = row.get(bi..bi + 8)
                && chunk.iter().all(|&b| b == 0)
            {
                bi += 8;
                continue;
            }
            let mut v = row[bi];
            while v != 0 {
                j.push(bi * 8 + v.trailing_zeros() as usize);
                v &= v - 1;
            }
            bi += 1;
        }
        j.finish()
    }

    /// Wall runs (`side` 0 = left-facing, 1 = right-facing) of columns `c0..c1` over the
    /// whole height, reading the plane a byte (8 columns) at a time.
    fn join_cols(&self, side: usize, c0: usize, c1: usize) -> Vec<Vec<(u32, u32)>> {
        let plane = if side == 0 { &self.left } else { &self.right };
        let (gap, min, fill) = (self.params.max_wall_gap as usize, self.params.min_wall_run as usize, self.params.min_fill);
        let mut js: Vec<Joiner> = (c0..c1).map(|_| Joiner::new(gap, min, fill)).collect();
        for y in 0..self.height {
            let row = plane.row(y);
            for (bi, &bits) in row.iter().enumerate().take(c1.div_ceil(8)).skip(c0 / 8) {
                let mut v = bits;
                while v != 0 {
                    let x = bi * 8 + v.trailing_zeros() as usize;
                    v &= v - 1;
                    if (c0..c1).contains(&x) {
                        js[x - c0].push(y);
                    }
                }
            }
        }
        js.into_iter().map(Joiner::finish).collect()
    }

    /// Re-join the wall runs of column `c` (`side` 0 = left-facing, 1 = right-facing)
    /// after rows `y0..y1` were rescanned. Only the window between the nearest gaps
    /// wider than `max_wall_gap` above and below is re-joined: no run can cross such a
    /// gap, so runs outside the window are unaffected.
    fn rejoin_col(&mut self, side: usize, c: usize, y0: usize, y1: usize) {
        let plane = if side == 0 { &self.left } else { &self.right };
        let bit = |y: usize| plane.bits[y * plane.row_bytes + c / 8] >> (c % 8) & 1 != 0;
        let gap = self.params.max_wall_gap as usize;
        let (mut w0, mut zeros, mut y) = (y0, 0, y0);
        while y > 0 && zeros <= gap {
            y -= 1;
            if bit(y) {
                (w0, zeros) = (y, 0);
            } else {
                zeros += 1;
            }
        }
        let (mut w1, mut zeros, mut y) = (y1, 0, y1);
        while y < self.height && zeros <= gap {
            if bit(y) {
                (w1, zeros) = (y + 1, 0);
            } else {
                zeros += 1;
            }
            y += 1;
        }
        let mut j = Joiner::new(gap, self.params.min_wall_run as usize, self.params.min_fill);
        for y in w0..w1 {
            if bit(y) {
                j.push(y);
            }
        }
        let runs = &mut self.col_runs[side][c];
        runs.retain(|&(a, b)| b as usize <= w0 || a as usize >= w1);
        runs.extend(j.finish());
        runs.sort_unstable();
    }

    /// Keep the topmost of near-duplicate rows: a run is cut where a kept surface lies
    /// at most `merge_rows` rows above it. `rows`: `(y, runs)` in increasing `y`.
    fn merge_rows<'r>(&self, rows: impl Iterator<Item = (i32, &'r [(u32, u32)])>) -> Vec<Surface> {
        merge_lanes(rows, self.params.merge_rows as i32, self.params.min_run as i32).into_iter().map(|(y, a, b)| Surface::new(a, b, y)).collect()
    }

    /// Keep the outermost of near-duplicate columns of the same facing (leftmost for
    /// left-facing walls, rightmost for right-facing ones). `left` / `right`: `(column,
    /// runs)` in increasing column order.
    fn merge_cols<'r>(&self, left: impl Iterator<Item = (i32, &'r [(u32, u32)])>, right: impl DoubleEndedIterator<Item = (i32, &'r [(u32, u32)])>) -> Vec<ContentWall> {
        let (merge, min) = (self.params.merge_cols as i32, self.params.min_wall_run as i32);
        let mut out: Vec<ContentWall> = merge_lanes(left, merge, min).into_iter().map(|(x, a, b)| ContentWall::new(x, a, b, Facing::Left)).collect();
        // Right-facing: scan right to left (mirror the column index), wall line at x + 1.
        let w = self.width as i32;
        let mirrored = right.rev().map(|(x, r)| (w - 1 - x, r));
        out.extend(merge_lanes(mirrored, merge, min).into_iter().map(|(m, a, b)| ContentWall::new(w - 1 - m + 1, a, b, Facing::Right)));
        out.sort_unstable_by_key(wall_seg);
        out
    }

    /// Hysteresis step at time `now` (seconds, monotonic). Returns `true` when
    /// [`Self::surfaces`] or [`Self::walls`] changed.
    ///
    /// Hysteresis runs on the *unmerged* runs and near-duplicates are merged afterwards,
    /// so a reported ledge never disappears just because a new (still pending) line
    /// showed up a few rows above it: it stays until its own pixels are gone.
    pub fn update(&mut self, now: f64) -> bool {
        let p = self.params.persist_secs;
        let raw_rows: Vec<Seg> = self
            .runs
            .iter()
            .enumerate()
            .flat_map(|(y, r)| r.iter().map(move |&(a, b)| Seg { lane: y as i64, a: a as i32, b: b as i32 }))
            .collect();
        let mut raw_cols: Vec<Seg> = (0..2)
            .flat_map(|side| {
                self.col_runs[side].iter().enumerate().flat_map(move |(c, r)| r.iter().map(move |&(a, b)| Seg { lane: c as i64 * 2 + side as i64, a: a as i32, b: b as i32 }))
            })
            .collect();
        raw_cols.sort_unstable();
        let rows_changed = self.hyst.update(&raw_rows, now, p, self.params.max_gap as i32);
        let cols_changed = self.hyst_walls.update(&raw_cols, now, p, self.params.max_wall_gap as i32);
        let mut changed = false;
        if rows_changed {
            let lanes = group_lanes(&self.hyst.confirmed, |l| Some(l as i32));
            let surfaces = self.merge_rows(lanes.iter().map(|(l, r)| (*l, r.as_slice())));
            changed |= surfaces != self.surfaces;
            self.surfaces = surfaces;
        }
        if cols_changed {
            let left = group_lanes(&self.hyst_walls.confirmed, |l| (l % 2 == 0).then_some((l / 2) as i32));
            let right = group_lanes(&self.hyst_walls.confirmed, |l| (l % 2 == 1).then_some((l / 2) as i32));
            let walls = self.merge_cols(left.iter().map(|(l, r)| (*l, r.as_slice())), right.iter().map(|(l, r)| (*l, r.as_slice())));
            changed |= walls != self.walls;
            self.walls = walls;
        }
        changed
    }
}

/// Confirmed segments (sorted by lane) grouped into `(index, runs)` for the lanes `pick`
/// maps to an index (increasing).
fn group_lanes(segs: &[Seg], pick: impl Fn(i64) -> Option<i32>) -> Vec<(i32, Vec<(u32, u32)>)> {
    let mut out: Vec<(i32, Vec<(u32, u32)>)> = Vec::new();
    for s in segs {
        let Some(i) = pick(s.lane) else { continue };
        match out.last_mut() {
            Some((l, runs)) if *l == i => runs.push((s.a as u32, s.b as u32)),
            _ => out.push((i, vec![(s.a as u32, s.b as u32)])),
        }
    }
    out
}

/// Joins increasing positions into runs, bridging gaps up to `gap` and keeping runs of
/// at least `min` that are at least `fill` solid.
struct Joiner {
    gap: usize,
    min: usize,
    fill: f32,
    /// Current run `(start, end, pixels)`.
    cur: Option<(usize, usize, usize)>,
    out: Vec<(u32, u32)>,
}

impl Joiner {
    fn new(gap: usize, min: usize, fill: f32) -> Self {
        Self { gap, min, fill, cur: None, out: Vec::new() }
    }

    fn close(&mut self, (s, e, n): (usize, usize, usize)) {
        if e - s >= self.min && n as f32 >= self.fill * (e - s) as f32 {
            self.out.push((s as u32, e as u32));
        }
    }

    fn push(&mut self, p: usize) {
        self.cur = match self.cur {
            Some((s, e, n)) if p <= e + self.gap => Some((s, p + 1, n + 1)),
            Some(run) => {
                self.close(run);
                Some((p, p + 1, 1))
            }
            None => Some((p, p + 1, 1)),
        };
    }

    fn finish(mut self) -> Vec<(u32, u32)> {
        if let Some(run) = self.cur.take() {
            self.close(run);
        }
        self.out
    }
}

/// Lanes in increasing order with their runs; a run is cut where a kept run lies at
/// most `merge` lanes before it. Returns `(lane, a, b)` pieces of length >= `min`.
fn merge_lanes<'a>(lanes: impl Iterator<Item = (i32, &'a [(u32, u32)])>, merge: i32, min: i32) -> Vec<(i32, i32, i32)> {
    let mut out: Vec<(i32, i32, i32)> = Vec::new();
    let mut window_start = 0;
    for (lane, runs) in lanes {
        if runs.is_empty() {
            continue;
        }
        while window_start < out.len() && out[window_start].0 < lane - merge {
            window_start += 1;
        }
        let lane_start = out.len();
        for &(a, b) in runs {
            let mut pieces = vec![(a as i32, b as i32)];
            for k in &out[window_start..lane_start] {
                pieces = pieces.into_iter().flat_map(|p| subtract(p, (k.1, k.2))).collect();
            }
            out.extend(pieces.into_iter().filter(|(a, b)| b - a >= min).map(|(a, b)| (lane, a, b)));
        }
    }
    out
}

/// The items of `v` (sorted by `key`) whose key equals `lane`.
fn lane_of<T>(v: &[T], lane: i64, key: impl Fn(&T) -> i64) -> &[T] {
    let lo = v.partition_point(|e| key(e) < lane);
    let hi = lo + v[lo..].partition_point(|e| key(e) <= lane);
    &v[lo..hi]
}

/// `r` minus `cut`: up to four rects.
fn rect_minus(r: RectI, cut: &RectI) -> Vec<RectI> {
    let i = r.intersect(cut);
    if i.is_empty() {
        return vec![r];
    }
    [
        RectI::new(r.left, r.top, r.right, i.top),
        RectI::new(r.left, i.bottom, r.right, r.bottom),
        RectI::new(r.left, i.top, i.left, i.bottom),
        RectI::new(i.right, i.top, r.right, i.bottom),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect()
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


    /// Copy of `r` grown by [`MARGIN`] px (clipped), as a separate buffer placed at its position:
    /// what the capture layer reads back from the GPU for one dirty rect.
    fn region(img: &Img, r: RectI) -> (Vec<u32>, RectI) {
        let g = RectI::new(r.left - MARGIN, r.top - MARGIN, r.right + MARGIN, r.bottom + MARGIN).intersect(&img.full());
        let mut buf = Vec::new();
        for y in g.top..g.bottom {
            let row = y as usize * img.w;
            buf.extend_from_slice(&img.px[row + g.left as usize..row + g.right as usize]);
        }
        (buf, g)
    }

    #[test]
    fn region_views_match_full_rescans() {
        let mut img = Img::new(500, 300, WHITE);
        img.fill(50, 50, 450, 52, BLACK);
        let mut m = SurfaceMap::new(500, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);

        // Change two regions, rescan each from its own small buffer.
        img.fill(100, 120, 400, 123, bgra(200, 20, 20));
        img.fill(0, 248, 500, 300, BLACK);
        img.fill(300, 50, 450, 52, WHITE);
        let dirty = [RectI::new(100, 120, 400, 123), RectI::new(0, 248, 500, 300), RectI::new(300, 50, 450, 52)];
        for r in dirty {
            let (buf, g) = region(&img, r);
            let view = PixelView::new(&buf, g.width() as usize, g.height() as usize, g.width() as usize).expect("view");
            assert!(m.scan(&view.at(g.left as usize, g.top as usize), &[r]));
        }
        m.finish();
        let mut full = SurfaceMap::new(500, 300, VisionParams::default());
        full.rescan(&img.view(), &[img.full()]);
        assert_eq!(m.raw_surfaces(), full.raw_surfaces());
        assert_eq!(m.raw_surfaces().len(), 3, "{:?}", m.raw_surfaces());
    }

    fn hex(rgb: u32) -> u32 {
        0xff00_0000 | rgb
    }

    /// Dark-theme list: a box with 1 px low-contrast separators (#30363d on #0d1117),
    /// bold text, a dotted underline and icons in every row.
    fn dark_table() -> Img {
        let (bg, line, text) = (hex(0x0d1117), hex(0x30363d), hex(0xc9d1d9));
        let mut img = Img::new(900, 560, bg);
        // Box outline.
        img.fill(100, 40, 800, 41, line);
        img.fill(100, 40, 101, 540, line);
        img.fill(799, 40, 800, 540, line);
        img.fill(100, 539, 800, 540, line);
        for row in 0..7 {
            let top = 40 + row * 70;
            if row > 0 {
                img.fill(101, top, 799, top + 1, line); // separator
            }
            // A line of bold glyphs (dense, 1 px apart) with uneven tops 22 px below.
            let mut x = 140;
            let mut k = 0;
            while x < 600 {
                let w = 5 + k % 3;
                let t = top + 22 + (k * 5 % 3);
                img.fill(x, t, x + w, top + 33, text);
                x += w + 1 + (k % 2);
                k += 1;
            }
            // Dotted underline 3 px under the text, then an icon.
            for x in (140..260).step_by(2) {
                img.fill(x, top + 36, x + 1, top + 37, text);
            }
            img.fill(110, top + 25, 124, top + 39, hex(0x8b949e));
        }
        img
    }

    #[test]
    fn dark_theme_separators_found_text_ignored() {
        let img = dark_table();
        let mut m = SurfaceMap::new(img.w, img.h, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        let ys: Vec<i32> = m.raw_surfaces().iter().map(|s| s.y).collect();
        // Box top, 6 separators, box bottom; nothing from text, underlines or icons.
        assert_eq!(ys, vec![40, 110, 180, 250, 320, 390, 460, 539], "{:?}", m.raw_surfaces());
        assert!(m.raw_surfaces()[1..7].iter().all(|s| s.x0 <= 102 && s.x1 >= 798), "{:?}", m.raw_surfaces());
        // The box sides are walls on both faces, continuous across the separators.
        let walls = m.raw_walls();
        for (x, face) in [(100, Facing::Left), (101, Facing::Right), (799, Facing::Left), (800, Facing::Right)] {
            assert!(walls.iter().any(|w| w.x == x && w.face == face && w.y0 <= 45 && w.y1 >= 535), "wall {x} {face:?} in {walls:?}");
        }
        assert_eq!(walls.len(), 4, "text stems must not make walls: {walls:?}");
    }

    #[test]
    fn vertical_line_blocks_from_both_sides() {
        let mut img = Img::new(300, 300, WHITE);
        img.fill(150, 40, 152, 260, BLACK);
        let mut m = SurfaceMap::new(300, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        assert_eq!(m.raw_walls(), &[ContentWall::new(150, 40, 260, Facing::Left), ContentWall::new(152, 40, 260, Facing::Right)]);
        assert!(m.raw_surfaces().is_empty(), "a 2 px wide top is too short for a ledge");
    }

    #[test]
    fn panel_boundaries_block_from_both_sides() {
        let mut img = Img::new(400, 300, WHITE);
        img.fill(100, 80, 300, 280, hex(0x335577));
        let mut m = SurfaceMap::new(400, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        assert_eq!(m.raw_surfaces()[0], Surface::new(100, 300, 80));
        // Both sides of each boundary are flat (air), so each blocks from both sides.
        assert_eq!(
            m.raw_walls(),
            &[
                ContentWall::new(100, 80, 280, Facing::Left),
                ContentWall::new(100, 81, 279, Facing::Right),
                ContentWall::new(300, 81, 279, Facing::Left),
                ContentWall::new(300, 80, 280, Facing::Right),
            ]
        );
    }

    #[test]
    fn short_vertical_strokes_are_not_walls() {
        let mut img = Img::new(300, 200, WHITE);
        for x in (20..280).step_by(9) {
            img.fill(x, 50, x + 2, 50 + 14 + (x % 20), BLACK); // glyph stems
        }
        let mut m = SurfaceMap::new(300, 200, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        assert!(m.raw_walls().is_empty(), "{:?}", m.raw_walls());
    }

    #[test]
    fn walls_have_hysteresis_and_incremental_updates() {
        let mut img = Img::new(300, 300, WHITE);
        let mut m = SurfaceMap::new(300, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.0);
        img.fill(100, 20, 103, 280, BLACK);
        m.rescan(&img.view(), &[RectI::new(100, 20, 103, 280)]);
        assert!(!m.update(1.0));
        assert!(m.walls().is_empty());
        assert!(m.update(1.1));
        assert_eq!(m.walls().len(), 2);
        // Erase the lower half through a small dirty rect: the walls shrink immediately.
        img.fill(100, 150, 103, 280, WHITE);
        m.rescan(&img.view(), &[RectI::new(100, 150, 103, 280)]);
        assert!(m.update(1.11));
        assert!(m.walls().iter().all(|w| w.y0 == 20 && w.y1 == 150), "{:?}", m.walls());
    }

    /// Paint a knight-like blob over `r` (what capture would show if our sprite window
    /// were visible to it).
    fn paint_sprite(img: &mut Img, r: RectI, frame: usize) {
        for y in r.top..r.bottom {
            for x in r.left..r.right {
                let v = ((x * 7 + y * 13 + frame as i32 * 5) % 97) as u8;
                img.px[y as usize * img.w + x as usize] = bgra(v, v / 2, 255 - v);
            }
        }
    }

    #[test]
    fn frozen_region_keeps_its_ledge_under_our_own_window() {
        let mut img = Img::new(500, 300, WHITE);
        img.fill(50, 150, 450, 152, BLACK);
        let mut m = SurfaceMap::new(500, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.0);
        m.update(0.1);
        let ledge = [Surface::new(50, 450, 150)];
        assert_eq!(m.surfaces(), &ledge);

        // The knight sits on it for 30 s; worst case, capture sees his (animated) sprite.
        let knight = RectI::new(200, 90, 260, 175);
        assert!(m.set_frozen(&[knight]).is_empty());
        let mut t = 0.1;
        for frame in 0..900 {
            paint_sprite(&mut img, knight, frame);
            m.rescan(&img.view(), &[knight]);
            t += 1.0 / 30.0;
            assert!(!m.update(t), "frame {frame}: terrain changed under the knight");
            assert_eq!(m.surfaces(), &ledge);
        }

        // He leaves: the area is rescanned once with what is really there.
        img.fill(200, 90, 260, 175, WHITE);
        img.fill(200, 150, 260, 152, BLACK);
        let old = m.set_frozen(&[]);
        m.rescan(&img.view(), &old);
        assert!(!m.update(t + 0.1));
        assert_eq!(m.surfaces(), &ledge);
    }

    #[test]
    fn without_freezing_a_visible_sprite_would_cut_the_ledge() {
        let mut img = Img::new(500, 300, WHITE);
        img.fill(50, 150, 450, 152, BLACK);
        let mut m = SurfaceMap::new(500, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.0);
        m.update(0.1);
        let knight = RectI::new(200, 90, 260, 175);
        paint_sprite(&mut img, knight, 0);
        m.rescan(&img.view(), &[knight]);
        assert!(m.update(0.2));
        assert!(m.surfaces().iter().all(|s| s.x1 <= 200 || s.x0 >= 260), "{:?}", m.surfaces());
    }

    #[test]
    fn content_changing_under_a_frozen_region_is_picked_up_after_unfreezing() {
        let mut img = Img::new(500, 300, WHITE);
        img.fill(50, 150, 450, 152, BLACK);
        let mut m = SurfaceMap::new(500, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.0);
        m.update(0.1);
        let knight = RectI::new(200, 90, 260, 175);
        m.set_frozen(&[knight]);
        img.fill(0, 150, 500, 152, WHITE); // the line is erased (e.g. scrolled away)
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.2);
        // Outside the frozen part it is gone at once; under the knight it is kept.
        assert_eq!(m.surfaces(), &[Surface::new(200, 260, 150)]);
        let old = m.set_frozen(&[]);
        m.rescan(&img.view(), &old);
        assert!(m.update(0.3));
        assert!(m.surfaces().is_empty() && m.raw_surfaces().is_empty());
    }

    #[test]
    fn rescanning_a_stationary_ledge_never_drops_or_moves_it() {
        let mut img = Img::new(600, 300, WHITE);
        img.fill(50, 200, 550, 202, BLACK);
        let mut m = SurfaceMap::new(600, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.0);
        m.update(0.1);
        let ledge = [Surface::new(50, 550, 200)];
        // Something animates right above it (a spinner 20 px up); its region, which
        // includes part of the ledge, is rescanned every frame.
        let region = RectI::new(250, 150, 350, 210);
        for frame in 0..300 {
            img.fill(280, 170, 300, 182, if frame % 2 == 0 { BLACK } else { WHITE });
            m.rescan(&img.view(), &[region]);
            m.update(0.1 + frame as f64 / 30.0);
            assert_eq!(m.surfaces(), &ledge, "frame {frame}");
        }
    }

    #[test]
    fn a_new_line_just_above_never_makes_the_ledge_blink() {
        let mut img = Img::new(600, 300, WHITE);
        img.fill(50, 200, 550, 202, BLACK);
        let mut m = SurfaceMap::new(600, 300, VisionParams::default());
        m.rescan(&img.view(), &[img.full()]);
        m.update(0.0);
        m.update(0.1);
        // A second line appears 4 rows above (a selection outline, a focus ring...).
        img.fill(50, 196, 550, 197, hex(0x3070ff));
        let dirty = [RectI::new(50, 196, 550, 197)];
        let mut t = 0.1;
        for _ in 0..10 {
            m.rescan(&img.view(), &dirty);
            t += 1.0 / 30.0;
            m.update(t);
            let s = m.surfaces();
            assert_eq!(s.len(), 1, "{s:?}");
            assert!(s[0].x0 <= 50 && s[0].x1 >= 550 && (196..=200).contains(&s[0].y), "{s:?}");
        }
        // Once confirmed, the topmost of the two wins (a 4 px step, within the knight's
        // step height).
        assert_eq!(m.surfaces(), &[Surface::new(50, 550, 196)]);
    }
    #[test]
    fn rescan_rejects_wrong_size() {
        let img = Img::new(10, 10, WHITE);
        let mut m = SurfaceMap::new(5, 10, VisionParams::default());
        assert!(!m.rescan(&img.view(), &[img.full()]));
        let mut m = SurfaceMap::new(20, 20, VisionParams::default());
        assert!(!m.rescan(&img.view().at(15, 0), &[img.full()]));
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
