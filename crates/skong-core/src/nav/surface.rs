//! Walkable surfaces: terrain ledges (plus the bounds floor) split wherever a wall stands
//! on them, so walking along one surface never runs into a wall.
use crate::controller::{Character, Side, Support};
use crate::desktop::WindowId;
use crate::terrain::{Facing, Ledge, LedgeKind, Terrain, Wall};

/// Index into [`NavGraph::surfaces`](super::NavGraph::surfaces).
pub type SurfaceId = usize;

/// A stretch of ledge the knight can walk along without hitting a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub kind: LedgeKind,
    pub owner: Option<WindowId>,
    /// Feet-center range the knight can actually stand at (walls at the ends inset it).
    pub lo: f32,
    pub hi: f32,
    /// Extent of the underlying terrain ledge (segments of one ledge share it).
    ledge_x0: f32,
    ledge_x1: f32,
}

impl Surface {
    /// Feet-center x clamped to the standable range.
    pub fn clamp(&self, x: f32) -> f32 {
        x.clamp(self.lo, self.hi)
    }

    /// Is a feet center at `x` (at this height) standing on this surface? Segment ends that
    /// are also ledge ends extend by `half` (the body may overhang the ledge).
    pub fn claims(&self, x: f32, half: f32) -> bool {
        let lo = if self.x0 == self.ledge_x0 { self.x0 - half } else { self.x0 };
        let hi = if self.x1 == self.ledge_x1 { self.x1 + half } else { self.x1 };
        lo <= x && x < hi
    }

    /// Is the character standing on this surface?
    pub fn holds(&self, ch: &Character) -> bool {
        ch.is_grounded() && (ch.pos.y - self.y).abs() <= 1.0 && self.claims(ch.pos.x, ch.size.x * 0.5)
    }

    /// Is the end in `dir` also the end of the ledge (the feet may overhang it)? Otherwise it
    /// is a cut where a wall stands, and walking on crosses into the next segment.
    pub fn ledge_end(&self, dir: Side) -> bool {
        match dir {
            Side::Left => self.x0 == self.ledge_x0,
            Side::Right => self.x1 == self.ledge_x1,
        }
    }

    /// Is the character standing on the ledge this surface was cut from (any segment)?
    pub fn shares_ledge(&self, ch: &Character) -> bool {
        let half = ch.size.x * 0.5;
        ch.is_grounded()
            && (ch.pos.y - self.y).abs() <= 1.0
            && self.ledge_x0 - half <= ch.pos.x
            && ch.pos.x < self.ledge_x1 + half
    }

    fn is_cut_from(&self, s: &Support) -> bool {
        s.y == self.y && s.x0 == self.ledge_x0 && s.x1 == self.ledge_x1 && s.owner == self.owner && s.kind == self.kind
    }
}

/// The surface `ch` stands on, if grounded. Prefers the ledge in `ch.support`, falls back to
/// geometry (the support may still describe the previous terrain generation).
pub(super) fn surface_at(surfaces: &[Surface], ch: &Character) -> Option<SurfaceId> {
    let support = ch.support?;
    let half = ch.size.x * 0.5;
    let x = ch.pos.x;
    let by_support = surfaces.iter().position(|s| s.is_cut_from(&support) && s.claims(x, half));
    by_support.or_else(|| {
        surfaces.iter().position(|s| (ch.pos.y - s.y).abs() <= 1.0 && s.claims(x, half)).or_else(|| {
            // Nearest segment of the supporting ledge (feet just past a split point).
            surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| s.is_cut_from(&support))
                .min_by(|a, b| dist_to(a.1, x).total_cmp(&dist_to(b.1, x)))
                .map(|(i, _)| i)
        })
    })
}

fn dist_to(s: &Surface, x: f32) -> f32 {
    (s.x0 - x).max(x - s.x1).max(0.0)
}

/// Terrain walls plus the inward-facing bounds sides (as the controller sees them).
pub(super) fn all_walls(terrain: &Terrain) -> impl Iterator<Item = Wall> + '_ {
    let b = terrain.bounds;
    let sides = (b.width() > 0.0 && b.height() > 0.0).then_some(b).into_iter().flat_map(|b| {
        [
            Wall { x: b.left, y0: b.top, y1: b.bottom, face: Facing::Right, owner: None },
            Wall { x: b.right, y0: b.top, y1: b.bottom, face: Facing::Left, owner: None },
        ]
    });
    terrain.walls.iter().copied().chain(sides)
}

/// Terrain ledges plus the implicit bounds floor where no ground ledge lies above it
/// (elsewhere the ground always catches the knight first).
pub(super) fn all_ledges(terrain: &Terrain) -> Vec<Ledge> {
    let mut ledges = terrain.ledges.clone();
    let b = terrain.bounds;
    if b.width() > 0.0 && b.height() > 0.0 {
        let mut ground: Vec<(f32, f32)> =
            terrain.ledges.iter().filter(|l| l.kind == LedgeKind::Ground).map(|l| (l.x0, l.x1)).collect();
        ground.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = b.left;
        for (x0, x1) in ground.into_iter().chain([(b.right, b.right)]) {
            if x0 > x {
                // The controller's floor spans the whole bounds; `surface_at` matches these
                // stretches by geometry.
                ledges.push(Ledge { x0: x, x1: x0.min(b.right), y: b.bottom, kind: LedgeKind::Ground, owner: None });
            }
            x = x.max(x1);
        }
    }
    ledges
}

/// Split every ledge where walls cut through the band a standing body occupies.
///
/// `half`/`height`: body half width and height; `step`: step height (walls lower than this
/// above the feet are stepped over).
pub(super) fn extract(terrain: &Terrain, half: f32, height: f32, step: f32) -> Vec<Surface> {
    let walls: Vec<Wall> = all_walls(terrain).collect();
    let mut out = Vec::new();
    let mut cuts = Vec::new();
    let top = if terrain.bounds.height() > 0.0 { terrain.bounds.top } else { f32::NEG_INFINITY };
    for l in all_ledges(terrain) {
        if l.y - height < top {
            // The body would stick out of the desktop: nobody stands (or lands) here.
            continue;
        }
        let (band_top, band_bottom) = (l.y - height, l.y - step);
        let blocks = |w: &Wall| w.y0 < band_bottom && band_top < w.y1;
        cuts.clear();
        cuts.push(l.x0);
        cuts.extend(walls.iter().filter(|w| blocks(w) && w.x > l.x0 && w.x < l.x1).map(|w| w.x));
        cuts.push(l.x1);
        cuts.sort_by(f32::total_cmp);
        cuts.dedup();
        for pair in cuts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b - a < 1.0 {
                continue;
            }
            // Walls at (or within a body of) the ends stop the feet center short of them.
            let mut lo = a + 1.0;
            let mut hi = b - 1.0;
            for w in walls.iter().filter(|w| blocks(w)) {
                if w.face == Facing::Right && w.x >= a - half - 1.0 && w.x <= a + half {
                    lo = lo.max(w.x + half + 0.5);
                }
                if w.face == Facing::Left && w.x <= b + half + 1.0 && w.x >= b - half {
                    hi = hi.min(w.x - half - 0.5);
                }
            }
            if lo > hi {
                let mid = (a + b) * 0.5;
                (lo, hi) = (mid, mid);
            }
            out.push(Surface { x0: a, x1: b, y: l.y, kind: l.kind, owner: l.owner, lo, hi, ledge_x0: l.x0, ledge_x1: l.x1 });
        }
    }
    out
}
