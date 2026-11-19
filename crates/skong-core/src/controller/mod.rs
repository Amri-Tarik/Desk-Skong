//! Platformer character controller: an AABB moving through the desktop [`Terrain`] with
//! Hollow Knight's movement kit (variable jump, double jump, wall slide/jump, dash,
//! shadow dash, drop-through, superdash, hard landing).
//!
//! * Pixels, x right, **y down**. `pos` is the feet center (bottom-middle of the hitbox).
//! * [`Character::step`] advances one fixed tick (intended 120 Hz; any `dt > 0` works).
//! * Fully deterministic: no globals, RNG or wall-clock, and [`Character`] is `Clone`, so a
//!   planner can forward-simulate copies. Tuning, provenance and reachability numbers are
//!   documented in [`params`].
//! * Collision is swept per axis (x then y) over the whole displacement, so even a
//!   superdash at thousands of px/s cannot tunnel through a zero-width wall.
//! * Windows move between ticks; call [`Character::on_terrain_rebuilt`] when the terrain
//!   is rebuilt so the character rides (or falls off) the window it stands on.
mod collide;
mod dive;
pub mod params;
#[cfg(test)]
mod tests;

pub use params::{Abilities, CharacterParams};
pub use dive::DivePhase;

use collide::{EPS, Query};

use crate::desktop::WindowId;
use crate::geom::{RectF, Vec2};
use crate::terrain::{Ledge, LedgeKind, Terrain};

/// Movement speed chosen by the player / AI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gait {
    /// Slow stroll ("Walk" clip).
    Walk,
    /// Normal ("Run" clip).
    #[default]
    Run,
    /// Fast ("Sprint" clip).
    Sprint,
    /// The Unn slug's slow crawl (walking acts only; never planned by navigation).
    Crawl,
}

/// One tick of intent. Identical for keyboard control and the AI.
///
/// Edge fields (`*_pressed`) are true for the single tick the button went down; a press
/// also counts as held that tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveInput {
    /// -1..1 horizontal intent.
    pub x: f32,
    pub gait: Gait,
    pub up: bool,
    pub down: bool,
    /// Jump button currently held (variable jump height).
    pub jump_held: bool,
    /// Jump pressed this tick (edge). With `down` on a one-way ledge: drop through at once.
    /// Holding `down` alone for `drop_hold_time` also slips through.
    pub jump_pressed: bool,
    pub dash_pressed: bool,
    /// Superdash button held: hold to charge (ground or wall), release to launch.
    pub superdash_held: bool,
    /// Desolate Dive pressed this tick (edge): hover, then plunge straight down until the
    /// first ledge. Holding `down` while falling smashes through one-way / content ledges;
    /// ground (monitor / work-area floor) always stops it. Starts in the air, on a wall, or
    /// standing on a one-way ledge (dives through it).
    pub dive_pressed: bool,
    /// The dive pressed this tick is Descending Dark (bigger effects, same movement).
    pub dive_dark: bool,
    /// Held while grounded: turn to face into the screen and walk *behind* what is drawn on
    /// it, so content walls (lines, borders) don't block. Window and monitor edges still do.
    /// Released: turn back out.
    pub behind: bool,
}

/// Stepping behind screen content ([`MoveInput::behind`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BehindPhase {
    /// Turning to face into the screen ("TurnToBG"); not moving yet.
    TurningIn,
    /// Facing into the screen, walking behind drawn lines.
    Behind,
    /// Turning back out ("TurnFromBG"); not moving.
    TurningOut,
}

/// Seconds to turn into the screen (TurnToBG: 3 frames at 12 fps) and back out (2 frames).
pub const BEHIND_TURN_TIME: f32 = 0.25;
pub const BEHIND_RETURN_TIME: f32 = 0.17;

/// Horizontal direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    #[default]
    Right,
}

impl Side {
    /// -1 for left, +1 for right.
    pub fn sign(self) -> f32 {
        match self {
            Side::Left => -1.0,
            Side::Right => 1.0,
        }
    }

    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Side of a signed value; `None` for (near) zero.
    pub fn of(x: f32) -> Option<Side> {
        if x > INPUT_DEADZONE {
            Some(Side::Right)
        } else if x < -INPUT_DEADZONE {
            Some(Side::Left)
        } else {
            None
        }
    }
}

const INPUT_DEADZONE: f32 = 0.01;

