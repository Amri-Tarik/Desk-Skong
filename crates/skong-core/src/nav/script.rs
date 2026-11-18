//! Edge scripts: small closed-loop input recipes. The graph validates them by
//! forward-simulating a cloned [`Character`]; the follower replays them tick by tick with the
//! very same [`Exec`], so whatever was validated is what gets executed.
use super::surface::Surface;
use crate::controller::{Character, Gait, MoveInput, MoveState, Side};

/// Proportional steering: full input this far (game units) from the target x.
const STEER_UNITS: f32 = 0.375;
/// Feet must be this far (px) above a ledge before a climb steers onto it.
const CLEAR_PX: f32 = 2.0;
/// Standing take-off tolerance (px) and residual speed (units/s).
const STAND_TOL_PX: f32 = 2.0;
const STAND_SPEED: f32 = 0.5;
/// A running take-off needs at least this fraction of the edge's gait speed.
const RUN_FRACTION: f32 = 0.8;

/// What the knight does after take-off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recipe {
    /// Walk/run towards `tx`: onto a touching ledge, or off an edge (steering in the air).
    Walk,
    /// Down + jump through a one-way ledge, then steer.
    DropThrough,
    /// Jump, steering towards `tx`. `double`: double jump at the apex; `dash`: dash once
    /// falling and within dash range (or level with the target); `low`: release jump as soon
    /// as the feet clear `ty`.
    Jump { double: bool, dash: bool, low: bool },
    /// Dash from the ground right at take-off, then steer.
    GroundDash,
    /// Wall-jump chain up the wall in `dir`, double jumping at apexes, then onto `tx`.
    Climb,
    /// Charge in place, fly in `dir`, cancel past `tx`, drop.
    Superdash,
}

/// How the take-off point must be reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// Crossing `takeoff` in `dir` at (most of) the edge gait speed, from `back` if needed.
    Run,
    /// Stopped at `takeoff`.
    Stand,
    /// Crossing `takeoff` in `dir` at any speed.
    Pass,
}

/// One validated traversal script (geometry in px, absolute coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Script {
    pub recipe: Recipe,
    pub start: Start,
    pub dir: Side,
    pub gait: Gait,
    pub takeoff: f32,
    /// Run-up start for [`Start::Run`] / [`Start::Pass`].
    pub back: f32,
    /// Landing target x and the target surface height.
    pub tx: f32,
    pub ty: f32,
    /// The validated run used a shadow dash through a window wall.
    pub needs_shadow: bool,
}

impl Script {
    fn uses_dash(&self) -> bool {
        matches!(self.recipe, Recipe::GroundDash | Recipe::Jump { dash: true, .. }) || self.needs_shadow
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Approach,
    Script,
}

/// Result of checking the character after a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Continue,
    /// Standing on the target surface.
    Done,
    /// Landed somewhere else.
    Wrong,
}

/// Executes a [`Script`]: approach the take-off, then the recipe. Copy, allocation free.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Exec {
    pub script: Script,
    pub stage: Stage,
    /// Ticks / seconds since the script stage began.
    pub ticks: u32,
    pub t: f32,
    /// Has been airborne since take-off.
    pub airborne: bool,
    backing: bool,
    ran: bool,
    double_done: bool,
    dash_done: bool,
    released: bool,
}

impl Exec {
    pub fn new(script: Script) -> Self {
        Self {
            script,
            stage: Stage::Approach,
            ticks: 0,
            t: 0.0,
            airborne: false,
            backing: false,
            ran: false,
            double_done: false,
            dash_done: false,
            released: false,
        }
    }

    /// Input for this tick. `approach_gait` is used while walking to the take-off from afar.
    pub fn input(&mut self, ch: &Character, approach_gait: Gait, dt: f32) -> MoveInput {
        if self.stage == Stage::Approach {
            match self.approach(ch, approach_gait) {
                Some(input) => return input,
                None => self.stage = Stage::Script,
            }
        }
        let input = self.recipe_input(ch, dt);
        self.ticks += 1;
        self.t += dt;
        input
    }

    /// Check the character after a tick of the script stage.
    ///
    /// Walking (never airborne) across other segments of the source ledge is fine: a ground
    /// dash or walk may pass through a window standing on the floor on its way to `to`.
    pub fn outcome(&mut self, ch: &Character, from: &Surface, to: &Surface) -> Outcome {
        if to.holds(ch) {
            return Outcome::Done;
        }
        if self.stage == Stage::Approach {
            return Outcome::Continue;
        }
        if !ch.is_grounded() {
            self.airborne = true;
            return Outcome::Continue;
        }
        // A superdash that ended (hit a wall, failed to launch) while still over the source.
        let fizzled = self.released && ch.state != MoveState::Superdash;
        if self.airborne || fizzled || !from.shares_ledge(ch) { Outcome::Wrong } else { Outcome::Continue }
    }

    /// No further upward impulse will come from the script (falling below the target is final).
    pub fn exhausted(&self) -> bool {
        match self.script.recipe {
            Recipe::Walk | Recipe::DropThrough | Recipe::GroundDash => true,
            Recipe::Jump { double, dash, .. } => (!double || self.double_done) && (!dash || self.dash_done),
            Recipe::Climb => false,
            Recipe::Superdash => self.released,
        }
    }

