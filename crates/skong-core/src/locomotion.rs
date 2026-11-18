//! Chooses the Knight clip for the controller's state, the way HK's hero animation
//! controller does: one-shot "accent" clips (jump, land, turn, wall jump, dash…) are
//! started by movement events and play out unless the state makes them wrong;
//! otherwise a state-based base clip (idle / walk / run / sprint / fall / wall slide…).
use anyhow::{Context, Result};

use crate::anim::{AnimEvent, ClipId, ClipLibrary};
use crate::controller::{Character, CharacterParams, Gait, MoveEvent, MoveState};

/// Clip ids used by locomotion, resolved once.
#[derive(Clone, Debug)]
struct Clips {
    idle: ClipId,
    walk: ClipId,
    run: ClipId,
    sprint: ClipId,
    run_to_idle: ClipId,
    turn: ClipId,
    airborne: ClipId,
    fall: ClipId,
    land: ClipId,
    hard_land: ClipId,
    double_jump: ClipId,
    wall_slide: ClipId,
    walljump: ClipId,
    dash: ClipId,
    dash_to_idle: ClipId,
    shadow_dash: ClipId,
    sd_charge_ground: ClipId,
    sd_charge_wall: ClipId,
    sd_dash: ClipId,
    sd_hit_wall: ClipId,
    sd_air_brake: ClipId,
}

impl Clips {
    fn resolve(lib: &ClipLibrary) -> Result<Self> {
        let id = |n: &str| lib.id(n).with_context(|| format!("locomotion needs clip '{n}'"));
        Ok(Self {
            idle: id("Idle")?,
            walk: id("Walk")?,
            run: id("Run")?,
            sprint: id("Sprint")?,
            run_to_idle: id("Run To Idle")?,
            turn: id("Turn")?,
            airborne: id("Airborne")?,
            fall: id("Fall")?,
            land: id("Land")?,
            hard_land: id("HardLand")?,
            double_jump: id("Double Jump")?,
            wall_slide: id("Wall Slide")?,
            walljump: id("Walljump")?,
            dash: id("Dash")?,
            dash_to_idle: id("Dash To Idle")?,
            shadow_dash: id("Shadow Dash")?,
            sd_charge_ground: id("SD Charge Ground")?,
            sd_charge_wall: id("SD Wall Charge")?,
            sd_dash: id("SD Dash")?,
            sd_hit_wall: id("SD Hit Wall")?,
            sd_air_brake: id("SD Air Brake")?,
        })
    }
}

/// What the animator should play this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRequest {
    pub clip: ClipId,
    /// Restart even if already playing (re-triggered one-shots).
    pub restart: bool,
}

#[derive(Clone, Debug)]
pub struct Locomotion {
    clips: Clips,
    /// Accent clip currently allowed to play out.
    one_shot: Option<ClipId>,
    restart: bool,
    /// Seconds since the current jump started (Airborne clip covers the rise).
    jump_t: Option<f32>,
    airborne_len: f32,
    was_moving: bool,
    /// Ground speed thresholds (px/s) between walk/run and run/sprint.
    walk_run: f32,
    run_sprint: f32,
    moving_eps: f32,
}

impl Locomotion {
    pub fn new(lib: &ClipLibrary, params: &CharacterParams) -> Result<Self> {
        let clips = Clips::resolve(lib)?;
        let (w, r, s) = (params.gait_speed_px(Gait::Walk), params.gait_speed_px(Gait::Run), params.gait_speed_px(Gait::Sprint));
        Ok(Self {
            airborne_len: lib.get(clips.airborne).duration(),
            clips,
            one_shot: None,
            restart: false,
            jump_t: None,
            was_moving: false,
            walk_run: (w + r) * 0.5,
            run_sprint: (r + s) * 0.5,
            moving_eps: w * 0.15,
        })
    }

