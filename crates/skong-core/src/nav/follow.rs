//! Route following: replays each edge's script with the executor that validated it, checks
//! every landing, and re-plans on drift, timeouts or a new terrain generation.
use super::script::{Exec, Outcome, Stage, steer};
use super::{Edge, FollowStatus, MoveKind, NavGoal, NavGraph, Route, TravelStyle, search};
use crate::controller::{Character, MoveInput, MoveState};
use crate::terrain::Terrain;

/// Final approach: done within this many px of the route end.
const ARRIVE_PX: f32 = 4.0;
/// Final approach steering gain (game units).
const ARRIVE_GAIN: f32 = 0.25;
/// Drift / timeout re-plans (in total, for one goal) before giving up.
const MAX_DRIFTS: u32 = 12;
/// An edge that slipped this many times is avoided (penalty), then excluded.
const SLIPS_AVOID: u8 = 2;
const SLIPS_EXCLUDE: u8 = 3;
/// Extra cost (s) for an edge being avoided: alternatives win unless much slower.
const AVOID_COST: f32 = 5.0;
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
    /// Seconds spent pressing a direction while grounded without moving (walking into
    /// something the plan didn't expect). Triggers a re-plan.
    push: f32,
    /// Drift re-plans so far.
    drifts: u32,
    /// Re-plans so far (any reason).
    replans: u32,
    /// Edges whose script failed while following, with their failure counts. A first slip
    /// is retried as is (usually a fixable miss), repeated ones are avoided, then excluded.
    slips: Vec<(EdgeKey, u8)>,
}

/// Identity of an edge across re-plans (and graph rebuilds that leave it unchanged).
type EdgeKey = (MoveKind, i32, i32, i32);

fn edge_key(e: &Edge) -> EdgeKey {
    let q = |v: f32| (v / 4.0).round() as i32;
    (e.kind, q(e.takeoff), q(e.land.x), q(e.land.y))
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

    /// Kind of the edge being executed (approach included); `None` on the final walk.
    pub fn current_move(&self) -> Option<MoveKind> {
        self.route.steps.get(self.step).map(|s| s.edge.kind)
    }

    /// May the app turn a fall into a dive now? Only while falling along a planned drop whose
    /// landing is straight below him: a dive kills horizontal speed, so anywhere else (a
    /// walk-off still drifting sideways, mid-jump, mid-climb, after a slip) it would land him
    /// somewhere the plan never meant.
    pub fn dive_ok(&self, ch: &Character) -> bool {
        const STRAIGHT_PX: f32 = 12.0;
        if self.replan.is_some() {
            return false;
        }
        self.route.steps.get(self.step).is_some_and(|step| {
            matches!(step.edge.kind, MoveKind::Walk | MoveKind::Drop | MoveKind::DropThrough)
                && (ch.pos.x - step.edge.land.x).abs() <= STRAIGHT_PX
                && ch.vel.x.abs() < ch.params.px(0.5)
        })
    }

    /// Re-plans so far (terrain changes, drift, timeouts).
    pub fn replans(&self) -> u32 {
        self.replans
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
                Outcome::Done => self.start_step(self.step + 1),
                Outcome::Wrong if self.replan == Some(Replan::Drift) => {}
                Outcome::Wrong => {
                    self.slipped(step.edge);
                    self.request(Replan::Drift);
                }
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
        if self.stage_time > self.stage_limit && self.replan != Some(Replan::Drift) {
            if in_script && let Some(step) = self.route.steps.get(self.step) {
                self.slipped(step.edge);
            }
            self.request(Replan::Drift);
        }
        if let Some(reason) = self.replan {
            if !ch.is_grounded() || matches!(ch.state, MoveState::HardLanding | MoveState::Dive { .. }) {
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
                // Still walking to the take-off (not into the scripted move itself).
                let walking = approaching && exec.stage == Stage::Approach;
                if approaching && exec.stage == Stage::Script {
                    self.stage_time = 0.0;
                    self.stage_limit = step.edge.duration + STAGE_SLACK;
                }
                if walking {
                    self.watch_push(ch, &input, dt);
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
        let input = MoveInput { x, gait: self.style.gait(), ..idle };
        self.watch_push(ch, &input, 1.0 / 120.0);
        (input, FollowStatus::Moving)
    }

    /// Walking (not a scripted move) but not getting anywhere — pressed against a wall the
    /// plan didn't account for: re-plan after half a second instead of pushing until the
    /// stage times out.
    fn watch_push(&mut self, ch: &Character, input: &MoveInput, dt: f32) {
        const PUSH_LIMIT: f32 = 0.5;
        let pushing = ch.is_grounded() && input.x.abs() > 0.2 && ch.vel.x.abs() < ch.params.px(0.15);
        self.push = if pushing { self.push + dt } else { 0.0 };
        if self.push > PUSH_LIMIT {
            self.push = 0.0;
            self.request(Replan::Drift);
        }
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
        self.replans += 1;
        if reason == Replan::Drift {
            self.drifts += 1;
        }
        let slips = &self.slips;
        let extra = |e: &Edge| match slips.iter().find(|(k, _)| *k == edge_key(e)) {
            Some(&(_, n)) if n >= SLIPS_EXCLUDE => f32::INFINITY,
            Some(&(_, n)) if n >= SLIPS_AVOID => AVOID_COST,
            _ => 0.0,
        };
        let route = match self.goal {
            Some(goal) if self.drifts <= MAX_DRIFTS => search::plan_with(graph, ch, goal, self.style, &extra),
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

    /// Count a failure of `edge`'s script.
    fn slipped(&mut self, edge: Edge) {
        let k = edge_key(&edge);
        match self.slips.iter_mut().find(|(e, _)| *e == k) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => self.slips.push((k, 1)),
        }
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