/// Locomotion state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveState {
    Grounded,
    Airborne,
    /// Sliding down a wall on `side` of the body.
    WallSlide { side: Side },
    Dashing { shadow: bool },
    /// Charging a superdash on the ground (`wall: None`) or clinging to a wall.
    SuperdashCharge { wall: Option<Side> },
    /// Flying horizontally in the `facing` direction.
    Superdash,
    /// Stunned after a long fall.
    HardLanding,
    /// Desolate Dive: antic hover, plunge, landing slam.
    Dive { phase: DivePhase, dark: bool },
}

/// The ledge the character currently stands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Support {
    /// Window owning the ledge, `None` for monitor ground / bounds floor.
    pub owner: Option<WindowId>,
    pub kind: LedgeKind,
    pub y: f32,
    pub x0: f32,
    pub x1: f32,
}

impl From<Ledge> for Support {
    fn from(l: Ledge) -> Self {
        Self { owner: l.owner, kind: l.kind, y: l.y, x0: l.x0, x1: l.x1 }
    }
}

/// Things that happened during a step, for the animator / AI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveEvent {
    Jumped,
    DoubleJumped,
    /// Kicked off a wall on the `wall` side of the body.
    WallJumped { wall: Side },
    Landed { hard: bool },
    WallSlideStart { side: Side },
    WallSlideEnd,
    DashStarted { shadow: bool },
    DashEnded,
    SuperdashCharging,
    /// Charge complete; releasing now launches.
    SuperdashCharged,
    /// Released before fully charged (or lost footing / wall while charging).
    SuperdashCancelled,
    SuperdashLaunched,
    SuperdashHitWall,
    /// Flight cancelled by jump / dash.
    SuperdashEnded,
    DroppedThrough,
    TurnedAround,
    /// A hard landing from an extremely long fall (follows `Landed { hard: true }`).
    SuperHardLanded,
    /// Dive antic started (`dark`: Descending Dark).
    DiveStarted { dark: bool },
    /// Antic over: plunging.
    DiveFalling,
    /// The dive hit a ledge (slam).
    DiveLanded,
    /// Recovered from the slam (or lost the ground while recovering).
    DiveEnded,
    /// Ground vanished under the feet (walked off, window closed / moved / occluded).
    LostSupport,
    /// The window we stood on moved (he falls / it slides out from under him).
    PlatformMoved { owner: WindowId },
}

/// Countdown / elapsed timers, seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Timers {
    /// Remaining constant-upward-speed phase of the current jump.
    jump_hold: f32,
    /// Time since the current jump started.
    jump_elapsed: f32,
    /// Rising from a jump: releasing the button cuts the rise.
    jump_active: bool,
    coyote: f32,
    jump_buffer: f32,
    dash_buffer: f32,
    /// Horizontal input ignored after a wall jump.
    wall_lock: f32,
    /// Time spent pressing away from the wall during a wall slide.
    unstick: f32,
    /// Remaining dash time.
    dash: f32,
    dash_cooldown: f32,
    shadow_cooldown: f32,
    /// Superdash charge accumulated.
    charge: f32,
    /// Superdash flight time elapsed.
    superdash: f32,
    hard_landing: f32,
    /// How long down has been held while grounded (hold-down drop-through).
    down_held: f32,
    /// Time spent falling (for hard landings).
    fall: f32,
    /// Dive antic / landing countdown.
    dive: f32,
}

impl Timers {
    fn tick(&mut self, dt: f32) {
        for t in [
            &mut self.coyote,
            &mut self.jump_buffer,
            &mut self.dash_buffer,
            &mut self.wall_lock,
            &mut self.dash_cooldown,
            &mut self.shadow_cooldown,
            &mut self.hard_landing,
        ] {
            *t = (*t - dt).max(0.0);
        }
        self.jump_elapsed += dt;
    }
}

