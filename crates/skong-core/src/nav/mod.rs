//! Navigation: plan routes across the desktop terrain using the knight's full movement
//! kit, then follow them by emitting the same `MoveInput`s a player would.
//!
//! Contract (shared by the planner implementation and the AI):
//! * [`NavGraph::build`] turns a [`Terrain`] into surfaces + traversal edges. Edges are
//!   validated by forward-simulating a cloned [`Character`], so anything planned is
//!   actually executable by the controller.
//! * [`TravelStyle`] is how the AI expresses mood: a *stroll* walks and only uses
//!   gentle moves (walk, drop, single jump); a *hurry* runs/sprints and allows the
//!   flashy kit (double jump, wall-jump chains, dash, shadow dash, superdash).
//! * [`PathFollower`] executes a [`Route`], re-planning when the terrain changes or the
//!   character drifts off the plan.
//!
//! Implementation:
//! * `surface`: nodes are ledge segments (ledges split where walls stand on them).
//! * `graph`: for every surface pair that the analytic reach (`controller::params`) does
//!   not rule out, candidate scripts (take-off x, direction, gait, jump / double jump / dash /
//!   wall-jump / superdash timing) are forward-simulated against a local copy of the
//!   terrain; the first that lands on the target becomes the edge. Results are cached by
//!   the geometry they depend on, so [`NavGraph::rebuild`] after a window drag only
//!   re-simulates nearby pairs. Long level runs (along one surface, or to another at the same
//!   height) also get crystal dash edges for hurried routes: one validated flight per run,
//!   landing at stations a few units apart, launched from wherever the knight stands.
//! * `search`: Dijkstra over landing states (surface + landing x), cost = estimated time
//!   plus small style penalties.
//! * `follow`: replays the edge scripts with the same executor the validation used.
mod debug;
mod follow;
mod graph;
mod script;
mod search;
mod surface;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_climb;

pub use debug::{DebugPath, DebugSpan, debug_paths, debug_spans, edge_path, route_paths};
pub use follow::PathFollower;
pub use graph::{BuildStats, Edge};
pub use script::{Climb, ClimbWall};
pub use surface::{Surface, SurfaceId};

use crate::controller::{Character, Gait};
use crate::desktop::WindowId;
use crate::geom::Vec2;
use crate::terrain::Terrain;

/// How to travel. Chosen by the AI from mood.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TravelStyle {
    /// Walk; walk/drop/single jump only. Gives up on goals needing more.
    Stroll,
    /// Run; everything except superdash.
    #[default]
    Normal,
    /// Sprint; everything, preferring fast moves (dash, superdash).
    Hurry,
}

impl TravelStyle {
    pub fn gait(self) -> Gait {
        match self {
            TravelStyle::Stroll => Gait::Walk,
            TravelStyle::Normal => Gait::Run,
            TravelStyle::Hurry => Gait::Sprint,
        }
    }

    /// Can this style use `edge`?
    pub fn allows(self, edge: &Edge) -> bool {
        match self {
            TravelStyle::Stroll => edge.gait == Gait::Walk && !matches!(edge.kind, MoveKind::Dash | MoveKind::ShadowDash | MoveKind::Superdash),
            TravelStyle::Normal => edge.kind != MoveKind::Superdash,
            TravelStyle::Hurry => true,
        }
    }
}

/// Where to go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavGoal {
    /// Stand as close as possible to this point (feet position).
    Point(Vec2),
    /// Stand anywhere on top of this window.
    OnWindow(WindowId),
}

/// Kind of traversal between two surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoveKind {
    Walk,
    Drop,
    DropThrough,
    Jump,
    DoubleJump,
    WallJump,
    Dash,
    ShadowDash,
    Superdash,
    /// Turn into the screen and walk behind a drawn line (content walls only).
    Behind,
}

impl MoveKind {
    pub const ALL: [MoveKind; 10] = [
        MoveKind::Walk,
        MoveKind::Drop,
        MoveKind::DropThrough,
        MoveKind::Jump,
        MoveKind::DoubleJump,
        MoveKind::WallJump,
        MoveKind::Dash,
        MoveKind::ShadowDash,
        MoveKind::Superdash,
        MoveKind::Behind,
    ];
}

