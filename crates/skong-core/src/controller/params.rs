//! Movement tuning, in Hollow Knight **game units** and **seconds**.
//!
//! # Provenance
//!
//! Values marked *HK* were read from the game's serialized data (Unity 6000.0.61f1 build)
//! with `HK_TK2D_Exporter --dump-mono`:
//!
//! * `HeroController` on the `Knight` prefab in `resources.assets` (speeds, step counts, timers),
//! * the `Rigidbody2D` on the same GameObject (`m_GravityScale = 0.79`),
//! * the `BoxCollider2D` on the same GameObject (`m_Size = 0.5 x 1.28125`, `m_Offset.y = -0.75`),
//! * `Physics2DSettings` in `globalgamemanagers` (`m_Gravity.y = -60`),
//! * `TimeManager` in `globalgamemanagers` (fixed timestep 0.02 s, i.e. 50 Hz),
//! * the `Superdash` PlayMakerFSM variables on the Knight (`Charge Time`, `Superdash Speed`,
//!   `Cancelable Time`).
//!
//! `*_STEPS` constants count 50 Hz `FixedUpdate`s; they are converted to seconds here
//! (`steps * 0.02`) so the controller can run at any rate (we use 120 Hz).
//!
//! Values marked *chosen* have no serialized counterpart (they live in game code or do not
//! exist in HK) and were picked to feel HK-like.
//!
//! # Reachability (render scale 1, i.e. `px_per_unit = 64`)
//!
//! The path planner relies on these. All are pinned by tests in `controller::tests`
//! (simulated at 120 Hz; tolerances there are a few px). Scale linearly with `px_per_unit`.
//!
//! | move                                        | units  | px @64 |
//! |---------------------------------------------|--------|--------|
//! | full jump apex (feet rise)                  | 5.92   | 379    |
//! | tap jump apex (minimum hop)                 | 1.43   | 92     |
//! | jump + double jump at apex                  | 11.84  | 758    |
//! | full jump flat distance, Walk (6.0/s)       | 6.25   | 400    |
//! | full jump flat distance, Run (8.3/s)        | 8.65   | 553    |
//! | full jump flat distance, Sprint (10/s)      | 10.42  | 667    |
//! | jump + double jump flat distance, Walk      | 11.25  | 720    |
//! | jump + double jump flat distance, Run       | 15.56  | 996    |
//! | jump + double jump flat distance, Sprint    | 18.75  | 1200   |
//! | dash distance (ground or air)               | 5.0    | 320    |
//! | superdash speed                             | 30 /s  | 1920/s |
//! | wall-jump kick-off speed                    | 16 /s  | 1024/s |
//! | wall slide speed                            | 8 /s   | 512/s  |
//!
//! "Flat distance" = horizontal travel from take-off (already at full gait speed, jump held
//! throughout, double jump pressed at the apex) until the feet are back at take-off height.
//! Air time on flat ground: 1.07 s for a full jump, 1.80 s with the double jump.
//! A dash adds 5 units horizontally, suspends gravity for 0.25 s and zeroes vertical speed
//! (so a dash at the apex extends a Run jump by ~5 units + 0.25 s of hang).
//! Hard landings need > 1.1 s of falling: roughly a 17.8 unit (1140 px) drop.

use super::Gait;
use crate::geom::Vec2;

/// Seconds per HK `FixedUpdate` (TimeManager fixed timestep, *HK*).
pub const HK_FIXED_DT: f32 = 0.02;

/// Unlockable parts of the movement kit (all on by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Abilities {
    /// Monarch Wings.
    pub double_jump: bool,
    /// Mantis Claw (wall slide + wall jump).
    pub wall_jump: bool,
    /// Mothwing Cloak.
    pub dash: bool,
    /// Shade Cloak: some dashes pass through window walls.
    pub shadow_dash: bool,
    /// Crystal Heart.
    pub superdash: bool,
}

impl Default for Abilities {
    fn default() -> Self {
        Self { double_jump: true, wall_jump: true, dash: true, shadow_dash: true, superdash: true }
    }
}

impl Abilities {
    /// Nothing unlocked: walk and jump only.
    pub const fn none() -> Self {
        Self { double_jump: false, wall_jump: false, dash: false, shadow_dash: false, superdash: false }
    }
}