/// The knight's body. Cheap to clone; deterministic.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub params: CharacterParams,
    pub abilities: Abilities,
    /// Feet center, px.
    pub pos: Vec2,
    /// px / s (y down).
    pub vel: Vec2,
    pub facing: Side,
    /// Hitbox size, px.
    pub size: Vec2,
    pub state: MoveState,
    pub support: Option<Support>,
    timers: Timers,
    /// Air dash still available this airtime.
    air_dash: bool,
    /// Double jump still available this airtime.
    double_jump: bool,
    /// One-way ledge height being dropped through.
    drop_y: Option<f32>,
    /// The current / last dash goes straight down (Dashmaster).
    dash_down: bool,
    /// Diving with down held: one-way ledges do not stop the dive.
    dive_pierce: bool,
    /// Previous tick's `superdash_held` (edge detection).
    superdash_was_held: bool,
    /// Seconds spent turned into the screen ([`MoveInput::behind`]); 0 when not.
    behind: f32,
    /// Seconds left turning back out of the screen.
    behind_out: f32,
}

impl Character {
    /// A character with its feet at `pos`, airborne until it lands.
    pub fn new(params: CharacterParams, abilities: Abilities, pos: Vec2) -> Self {
        Self {
            params,
            abilities,
            pos,
            vel: Vec2::ZERO,
            facing: Side::Right,
            size: params.hitbox_px(),
            state: MoveState::Airborne,
            support: None,
            timers: Timers::default(),
            air_dash: true,
            double_jump: true,
            drop_y: None,
            dash_down: false,
            dive_pierce: false,
            superdash_was_held: false,
            behind: 0.0,
            behind_out: 0.0,
        }
    }

    /// Hitbox in px.
    pub fn body(&self) -> RectF {
        RectF::from_feet(self.pos, self.size)
    }

    pub fn is_grounded(&self) -> bool {
        self.support.is_some()
    }

    /// Dash usable right now (ability, cooldown and air-dash budget).
    pub fn can_dash(&self) -> bool {
        self.abilities.dash && self.timers.dash_cooldown <= 0.0 && (self.is_grounded() || self.air_dash)
    }

    /// Double jump still available this airtime.
    pub fn can_double_jump(&self) -> bool {
        self.abilities.double_jump && self.double_jump
    }

    /// The next dash would be a shadow dash (passes through window walls).
    pub fn shadow_dash_ready(&self) -> bool {
        self.abilities.shadow_dash && self.timers.shadow_cooldown <= 0.0
    }

    /// Where the knight is in a step behind screen content, if in one.
    pub fn behind_phase(&self) -> Option<BehindPhase> {
        if self.behind > 0.0 {
            Some(if self.behind < BEHIND_TURN_TIME { BehindPhase::TurningIn } else { BehindPhase::Behind })
        } else {
            (self.behind_out > 0.0).then_some(BehindPhase::TurningOut)
        }
    }

    /// Advance one tick.
    pub fn step(&mut self, input: &MoveInput, terrain: &Terrain, dt: f32, events: &mut Vec<MoveEvent>) {
        if !(dt > 0.0 && dt.is_finite()) {
            return;
        }
        let x_in = if input.x.is_finite() { input.x.clamp(-1.0, 1.0) } else { 0.0 };
        let superdash_pressed = input.superdash_held && !self.superdash_was_held;
        self.superdash_was_held = input.superdash_held;
        self.timers.tick(dt);
        if input.jump_pressed {
            self.timers.jump_buffer = self.params.jump_buffer_time;
        }
        if input.dash_pressed {
            self.timers.dash_buffer = self.params.dash_queue_time;
        }
        let start_facing = self.facing;

        // The ignore rule uses the body at the start of the step.
        let mut q = Query::new(terrain, self.body(), self.params.step_height_px(), self.drop_y);
        self.control(input, x_in, superdash_pressed, &q, dt, events);
        q.drop_y = self.drop_y;
        self.move_and_collide(&q, dt, events);
        self.resolve_contacts(x_in, &q, events);
        self.clamp_to_bounds(terrain.bounds);
        if self.support.is_none() {
            self.behind = 0.0;
            self.behind_out = 0.0;
        }
        if self.drop_y.is_some_and(|y| self.pos.y > y + 1.0 || self.support.is_some()) {
            self.drop_y = None;
        }
        if self.facing != start_facing {
            events.push(MoveEvent::TurnedAround);
        }
    }

    // ------------------------------------------------------------------ control