    /// Walk to the take-off; `None` once it is reached (the script starts this tick).
    fn approach(&mut self, ch: &Character, approach_gait: Gait) -> Option<MoveInput> {
        let s = self.script;
        let p = &ch.params;
        let x = ch.pos.x;
        let gain = p.px(STEER_UNITS);
        let ready = !s.uses_dash() || (ch.can_dash() && (!s.needs_shadow || ch.shadow_dash_ready()));
        let walk_to = |target: f32, gait: Gait| MoveInput { x: steer(x, target, gain), gait, ..MoveInput::default() };
        if s.start == Start::Stand {
            let settled = (x - s.takeoff).abs() <= STAND_TOL_PX
                && ch.vel.x.abs() <= p.px(STAND_SPEED)
                && ch.state == MoveState::Grounded;
            let near = (x - s.takeoff).abs() < 2.0 * gain;
            return (!(settled && ready)).then(|| walk_to(s.takeoff, if near { s.gait } else { approach_gait }));
        }
        let sign = s.dir.sign();
        if self.backing {
            if (x - s.back).abs() > 3.0 {
                return Some(walk_to(s.back, approach_gait));
            }
            self.backing = false;
            self.ran = true;
        }
        let d = (s.takeoff - x) * sign;
        let fast = ch.vel.x * sign >= RUN_FRACTION * p.gait_speed_px(s.gait);
        if d <= 0.0 {
            if ready && (s.start == Start::Pass || self.ran || fast) && ch.state == MoveState::Grounded {
                return None;
            }
            self.backing = true;
            return Some(walk_to(s.back, approach_gait));
        }
        if (s.back - x) * sign >= -1.0 {
            self.ran = true;
        }
        if !ready {
            // Wait (behind the take-off) for the dash / shadow dash cooldown.
            return Some(walk_to(x, s.gait));
        }
        let runup = run_up(ch, s.gait);
        let gait = if d < 2.0 * runup { s.gait } else { approach_gait };
        Some(MoveInput { x: sign, gait, ..MoveInput::default() })
    }

    fn recipe_input(&mut self, ch: &Character, dt: f32) -> MoveInput {
        let s = self.script;
        let p = &ch.params;
        let first = self.ticks == 0;
        let toward_tx = steer(ch.pos.x, s.tx, p.px(STEER_UNITS));
        let mut input = MoveInput { x: toward_tx, gait: s.gait, ..MoveInput::default() };
        let press_jump = |input: &mut MoveInput| {
            input.jump_pressed = true;
            input.jump_held = true;
        };
        let falling = ch.vel.y >= 0.0 && ch.state == MoveState::Airborne;
        match s.recipe {
            Recipe::Walk => {
                // Keep walking until off the edge (or onto the target); steer once falling.
                if ch.is_grounded() {
                    input.x = s.dir.sign();
                }
            }
            Recipe::DropThrough => {
                if first {
                    input = MoveInput { down: true, gait: s.gait, ..MoveInput::default() };
                    press_jump(&mut input);
                }
            }
            Recipe::GroundDash => {
                if first {
                    input.x = s.dir.sign();
                    input.dash_pressed = true;
                }
            }
            Recipe::Jump { double, dash, low } => {
                input.jump_held = !low || ch.pos.y > s.ty - CLEAR_PX - p.px(0.25);
                if first {
                    press_jump(&mut input);
                } else if falling && double && !self.double_done {
                    self.double_done = true;
                    press_jump(&mut input);
                } else if falling && dash && !self.dash_done && (!double || self.double_done) {
                    let remaining = (s.tx - ch.pos.x).abs();
                    let dash_len = p.px(p.dash_distance());
                    if remaining <= dash_len || ch.pos.y >= s.ty - p.px(0.5) {
                        self.dash_done = true;
                        input.dash_pressed = true;
                        input.x = if s.tx >= ch.pos.x { 1.0 } else { -1.0 };
                    }
                }
            }
            Recipe::Climb => {
                input.jump_held = true;
                let above = ch.pos.y < s.ty - CLEAR_PX;
                if first {
                    input.x = s.dir.sign();
                    press_jump(&mut input);
                } else if let MoveState::WallSlide { side } = ch.state {
                    // Kick off pressing away: the shorter input lock brings us back sooner.
                    input.x = side.opposite().sign();
                    press_jump(&mut input);
                } else if !above {
                    input.x = s.dir.sign();
                    if falling && ch.can_double_jump() {
                        press_jump(&mut input);
                    }
                }
            }
            Recipe::Superdash => {
                input.x = 0.0;
                if !self.released {
                    if self.t < p.superdash_charge_time + dt {
                        input.superdash_held = true;
                        input.x = s.dir.sign();
                    } else {
                        self.released = true;
                    }
                } else if ch.state == MoveState::Superdash {
                    if (ch.pos.x - s.tx) * s.dir.sign() >= 0.0 {
                        input.jump_pressed = true;
                    }
                } else {
                    input.x = toward_tx;
                }
            }
        }
        input
    }
}

/// Proportional steering input towards `target`.
pub(crate) fn steer(x: f32, target: f32, gain_px: f32) -> f32 {
    ((target - x) / gain_px).clamp(-1.0, 1.0)
}

/// Distance (px) needed to reach `gait` speed from standing, plus a little margin.
pub(crate) fn run_up(ch: &Character, gait: Gait) -> f32 {
    let p = &ch.params;
    let v = p.gait_speed(gait);
    p.px(v * v / (2.0 * p.ground_accel) + 0.5)
}
