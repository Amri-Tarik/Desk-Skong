//! Downward moves: Desolate Dive (antic hover → plunge → slam) and the Dashmaster
//! down-dash. Both reuse the swept vertical collision, so neither can tunnel.
//!
//! Dive rule: the plunge stops on the first ledge under the body, except that one-way and
//! content ledges are smashed through while `down` is held. Ground ledges (monitor /
//! work-area floor) and the bounds floor always stop it.
use super::collide::Query;
use super::{Character, MoveEvent, MoveInput, MoveState};
use crate::geom::Vec2;
use crate::terrain::Terrain;

/// Phase of a [`MoveState::Dive`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivePhase {
    /// Hovering in place, gathering the spell.
    Antic,
    /// Plunging straight down at `dive_speed`.
    Fall,
    /// Slammed into a ledge; recovering.
    Land,
}

impl Character {
    /// The current (or last) dash went straight down (Dashmaster).
    pub fn dashing_down(&self) -> bool {
        self.dash_down
    }

    /// Distance (px) from the feet down to the first ledge a fall or dive straight down
    /// would land on (`None`: nothing below, not even the bounds floor).
    pub fn drop_below(&self, terrain: &Terrain) -> Option<f32> {
        let body = self.body();
        let q = Query::new(terrain, body, self.params.step_height_px(), self.drop_y);
        q.find_ledge(body, 0.0, f32::MAX, |_| true).map(|l| l.y - self.pos.y)
    }

    /// Planner-less dive decision for this tick's `input`: dive when already falling
    /// straight (no real sideways steering, so the dive lands where the fall would) with at
    /// least [`AUTO_DIVE_UNITS`] of air below ([`AUTO_DIVE_EXCITED_UNITS`] when `excited`).
    /// Returns `Some(dark)`: Descending Dark when excited or the drop is huge.
    pub fn auto_dive(&self, terrain: &Terrain, input: &MoveInput, excited: bool) -> Option<bool> {
        let falling = self.state == MoveState::Airborne && self.vel.y > 0.0 && self.timers.jump_hold <= 0.0;
        let quiet = input.x.abs() < 0.25 && !input.jump_pressed && !input.dash_pressed && !input.superdash_held;
        if !(self.abilities.dive && falling && quiet) {
            return None;
        }
        let units = self.drop_below(terrain)? / self.params.px_per_unit;
        let need = if excited { AUTO_DIVE_EXCITED_UNITS } else { AUTO_DIVE_UNITS };
        (units >= need).then_some(excited || units >= AUTO_DIVE_DARK_UNITS)
    }
}

/// Free fall (game units) below a straight drop that makes the knight dive.
pub const AUTO_DIVE_UNITS: f32 = 5.0;
/// Same when the brain is excited.
pub const AUTO_DIVE_EXCITED_UNITS: f32 = 3.0;
/// Drops this deep use Descending Dark.
pub const AUTO_DIVE_DARK_UNITS: f32 = 12.0;

impl Character {

    pub(super) fn start_dive(&mut self, dark: bool, events: &mut Vec<MoveEvent>) {
        let t = &mut self.timers;
        t.dive = self.params.dive_antic_time;
        t.jump_hold = 0.0;
        t.jump_active = false;
        t.jump_buffer = 0.0;
        t.dash_buffer = 0.0;
        t.coyote = 0.0;
        t.wall_lock = 0.0;
        t.fall = 0.0;
        self.vel = Vec2::ZERO;
        self.dive_pierce = false;
        self.state = MoveState::Dive { phase: DivePhase::Antic, dark };
        events.push(MoveEvent::DiveStarted { dark });
    }

