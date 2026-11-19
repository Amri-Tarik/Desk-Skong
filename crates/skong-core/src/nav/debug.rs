//! Geometry for visualising a [`NavGraph`] (debug overlay): standable spans of surfaces
//! and approximate trajectories of edges.
//!
//! Trajectories are a cheap quadratic curve per edge (take-off on the source surface →
//! land point) shaped by the move kind — jumps arc above both ends, drops and dashes
//! leave horizontally then fall. Climbs zig-zag up the wall(s) they actually use. They are
//! illustrations, not re-simulations.
use super::{Climb, Edge, MoveKind, NavGraph, Route};
use crate::geom::Vec2;

/// Segments per trajectory.
const STEPS: usize = 10;
/// Climb drawings: height per drawn hop, sideways kick of a single-wall climb, hop cap.
const CLIMB_HOP_PX: f32 = 120.0;
const CLIMB_KICK_PX: f32 = 28.0;
const MAX_HOPS: usize = 16;

/// A short polyline in screen coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugPath {
    pub kind: MoveKind,
    pub points: Vec<Vec2>,
    /// Part of the current route.
    pub highlighted: bool,
    /// A wall climb: after the take-off point, `points` alternate wall contact / kick-out up
    /// the wall(s) used ([`DebugPath::wall_contacts`]), then reach the landing.
    pub climb: bool,
}

impl DebugPath {
    /// Points where a climb touches its wall (empty for other moves).
    pub fn wall_contacts(&self) -> impl Iterator<Item = Vec2> + '_ {
        let n = if self.climb { self.points.len().saturating_sub(1) } else { 0 };
        self.points.iter().take(n).skip(1).step_by(2).copied()
    }
}

/// Feet range a knight can stand on (`lo..hi` at `y`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugSpan {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
}

/// Standable spans of every surface.
pub fn debug_spans(graph: &NavGraph) -> Vec<DebugSpan> {
    graph.surfaces().iter().map(|s| DebugSpan { x0: s.lo, x1: s.hi.max(s.lo + 1.0), y: s.y }).collect()
}

/// Every edge of `graph`, then the edges of `route` (highlighted, drawn last = on top).
pub fn debug_paths(graph: &NavGraph, route: Option<&Route>) -> Vec<DebugPath> {
    let surfaces = graph.surfaces();
    let mut out: Vec<DebugPath> =
        graph.edges().iter().filter_map(|e| surfaces.get(e.from).map(|from| edge_path(e, from.y, false))).collect();
    if let Some(r) = route {
        out.extend(route_paths(r));
    }
    out
}

/// Only the route's edges (highlighted). Valid even if the graph has changed since.
pub fn route_paths(route: &Route) -> Vec<DebugPath> {
    route.steps.iter().map(|s| edge_path(&s.edge, s.from.y, true)).collect()
}

/// Approximate trajectory of one edge taking off at height `from_y`.
pub fn edge_path(edge: &Edge, from_y: f32, highlighted: bool) -> DebugPath {
    let p0 = Vec2::new(edge.takeoff, from_y);
    if let Some(c) = edge.climb {
        return DebugPath { kind: edge.kind, points: climb_points(p0, edge.land, &c), highlighted, climb: true };
    }
    let p2 = edge.land;
    let mid = (p0 + p2) * 0.5;
    let dx = (p2.x - p0.x).abs();
    let falls = p2.y > p0.y + 2.0;
    let arc = |rise: f32| {
        // A quadratic Bézier peaks halfway between the chord midpoint and the control point.
        let top = p0.y.min(p2.y) - rise - 0.1 * dx;
        Vec2::new(mid.x, 2.0 * top - mid.y)
    };
    let control = match edge.kind {
        MoveKind::Walk => mid,
        MoveKind::Drop | MoveKind::DropThrough => Vec2::new(p2.x, p0.y),
        MoveKind::Jump => arc(40.0),
        MoveKind::DoubleJump => arc(110.0),
        MoveKind::WallJump => arc(80.0),
        MoveKind::Dash | MoveKind::ShadowDash | MoveKind::Superdash if falls => Vec2::new(p2.x, p0.y),
        MoveKind::Dash | MoveKind::ShadowDash | MoveKind::Superdash => mid,
    };
    let points = (0..=STEPS)
        .map(|i| {
            let t = i as f32 / STEPS as f32;
            let u = 1.0 - t;
            p0 * (u * u) + control * (2.0 * u * t) + p2 * (t * t)
        })
        .collect();
    DebugPath { kind: edge.kind, points, highlighted, climb: false }
}

