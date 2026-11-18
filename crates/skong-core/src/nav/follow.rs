//! Route following: replays each edge's script with the executor that validated it, checks
//! every landing, and re-plans on drift, timeouts or a new terrain generation.
use super::script::{Exec, Outcome, Stage, steer};
use super::{FollowStatus, NavGoal, NavGraph, Route, TravelStyle};
use crate::controller::{Character, MoveInput, MoveState};
use crate::terrain::Terrain;

/// Final approach: done within this many px of the route end.
const ARRIVE_PX: f32 = 4.0;
/// Final approach steering gain (game units).
const ARRIVE_GAIN: f32 = 0.25;
/// Consecutive drift / timeout re-plans before giving up.
const MAX_FAILURES: u32 = 4;
/// Slack (s) on top of the expected duration of a stage before it counts as stuck.
const STAGE_SLACK: f32 = 3.0;
/// Worst wait for dash / shadow dash cooldowns at a take-off (s).
const COOLDOWN_WAIT: f32 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Replan {
    /// The graph moved to a new terrain generation.
    Stale,
    /// Off the plan (wrong landing, knocked off, stuck).
    Drift,
}

/// Drives a character along a route.
#[derive(Clone, Debug, Default)]
pub struct PathFollower {
    route: Route,
    goal: Option<NavGoal>,
    style: TravelStyle,
    /// Index of the current step; `route.steps.len()` = final walk on the last surface.
    step: usize,
    exec: Option<Exec>,
    replan: Option<Replan>,
    done: Option<FollowStatus>,
    /// Time in the current stage (approach / script / final walk) and its budget.
    stage_time: f32,
    stage_limit: f32,
    failures: u32,
}

impl PathFollower {
    pub fn new(route: Route, goal: NavGoal, style: TravelStyle) -> Self {
        let mut f = Self { route, goal: Some(goal), style, ..Self::default() };
        f.start_step(0);
        f
    }

    /// The route currently followed (changes on re-plan).
    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn goal(&self) -> Option<NavGoal> {
        self.goal
    }

    pub fn style(&self) -> TravelStyle {
        self.style
    }

    /// Input for this tick. Call once per fixed step before `Character::step`.
    pub fn next_input(&mut self, ch: &Character, graph: &NavGraph, _terrain: &Terrain, dt: f32) -> (MoveInput, FollowStatus) {
        let idle = MoveInput::default();
        if let Some(status) = self.done {
            return (idle, status);
        }
        self.stage_time += dt;

        // Outcome of the previous tick.
        if let (Some(exec), Some(step)) = (self.exec.as_mut(), self.route.steps.get(self.step)) {
            let was_script = exec.stage == Stage::Script;
            match exec.outcome(ch, &step.from, &step.to) {
                Outcome::Done => {
                    self.failures = 0;
                    self.start_step(self.step + 1);
                }
                Outcome::Wrong => self.request(Replan::Drift),
                Outcome::Continue => {
                    if !was_script && exec.stage == Stage::Script {
                        self.stage_time = 0.0;
                    }
                }
            }
        }
        let in_script = self.exec.is_some_and(|e| e.stage == Stage::Script);
        if graph.generation != self.route.generation && !in_script {
            self.request(Replan::Stale);
        }
        if self.stage_time > self.stage_limit {
            self.request(Replan::Drift);
        }
        if let Some(reason) = self.replan {
            if !ch.is_grounded() || ch.state == MoveState::HardLanding {
                return (idle, FollowStatus::Moving);
            }
            if let Some(status) = self.do_replan(reason, ch, graph) {
                return (idle, status);
            }
        }

        match (self.exec.as_mut(), self.route.steps.get(self.step)) {
            (Some(exec), Some(step)) => {
                let approaching = exec.stage == Stage::Approach;
                if approaching && !step.from.holds(ch) {
                    // Knocked off, carried away or the surface vanished.
                    self.request(Replan::Drift);
                    return (idle, FollowStatus::Moving);
                }
                let input = exec.input(ch, self.style.gait(), dt);
                if approaching && exec.stage == Stage::Script {
                    self.stage_time = 0.0;
                    self.stage_limit = step.edge.duration + STAGE_SLACK;
                }
                (input, FollowStatus::Moving)
            }
            _ => self.final_walk(ch),
        }
    }

    /// Walk to the route end on the final surface.
    fn final_walk(&mut self, ch: &Character) -> (MoveInput, FollowStatus) {
        let idle = MoveInput::default();
        let Some(finish) = self.route.finish else {
            self.done = Some(FollowStatus::Failed);
            return (idle, FollowStatus::Failed);
        };
        if !finish.holds(ch) {
            self.request(Replan::Drift);
            return (idle, FollowStatus::Moving);
        }
        let target = self.route.end.x;
        if (ch.pos.x - target).abs() <= ARRIVE_PX {
            self.done = Some(FollowStatus::Arrived);
            return (idle, FollowStatus::Arrived);
        }
        let x = steer(ch.pos.x, target, ch.params.px(ARRIVE_GAIN));
        (MoveInput { x, gait: self.style.gait(), ..idle }, FollowStatus::Moving)
    }

    fn request(&mut self, reason: Replan) {
        // A drift report wins over a stale one (it counts towards giving up).
        if self.replan != Some(Replan::Drift) {
            self.replan = Some(reason);
        }
    }

    /// Re-plan from the current state; `Some(status)` when following has ended.
    fn do_replan(&mut self, reason: Replan, ch: &Character, graph: &NavGraph) -> Option<FollowStatus> {
        self.replan = None;
        if reason == Replan::Drift {
            self.failures += 1;
        }
        let route = match self.goal {
            Some(goal) if self.failures <= MAX_FAILURES => graph.plan(ch, goal, self.style),
            _ => None,
        };
        let Some(route) = route else {
            self.done = Some(FollowStatus::Failed);
            return self.done;
        };
        self.route = route;
        self.start_step(0);
        None
    }

    fn start_step(&mut self, i: usize) {
        self.step = i;
        self.stage_time = 0.0;
        match self.route.steps.get(i) {
            Some(step) => {
                self.exec = Some(Exec::new(step.edge.script));
                // Walking budget is refined once the knight is moving; be generous here.
                let span = (step.from.x1 - step.from.x0).abs();
                self.stage_limit = span / 100.0 + STAGE_SLACK + COOLDOWN_WAIT;
            }
            None => {
                self.exec = None;
                let span = self.route.finish.map_or(0.0, |f| (f.x1 - f.x0).abs());
                self.stage_limit = span / 100.0 + STAGE_SLACK;
            }
        }
    }
}