    pub(super) fn control_dive(&mut self, phase: DivePhase, input: &MoveInput, dt: f32, events: &mut Vec<MoveEvent>) {
        let MoveState::Dive { dark, .. } = self.state else { return };
        match phase {
            DivePhase::Antic => {
                self.vel = Vec2::ZERO;
                self.timers.dive -= dt;
                if self.timers.dive <= 0.0 {
                    self.state = MoveState::Dive { phase: DivePhase::Fall, dark };
                    self.vel = Vec2::new(0.0, self.params.dive_speed_px());
                    events.push(MoveEvent::DiveFalling);
                }
            }
            DivePhase::Fall => {
                self.vel = Vec2::new(0.0, self.params.dive_speed_px());
                self.dive_pierce = input.down;
            }
            DivePhase::Land => {
                self.vel = Vec2::ZERO;
                self.timers.dive -= dt;
                if self.timers.dive <= 0.0 {
                    self.state = self.rest_state();
                    events.push(MoveEvent::DiveEnded);
                }
            }
        }
    }

    pub(super) fn start_down_dash(&mut self, events: &mut Vec<MoveEvent>) {
        self.start_dash(self.facing, events);
        self.dash_down = true;
        self.vel = Vec2::new(0.0, self.params.dash_speed_px());
    }