    fn control(
        &mut self,
        input: &MoveInput,
        x_in: f32,
        superdash_pressed: bool,
        q: &Query,
        dt: f32,
        events: &mut Vec<MoveEvent>,
    ) {
        let p = self.params;
        let dash_dir = Side::of(x_in).unwrap_or(self.facing);
        match self.state {
            MoveState::Grounded => {
                if self.behind_step(input, x_in, dt) {
                    return;
                }
                self.timers.down_held = if input.down { self.timers.down_held + dt } else { 0.0 };
                if input.dive_pressed && self.abilities.dive && q.can_drop_through(self.body()) {
                    self.drop_y = Some(self.pos.y);
                    self.support = None;
                    self.start_dive(input.dive_dark, events);
                    return;
                }
                if self.timers.down_held >= p.drop_hold_time && q.can_drop_through(self.body()) {
                    self.timers.down_held = 0.0;
                    self.drop_through(events);
                    return;
                }
                if self.timers.dash_buffer > 0.0 && self.can_dash() {
                    self.start_dash(dash_dir, events);
                    return;
                }
                if superdash_pressed && self.abilities.superdash {
                    self.start_charge(None, events);
                    return;
                }
                if self.timers.jump_buffer > 0.0 {
                    if input.down && q.can_drop_through(self.body()) {
                        self.drop_through(events);
                    } else {
                        self.start_jump(events);
                    }
                }
                self.steer(x_in, input.gait, dt);
            }
            MoveState::Airborne => {
                if input.dive_pressed && self.abilities.dive {
                    self.start_dive(input.dive_dark, events);
                    return;
                }
                if self.timers.dash_buffer > 0.0 && self.can_dash() {
                    if input.down && self.abilities.down_dash {
                        self.start_down_dash(events);
                    } else {
                        self.start_dash(dash_dir, events);
                    }
                    return;
                }
                if self.timers.jump_buffer > 0.0 {
                    if self.timers.coyote > 0.0 {
                        self.start_jump(events);
                    } else if let Some(wall) = self.wall_jump_side(x_in, q) {
                        self.start_wall_jump(wall, x_in, events);
                    } else if self.can_double_jump() {
                        self.start_double_jump(events);
                    }
                }
                let t = &mut self.timers;
                if t.jump_active && !input.jump_held && !input.jump_pressed && t.jump_elapsed >= p.jump_min_time {
                    // Early release cuts the rise; HK's MIN_JUMP_SPEED is what is left.
                    t.jump_active = false;
                    t.jump_hold = 0.0;
                    self.vel.y = self.vel.y.max(-p.px(p.min_jump_speed));
                }
                self.steer(x_in, input.gait, dt);
            }
            MoveState::WallSlide { side } => {
                if input.dive_pressed && self.abilities.dive {
                    events.push(MoveEvent::WallSlideEnd);
                    self.start_dive(input.dive_dark, events);
                    return;
                }
                if self.timers.dash_buffer > 0.0 && self.can_dash() {
                    events.push(MoveEvent::WallSlideEnd);
                    self.start_dash(side.opposite(), events);
                    return;
                }
                if superdash_pressed && self.abilities.superdash {
                    self.start_charge(Some(side), events);
                    return;
                }
                if self.timers.jump_buffer > 0.0 && self.abilities.wall_jump {
                    self.start_wall_jump(side, x_in, events);
                    return;
                }
                if Side::of(x_in) == Some(side.opposite()) {
                    self.timers.unstick += dt;
                    if self.timers.unstick >= p.wall_sticky_time {
                        events.push(MoveEvent::WallSlideEnd);
                        self.state = MoveState::Airborne;
                        self.steer(x_in, input.gait, dt);
                        return;
                    }
                } else {
                    self.timers.unstick = 0.0;
                }
                self.facing = side.opposite();
                self.vel = Vec2::new(0.0, p.wall_slide_speed_px());
            }
            MoveState::Dashing { .. } => {
                // Velocity is fixed for the whole dash; duration handled in `move_and_collide`.
                self.vel = if self.dash_down {
                    Vec2::new(0.0, p.dash_speed_px())
                } else {
                    Vec2::new(self.facing.sign() * p.dash_speed_px(), 0.0)
                };
            }
            MoveState::Dive { phase, .. } => self.control_dive(phase, input, dt, events),
            MoveState::SuperdashCharge { wall } => {
                self.vel = Vec2::ZERO;
                let was_charged = self.timers.charge >= p.superdash_charge_time;
                self.timers.charge += dt;
                let charged = self.timers.charge >= p.superdash_charge_time;
                if charged && !was_charged {
                    events.push(MoveEvent::SuperdashCharged);
                }
                if let Some(side) = wall {
                    self.facing = side.opposite();
                } else if let Some(side) = Side::of(x_in) {
                    self.facing = side;
                }
                if !input.superdash_held {
                    if charged {
                        self.launch_superdash(events);
                    } else {
                        self.cancel_charge(events);
                    }
                }
            }
            MoveState::Superdash => {
                self.timers.superdash += dt;
                let cancel = (self.timers.jump_buffer > 0.0 || self.timers.dash_buffer > 0.0)
                    && self.timers.superdash >= p.superdash_cancel_time;
                if cancel {
                    self.timers.jump_buffer = 0.0;
                    self.timers.dash_buffer = 0.0;
                    self.vel = Vec2::ZERO;
                    self.state = self.rest_state();
                    events.push(MoveEvent::SuperdashEnded);
                } else {
                    self.vel = Vec2::new(self.facing.sign() * p.superdash_speed_px(), 0.0);
                }
            }
            MoveState::HardLanding => {
                self.vel = Vec2::ZERO;
                if self.timers.hard_landing <= 0.0 {
                    self.state = MoveState::Grounded;
                }
            }
        }
    }

