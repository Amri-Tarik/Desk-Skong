//! The brain's view of navigation. The brain *commands* a [`Navigator`] (go there,
//! stop, how far is it?) and reads back its status; the app pumps the navigator for the
//! actual per-tick [`MoveInput`] whenever the intent says [`super::Motion::Navigate`].
//!
//! [`GraphNavigator`] adapts the planner in [`crate::nav`]; tests use a fake.
use std::sync::Arc;

use crate::controller::{Character, MoveInput};
use crate::geom::Vec2;
use crate::nav::{FollowStatus, NavGoal, NavGraph, PathFollower, TravelStyle};
use crate::terrain::Terrain;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavStatus {
    /// No goal.
    #[default]
    Idle,
    Moving,
    Arrived,
    /// No route / stuck.
    Failed,
}

pub trait Navigator {
    /// Estimated travel time to `goal` (s), `None` if unreachable with that style.
    /// Called for a handful of candidates per evaluation, so it should be cheapish.
    fn estimate(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<f32>;
    /// Start (or re-target) travel. `false` if no route exists.
    fn go(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> bool;
    /// Abandon the current goal.
    fn stop(&mut self);
    fn status(&self) -> NavStatus;

    /// Kind of the planned move being executed, when the navigator knows (a wall jump means
    /// sliding on a wall is part of the plan).
    fn current_move(&self) -> Option<crate::nav::MoveKind> {
        None
    }

    /// Plan without moving: the travel time and where the route actually ends (planners
    /// may stop at the closest reachable point, e.g. under a ledge a stroll can't mount).
    /// `end: None` = this navigator can't tell; the default only knows the eta.
    fn reach(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<Reach> {
        self.estimate(from, goal, style).map(|eta| Reach { eta, end: None })
    }

    /// Does a route with `style` get *onto* `p` (end within `tol` px horizontally and on
    /// the same height), not just somewhere near it?
    fn reaches(&mut self, from: &Character, p: Vec2, style: TravelStyle, tol: f32) -> bool {
        self.reach(from, NavGoal::Point(p), style).is_some_and(|r| r.lands_on(p, tol))
    }
}

/// Result of [`Navigator::reach`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub eta: f32,
    /// Planned final feet position, if known.
    pub end: Option<Vec2>,
}

impl Reach {
    /// The route ends on `p` (unknown ends are given the benefit of the doubt).
    pub fn lands_on(&self, p: Vec2, tol: f32) -> bool {
        self.end.is_none_or(|e| (e.x - p.x).abs() <= tol && (e.y - p.y).abs() <= 3.0)
    }
}

/// [`Navigator`] backed by [`NavGraph`] + [`PathFollower`].
///
/// App duties: call [`GraphNavigator::rebuild`] whenever the terrain is rebuilt, and
/// [`GraphNavigator::next_input`] on ticks where the brain's motion is `Navigate`.
#[derive(Debug, Default)]
pub struct GraphNavigator {
    graph: Option<Arc<NavGraph>>,
    follower: Option<PathFollower>,
    status: NavStatus,
}

impl GraphNavigator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rebuild(&mut self, terrain: &Terrain, template: &Character) {
        // Rebuilding from the previous graph reuses its simulated edges where nothing moved.
        self.graph = Some(Arc::new(self.graph.as_ref().map_or_else(|| NavGraph::build(terrain, template), |g| g.rebuild(terrain, template))));
    }

    /// Install a graph built elsewhere (e.g. on a background thread). An active route keeps
    /// following; the follower re-plans if it references surfaces that no longer exist.
    pub fn set_graph(&mut self, graph: Arc<NavGraph>) {
        self.graph = Some(graph);
    }

    pub fn graph(&self) -> Option<&Arc<NavGraph>> {
        self.graph.as_ref()
    }

    /// The route being followed, with its goal and style (debug overlay / status).
    pub fn follower(&self) -> Option<&PathFollower> {
        self.follower.as_ref()
    }

    /// Input for this tick; neutral when idle. Updates [`Navigator::status`].
    pub fn next_input(&mut self, ch: &Character, terrain: &Terrain, dt: f32) -> MoveInput {
        let (Some(graph), Some(follower)) = (self.graph.as_ref(), self.follower.as_mut()) else {
            return MoveInput::default();
        };
        let (input, st) = follower.next_input(ch, graph, terrain, dt);
        self.status = match st {
            FollowStatus::Moving => NavStatus::Moving,
            FollowStatus::Arrived => NavStatus::Arrived,
            FollowStatus::Failed => NavStatus::Failed,
        };
        if self.status != NavStatus::Moving {
            self.follower = None;
        }
        input
    }
}

impl Navigator for GraphNavigator {
    fn current_move(&self) -> Option<crate::nav::MoveKind> {
        self.follower.as_ref().and_then(|f| f.current_move())
    }

    fn reach(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<Reach> {
        let r = self.graph.as_ref()?.plan(from, goal, style)?;
        Some(Reach { eta: r.eta, end: Some(r.end) })
    }

    fn estimate(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<f32> {
        self.graph.as_ref()?.plan(from, goal, style).map(|r| r.eta)
    }

    fn go(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> bool {
        let route = self.graph.as_ref().and_then(|g| g.plan(from, goal, style));
        match route {
            Some(route) => {
                self.follower = Some(PathFollower::new(route, goal, style));
                self.status = NavStatus::Moving;
                true
            }
            None => {
                self.follower = None;
                self.status = NavStatus::Failed;
                false
            }
        }
    }

    fn stop(&mut self) {
        self.follower = None;
        self.status = NavStatus::Idle;
    }

    fn status(&self) -> NavStatus {
        self.status
    }
}