    /// Landing while in a state other than airborne / wall slide: dive slams and down-dashes.
    pub(super) fn land_special(&mut self, events: &mut Vec<MoveEvent>) {
        match self.state {
            MoveState::Dive { phase: DivePhase::Fall, dark } => {
                self.vel = Vec2::ZERO;
                self.dive_pierce = false;
                self.timers.dive = self.params.dive_land_time;
                self.state = MoveState::Dive { phase: DivePhase::Land, dark };
                events.push(MoveEvent::DiveLanded);
            }
            MoveState::Dashing { .. } if self.dash_down => {
                self.vel = Vec2::ZERO;
                self.timers.dash = 0.0;
                self.state = MoveState::Grounded;
                events.push(MoveEvent::DashEnded);
                events.push(MoveEvent::Landed { hard: false });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::terrain::{Ledge, LedgeKind, WindowBox};

    const DT: f32 = 1.0 / 120.0;
    const FLOOR: f32 = 1040.0;

    /// Floor (Ground) at 1040 plus a one-way window top at y=600 over x 300..900.
    fn world(h: f32) -> Terrain {
        let floor = h - 40.0;
        let r = RectF::new(300.0, 600.0, 900.0, 800.0);
        Terrain {
            windows: vec![WindowBox { id: 7, rect: r }],
            ledges: vec![
                Ledge { x0: 0.0, x1: 1920.0, y: floor, kind: LedgeKind::Ground, owner: None },
                Ledge { x0: r.left, x1: r.right, y: r.top, kind: LedgeKind::OneWay, owner: Some(7) },
            ],
            bounds: RectF::new(0.0, 0.0, 1920.0, h),
            ..Default::default()
        }
    }

    fn knight(x: f32, y: f32) -> Character {
        Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(x, y))
    }

    fn run(c: &mut Character, t: &Terrain, input: MoveInput, secs: f32, dt: f32) -> Vec<MoveEvent> {
        let mut ev = vec![];
        let mut first = true;
        for _ in 0..(secs / dt) as usize {
            let mut i = input;
            i.dive_pressed &= first;
            i.dash_pressed &= first;
            first = false;
            c.step(&i, t, dt, &mut ev);
        }
        ev
    }

    fn dive(down: bool) -> MoveInput {
        MoveInput { dive_pressed: true, down, ..Default::default() }
    }

    #[test]
    fn dive_hovers_plunges_and_stops_on_first_ledge() {
        let t = world(1080.0);
        let mut c = knight(600.0, 200.0);
        let mut ev = vec![];
        c.step(&dive(false), &t, DT, &mut ev);
        assert_eq!(c.state, MoveState::Dive { phase: DivePhase::Antic, dark: false });
        assert_eq!(ev, vec![MoveEvent::DiveStarted { dark: false }]);
        // Antic: hovering.
        run(&mut c, &t, MoveInput::default(), 0.2, DT);
        assert!((c.pos.y - 200.0).abs() < 0.01, "{c:?}");
        let ev = run(&mut c, &t, MoveInput::default(), 0.3, DT);
        assert!(ev.contains(&MoveEvent::DiveFalling) && ev.contains(&MoveEvent::DiveLanded), "{ev:?}");
        assert_eq!(c.pos.y, 600.0);
        assert!(!ev.iter().any(|e| matches!(e, MoveEvent::Landed { .. })));
        assert!(matches!(c.state, MoveState::Dive { phase: DivePhase::Land, .. }));
        let secs = c.params.dive_land_time + 0.05;
        let ev = run(&mut c, &t, MoveInput::default(), secs, DT);
        assert!(ev.contains(&MoveEvent::DiveEnded));
        assert_eq!(c.state, MoveState::Grounded);
        assert_eq!(c.pos.y, 600.0);
    }

    #[test]
    fn dive_holding_down_smashes_one_way_but_never_ground() {
        let t = world(1080.0);
        let mut c = knight(600.0, 200.0);
        run(&mut c, &t, dive(true), 1.0, DT);
        assert_eq!(c.pos.y, FLOOR, "{c:?}");
        assert!(matches!(c.state, MoveState::Dive { phase: DivePhase::Land, .. }) || c.state == MoveState::Grounded);
        // A solid Ground ledge mid-air stops a down-held dive too.
        let mut t2 = world(1080.0);
        t2.ledges.push(Ledge { x0: 0.0, x1: 1920.0, y: 400.0, kind: LedgeKind::Ground, owner: None });
        let mut c = knight(600.0, 200.0);
        run(&mut c, &t2, dive(true), 1.0, DT);
        assert_eq!(c.pos.y, 400.0);
    }

    #[test]
    fn dive_does_not_tunnel_with_huge_steps() {
        // 0.1 s steps move 320 px per tick: still stops exactly on the thin one-way ledge.
        let t = world(1080.0);
        let mut c = knight(600.0, 200.0);
        run(&mut c, &t, dive(false), 1.0, 0.1);
        assert_eq!(c.pos.y, 600.0, "{c:?}");
        let mut c = knight(600.0, 200.0);
        run(&mut c, &t, dive(true), 1.0, 0.1);
        assert_eq!(c.pos.y, FLOOR, "{c:?}");
    }

    #[test]
    fn dive_from_a_one_way_ledge_goes_through_it_but_not_from_ground() {
        let t = world(1080.0);
        let mut c = knight(600.0, 600.0);
        run(&mut c, &t, MoveInput::default(), 0.1, DT);
        assert!(c.is_grounded());
        run(&mut c, &t, dive(false), 1.0, DT);
        assert_eq!(c.pos.y, FLOOR, "dives through the ledge it stood on: {c:?}");
        // On monitor ground the press is ignored.
        let ev = run(&mut c, &t, dive(false), 1.0, DT);
        assert!(!ev.iter().any(|e| matches!(e, MoveEvent::DiveStarted { .. })));
        assert_eq!(c.state, MoveState::Grounded);
    }

    #[test]
    fn dive_respects_ability() {
        let t = world(1080.0);
        let mut c = knight(600.0, 200.0);
        c.abilities.dive = false;
        let ev = run(&mut c, &t, dive(false), 0.05, DT);
        assert!(!ev.iter().any(|e| matches!(e, MoveEvent::DiveStarted { .. })));
    }

    #[test]
    fn down_dash_goes_straight_down_for_a_dash_length() {
        let t = world(3000.0);
        let mut c = knight(1500.0, 200.0);
        let input = MoveInput { dash_pressed: true, down: true, ..Default::default() };
        let mut ev = vec![];
        c.step(&input, &t, DT, &mut ev);
        assert!(ev.contains(&MoveEvent::DashStarted { shadow: true }) || ev.contains(&MoveEvent::DashStarted { shadow: false }));
        assert!(c.dashing_down() && matches!(c.state, MoveState::Dashing { .. }));
        let x0 = c.pos.x;
        let y0 = 200.0;
        let secs = c.params.dash_time + 0.02;
        let ev = run(&mut c, &t, MoveInput::default(), secs, DT);
        assert!(ev.contains(&MoveEvent::DashEnded));
        assert_eq!(c.state, MoveState::Airborne);
        assert_eq!(c.pos.x, x0);
        let dist = (c.pos.y - y0) / 64.0;
        assert!((dist - c.params.dash_distance()).abs() < 0.6, "{dist}");
        // Without down: a normal horizontal air dash.
        let mut c = knight(1500.0, 200.0);
        run(&mut c, &t, MoveInput { dash_pressed: true, ..Default::default() }, 0.1, DT);
        assert!(!c.dashing_down() && c.pos.x > 1500.0);
    }

    #[test]
    fn down_dash_lands_and_never_passes_ground() {
        let t = world(1080.0);
        let mut c = knight(1500.0, FLOOR - 100.0);
        let ev = run(&mut c, &t, MoveInput { dash_pressed: true, down: true, ..Default::default() }, 0.2, 0.05);
        assert!(ev.contains(&MoveEvent::DashEnded) && ev.contains(&MoveEvent::Landed { hard: false }), "{ev:?}");
        assert_eq!(c.pos.y, FLOOR);
        assert_eq!(c.state, MoveState::Grounded);
    }

    #[test]
    fn very_long_falls_are_super_hard() {
        let t = world(5000.0);
        let mut c = knight(1500.0, 100.0);
        let ev = run(&mut c, &t, MoveInput::default(), 6.0, DT);
        assert!(ev.contains(&MoveEvent::Landed { hard: true }) && ev.contains(&MoveEvent::SuperHardLanded), "{ev:?}");
        // A merely long fall is only hard.
        let t = world(1080.0 + 900.0);
        let mut c = knight(1500.0, 100.0);
        let ev = run(&mut c, &t, MoveInput::default(), 3.0, DT);
        assert!(ev.contains(&MoveEvent::Landed { hard: true }) && !ev.contains(&MoveEvent::SuperHardLanded), "{ev:?}");
    }

    #[test]
    fn auto_dive_only_on_long_straight_drops() {
        let t = world(1080.0);
        let quiet = MoveInput::default();
        // Falling at x=1500 from y=200: 840 px (13 units) to the floor.
        let mut c = knight(1500.0, 200.0);
        assert_eq!(c.auto_dive(&t, &quiet, false), None, "not falling yet");
        run(&mut c, &t, quiet, 0.1, DT);
        assert_eq!(c.auto_dive(&t, &quiet, false), Some(true), "huge drop: dark");
        assert_eq!(c.auto_dive(&t, &MoveInput { x: 1.0, ..quiet }, false), None, "steering sideways");
        // Above the window: 400 px = 6 units.
        let mut c = knight(600.0, 200.0);
        run(&mut c, &t, quiet, 0.1, DT);
        assert_eq!(c.auto_dive(&t, &quiet, false), Some(false));
        // Short drop (~3.5 units): only when excited.
        let mut c = knight(600.0, 370.0);
        run(&mut c, &t, quiet, 0.05, DT);
        assert_eq!(c.auto_dive(&t, &quiet, false), None);
        assert_eq!(c.auto_dive(&t, &quiet, true), Some(true));
        c.abilities.dive = false;
        assert_eq!(c.auto_dive(&t, &quiet, true), None);
    }

    #[test]
    fn drop_below_measures_the_first_landing() {
        let t = world(1080.0);
        let c = knight(600.0, 200.0);
        assert_eq!(c.drop_below(&t), Some(400.0));
        let c = knight(1500.0, 200.0);
        assert_eq!(c.drop_below(&t), Some(FLOOR - 200.0));
    }
}