/// Movement tuning. Distances in game units, times in seconds; use the `*_px` accessors
/// (or [`CharacterParams::px`]) to get virtual-desktop pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterParams {
    /// Pixels per game unit: render scale x 64 (tk2d Knight sprites are 64 px/unit).
    pub px_per_unit: f32,

    /// Hitbox width x height (*HK* BoxCollider2D `m_Size`).
    pub hitbox: Vec2,

    /// *HK* `WALK_SPEED`.
    pub walk_speed: f32,
    /// *HK* `RUN_SPEED`.
    pub run_speed: f32,
    /// *HK* `RUN_SPEED_CH` (Sprintmaster run speed; HK has no plain sprint).
    pub sprint_speed: f32,
    /// Ground acceleration towards the target speed (*chosen*; HK sets velocity instantly).
    pub ground_accel: f32,
    /// Ground deceleration when stopping / reversing (*chosen*).
    pub ground_decel: f32,
    /// Air acceleration (*chosen*; HK air control is also instant).
    pub air_accel: f32,

    /// Downward acceleration: Physics2D gravity 60 x Rigidbody2D gravity scale 0.79 (*HK*).
    pub gravity: f32,
    /// *HK* `MAX_FALL_VELOCITY`.
    pub max_fall_speed: f32,

    /// *HK* `JUMP_SPEED`: upward speed held constant while the jump button is held.
    pub jump_speed: f32,
    /// *HK* `JUMP_STEPS` (9) x 0.02 s: maximum duration of the constant-speed phase.
    pub jump_time: f32,
    /// *HK* `JUMP_STEPS_MIN` (4) x 0.02 s: a tap still rises at full speed this long.
    pub jump_min_time: f32,
    /// *HK* `MIN_JUMP_SPEED`: upward speed left after releasing jump early.
    pub min_jump_speed: f32,
    /// Double-jump upward speed (*chosen* = `JUMP_SPEED`).
    pub double_jump_speed: f32,
    /// *HK* `DOUBLE_JUMP_STEPS` (9) x 0.02 s.
    pub double_jump_time: f32,
    /// Jump still allowed this long after walking off a ledge (*chosen*).
    pub coyote_time: f32,
    /// A jump press is remembered this long before landing (*chosen*).
    pub jump_buffer_time: f32,

    /// *HK* `WALLSLIDE_SPEED` (magnitude).
    pub wall_slide_speed: f32,
    /// *HK* `WJ_KICKOFF_SPEED`: horizontal speed away from the wall.
    pub wall_jump_kickoff: f32,
    /// *HK* `WJLOCK_STEPS_SHORT` (5) x 0.02 s: input lock when already pressing away.
    pub wall_jump_lock_short: f32,
    /// *HK* `WJLOCK_STEPS_LONG` (10) x 0.02 s: input lock otherwise.
    pub wall_jump_lock_long: f32,
    /// *HK* `WALL_STICKY_STEPS` (3) x 0.02 s: pressing away this long detaches from a wall.
    pub wall_sticky_time: f32,

    /// *HK* `DASH_SPEED`.
    pub dash_speed: f32,
    /// *HK* `DASH_TIME`.
    pub dash_time: f32,
    /// *HK* `DASH_COOLDOWN`.
    pub dash_cooldown: f32,
    /// *HK* `SHADOW_DASH_COOLDOWN`: a dash passes through walls at most this often.
    pub shadow_dash_cooldown: f32,
    /// *HK* `DASH_QUEUE_STEPS` (10) x 0.02 s: a dash press is remembered this long.
    pub dash_queue_time: f32,

    /// *HK* Superdash FSM `Superdash Speed`.
    pub superdash_speed: f32,
    /// *HK* Superdash FSM `Charge Time`.
    pub superdash_charge_time: f32,
    /// *HK* Superdash FSM `Cancelable Time`: flight can be cancelled after this long.
    pub superdash_cancel_time: f32,

    /// *HK* `BIG_FALL_TIME`: falling longer than this ends in a hard landing.
    pub big_fall_time: f32,
    /// *HK* `HARD_LANDING_TIME`: stun duration.
    pub hard_landing_time: f32,

    /// Ledges this far above the feet are stepped onto while grounded (*chosen*).
    pub step_height: f32,
    /// A window appearing/rising under the body by at most this much lifts the
    /// character onto it (*chosen*).
    pub pop_up_height: f32,
}