    /// Accelerate towards the gait speed; update facing.
    /// Turning into / walking behind / turning out of the screen. True while it owns the
    /// tick (no jumping, dashing or dropping meanwhile).
    fn behind_step(&mut self, input: &MoveInput, x_in: f32, dt: f32) -> bool {
        if input.behind {
            self.behind_out = 0.0;
            self.behind += dt;
            if self.behind < BEHIND_TURN_TIME {
                self.vel.x = 0.0;
            } else {
                self.steer(x_in, Gait::Walk, dt);
            }
            return true;
        }
        if self.behind > 0.0 {
            self.behind = 0.0;
            self.behind_out = BEHIND_RETURN_TIME;
        }
        if self.behind_out > 0.0 {
            self.behind_out -= dt;
            self.vel.x = 0.0;
            return true;
        }
        false
    }

    fn steer(&mut self, x_in: f32, gait: Gait, dt: f32) {
        let p = self.params;
        if self.timers.wall_lock > 0.0 {
            return;
        }
        if let Some(side) = Side::of(x_in) {
            self.facing = side;
        }
        let target = x_in * p.gait_speed_px(gait);
        let v = self.vel.x;
        let grounded = self.state == MoveState::Grounded;
        let speeding_up = target != 0.0 && v * target >= 0.0 && target.abs() > v.abs();
        let rate = match (grounded, speeding_up) {
            (true, true) => p.ground_accel,
            (true, false) => p.ground_decel,
            (false, _) => p.air_accel,
        };
        let max_dv = p.px(rate) * dt;
        self.vel.x = v + (target - v).clamp(-max_dv, max_dv);
    }

    /// Wall usable for a wall jump while airborne (prefers the side pressed into).
    fn wall_jump_side(&self, x_in: f32, q: &Query) -> Option<Side> {
        if !self.abilities.wall_jump {
            return None;
        }
        let body = self.body();
        let preferred = Side::of(x_in).unwrap_or(self.facing);
        [preferred, preferred.opposite()].into_iter().find(|&s| q.touching_wall(body, s).is_some())
    }

    fn begin_rise(&mut self, speed_units: f32, hold: f32) {
        self.vel.y = -self.params.px(speed_units);
        let t = &mut self.timers;
        t.jump_hold = hold;
        t.jump_elapsed = 0.0;
        t.jump_active = true;
        t.jump_buffer = 0.0;
        t.coyote = 0.0;
        t.fall = 0.0;
        self.support = None;
        self.state = MoveState::Airborne;
    }

    fn start_jump(&mut self, events: &mut Vec<MoveEvent>) {
        self.begin_rise(self.params.jump_speed, self.params.jump_time);
        events.push(MoveEvent::Jumped);
    }

    fn start_double_jump(&mut self, events: &mut Vec<MoveEvent>) {
        self.double_jump = false;
        self.begin_rise(self.params.double_jump_speed, self.params.double_jump_time);
        events.push(MoveEvent::DoubleJumped);
    }