/// A planned sequence of moves (opaque to the AI beyond inspection).
#[derive(Clone, Debug, Default)]
pub struct Route {
    /// Kinds of the edges in order (for the AI / debug overlay).
    pub moves: Vec<MoveKind>,
    /// Estimated travel time in seconds.
    pub eta: f32,
    /// Final feet position.
    pub end: Vec2,
    /// Edges to replay, with copies of their surfaces (valid even if the graph changes).
    pub(crate) steps: Vec<RouteStep>,
    /// Surface the route ends on.
    pub(crate) finish: Option<Surface>,
    /// Graph generation the route was planned on.
    pub(crate) generation: u64,
}

impl Route {
    /// The edges of the route (for the debug overlay).
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.steps.iter().map(|s| &s.edge)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RouteStep {
    pub edge: Edge,
    pub from: Surface,
    pub to: Surface,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowStatus {
    Moving,
    Arrived,
    /// Route impossible now (terrain changed and no re-plan found, or stuck).
    Failed,
}

/// Navigation graph for one terrain generation.
#[derive(Clone, Debug, Default)]
pub struct NavGraph {
    pub generation: u64,
    surfaces: Vec<Surface>,
    edges: Vec<Edge>,
    /// Edge indices grouped by source surface: `edges[out_start[s]..out_start[s + 1]]`.
    out_start: Vec<usize>,
    /// Copy of the terrain (predicting where an airborne character lands).
    terrain: Terrain,
    cache: graph::EdgeCache,
    stats: BuildStats,
}

impl NavGraph {
    pub fn build(terrain: &Terrain, template: &Character) -> Self {
        Self::default().rebuild(terrain, template)
    }

    /// Build for a new terrain generation, reusing this graph's simulation results for every
    /// surface pair whose surrounding geometry did not change (e.g. after one window moved).
    pub fn rebuild(&self, terrain: &Terrain, template: &Character) -> Self {
        let p = template.params;
        let hit = p.hitbox_px();
        let surfaces = surface::extract(terrain, hit.x * 0.5, hit.y, p.step_height_px());
        let (mut edges, cache, stats) = graph::build_edges(terrain, template, &surfaces, &self.cache);
        edges.sort_by_key(|e| e.from);
        let mut out_start = vec![0; surfaces.len() + 1];
        for e in &edges {
            out_start[e.from + 1] += 1;
        }
        for i in 0..surfaces.len() {
            out_start[i + 1] += out_start[i];
        }
        Self {
            generation: terrain.generation,
            surfaces,
            edges,
            out_start,
            terrain: terrain.clone(),
            cache,
            stats,
        }
    }

    /// Plan from the character's current state.
    pub fn plan(&self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<Route> {
        search::plan(self, from, goal, style)
    }

    pub fn surfaces(&self) -> &[Surface] {
        &self.surfaces
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Edges leaving surface `s`.
    pub fn edges_from(&self, s: SurfaceId) -> &[Edge] {
        match (self.out_start.get(s), self.out_start.get(s + 1)) {
            (Some(&a), Some(&b)) => &self.edges[a..b],
            _ => &[],
        }
    }

    /// The surface the character stands on (None while airborne).
    pub fn surface_at(&self, ch: &Character) -> Option<SurfaceId> {
        surface::surface_at(&self.surfaces, ch)
    }

    /// Counters and timing of the build that produced this graph.
    pub fn stats(&self) -> BuildStats {
        self.stats
    }

    /// Number of edges of each kind (in [`MoveKind::ALL`] order).
    pub fn edge_counts(&self) -> [usize; MoveKind::ALL.len()] {
        let mut n = [0; MoveKind::ALL.len()];
        for e in &self.edges {
            if let Some(i) = MoveKind::ALL.iter().position(|k| *k == e.kind) {
                n[i] += 1;
            }
        }
        n
    }
}
