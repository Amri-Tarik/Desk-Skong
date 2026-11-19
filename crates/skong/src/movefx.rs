//! Movement polish on the app side: game effects for dashes, jumps, landings and dives,
//! plus the `--move-test` script (dive + turn transitions, for screenshots).
use skong_core::controller::{Character, MoveEvent, MoveInput, MoveState};
use skong_core::geom::Vec2;
use skong_core::particles as pt;

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
    particles(fx, events, ch);
}

/// The game's particle bits (procedural, `skong_core::particles`) next to the sprite
/// effects: dust from the feet on landings, jumps, grounded dashes, wall jumps and fast
/// runs; crystal shards while charging, launching and breaking a superdash.
fn particles(fx: &mut Fx, events: &[MoveEvent], ch: &Character) {
    let ppu = ch.params.px_per_unit;
    let dir = ch.facing.sign();
    let feet = ch.pos;
    let body = ch.body();
    for e in events {
        match e {
            MoveEvent::Landed { hard } => fx.particles(if *hard { pt::HARD_LAND_DUST } else { pt::LAND_DUST }, feet, dir, ppu),
            MoveEvent::Jumped => fx.particles(pt::JUMP_DUST, feet, dir, ppu),
            MoveEvent::DashStarted { shadow: false } if ch.is_grounded() && !ch.dashing_down() => fx.particles(pt::DASH_DUST, feet, dir, ppu),
            MoveEvent::WallJumped { wall } => {
                let x = if wall.sign() < 0.0 { body.left } else { body.right };
                fx.particles(pt::WALL_DUST, Vec2::new(x, body.bottom - 0.3 * body.height()), -wall.sign(), ppu);
            }
            MoveEvent::SuperdashLaunched => fx.particles(pt::SD_BURST_SHARDS, body.center(), dir, ppu),
            MoveEvent::SuperdashHitWall => {
                let front = if dir > 0.0 { body.right } else { body.left };
                fx.particles(pt::SD_BREAK_SHARDS, Vec2::new(front, body.center().y), dir, ppu);
            }
            _ => {}
        }
    }
    match ch.state {
        MoveState::SuperdashCharge { .. } => fx.stream("sd_charge", pt::SD_CHARGE_SHARDS, body.center(), dir, ppu),
        MoveState::Grounded if ch.vel.x.abs() >= 0.95 * ch.params.px(ch.params.run_speed) => fx.stream("run_dust", pt::RUN_DUST, feet, ch.vel.x.signum(), ppu),
        _ => {}
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
