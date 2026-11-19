//! Swept AABB queries against terrain segments.
//!
//! Motion is resolved one axis at a time (x, then y). Every query looks at the whole swept
//! interval, so nothing tunnels regardless of speed or `dt`.
//!
//! Rules (see `terrain` module docs):
//! * a [`Wall`] blocks only bodies that start on its face side and move into it;
//! * a [`Ledge`] blocks only bodies whose feet start at/above it and move down;
//! * edges of a window are ignored while the body overlaps that window's interior, except
//!   content ledges and walls ([`LedgeKind::Content`], [`WallKind::Content`]): content drawn
//!   inside a window is meant to be stood on and bumped into;
//! * `Terrain::bounds` adds an implicit ground floor and inward walls.
use super::Side;
use crate::desktop::WindowId;
use crate::geom::RectF;
use crate::terrain::{Facing, Ledge, LedgeKind, Terrain, Wall, WallKind};

/// Float tolerance in px: a body this far past a segment still counts as "at" it.
pub(super) const EPS: f32 = 0.5;
/// Max gap (px) between a body side and a wall for them to be "touching".
pub(super) const TOUCH: f32 = 1.0;

/// Terrain view for one step, with the window-overlap ignore rule pre-applied.
pub(super) struct Query<'a> {
    terrain: &'a Terrain,
    /// Body used for the ignore rule (shrunk by the caller's tolerance at the feet).
    probe: RectF,
    /// Height of a one-way ledge currently being dropped through.
    pub drop_y: Option<f32>,
}

impl<'a> Query<'a> {
    /// `feet_allowance`: the bottom this many px of `body` do not count for the ignore rule,
    /// so a window whose top is slightly above the feet can be stepped onto.
    pub fn new(terrain: &'a Terrain, body: RectF, feet_allowance: f32, drop_y: Option<f32>) -> Self {
        let probe = RectF::new(
            body.left + EPS,
            body.top + EPS,
            body.right - EPS,
            (body.bottom - feet_allowance - EPS).max(body.top + EPS),
        );
        Self { terrain, probe, drop_y }
    }

    /// Is this owner's window overlapping the body (so its edges are ignored)?
    fn ignores(&self, owner: Option<WindowId>) -> bool {
        owner.is_some_and(|id| self.terrain.windows.iter().any(|w| w.id == id && w.rect.overlaps(&self.probe)))
    }

    fn bounds(&self) -> Option<RectF> {
        let b = self.terrain.bounds;
        (b.width() > 0.0 && b.height() > 0.0).then_some(b)
    }

    /// Active ledges accepted by `keep`, plus the bounds floor if it is. `keep` (cheap
    /// geometry) runs before the ignore rule, which scans the windows.
    fn ledges<'b>(&'b self, keep: impl Fn(&Ledge) -> bool + 'b) -> impl Iterator<Item = Ledge> + 'b {
        let floor = self
            .bounds()
            .map(|b| Ledge { x0: b.left, x1: b.right, y: b.bottom, kind: LedgeKind::Ground, owner: None })
            .filter(&keep);
        let active = |l: &Ledge| (l.kind == LedgeKind::Content || !self.ignores(l.owner)) && !self.dropping_through(l);
        self.terrain.ledges.iter().copied().filter(move |l| keep(l) && active(l)).chain(floor)
    }

    fn dropping_through(&self, l: &Ledge) -> bool {
        l.kind.is_one_way() && self.drop_y.is_some_and(|d| (l.y - d).abs() < EPS)
    }

    /// Active walls accepted by `keep`, plus the inward-facing bounds sides that are (`keep`
    /// runs first, as for [`Self::ledges`]).
    fn walls<'b>(&'b self, keep: impl Fn(&Wall) -> bool + Copy + 'b) -> impl Iterator<Item = Wall> + 'b {
        let sides = self.bounds().into_iter().flat_map(|b| {
            [
                Wall { x: b.left, y0: b.top, y1: b.bottom, face: Facing::Right, owner: None, kind: WallKind::Monitor },
                Wall { x: b.right, y0: b.top, y1: b.bottom, face: Facing::Left, owner: None, kind: WallKind::Monitor },
            ]
        });
        let active = |w: &Wall| w.kind == WallKind::Content || !self.ignores(w.owner);
        self.terrain.walls.iter().copied().filter(move |w| keep(w) && active(w)).chain(sides.filter(move |w| keep(w)))
    }

