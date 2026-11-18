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
//! TODO(path_planner): implementation.
use crate::controller::{Character, Gait, MoveInput};
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
}

impl NavGraph {
    pub fn build(_terrain: &Terrain, _template: &Character) -> Self {
        todo!("path_planner")
    }

    /// Plan from the character's current state.
    pub fn plan(&self, _from: &Character, _goal: NavGoal, _style: TravelStyle) -> Option<Route> {
        todo!("path_planner")
    }
}

/// Drives a character along a route.
#[derive(Clone, Debug, Default)]
pub struct PathFollower;

impl PathFollower {
    pub fn new(_route: Route, _goal: NavGoal, _style: TravelStyle) -> Self {
        todo!("path_planner")
    }

    /// Input for this tick. Call once per fixed step before `Character::step`.
    pub fn next_input(&mut self, _ch: &Character, _graph: &NavGraph, _terrain: &Terrain, _dt: f32) -> (MoveInput, FollowStatus) {
        todo!("path_planner")
    }
}