    fn start_wall_jump(&mut self, wall: Side, x_in: f32, events: &mut Vec<MoveEvent>) {
        let p = self.params;
        if matches!(self.state, MoveState::WallSlide { .. }) {
            events.push(MoveEvent::WallSlideEnd);
        }
        let away = wall.opposite();
        self.begin_rise(p.jump_speed, p.jump_time);
        self.vel.x = away.sign() * p.wall_jump_kickoff_px();
        self.facing = away;
        self.timers.wall_lock =
            if Side::of(x_in) == Some(away) { p.wall_jump_lock_short } else { p.wall_jump_lock_long };
        self.refill_air_moves();
        events.push(MoveEvent::WallJumped { wall });
    }

    fn start_dash(&mut self, dir: Side, events: &mut Vec<MoveEvent>) {
        let p = self.params;
        let shadow = self.shadow_dash_ready();
        if shadow {
            self.timers.shadow_cooldown = p.shadow_dash_cooldown;
        }
        if !self.is_grounded() {
            self.air_dash = false;
        }
        let t = &mut self.timers;
        t.dash = p.dash_time;
        t.dash_cooldown = p.dash_cooldown;
        t.dash_buffer = 0.0;
        t.jump_hold = 0.0;
        t.jump_active = false;
        t.wall_lock = 0.0;
        t.fall = 0.0;
        self.dash_down = false;
        self.facing = dir;
        self.vel = Vec2::new(dir.sign() * p.dash_speed_px(), 0.0);
        self.state = MoveState::Dashing { shadow };
        events.push(MoveEvent::DashStarted { shadow });
    }

    fn end_dash(&mut self, events: &mut Vec<MoveEvent>) {
        self.vel = Vec2::ZERO;
        self.state = self.rest_state();
        events.push(MoveEvent::DashEnded);
    }

    fn start_charge(&mut self, wall: Option<Side>, events: &mut Vec<MoveEvent>) {
        self.timers.charge = 0.0;
        self.vel = Vec2::ZERO;
        self.state = MoveState::SuperdashCharge { wall };
        events.push(MoveEvent::SuperdashCharging);
    }

    fn cancel_charge(&mut self, events: &mut Vec<MoveEvent>) {
        self.state = match self.state {
            MoveState::SuperdashCharge { wall: Some(side) } => MoveState::WallSlide { side },
            _ => self.rest_state(),
        };
        events.push(MoveEvent::SuperdashCancelled);
    }

    fn launch_superdash(&mut self, events: &mut Vec<MoveEvent>) {
        let t = &mut self.timers;
        t.superdash = 0.0;
        t.fall = 0.0;
        t.jump_buffer = 0.0;
        t.dash_buffer = 0.0;
        self.vel = Vec2::new(self.facing.sign() * self.params.superdash_speed_px(), 0.0);
        self.state = MoveState::Superdash;
        events.push(MoveEvent::SuperdashLaunched);
    }

    fn drop_through(&mut self, events: &mut Vec<MoveEvent>) {
        self.drop_y = Some(self.pos.y);
        self.support = None;
        self.state = MoveState::Airborne;
        self.vel.y = 0.0;
        self.timers.jump_buffer = 0.0;
        self.timers.coyote = 0.0;
        self.timers.fall = 0.0;
        events.push(MoveEvent::DroppedThrough);
    }

    /// Grounded if supported, else airborne.
    fn rest_state(&self) -> MoveState {
        if self.support.is_some() { MoveState::Grounded } else { MoveState::Airborne }
    }

    fn refill_air_moves(&mut self) {
        self.air_dash = true;
        self.double_jump = true;
    }

    // ------------------------------------------------------------------ motion