impl CharacterParams {
    /// Hollow Knight's Knight. `px_per_unit` = render scale x 64.
    pub fn hollow_knight(px_per_unit: f32) -> Self {
        let steps = |n: f32| n * HK_FIXED_DT;
        Self {
            px_per_unit,
            hitbox: Vec2::new(0.5, 1.28125),
            walk_speed: 6.0,
            run_speed: 8.3,
            sprint_speed: 10.0,
            ground_accel: 120.0,
            ground_decel: 160.0,
            air_accel: 90.0,
            gravity: 60.0 * 0.79,
            max_fall_speed: 20.0,
            jump_speed: 16.65,
            jump_time: steps(9.0),
            jump_min_time: steps(4.0),
            min_jump_speed: 3.0,
            double_jump_speed: 16.65,
            double_jump_time: steps(9.0),
            coyote_time: 0.08,
            jump_buffer_time: 0.1,
            wall_slide_speed: 8.0,
            wall_jump_kickoff: 16.0,
            wall_jump_lock_short: steps(5.0),
            wall_jump_lock_long: steps(10.0),
            wall_sticky_time: steps(3.0),
            dash_speed: 20.0,
            dash_time: 0.25,
            dash_cooldown: 0.6,
            shadow_dash_cooldown: 1.5,
            dash_queue_time: steps(10.0),
            superdash_speed: 30.0,
            superdash_charge_time: 0.8,
            superdash_cancel_time: 0.2,
            big_fall_time: 1.1,
            hard_landing_time: 0.8,
            step_height: 0.1,
            pop_up_height: 0.5,
        }
    }

    /// Game units -> pixels.
    #[inline]
    pub fn px(&self, units: f32) -> f32 {
        units * self.px_per_unit
    }

    /// Hitbox size in pixels.
    pub fn hitbox_px(&self) -> Vec2 {
        self.hitbox * self.px_per_unit
    }

    /// Gait speed in game units / s.
    pub fn gait_speed(&self, gait: Gait) -> f32 {
        match gait {
            Gait::Walk => self.walk_speed,
            Gait::Run => self.run_speed,
            Gait::Sprint => self.sprint_speed,
        }
    }

    /// Gait speed in px / s.
    pub fn gait_speed_px(&self, gait: Gait) -> f32 {
        self.px(self.gait_speed(gait))
    }

    pub fn gravity_px(&self) -> f32 {
        self.px(self.gravity)
    }

    pub fn max_fall_speed_px(&self) -> f32 {
        self.px(self.max_fall_speed)
    }

    pub fn jump_speed_px(&self) -> f32 {
        self.px(self.jump_speed)
    }

    pub fn dash_speed_px(&self) -> f32 {
        self.px(self.dash_speed)
    }

    pub fn superdash_speed_px(&self) -> f32 {
        self.px(self.superdash_speed)
    }

    pub fn wall_slide_speed_px(&self) -> f32 {
        self.px(self.wall_slide_speed)
    }

    pub fn wall_jump_kickoff_px(&self) -> f32 {
        self.px(self.wall_jump_kickoff)
    }

    pub fn step_height_px(&self) -> f32 {
        self.px(self.step_height)
    }

    pub fn pop_up_height_px(&self) -> f32 {
        self.px(self.pop_up_height)
    }

    /// Analytic feet rise of a held jump (constant-speed phase + ballistic), in units.
    pub fn jump_height(&self) -> f32 {
        rise(self.jump_speed, self.jump_time, self.gravity)
    }

    /// Analytic feet rise of a tapped jump (released immediately), in units.
    pub fn min_jump_height(&self) -> f32 {
        self.jump_speed * self.jump_min_time + rise(self.min_jump_speed, 0.0, self.gravity)
    }

    /// Analytic feet rise of a held double jump, in units.
    pub fn double_jump_height(&self) -> f32 {
        rise(self.double_jump_speed, self.double_jump_time, self.gravity)
    }

    /// Horizontal distance covered by one dash, in units.
    pub fn dash_distance(&self) -> f32 {
        self.dash_speed * self.dash_time
    }
}

/// Height gained moving up at `speed` for `hold` seconds, then decelerating under `g`.
fn rise(speed: f32, hold: f32, g: f32) -> f32 {
    speed * hold + speed * speed / (2.0 * g)
}