    /// Feed this tick's controller and animation events; returns the clip to play.
    pub fn update(&mut self, ch: &Character, move_events: &[MoveEvent], anim_events: &[AnimEvent], dt: f32) -> ClipRequest {
        let c = self.clips.clone();
        for e in anim_events {
            if let AnimEvent::Finished(id) = e
                && Some(*id) == self.one_shot
            {
                self.one_shot = None;
            }
        }
        if let Some(t) = self.jump_t.as_mut() {
            *t += dt;
        }
        for e in move_events {
            match e {
                MoveEvent::Jumped => {
                    self.jump_t = Some(0.0);
                    self.accent(None);
                }
                MoveEvent::WallJumped { .. } => {
                    self.jump_t = Some(0.0);
                    self.accent(Some(c.walljump));
                }
                MoveEvent::DoubleJumped => self.accent(Some(c.double_jump)),
                MoveEvent::Landed { hard: true } => self.accent(Some(c.hard_land)),
                MoveEvent::Landed { hard: false } => {
                    self.jump_t = None;
                    // Landing while running flows straight into the run cycle.
                    let running = ch.vel.x.abs() > self.moving_eps;
                    self.accent((!running).then_some(c.land));
                }
                MoveEvent::DashStarted { shadow } => self.accent(Some(if *shadow { c.shadow_dash } else { c.dash })),
                MoveEvent::DashEnded if ch.is_grounded() && ch.vel.x.abs() <= self.moving_eps => self.accent(Some(c.dash_to_idle)),
                MoveEvent::SuperdashHitWall => self.accent(Some(c.sd_hit_wall)),
                MoveEvent::SuperdashEnded if !ch.is_grounded() => self.accent(Some(c.sd_air_brake)),
                MoveEvent::TurnedAround if ch.is_grounded() && ch.vel.x.abs() <= self.moving_eps => self.accent(Some(c.turn)),
                MoveEvent::LostSupport | MoveEvent::DroppedThrough => self.jump_t = None,
                _ => {}
            }
        }

        let moving = ch.is_grounded() && ch.vel.x.abs() > self.moving_eps;
        if self.was_moving && !moving && matches!(ch.state, MoveState::Grounded) && self.one_shot.is_none() {
            self.accent(Some(c.run_to_idle));
        }
        self.was_moving = moving;

        // Drop accents that no longer fit the state.
        if let Some(a) = self.one_shot {
            let grounded_accent = [c.land, c.turn, c.run_to_idle, c.dash_to_idle, c.hard_land].contains(&a);
            let air_accent = [c.double_jump, c.walljump, c.sd_air_brake].contains(&a);
            let dash_accent = a == c.dash || a == c.shadow_dash;
            let ok = match ch.state {
                MoveState::Grounded => grounded_accent && !(moving && a != c.land),
                MoveState::HardLanding => a == c.hard_land,
                MoveState::Airborne => air_accent,
                MoveState::Dashing { .. } => dash_accent,
                MoveState::Superdash | MoveState::SuperdashCharge { .. } => a == c.sd_hit_wall,
                MoveState::WallSlide { .. } => false,
            };
            if !ok {
                self.one_shot = None;
            }
        }
        if let Some(a) = self.one_shot {
            let restart = std::mem::take(&mut self.restart);
            return ClipRequest { clip: a, restart };
        }

        let base = match ch.state {
            MoveState::Grounded if moving => {
                let v = ch.vel.x.abs();
                if v >= self.run_sprint {
                    c.sprint
                } else if v >= self.walk_run {
                    c.run
                } else {
                    c.walk
                }
            }
            MoveState::Grounded => c.idle,
            MoveState::HardLanding => c.hard_land,
            MoveState::Airborne => match self.jump_t {
                Some(t) if t < self.airborne_len => c.airborne,
                _ => c.fall,
            },
            MoveState::WallSlide { .. } => c.wall_slide,
            MoveState::Dashing { shadow } => {
                if shadow {
                    c.shadow_dash
                } else {
                    c.dash
                }
            }
            MoveState::SuperdashCharge { wall: None } => c.sd_charge_ground,
            MoveState::SuperdashCharge { wall: Some(_) } => c.sd_charge_wall,
            MoveState::Superdash => c.sd_dash,
        };
        let restart = std::mem::take(&mut self.restart);
        ClipRequest { clip: base, restart }
    }

    /// True while an accent clip owns the animation (acts should wait).
    pub fn busy(&self) -> bool {
        self.one_shot.is_some()
    }

    fn accent(&mut self, clip: Option<ClipId>) {
        self.one_shot = clip;
        self.restart = true;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::anim::Animator;
    use crate::assets::SpriteBank;
    use crate::controller::{Abilities, MoveInput};
    use crate::geom::{RectF, Vec2};
    use crate::terrain::{Ledge, LedgeKind, Terrain};

    fn setup() -> (ClipLibrary, Locomotion, Character, Terrain) {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = SpriteBank::load_metadata(&data).expect("bank");
        let lib = ClipLibrary::load(&data.join("animations/Knight.json"), &bank).expect("lib");
        let params = CharacterParams::hollow_knight(64.0);
        let loco = Locomotion::new(&lib, &params).expect("loco");
        let ch = Character::new(params, Abilities::default(), Vec2::new(500.0, 1000.0));
        let terrain = Terrain {
            ledges: vec![Ledge { x0: 0.0, x1: 3000.0, y: 1000.0, kind: LedgeKind::Ground, owner: None }],
            bounds: RectF::new(0.0, 0.0, 3000.0, 1080.0),
            ..Default::default()
        };
        (lib, loco, ch, terrain)
    }

    /// Simulate `secs` with constant input; returns the distinct clip names played.
    fn sim(lib: &ClipLibrary, loco: &mut Locomotion, ch: &mut Character, t: &Terrain, input: MoveInput, secs: f32) -> Vec<String> {
        let dt = 1.0 / 120.0;
        let mut anim = Animator::new(lib.id("Idle").expect("idle"));
        let (mut me, mut ae) = (Vec::new(), Vec::new());
        let mut seen: Vec<String> = Vec::new();
        let mut first = true;
        for _ in 0..(secs / dt) as usize {
            me.clear();
            let mut inp = input;
            inp.jump_pressed &= first;
            first = false;
            ch.step(&inp, t, dt, &mut me);
            let req = loco.update(ch, &me, &ae, dt);
            anim.play(lib, req.clip, req.restart);
            ae.clear();
            anim.update(lib, dt, &mut ae);
            let n = lib.get(anim.clip()).name.clone();
            if seen.last() != Some(&n) {
                seen.push(n);
            }
        }
        seen
    }

    #[test]
    fn gaits_pick_matching_cycles() {
        for (gait, clip) in [(Gait::Walk, "Walk"), (Gait::Run, "Run"), (Gait::Sprint, "Sprint")] {
            let (lib, mut loco, mut ch, t) = setup();
            let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { x: 1.0, gait, ..Default::default() }, 1.0);
            assert_eq!(seen.last().map(String::as_str), Some(clip), "{gait:?}: {seen:?}");
        }
    }

    #[test]
    fn jump_goes_airborne_fall_land() {
        let (lib, mut loco, mut ch, t) = setup();
        let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { jump_pressed: true, jump_held: true, ..Default::default() }, 2.5);
        let pos = |n: &str| seen.iter().position(|s| s == n);
        assert!(pos("Airborne") < pos("Fall") && pos("Fall") < pos("Land"), "{seen:?}");
        assert_eq!(seen.last().map(String::as_str), Some("Idle"));
    }
}