    fn move_and_collide(&mut self, q: &Query, dt: f32, events: &mut Vec<MoveEvent>) {
        let p = self.params;
        let mut dash_ends = false;
        let (dx, dy) = match self.state {
            MoveState::Airborne => {
                // Constant-speed jump phase, then gravity (exact for constant acceleration).
                let mut rest = dt;
                let mut dy = 0.0;
                if self.timers.jump_hold > 0.0 {
                    let h = self.timers.jump_hold.min(dt);
                    dy += self.vel.y * h;
                    self.timers.jump_hold -= h;
                    rest -= h;
                }
                if rest > 0.0 {
                    let v0 = self.vel.y;
                    let v1 = (v0 + p.gravity_px() * rest).min(p.max_fall_speed_px());
                    dy += 0.5 * (v0 + v1) * rest;
                    self.vel.y = v1;
                }
                if self.vel.y > 0.0 {
                    self.timers.fall += dt;
                    self.timers.jump_active = false;
                }
                (self.vel.x * dt, dy)
            }
            MoveState::Dashing { .. } => {
                let t = self.timers.dash.min(dt);
                self.timers.dash -= dt;
                dash_ends = self.timers.dash <= 0.0;
                if self.dash_down { (0.0, self.vel.y * t) } else { (self.vel.x * t, 0.0) }
            }
            _ => (self.vel.x * dt, self.vel.y * dt),
        };

        // Horizontal.
        let foot_tol = if self.support.is_some() { p.step_height_px() } else { 0.0 };
        let shadow = matches!(self.state, MoveState::Dashing { shadow: true });
        let behind = self.behind >= BEHIND_TURN_TIME;
        let (ax, hit) = q.sweep_x(self.body(), dx, shadow, behind, foot_tol);
        self.pos.x += ax;
        if hit {
            self.vel.x = 0.0;
            self.timers.wall_lock = 0.0;
            if self.state == MoveState::Superdash {
                self.state = self.rest_state();
                events.push(MoveEvent::SuperdashHitWall);
            }
        }

        // Vertical.
        if dy > 0.0 {
            let pierce = self.dive_pierce && matches!(self.state, MoveState::Dive { phase: DivePhase::Fall, .. });
            match q.sweep_down_where(self.body(), dy, |l| !(pierce && l.kind.is_one_way())) {
                Some(l) => {
                    self.pos.y = l.y;
                    self.land(l, events);
                }
                None => self.pos.y += dy,
            }
        } else {
            self.pos.y += dy;
        }

        if dash_ends && matches!(self.state, MoveState::Dashing { .. }) {
            self.end_dash(events);
        }
    }

    fn land(&mut self, l: Ledge, events: &mut Vec<MoveEvent>) {
        let p = self.params;
        self.support = Some(l.into());
        self.vel.y = 0.0;
        self.refill_air_moves();
        let t = &mut self.timers;
        let hard = t.fall > p.big_fall_time;
        let super_hard = t.fall > p.super_hard_fall_time;
        t.fall = 0.0;
        t.jump_hold = 0.0;
        t.jump_active = false;
        t.wall_lock = 0.0;
        if let MoveState::Airborne | MoveState::WallSlide { .. } = self.state {
            if matches!(self.state, MoveState::WallSlide { .. }) {
                events.push(MoveEvent::WallSlideEnd);
            }
            if hard {
                self.timers.hard_landing = if super_hard { p.super_hard_landing_time } else { p.hard_landing_time };
                self.vel.x = 0.0;
                self.state = MoveState::HardLanding;
            } else {
                self.state = MoveState::Grounded;
            }
            events.push(MoveEvent::Landed { hard });
            if super_hard {
                events.push(MoveEvent::SuperHardLanded);
            }
        } else {
            self.land_special(events);
        }
    }

    /// Post-move: keep / lose ground, start / end wall slides.
    fn resolve_contacts(&mut self, x_in: f32, q: &Query, events: &mut Vec<MoveEvent>) {
        let p = self.params;
        let body = self.body();
        if self.support.is_some() {
            let step = p.step_height_px();
            match q.find_ledge(body, step, step, |_| true) {
                Some(l) => {
                    self.pos.y = l.y;
                    self.support = Some(l.into());
                }
                None => self.lose_support(events),
            }
            return;
        }
        match self.state {
            MoveState::Airborne => {
                let pressed = Side::of(x_in).filter(|_| self.abilities.wall_jump && self.vel.y > 0.0);
                if let Some(side) = pressed
                    && q.touching_wall(body, side).is_some()
                {
                    self.state = MoveState::WallSlide { side };
                    self.vel = Vec2::new(0.0, self.vel.y.min(p.wall_slide_speed_px()));
                    self.facing = side.opposite();
                    let t = &mut self.timers;
                    t.unstick = 0.0;
                    t.fall = 0.0;
                    t.jump_hold = 0.0;
                    t.jump_active = false;
                    t.wall_lock = 0.0;
                    self.refill_air_moves();
                    events.push(MoveEvent::WallSlideStart { side });
                }
            }
            MoveState::WallSlide { side } if q.touching_wall(body, side).is_none() => {
                self.state = MoveState::Airborne;
                events.push(MoveEvent::WallSlideEnd);
            }
            MoveState::SuperdashCharge { wall: Some(side) } if q.touching_wall(body, side).is_none() => {
                self.state = MoveState::Airborne;
                events.push(MoveEvent::SuperdashCancelled);
            }
            _ => {}
        }
    }