/// Take-off, then wall contact / kick-out pairs up the climbed wall (bouncing to the facing
/// wall of a shaft), from where the wall starts (or the take-off height) to its top (or the
/// landing height), then the landing.
fn climb_points(p0: Vec2, land: Vec2, c: &Climb) -> Vec<Vec2> {
    let w = c.wall;
    let toward = if w.x >= p0.x { 1.0 } else { -1.0 };
    let out_x = c.shaft.map_or(w.x - toward * CLIMB_KICK_PX, |o| o.x);
    let bottom = w.y1.min(p0.y);
    let top = w.y0.max(land.y).min(bottom);
    let hops = (((bottom - top) / CLIMB_HOP_PX).ceil() as usize).clamp(1, MAX_HOPS);
    let step = (bottom - top) / hops as f32;
    let mut points = Vec::with_capacity(2 * hops + 3);
    points.push(p0);
    for i in 0..hops {
        let y = bottom - step * i as f32;
        points.push(Vec2::new(w.x, y));
        points.push(Vec2::new(out_x, y - 0.5 * step));
    }
    points.push(Vec2::new(w.x, top));
    points.push(land);
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{Abilities, Character, CharacterParams};
    use crate::geom::RectF;
    use crate::terrain::{Facing, Ledge, LedgeKind, Terrain, Wall, WallKind};

    fn terrain() -> Terrain {
        let ledges = vec![
            Ledge { x0: 0.0, x1: 1920.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
            Ledge { x0: 700.0, x1: 1100.0, y: 800.0, kind: LedgeKind::OneWay, owner: None },
        ];
        let walls = vec![
            Wall { x: 0.0, y0: 0.0, y1: 1040.0, face: Facing::Right, owner: None, kind: WallKind::Monitor },
            Wall { x: 1920.0, y0: 0.0, y1: 1040.0, face: Facing::Left, owner: None, kind: WallKind::Monitor },
        ];
        Terrain { generation: 1, windows: vec![], ledges, walls, bounds: RectF::new(0.0, 0.0, 1920.0, 1080.0) }
    }

    #[test]
    fn paths_start_at_takeoff_end_at_landing_and_jumps_arc_up() {
        let t = terrain();
        let ch = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(300.0, 1040.0));
        let g = NavGraph::build(&t, &ch);
        let spans = debug_spans(&g);
        assert_eq!(spans.len(), g.surfaces().len());
        assert!(spans.iter().all(|s| s.x0 < s.x1));
        let paths = debug_paths(&g, None);
        assert_eq!(paths.len(), g.edges().len());
        assert!(paths.iter().all(|p| !p.highlighted && p.points.len() == STEPS + 1));
        for (p, e) in paths.iter().zip(g.edges()) {
            assert!((p.points[0].x - e.takeoff).abs() < 1e-3);
            assert!((*p.points.last().unwrap() - e.land).length() < 1e-3);
            if e.kind == MoveKind::Jump {
                let top = p.points.iter().map(|q| q.y).fold(f32::MAX, f32::min);
                assert!(top < e.land.y.min(p.points[0].y), "jump arcs above both ends");
            }
        }
        // Up onto the platform and back down both exist, and a route highlights its edges.
        assert!(paths.iter().any(|p| p.points[0].y > 1000.0 && p.points.last().unwrap().y < 900.0));
        let route = g.plan(&ch, super::super::NavGoal::Point(Vec2::new(900.0, 800.0)), super::super::TravelStyle::Normal).expect("route");
        let with = debug_paths(&g, Some(&route));
        assert_eq!(with.len(), paths.len() + route.moves.len());
        assert!(with[paths.len()..].iter().all(|p| p.highlighted));
        assert_eq!(route_paths(&route).len(), route.moves.len());
    }
}
