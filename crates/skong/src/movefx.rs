//! Movement polish on the app side: game effects for dashes, jumps, landings and dives,
//! plus the `--move-test` script (dive + turn transitions, for screenshots).
use skong_core::controller::{Character, MoveEvent, MoveInput, MoveState};
use skong_core::geom::Vec2;

use crate::fx::Fx;

/// Effects for this tick's movement events (positions from the game prefabs).
pub fn spawn(fx: &mut Fx, events: &[MoveEvent], ch: &Character, origin: Vec2) {
    let right = ch.facing.sign() > 0.0;
    let dark = matches!(ch.state, MoveState::Dive { dark: true, .. });
    for e in events {
        match e {
            MoveEvent::DashStarted { shadow } => match (shadow, ch.dashing_down()) {
                (true, true) => fx.spawn("shadow_burst_down", origin, right),
                (true, false) => {} // "shadow_burst" (app)
                (false, true) => fx.spawn("dash_burst_down", origin, right),
                (false, false) => {
                    fx.spawn("dash_burst", origin, right);
                    if ch.is_grounded() {
                        // The game spawns the puff prefab facing against the dash: left behind.
                        fx.spawn("dash_puff", origin, !right);
                    }
                }
            },
            MoveEvent::Jumped => fx.spawn("jump_puff", origin, right),
            MoveEvent::Landed { hard: true } => fx.spawn("hard_land_lines", origin, right),
            MoveEvent::DiveStarted { .. } => fx.spawn("q_charge", origin, right),
            MoveEvent::DiveFalling => {
                fx.stop("q_charge");
                fx.spawn(if dark { "q_trail_2" } else { "q_trail" }, origin, right);
            }
            MoveEvent::DiveLanded => {
                stop_trails(fx);
                let names: &[&str] = if dark { &["q_slam_2", "q_slam_lines", "q_pillar_2", "q_mega"] } else { &["q_slam", "q_pillar"] };
                for n in names {
                    fx.spawn(n, origin, right);
                }
            }
            MoveEvent::DiveEnded | MoveEvent::LostSupport => stop_trails(fx),
            _ => {}
        }
    }
}

fn stop_trails(fx: &mut Fx) {
    for n in ["q_charge", "q_trail", "q_trail_2"] {
        fx.stop(n);
    }
}

/// `--move-test`: every 8 s teleport above `spawn`, dive (dark every other cycle), then
/// turn in place left and right (TurnToIdle).
pub struct MoveTest {
    t: f32,
    cycle: u32,
    started: bool,
}

impl MoveTest {
    pub fn new() -> Self {
        Self { t: -1.0, cycle: 0, started: false }
    }

    pub fn input(&mut self, dt: f32, ch: &mut Character, spawn: Vec2) -> MoveInput {
        self.t += dt;
        if self.t < 0.0 {
            return MoveInput::default();
        }
        let fresh = !self.started || self.t >= 8.0;
        if self.t >= 8.0 {
            self.t = 0.0;
            self.cycle += 1;
        }
        self.started = true;
        let t = self.t;
        let mut i = MoveInput::default();
        if fresh {
            *ch = Character::new(ch.params, ch.abilities, spawn);
            log::info!("move-test: cycle {} at ({:.0},{:.0})", self.cycle, spawn.x, spawn.y);
        } else if (0.6..0.6 + dt * 1.5).contains(&t) {
            i.dive_pressed = true;
            i.dive_dark = self.cycle % 2 == 1;
        } else if (4.0..4.3).contains(&t) {
            i.x = -0.02;
        } else if (6.0..6.3).contains(&t) {
            i.x = 0.02;
        }
        i
    }
}