    /// The ground vanished: fall (with coyote time if we were simply standing).
    fn lose_support(&mut self, events: &mut Vec<MoveEvent>) {
        self.support = None;
        events.push(MoveEvent::LostSupport);
        match self.state {
            MoveState::Grounded => {
                self.timers.coyote = self.params.coyote_time;
                self.state = MoveState::Airborne;
            }
            MoveState::HardLanding => self.state = MoveState::Airborne,
            MoveState::Dive { phase: DivePhase::Land, .. } => {
                self.state = MoveState::Airborne;
                events.push(MoveEvent::DiveEnded);
            }
            MoveState::SuperdashCharge { .. } => {
                self.state = MoveState::Airborne;
                events.push(MoveEvent::SuperdashCancelled);
            }
            // Dashes and superdashes keep flying horizontally.
            _ => {}
        }
    }

    fn clamp_to_bounds(&mut self, b: RectF) {
        if b.width() <= 0.0 || b.height() <= 0.0 {
            return;
        }
        let half = self.size.x * 0.5;
        if b.width() >= self.size.x {
            self.pos.x = self.pos.x.clamp(b.left + half, b.right - half);
        }
        if self.pos.y - self.size.y < b.top {
            self.pos.y = b.top + self.size.y;
            if self.vel.y < 0.0 {
                self.vel.y = 0.0;
                self.timers.jump_hold = 0.0;
                self.timers.jump_active = false;
            }
        }
        self.pos.y = self.pos.y.min(b.bottom);
    }

    // ------------------------------------------------------------------ moving platforms

    /// Call after the terrain was rebuilt from a new desktop snapshot.
    ///
    /// * Standing on a window that moved ([`MoveEvent::PlatformMoved`]): never carried. Moved
    ///   up or down: falls at once; moved sideways: stays put while it slides under him.
    ///   This includes content ledges, which belong to the window showing them.
    /// * Support closed / hidden / moved away: falls ([`MoveEvent::LostSupport`]).
    /// * A window appeared or moved so its top is at most `pop_up_height` above the feet:
    ///   stand on it. Deeper overlaps fall under the overlap-ignore rule.
    pub fn on_terrain_rebuilt(&mut self, old: &Terrain, new: &Terrain, events: &mut Vec<MoveEvent>) {
        let p = self.params;
        // The floor moved under him: he isn't glued to it. He drops into a free fall right
        // away; if the window only slid sideways he lands back on its top a tick later (and
        // it slides out from under him), if it moved up or down he falls.
        if let Some(owner) = self.support.and_then(|s| s.owner)
            && let (Some(a), Some(b)) = (old.window_rect(owner), new.window_rect(owner))
            && (a.left != b.left || a.top != b.top)
        {
            events.push(MoveEvent::PlatformMoved { owner });
            if a.top != b.top {
                self.clamp_to_bounds(new.bounds);
                self.lose_support(events);
                return;
            }
        }
        self.clamp_to_bounds(new.bounds);

        let body = self.body();
        let pop = p.pop_up_height_px();
        let step = p.step_height_px();
        let q = Query::new(new, body, pop, self.drop_y);
        // Ledges more than a step above the feet only count if their window just arrived
        // under the body (otherwise we may be inside it on purpose, or rising through it).
        // Content ledges never pop the body up: they appear wherever something is drawn.
        let arrived = |l: &Ledge| {
            l.y >= body.bottom - step
                || (l.kind != LedgeKind::Content && l.owner.is_some_and(|id| old.window_rect(id).is_none_or(|r| !r.overlaps(&body))))
        };
        if self.support.is_some() {
            match q.find_ledge(body, pop, step, arrived) {
                Some(l) => {
                    self.pos.y = l.y;
                    self.support = Some(l.into());
                }
                None => self.lose_support(events),
            }
        } else if matches!(self.state, MoveState::Airborne | MoveState::WallSlide { .. } | MoveState::Dive { phase: DivePhase::Fall, .. })
            && self.vel.y >= 0.0
            && let Some(l) = q.find_ledge(body, pop, EPS, |l| l.owner.is_some() && arrived(l))
        {
            self.pos.y = l.y;
            self.land(l, events);
        }
    }
}
