//! Chooses the Knight clip for the controller's state, the way HK's hero animation
//! controller does: one-shot "accent" clips (jump, land, wall jump, dash…) are
//! started by movement events and play out unless the state makes them wrong;
//! otherwise a state-based base clip (idle / walk / run / sprint / fall / wall slide…).
//!
//! Standing still uses an *idle variant*: HK's `…ToIdle` clips (`TurnToIdle`,
//! `LookUpToIdle`, `GetUpToIdle`…) play a short transition and then loop the idle frames,
//! so turning in place, getting up or finishing an act flows into idle without a snap.
use anyhow::{Context, Result};

use crate::anim::{AnimEvent, ClipId, ClipLibrary};
use crate::controller::{Character, CharacterParams, BehindPhase, DivePhase, Gait, MoveEvent, MoveState};

/// Clip ids used by locomotion, resolved once.
#[derive(Clone, Debug)]
struct Clips {
    idle: ClipId,
    walk: ClipId,
    run: ClipId,
    sprint: ClipId,
    run_to_idle: ClipId,
    turn_to_idle: ClipId,
    turn_to_bg: ClipId,
    turn_from_bg: ClipId,
    look_up_to_idle: ClipId,
    look_down_to_idle: ClipId,
    get_up_to_idle: ClipId,
    collect_stand_to_idle: ClipId,
    airborne: ClipId,
    fall: ClipId,
    land: ClipId,
    hard_land: ClipId,
    super_hard_land: ClipId,
    double_jump: ClipId,
    wall_slide: ClipId,
    wall_slash: ClipId,
    slash: ClipId,
    up_slash: ClipId,
    down_slash: ClipId,
    walljump: ClipId,
    dash: ClipId,
    dash_to_idle: ClipId,
    dash_down: ClipId,
    dash_down_land: ClipId,
    shadow_dash: ClipId,
    shadow_dash_sharp: ClipId,
    shadow_dash_down: ClipId,
    shadow_dash_down_sharp: ClipId,
    sd_charge_ground: ClipId,
    sd_charge_wall: ClipId,
    sd_dash: ClipId,
    sd_hit_wall: ClipId,
    sd_air_brake: ClipId,
    quake_antic: ClipId,
    quake_fall: ClipId,
    quake_fall_2: ClipId,
    quake_land: ClipId,
    quake_land_2: ClipId,
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
            turn_to_idle: id("TurnToIdle")?,
            turn_to_bg: id("TurnToBG")?,
            turn_from_bg: id("TurnFromBG")?,
            look_up_to_idle: id("LookUpToIdle")?,
            look_down_to_idle: id("LookDownToIdle")?,
            get_up_to_idle: id("GetUpToIdle")?,
            collect_stand_to_idle: id("Collect StandToIdle")?,
            airborne: id("Airborne")?,
            fall: id("Fall")?,
            land: id("Land")?,
            hard_land: id("HardLand")?,
            super_hard_land: id("Super Hard Land")?,
            double_jump: id("Double Jump")?,
            wall_slide: id("Wall Slide")?,
            wall_slash: id("Wall Slash")?,
            slash: id("Slash")?,
            up_slash: id("UpSlash")?,
            down_slash: id("DownSlash")?,
            walljump: id("Walljump")?,
            dash: id("Dash")?,
            dash_to_idle: id("Dash To Idle")?,
            dash_down: id("Dash Down")?,
            dash_down_land: id("Dash Down Land")?,
            shadow_dash: id("Shadow Dash")?,
            shadow_dash_sharp: id("Shadow Dash Sharp")?,
            shadow_dash_down: id("Shadow Dash Down")?,
            shadow_dash_down_sharp: id("Shadow Dash Down Sharp")?,
            sd_charge_ground: id("SD Charge Ground")?,
            sd_charge_wall: id("SD Wall Charge")?,
            sd_dash: id("SD Dash")?,
            sd_hit_wall: id("SD Hit Wall")?,
            sd_air_brake: id("SD Air Brake")?,
            quake_antic: id("Quake Antic")?,
            quake_fall: id("Quake Fall")?,
            quake_fall_2: id("Quake Fall 2")?,
            quake_land: id("Quake Land")?,
            quake_land_2: id("Quake Land 2")?,
        })
    }

    /// Transition clips that end by looping idle (usable as the standing-still base).
    fn idle_variants(&self) -> [ClipId; 6] {
        [self.idle, self.turn_to_idle, self.look_up_to_idle, self.look_down_to_idle, self.get_up_to_idle, self.collect_stand_to_idle]
    }

    fn dash_clip(&self, shadow: bool, sharp: bool, down: bool) -> ClipId {
        match (shadow, sharp, down) {
            (false, _, false) => self.dash,
            (false, _, true) => self.dash_down,
            (true, false, false) => self.shadow_dash,
            (true, true, false) => self.shadow_dash_sharp,
            (true, false, true) => self.shadow_dash_down,
            (true, true, true) => self.shadow_dash_down_sharp,
        }
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
    /// Clip used while standing still (Idle or a `…ToIdle` transition looping idle).
    idle_variant: ClipId,
    /// The current hard landing is a super hard one.
    super_landing: bool,
    /// The current / last dash goes down.
    down_dash: bool,
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
            idle_variant: clips.idle,
            clips,
            one_shot: None,
            restart: false,
            jump_t: None,
            was_moving: false,
            super_landing: false,
            down_dash: false,
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
        let still = ch.is_grounded() && ch.vel.x.abs() <= self.moving_eps;
        let mut dash_ended = false;
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
                MoveEvent::Landed { hard: true } => {
                    self.super_landing = false;
                    self.accent(Some(c.hard_land));
                }
                MoveEvent::SuperHardLanded => {
                    // Impact pose for the stun, then get up into idle.
                    self.super_landing = true;
                    self.accent(Some(c.super_hard_land));
                    self.idle_variant = c.get_up_to_idle;
                }
                MoveEvent::Landed { hard: false } => {
                    self.jump_t = None;
                    if self.down_dash && dash_ended {
                        self.accent(Some(c.dash_down_land));
                    } else {
                        // Landing while running flows straight into the run cycle.
                        let running = ch.vel.x.abs() > self.moving_eps;
                        self.accent((!running).then_some(c.land));
                    }
                }
                MoveEvent::DashStarted { shadow } => {
                    self.down_dash = ch.dashing_down();
                    self.accent(Some(c.dash_clip(*shadow, ch.abilities.sharp_shadow, self.down_dash)));
                }
                MoveEvent::DashEnded => {
                    dash_ended = true;
                    if still && !self.down_dash {
                        self.accent(Some(c.dash_to_idle));
                    }
                }
                MoveEvent::SuperdashHitWall => self.accent(Some(c.sd_hit_wall)),
                MoveEvent::SuperdashEnded if !ch.is_grounded() => self.accent(Some(c.sd_air_brake)),
                MoveEvent::TurnedAround if still => self.idle_intro(c.turn_to_idle),
                MoveEvent::DiveStarted { .. } => {
                    self.jump_t = None;
                    self.accent(None);
                }
                MoveEvent::DiveLanded => {
                    let dark = matches!(ch.state, MoveState::Dive { dark: true, .. });
                    self.accent(Some(if dark { c.quake_land_2 } else { c.quake_land }));
                }
                MoveEvent::LostSupport | MoveEvent::DroppedThrough => self.jump_t = None,
                _ => {}
            }
        }

        let moving = ch.is_grounded() && ch.vel.x.abs() > self.moving_eps;
        if self.was_moving && !moving && matches!(ch.state, MoveState::Grounded) && self.one_shot.is_none() {
            self.accent(Some(c.run_to_idle));
        }
        self.was_moving = moving;
        if moving || !matches!(ch.state, MoveState::Grounded | MoveState::HardLanding) {
            self.idle_variant = c.idle;
        }

        // Drop accents that no longer fit the state.
        if let Some(a) = self.one_shot {
            let grounded_accent =
                [c.land, c.run_to_idle, c.dash_to_idle, c.hard_land, c.dash_down_land, c.quake_land, c.quake_land_2].contains(&a);
            let air_accent = [c.double_jump, c.walljump, c.sd_air_brake, c.slash, c.up_slash, c.down_slash].contains(&a);
            let dash_accent =
                [c.dash, c.shadow_dash, c.shadow_dash_sharp, c.dash_down, c.shadow_dash_down, c.shadow_dash_down_sharp].contains(&a);
            let ok = match ch.state {
                MoveState::Grounded => grounded_accent && !(moving && a != c.land),
                MoveState::HardLanding => a == c.hard_land || a == c.super_hard_land,
                MoveState::Airborne => air_accent,
                MoveState::Dashing { .. } => dash_accent,
                MoveState::Superdash | MoveState::SuperdashCharge { .. } => a == c.sd_hit_wall,
                MoveState::WallSlide { .. } => a == c.wall_slash,
                MoveState::Dive { .. } => a == c.quake_land || a == c.quake_land_2,
            };
            if !ok {
                self.one_shot = None;
            }
        }
        // Stepping behind screen content: face into the screen (the clip holds its last
        // frame while he walks behind the line), then turn back out.
        if let Some(phase) = ch.behind_phase() {
            self.one_shot = None;
            self.was_moving = false;
            let clip = if phase == BehindPhase::TurningOut { c.turn_from_bg } else { c.turn_to_bg };
            return ClipRequest { clip, restart: false };
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
            MoveState::Grounded => self.idle_variant,
            MoveState::HardLanding if self.super_landing => c.super_hard_land,
            MoveState::HardLanding => c.hard_land,
            MoveState::Airborne => match self.jump_t {
                Some(t) if t < self.airborne_len => c.airborne,
                _ => c.fall,
            },
            MoveState::WallSlide { .. } => c.wall_slide,
            MoveState::Dashing { shadow } => c.dash_clip(shadow, ch.abilities.sharp_shadow, self.down_dash),
            MoveState::SuperdashCharge { wall: None } => c.sd_charge_ground,
            MoveState::SuperdashCharge { wall: Some(_) } => c.sd_charge_wall,
            MoveState::Superdash => c.sd_dash,
            MoveState::Dive { phase: DivePhase::Antic, .. } => c.quake_antic,
            MoveState::Dive { phase: DivePhase::Fall, dark: false } => c.quake_fall,
            MoveState::Dive { phase: DivePhase::Fall, dark: true } => c.quake_fall_2,
            MoveState::Dive { phase: DivePhase::Land, dark: false } => c.quake_land,
            MoveState::Dive { phase: DivePhase::Land, dark: true } => c.quake_land_2,
        };
        let restart = std::mem::take(&mut self.restart);
        ClipRequest { clip: base, restart }
    }

    /// True while an accent clip owns the animation (acts should wait).
    pub fn busy(&self) -> bool {
        self.one_shot.is_some()
    }

    /// Slash while wall sliding ("Wall Slash"); dropped by the next update unless the
    /// character is on a wall.
    pub fn wall_slash(&mut self) {
        self.accent(Some(self.clips.wall_slash));
    }

    /// Slash in mid-air (`Slash` / `UpSlash` / `DownSlash`); dropped on landing.
    pub fn air_slash(&mut self, dir: crate::ai::AttackDir) {
        let c = &self.clips;
        let clip = match dir {
            crate::ai::AttackDir::Up => c.up_slash,
            crate::ai::AttackDir::Down => c.down_slash,
            crate::ai::AttackDir::Forward => c.slash,
        };
        self.accent(Some(clip));
    }

    /// The facing was flipped from outside the controller while standing (the AI turning
    /// to look at something): play the turn into idle instead of snapping.
    pub fn turned_in_place(&mut self) {
        self.idle_intro(self.clips.turn_to_idle);
    }

    /// An act just ended on `last_clip`. Returns the clip to continue with: the same clip
    /// if it is a `…ToIdle` transition (it keeps looping idle, no snap), else Idle.
    pub fn after_act(&mut self, last_clip: ClipId) -> ClipId {
        self.one_shot = None;
        self.restart = false;
        self.was_moving = false;
        self.idle_variant = if self.clips.idle_variants().contains(&last_clip) { last_clip } else { self.clips.idle };
        self.idle_variant
    }

    fn accent(&mut self, clip: Option<ClipId>) {
        self.one_shot = clip;
        self.restart = true;
    }

    /// Stand still through `clip` (a `…ToIdle` transition), restarted now.
    fn idle_intro(&mut self, clip: ClipId) {
        self.idle_variant = clip;
        self.one_shot = None;
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
            inp.dash_pressed &= first;
            inp.dive_pressed &= first;
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

    fn tall(h: f32) -> Terrain {
        Terrain {
            ledges: vec![
                Ledge { x0: 0.0, x1: 3000.0, y: h, kind: LedgeKind::Ground, owner: None },
                Ledge { x0: 0.0, x1: 3000.0, y: 400.0, kind: LedgeKind::OneWay, owner: None },
            ],
            bounds: RectF::new(0.0, 0.0, 3000.0, h + 40.0),
            ..Default::default()
        }
    }

    fn settle(lib: &ClipLibrary, loco: &mut Locomotion, ch: &mut Character, t: &Terrain) {
        sim(lib, loco, ch, t, MoveInput::default(), 1.0);
    }

    fn order(seen: &[String], names: &[&str]) -> bool {
        let pos: Vec<Option<usize>> = names.iter().map(|n| seen.iter().position(|s| s == n)).collect();
        pos.iter().all(Option::is_some) && pos.windows(2).all(|w| w[0] < w[1])
    }

    #[test]
    fn turning_in_place_flows_into_idle() {
        let (lib, mut loco, mut ch, t) = setup();
        settle(&lib, &mut loco, &mut ch, &t);
        // A tiny push the other way turns the knight without walking.
        let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { x: -0.02, ..Default::default() }, 2.0);
        assert_eq!(seen.last().map(String::as_str), Some("TurnToIdle"), "{seen:?}");
        assert!(!seen.iter().any(|s| s == "Turn"), "{seen:?}");
        // The AI flipping the facing while standing does the same.
        loco.turned_in_place();
        let req = loco.update(&ch, &[], &[], 1.0 / 120.0);
        assert_eq!(req, ClipRequest { clip: lib.id("TurnToIdle").unwrap(), restart: true });
        // Walking resets the idle variant.
        let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { x: 1.0, ..Default::default() }, 0.5);
        let seen2 = sim(&lib, &mut loco, &mut ch, &t, MoveInput::default(), 1.5);
        assert!(seen.iter().any(|s| s == "Run") && seen2.last().map(String::as_str) == Some("Idle"), "{seen:?} {seen2:?}");
    }

    #[test]
    fn acts_ending_on_to_idle_clips_keep_playing() {
        let (lib, mut loco, ch, _) = setup();
        for n in ["LookUpToIdle", "LookDownToIdle", "GetUpToIdle", "Collect StandToIdle", "TurnToIdle"] {
            let id = lib.id(n).unwrap();
            assert_eq!(loco.after_act(id), id, "{n}");
            let mut grounded = ch.clone();
            grounded.state = MoveState::Grounded;
            grounded.support = Some(crate::controller::Support { owner: None, kind: LedgeKind::Ground, y: 1000.0, x0: 0.0, x1: 3000.0 });
            assert_eq!(loco.update(&grounded, &[], &[], 0.01), ClipRequest { clip: id, restart: false }, "{n}");
        }
        assert_eq!(loco.after_act(lib.id("Get Off").unwrap()), lib.id("Idle").unwrap());
    }

    #[test]
    fn super_hard_landing_gets_up_into_idle() {
        let (lib, mut loco, _, _) = setup();
        let mut t = tall(6000.0);
        t.ledges.truncate(1);
        let mut ch = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(1500.0, 100.0));
        let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput::default(), 7.0);
        assert!(order(&seen, &["Fall", "Super Hard Land", "GetUpToIdle"]), "{seen:?}");
        assert!(!seen.iter().any(|s| s == "HardLand"), "{seen:?}");
        // A plain hard landing still plays HardLand.
        let (lib, mut loco, _, _) = setup();
        let mut t = tall(2000.0);
        t.ledges.truncate(1);
        let mut ch = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(1500.0, 100.0));
        let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput::default(), 4.0);
        assert!(order(&seen, &["Fall", "HardLand", "Idle"]), "{seen:?}");
    }

    #[test]
    fn down_dash_plays_dash_down_and_its_landing() {
        let (lib, mut loco, _, t) = setup();
        let mut ch = Character::new(CharacterParams::hollow_knight(64.0), Abilities { shadow_dash: false, ..Abilities::default() }, Vec2::new(1500.0, 850.0));
        let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { dash_pressed: true, down: true, ..Default::default() }, 1.5);
        assert!(order(&seen, &["Dash Down", "Dash Down Land", "Idle"]), "{seen:?}");
        // Shadow (and Sharp Shadow) variants.
        for (sharp, clip) in [(false, "Shadow Dash Down"), (true, "Shadow Dash Down Sharp")] {
            let (lib, mut loco, _, t) = setup();
            let mut ch = Character::new(CharacterParams::hollow_knight(64.0), Abilities { sharp_shadow: sharp, ..Abilities::default() }, Vec2::new(1500.0, 850.0));
            let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { dash_pressed: true, down: true, ..Default::default() }, 1.5);
            assert!(order(&seen, &[clip, "Dash Down Land"]), "{seen:?}");
        }
    }

    #[test]
    fn dive_plays_antic_fall_land() {
        for (dark, fall, land) in [(false, "Quake Fall", "Quake Land"), (true, "Quake Fall 2", "Quake Land 2")] {
            let (lib, mut loco, _, _) = setup();
            let t = tall(1000.0);
            let mut ch = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(1500.0, 100.0));
            let seen = sim(&lib, &mut loco, &mut ch, &t, MoveInput { dive_pressed: true, dive_dark: dark, down: true, ..Default::default() }, 2.5);
            assert!(order(&seen, &["Quake Antic", fall, land, "Idle"]), "{seen:?}");
            assert_eq!(ch.pos.y, 1000.0, "down held: smashed through the one-way ledge");
        }
    }

    #[test]
    fn wall_slash_only_on_a_wall() {
        let (lib, mut loco, ch, _) = setup();
        let mut on_wall = ch.clone();
        on_wall.state = MoveState::WallSlide { side: crate::controller::Side::Right };
        loco.wall_slash();
        assert_eq!(loco.update(&on_wall, &[], &[], 0.01).clip, lib.id("Wall Slash").unwrap());
        assert_eq!(loco.update(&on_wall, &[], &[AnimEvent::Finished(lib.id("Wall Slash").unwrap())], 0.01).clip, lib.id("Wall Slide").unwrap());
        loco.wall_slash();
        assert_ne!(loco.update(&ch, &[], &[], 0.01).clip, lib.id("Wall Slash").unwrap());
    }

    #[test]
    fn air_slashes_play_in_the_air_only() {
        use crate::ai::AttackDir;
        let (lib, mut loco, ch, _) = setup();
        let mut air = ch.clone();
        air.state = MoveState::Airborne;
        loco.air_slash(AttackDir::Down);
        assert_eq!(loco.update(&air, &[], &[], 0.01).clip, lib.id("DownSlash").unwrap());
        loco.air_slash(AttackDir::Up);
        assert_eq!(loco.update(&air, &[], &[], 0.01).clip, lib.id("UpSlash").unwrap());
        // Landed: the slash is dropped.
        let mut ground = ch.clone();
        ground.state = MoveState::Grounded;
        assert_ne!(loco.update(&ground, &[], &[], 0.01).clip, lib.id("UpSlash").unwrap());
    }
}