    /// Horizontal sweep by `dx`. Returns the allowed displacement and whether a wall was hit.
    ///
    /// `pass_window_walls`: shadow dash (window and content walls don't block; monitor walls do).
    /// `pass_content`: stepping behind screen content (only content walls don't block).
    /// `foot_tol`: walls overlapping only the bottom `foot_tol` px of the body don't block
    /// (lets a grounded body step onto slightly higher ledges).
    pub fn sweep_x(&self, body: RectF, dx: f32, pass_window_walls: bool, pass_content: bool, foot_tol: f32) -> (f32, bool) {
        if dx == 0.0 {
            return (0.0, false);
        }
        let (blocking, edge) = if dx > 0.0 { (Facing::Left, body.right) } else { (Facing::Right, body.left) };
        // Must start on the face side (within tolerance) and be crossed by the sweep.
        let crossed = |w: &Wall, allowed: f32| {
            if dx > 0.0 { w.x >= edge - EPS && w.x < edge + allowed } else { w.x <= edge + EPS && w.x > edge + allowed }
        };
        let blocks = |w: &Wall| {
            w.face == blocking
                && !(pass_window_walls && w.kind != WallKind::Monitor)
                && !(pass_content && w.kind == WallKind::Content)
                && w.y0 < body.bottom - foot_tol
                && body.top < w.y1
                // `allowed` only shrinks from `dx`: a wall the full sweep misses stays missed.
                && crossed(w, dx)
        };
        let mut allowed = dx;
        let mut hit = false;
        for w in self.walls(blocks) {
            if crossed(&w, allowed) {
                allowed = w.x - edge;
                hit = true;
            }
        }
        (allowed, hit)
    }

    /// Downward sweep of the feet by `dy > 0`. Returns the first ledge landed on.
    /// Only ledges accepted by `solid` count (a diving body smashes through the others).
    pub fn sweep_down_where(&self, body: RectF, dy: f32, solid: impl Fn(&Ledge) -> bool) -> Option<Ledge> {
        let feet = body.bottom;
        self.ledges(|l| overlaps_x(l, body) && l.y >= feet - EPS && l.y <= feet + dy && solid(l)).min_by(|a, b| a.y.total_cmp(&b.y))
    }

    /// Highest ledge under the body with `feet - up <= y <= feet + down` accepted by `filter`.
    pub fn find_ledge(&self, body: RectF, up: f32, down: f32, filter: impl Fn(&Ledge) -> bool) -> Option<Ledge> {
        let feet = body.bottom;
        self.ledges(|l| overlaps_x(l, body) && l.y >= feet - up && l.y <= feet + down && filter(l)).min_by(|a, b| a.y.total_cmp(&b.y))
    }

    /// True when the body stands only on one-way ledges (at least one).
    pub fn can_drop_through(&self, body: RectF) -> bool {
        let mut any = false;
        for l in self.ledges(|l| overlaps_x(l, body) && (l.y - body.bottom).abs() <= EPS) {
            if !l.kind.is_one_way() {
                return false;
            }
            any = true;
        }
        any
    }

    /// A wall that blocks motion towards `side` and touches that side of the body
    /// (covering the body's vertical center).
    pub fn touching_wall(&self, body: RectF, side: Side) -> Option<Wall> {
        let mid = body.center().y;
        let (blocking, edge) = match side {
            Side::Right => (Facing::Left, body.right),
            Side::Left => (Facing::Right, body.left),
        };
        self.walls(|w| w.face == blocking && (w.x - edge).abs() <= TOUCH && w.y0 <= mid && mid < w.y1).next()
    }
}

fn overlaps_x(l: &Ledge, body: RectF) -> bool {
    l.x0 < body.right && body.left < l.x1
}
