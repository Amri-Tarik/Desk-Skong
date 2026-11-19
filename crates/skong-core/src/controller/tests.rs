//! Deterministic controller tests on synthetic terrains (render scale 1: 64 px/unit).
use super::*;
use crate::terrain::{Facing, Wall, WallKind, WindowBox};

const DT: f32 = 1.0 / 120.0;
const PPU: f32 = 64.0;
/// Monitor work-area bottom in [`world`].
const FLOOR: f32 = 1040.0;

/// One 1920x1080 monitor, taskbar 40 px.
fn world() -> Terrain {
    world_sized(1920.0, 1080.0)
}

fn world_sized(w: f32, h: f32) -> Terrain {
    let floor = h - 40.0;
    Terrain {
        generation: 1,
        windows: vec![],
        ledges: vec![Ledge { x0: 0.0, x1: w, y: floor, kind: LedgeKind::Ground, owner: None }],
        walls: vec![
            Wall { x: 0.0, y0: 0.0, y1: floor, face: Facing::Right, owner: None, kind: WallKind::Monitor },
            Wall { x: w, y0: 0.0, y1: floor, face: Facing::Left, owner: None, kind: WallKind::Monitor },
        ],
        bounds: RectF::new(0.0, 0.0, w, h),
    }
}

/// Window platform box: one-way top, outward-facing side walls (no occlusion clipping).
fn add_window(t: &mut Terrain, id: WindowId, r: RectF) {
    t.windows.push(WindowBox { id, rect: r });
    t.ledges.push(Ledge { x0: r.left, x1: r.right, y: r.top, kind: LedgeKind::OneWay, owner: Some(id) });
    t.walls.push(Wall { x: r.left, y0: r.top, y1: r.bottom, face: Facing::Left, owner: Some(id), kind: WallKind::Window });
    t.walls.push(Wall { x: r.right, y0: r.top, y1: r.bottom, face: Facing::Right, owner: Some(id), kind: WallKind::Window });
}

fn with_window(id: WindowId, r: RectF) -> Terrain {
    let mut t = world();
    add_window(&mut t, id, r);
    t
}

fn knight(x: f32, y: f32) -> Character {
    Character::new(CharacterParams::hollow_knight(PPU), Abilities::default(), Vec2::new(x, y))
}

/// Knight standing on terrain at `x` (settled by stepping).
fn standing(t: &Terrain, x: f32, y: f32) -> Character {
    let mut c = knight(x, y);
    let mut ev = vec![];
    for _ in 0..240 {
        c.step(&MoveInput::default(), t, DT, &mut ev);
        if c.state == MoveState::Grounded {
            return c;
        }
    }
    panic!("never landed: {c:?}");
}

fn idle() -> MoveInput {
    MoveInput::default()
}

fn right() -> MoveInput {
    MoveInput { x: 1.0, ..idle() }
}

fn left() -> MoveInput {
    MoveInput { x: -1.0, ..idle() }
}

fn press_jump(x: f32) -> MoveInput {
    MoveInput { x, jump_pressed: true, jump_held: true, ..idle() }
}

fn hold_jump(x: f32) -> MoveInput {
    MoveInput { x, jump_held: true, ..idle() }
}

/// Step `n` ticks with the same input; returns events.
fn run(c: &mut Character, t: &Terrain, input: MoveInput, n: usize) -> Vec<MoveEvent> {
    let mut ev = vec![];
    for _ in 0..n {
        c.step(&input, t, DT, &mut ev);
    }
    ev
}

/// Step until grounded (max `n` ticks); returns events.
fn run_until_grounded(c: &mut Character, t: &Terrain, input: MoveInput, n: usize) -> Vec<MoveEvent> {
    let mut ev = vec![];
    for _ in 0..n {
        c.step(&input, t, DT, &mut ev);
        if c.is_grounded() {
            break;
        }
    }
    ev
}

fn secs(s: f32) -> usize {
    (s / DT).round() as usize
}

fn count(ev: &[MoveEvent], e: MoveEvent) -> usize {
    ev.iter().filter(|&&x| x == e).count()
}

// ------------------------------------------------------------------ ledges

#[test]
fn falls_and_lands_on_window_ledge() {
    let t = with_window(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    let mut c = knight(1000.0, 300.0);
    let ev = run_until_grounded(&mut c, &t, idle(), 240);
    assert_eq!(c.pos.y, 600.0);
    assert_eq!(c.support.and_then(|s| s.owner), Some(1));
    assert!(ev.contains(&MoveEvent::Landed { hard: false }));
    assert_eq!(c.state, MoveState::Grounded);
}

#[test]
fn lands_on_monitor_ground() {
    let t = world();
    let c = standing(&t, 500.0, 100.0);
    assert_eq!(c.pos.y, FLOOR);
    let s = c.support.expect("support");
    assert_eq!((s.owner, s.kind), (None, LedgeKind::Ground));
}

#[test]
fn jumps_up_through_one_way_ledge_and_lands_on_it() {
    let t = with_window(1, RectF::new(900.0, 750.0, 1100.0, 850.0));
    let mut c = standing(&t, 1000.0, FLOOR);
    let mut ev = run(&mut c, &t, press_jump(0.0), 1);
    assert!(ev.contains(&MoveEvent::Jumped));
    ev.extend(run_until_grounded(&mut c, &t, hold_jump(0.0), 240));
    assert_eq!(c.pos.y, 750.0);
    assert_eq!(c.support.and_then(|s| s.owner), Some(1));
}

#[test]
fn walks_off_ledge_and_falls() {
    let t = with_window(1, RectF::new(800.0, 600.0, 1000.0, 900.0));
    let mut c = standing(&t, 950.0, 500.0);
    let ev = run(&mut c, &t, right(), secs(0.3));
    assert!(ev.contains(&MoveEvent::LostSupport));
    assert!(!c.is_grounded());
    run_until_grounded(&mut c, &t, idle(), 240);
    assert_eq!(c.pos.y, FLOOR);
}

#[test]
fn stands_across_touching_ledges() {
    let mut t = world();
    add_window(&mut t, 1, RectF::new(500.0, 800.0, 800.0, 1040.0));
    add_window(&mut t, 2, RectF::new(800.0, 800.0, 1100.0, 1040.0));
    let mut c = standing(&t, 600.0, 700.0);
    assert_eq!(c.support.and_then(|s| s.owner), Some(1));
    let mut ev = vec![];
    for _ in 0..secs(1.0) {
        c.step(&right(), &t, DT, &mut ev);
        assert_eq!(c.pos.y, 800.0, "dipped at x={}", c.pos.x);
        if c.pos.x > 1000.0 {
            break;
        }
    }
    assert!(c.pos.x > 1000.0, "blocked at {}", c.pos.x);
    assert_eq!(c.support.and_then(|s| s.owner), Some(2));
    assert!(!ev.contains(&MoveEvent::LostSupport));
    assert!(!ev.iter().any(|e| matches!(e, MoveEvent::Landed { .. })));
}

#[test]
fn steps_up_onto_slightly_higher_touching_ledge() {
    let mut t = world();
    add_window(&mut t, 1, RectF::new(500.0, 800.0, 800.0, 1040.0));
    add_window(&mut t, 2, RectF::new(800.0, 797.0, 1100.0, 1040.0));
    let mut c = standing(&t, 600.0, 700.0);
    let ev = run(&mut c, &t, right(), secs(0.8));
    assert!(c.pos.x > 1000.0);
    assert_eq!(c.pos.y, 797.0);
    assert!(!ev.contains(&MoveEvent::LostSupport));
}

// ------------------------------------------------------------------ walls

#[test]
fn window_walls_block_from_outside_on_both_sides() {
    let t = with_window(1, RectF::new(900.0, 800.0, 1100.0, 1040.0));
    let half = 16.0;

    let mut c = standing(&t, 700.0, FLOOR);
    run(&mut c, &t, right(), secs(1.0));
    assert_eq!(c.pos.x, 900.0 - half);

    let mut c = standing(&t, 1300.0, FLOOR);
    run(&mut c, &t, left(), secs(1.0));
    assert_eq!(c.pos.x, 1100.0 + half);
}

#[test]
fn monitor_walls_and_bounds_contain_the_body() {
    let t = world();
    let mut c = standing(&t, 100.0, FLOOR);
    run(&mut c, &t, left(), secs(1.0));
    assert_eq!(c.pos.x, 16.0);
    let mut c = standing(&t, 1800.0, FLOOR);
    run(&mut c, &t, right(), secs(1.0));
    assert_eq!(c.pos.x, 1904.0);

    // Bounds alone (no terrain segments) still clamp.
    let bare = Terrain { bounds: RectF::new(0.0, 0.0, 400.0, 300.0), ..Default::default() };
    let mut c = knight(200.0, 100.0);
    run(&mut c, &bare, right(), secs(2.0));
    assert_eq!(c.pos, Vec2::new(384.0, 300.0));
    assert!(c.is_grounded());
}

#[test]
fn window_edges_ignored_while_body_is_inside_it() {
    // Window opened on top of the knight: he walks out of it and is not caught by its top.
    let t = with_window(7, RectF::new(800.0, 900.0, 1200.0, 1040.0));
    let mut c = standing(&t, 1000.0, FLOOR);
    assert_eq!(c.support.and_then(|s| s.owner), None);
    let ev = run(&mut c, &t, right(), secs(1.0));
    assert!(c.pos.x > 1200.0 + 16.0, "stuck at {}", c.pos.x);
    assert_eq!(c.pos.y, FLOOR);
    assert!(!ev.contains(&MoveEvent::LostSupport));

    // And back in from the outside is blocked.
    run(&mut c, &t, left(), secs(1.0));
    assert_eq!(c.pos.x, 1200.0 + 16.0);
}

// ------------------------------------------------------------------ wall slide / jump

#[test]
fn wall_slide_then_wall_jump() {
    let t = with_window(1, RectF::new(1000.0, 300.0, 1400.0, 1040.0));
    let mut c = standing(&t, 900.0, FLOOR);
    let mut ev = run(&mut c, &t, press_jump(1.0), 1);
    for _ in 0..secs(1.0) {
        c.step(&hold_jump(1.0), &t, DT, &mut ev);
        if matches!(c.state, MoveState::WallSlide { .. }) {
            break;
        }
    }
    assert_eq!(c.state, MoveState::WallSlide { side: Side::Right });
    assert!(ev.contains(&MoveEvent::WallSlideStart { side: Side::Right }));
    assert_eq!(c.body().right, 1000.0);
    assert_eq!(c.facing, Side::Left);

    run(&mut c, &t, right(), 10);
    assert_eq!(c.vel.y, c.params.wall_slide_speed_px());

    let ev = run(&mut c, &t, press_jump(1.0), 1);
    assert!(ev.contains(&MoveEvent::WallJumped { wall: Side::Right }));
    assert!(ev.contains(&MoveEvent::WallSlideEnd));
    assert_eq!(c.vel.x, -c.params.wall_jump_kickoff_px());
    assert!(c.vel.y < 0.0);
    // Input is locked briefly: still moving away even though pressing into the wall.
    run(&mut c, &t, hold_jump(1.0), 6);
    assert!(c.vel.x < 0.0);
    assert!(c.body().right < 1000.0 - 20.0);
}

#[test]
fn wall_slide_ends_when_pressing_away() {
    let t = with_window(1, RectF::new(1000.0, 300.0, 1400.0, 1040.0));
    let mut c = standing(&t, 900.0, FLOOR);
    run(&mut c, &t, press_jump(1.0), 1);
    run(&mut c, &t, hold_jump(1.0), secs(0.7));
    assert!(matches!(c.state, MoveState::WallSlide { .. }));
    // Sticky for a few steps, then lets go.
    let ev = run(&mut c, &t, left(), 2);
    assert!(matches!(c.state, MoveState::WallSlide { .. }), "{ev:?}");
    let ev = run(&mut c, &t, left(), 10);
    assert!(ev.contains(&MoveEvent::WallSlideEnd));
    assert_eq!(c.state, MoveState::Airborne);
}

#[test]
fn no_wall_slide_without_mantis_claw() {
    let t = with_window(1, RectF::new(1000.0, 300.0, 1400.0, 1040.0));
    let mut c = standing(&t, 900.0, FLOOR);
    c.abilities.wall_jump = false;
    let mut ev = run(&mut c, &t, press_jump(1.0), 1);
    ev.extend(run_until_grounded(&mut c, &t, hold_jump(1.0), 240));
    assert!(!ev.iter().any(|e| matches!(e, MoveEvent::WallSlideStart { .. })));
}

// ------------------------------------------------------------------ jumps

#[test]
fn double_jump_only_once_per_airtime() {
    let t = world();
    let mut c = standing(&t, 960.0, FLOOR);
    let mut ev = run(&mut c, &t, press_jump(0.0), 1);
    ev.extend(run(&mut c, &t, hold_jump(0.0), secs(0.4)));
    ev.extend(run(&mut c, &t, press_jump(0.0), 1));
    ev.extend(run(&mut c, &t, hold_jump(0.0), secs(0.3)));
    ev.extend(run(&mut c, &t, press_jump(0.0), 1));
    assert_eq!(count(&ev, MoveEvent::DoubleJumped), 1);
    assert!(!c.can_double_jump());
    // Landing refills it.
    run_until_grounded(&mut c, &t, idle(), 600);
    assert!(c.can_double_jump());
}

#[test]
fn variable_jump_height() {
    let t = world();
    let apex = |hold_ticks: usize| {
        let mut c = standing(&t, 960.0, FLOOR);
        run(&mut c, &t, press_jump(0.0), 1);
        let mut top = c.pos.y;
        for i in 0..secs(1.5) {
            let input = if i < hold_ticks { hold_jump(0.0) } else { idle() };
            c.step(&input, &t, DT, &mut vec![]);
            top = top.min(c.pos.y);
        }
        FLOOR - top
    };
    let tap = apex(0);
    let mid = apex(12);
    let full = apex(120);
    assert!(tap < mid && mid < full, "{tap} {mid} {full}");
    let p = CharacterParams::hollow_knight(PPU);
    assert!((tap - p.px(p.min_jump_height())).abs() < 4.0, "tap {tap}");
    assert!((full - p.px(p.jump_height())).abs() < 4.0, "full {full}");
}

#[test]
fn jump_buffer_and_coyote_time() {
    let t = world();
    // Buffer: press shortly before touching down. (With wings an air press would
    // double-jump instead, as in HK, so the buffer matters once they are spent.)
    let mut c = knight(960.0, FLOOR - 30.0);
    c.abilities.double_jump = false;
    run(&mut c, &t, idle(), 1);
    c.vel.y = 600.0;
    let mut ev = run(&mut c, &t, press_jump(0.0), 1);
    assert!(!c.is_grounded());
    ev.extend(run(&mut c, &t, hold_jump(0.0), 10));
    assert!(ev.contains(&MoveEvent::Landed { hard: false }));
    assert!(ev.contains(&MoveEvent::Jumped));
    assert!(!ev.contains(&MoveEvent::DoubleJumped));

    // Coyote: jump just after walking off a ledge is a ground jump.
    let tw = with_window(1, RectF::new(800.0, 600.0, 1000.0, 900.0));
    let mut c = standing(&tw, 990.0, 500.0);
    let mut ev = vec![];
    for _ in 0..secs(0.5) {
        c.step(&right(), &tw, DT, &mut ev);
        if ev.contains(&MoveEvent::LostSupport) {
            break;
        }
    }
    run(&mut c, &tw, right(), 3);
    let ev = run(&mut c, &tw, press_jump(1.0), 1);
    assert!(ev.contains(&MoveEvent::Jumped), "{ev:?}");
    assert!(c.can_double_jump());
}

#[test]
fn drop_through_one_way_but_not_ground() {
    let t = with_window(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    let mut c = standing(&t, 1000.0, 500.0);
    let ev = run(&mut c, &t, MoveInput { down: true, ..press_jump(0.0) }, 1);
    assert!(ev.contains(&MoveEvent::DroppedThrough));
    assert!(!ev.contains(&MoveEvent::Jumped));
    run_until_grounded(&mut c, &t, MoveInput { down: true, ..idle() }, 240);
    assert_eq!(c.pos.y, FLOOR);

    // On monitor ground the same input is just a jump.
    let ev = run(&mut c, &t, MoveInput { down: true, ..press_jump(0.0) }, 1);
    assert!(ev.contains(&MoveEvent::Jumped));
    assert!(!ev.contains(&MoveEvent::DroppedThrough));
}

#[test]
fn holding_down_slips_through_one_way_but_not_ground() {
    let t = with_window(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    let mut c = standing(&t, 1000.0, 500.0);
    let hold = MoveInput { down: true, ..idle() };
    // A brief tap does nothing.
    let ev = run(&mut c, &t, hold, secs(0.1));
    assert!(!ev.contains(&MoveEvent::DroppedThrough));
    run(&mut c, &t, idle(), 1);
    // Holding past `drop_hold_time` slips through, then lands on the ground below.
    let ev = run(&mut c, &t, hold, secs(0.3));
    assert!(ev.contains(&MoveEvent::DroppedThrough), "{ev:?}");
    run_until_grounded(&mut c, &t, hold, 240);
    assert_eq!(c.pos.y, FLOOR);
    // Holding down on monitor ground never drops.
    let ev = run(&mut c, &t, hold, secs(1.0));
    assert!(!ev.contains(&MoveEvent::DroppedThrough));
    assert_eq!(c.pos.y, FLOOR);
}

#[test]
fn hard_landing_after_long_fall() {
    // Two stacked monitors: a long way down.
    let t = world_sized(1920.0, 2160.0);
    let mut c = knight(960.0, 100.0);
    let ev = run_until_grounded(&mut c, &t, idle(), 600);
    assert!(ev.contains(&MoveEvent::Landed { hard: true }));
    assert_eq!(c.state, MoveState::HardLanding);
    // Stunned: input ignored.
    let x = c.pos.x;
    run(&mut c, &t, right(), secs(0.5));
    assert_eq!(c.pos.x, x);
    run(&mut c, &t, idle(), secs(0.4));
    assert_eq!(c.state, MoveState::Grounded);

    // A normal jump does not hard-land.
    let ev = {
        let mut ev = run(&mut c, &t, press_jump(0.0), 1);
        ev.extend(run_until_grounded(&mut c, &t, hold_jump(0.0), 600));
        ev
    };
    assert!(ev.contains(&MoveEvent::Landed { hard: false }));
}

// ------------------------------------------------------------------ dash

#[test]
fn dash_distance_and_cooldown() {
    let t = world();
    let mut c = standing(&t, 200.0, FLOOR);
    c.abilities.shadow_dash = false;
    let x0 = c.pos.x;
    let mut ev = run(&mut c, &t, MoveInput { dash_pressed: true, ..idle() }, 1);
    ev.extend(run(&mut c, &t, idle(), secs(0.3)));
    assert_eq!(count(&ev, MoveEvent::DashStarted { shadow: false }), 1);
    assert!(ev.contains(&MoveEvent::DashEnded));
    let d = c.pos.x - x0;
    assert!((d - c.params.px(c.params.dash_distance())).abs() < 1.0, "dash {d}");

    // Pressed during cooldown (and beyond the 0.2 s queue): ignored.
    let mut ev = run(&mut c, &t, MoveInput { dash_pressed: true, ..idle() }, 1);
    ev.extend(run(&mut c, &t, idle(), secs(0.25)));
    assert!(!ev.iter().any(|e| matches!(e, MoveEvent::DashStarted { .. })));
    // Once cooled down it works again.
    run(&mut c, &t, idle(), secs(0.1));
    let ev = run(&mut c, &t, MoveInput { dash_pressed: true, ..idle() }, 1);
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::DashStarted { .. })));
}

#[test]
fn queued_dash_fires_when_cooldown_ends() {
    let t = world();
    let mut c = standing(&t, 200.0, FLOOR);
    let dash = MoveInput { dash_pressed: true, ..idle() };
    let mut ev = run(&mut c, &t, dash, 1);
    ev.extend(run(&mut c, &t, idle(), secs(0.5)));
    ev.extend(run(&mut c, &t, dash, 1));
    ev.extend(run(&mut c, &t, idle(), secs(0.15)));
    assert_eq!(ev.iter().filter(|e| matches!(e, MoveEvent::DashStarted { .. })).count(), 2);
}

#[test]
fn one_air_dash_per_airtime() {
    let t = world();
    let mut c = standing(&t, 200.0, FLOOR);
    let mut ev = run(&mut c, &t, press_jump(0.0), 1);
    ev.extend(run(&mut c, &t, hold_jump(0.0), secs(0.5)));
    ev.extend(run(&mut c, &t, MoveInput { dash_pressed: true, ..hold_jump(0.0) }, 1));
    ev.extend(run(&mut c, &t, hold_jump(0.0), secs(0.62)));
    assert!(!c.is_grounded());
    assert!(!c.can_dash());
    ev.extend(run(&mut c, &t, MoveInput { dash_pressed: true, ..idle() }, 1));
    assert_eq!(ev.iter().filter(|e| matches!(e, MoveEvent::DashStarted { .. })).count(), 1);
    run_until_grounded(&mut c, &t, idle(), 600);
    assert!(c.can_dash());
}

#[test]
fn shadow_dash_passes_window_wall_but_not_monitor_wall() {
    let t = with_window(1, RectF::new(1000.0, 800.0, 1200.0, 1040.0));
    let dash = MoveInput { dash_pressed: true, ..right() };

    let mut c = standing(&t, 950.0, FLOOR);
    let ev = run(&mut c, &t, dash, 1);
    assert!(ev.contains(&MoveEvent::DashStarted { shadow: true }));
    run(&mut c, &t, idle(), secs(0.3));
    assert!(c.pos.x > 1200.0 + 16.0, "shadow dash stopped at {}", c.pos.x);
    // Shade Cloak recharges slower than the dash: the next dash is solid.
    assert!(!c.shadow_dash_ready());

    let mut c = standing(&t, 950.0, FLOOR);
    c.abilities.shadow_dash = false;
    run(&mut c, &t, dash, 1);
    run(&mut c, &t, idle(), secs(0.3));
    assert_eq!(c.pos.x, 1000.0 - 16.0);

    let mut c = standing(&t, 1800.0, FLOOR);
    let ev = run(&mut c, &t, dash, 1);
    assert!(ev.contains(&MoveEvent::DashStarted { shadow: true }));
    run(&mut c, &t, idle(), secs(0.3));
    assert_eq!(c.pos.x, 1920.0 - 16.0);
}

// ------------------------------------------------------------------ superdash

fn charge_and_launch(c: &mut Character, t: &Terrain, dt: f32) -> Vec<MoveEvent> {
    let mut ev = vec![];
    let hold = MoveInput { superdash_held: true, ..idle() };
    let n = (0.85 / dt).ceil() as usize;
    for _ in 0..n {
        c.step(&hold, t, dt, &mut ev);
    }
    c.step(&idle(), t, dt, &mut ev);
    ev
}

#[test]
fn superdash_charges_launches_and_stops_at_thin_wall_without_tunnelling() {
    for dt in [DT, 1.0 / 30.0, 0.1] {
        let mut t = world();
        // Zero-thickness wall: only the left face of a window.
        t.walls.push(Wall { x: 1500.0, y0: 500.0, y1: 1040.0, face: Facing::Left, owner: Some(9), kind: WallKind::Window });
        let mut c = standing(&t, 200.0, FLOOR);
        let mut ev = charge_and_launch(&mut c, &t, dt);
        assert!(ev.contains(&MoveEvent::SuperdashCharging));
        assert!(ev.contains(&MoveEvent::SuperdashCharged));
        assert!(ev.contains(&MoveEvent::SuperdashLaunched), "dt {dt}: {ev:?}");
        for _ in 0..(2.0 / dt) as usize {
            c.step(&idle(), &t, dt, &mut ev);
        }
        assert!(ev.contains(&MoveEvent::SuperdashHitWall), "dt {dt}");
        assert_eq!(c.pos.x, 1500.0 - 16.0, "dt {dt}");
        assert_eq!(c.state, MoveState::Grounded);
    }
}

#[test]
fn superdash_released_early_cancels() {
    let t = world();
    let mut c = standing(&t, 200.0, FLOOR);
    let mut ev = run(&mut c, &t, MoveInput { superdash_held: true, ..idle() }, secs(0.4));
    ev.extend(run(&mut c, &t, idle(), 1));
    assert!(ev.contains(&MoveEvent::SuperdashCancelled));
    assert!(!ev.contains(&MoveEvent::SuperdashLaunched));
    assert_eq!(c.state, MoveState::Grounded);
}

#[test]
fn superdash_flight_cancelled_by_jump_and_flies_over_gaps() {
    let t = with_window(1, RectF::new(100.0, 600.0, 400.0, 900.0));
    let mut c = standing(&t, 300.0, 500.0);
    charge_and_launch(&mut c, &t, DT);
    assert_eq!(c.state, MoveState::Superdash);
    // Off the window's edge it keeps flying level.
    let ev = run(&mut c, &t, idle(), secs(0.25));
    assert!(ev.contains(&MoveEvent::LostSupport));
    assert_eq!(c.pos.y, 600.0);
    assert_eq!(c.state, MoveState::Superdash);
    let ev = run(&mut c, &t, press_jump(0.0), 1);
    assert!(ev.contains(&MoveEvent::SuperdashEnded));
    assert_eq!(c.state, MoveState::Airborne);
}

#[test]
fn superdash_from_wall_launches_away_from_it() {
    let t = with_window(1, RectF::new(1000.0, 300.0, 1400.0, 1040.0));
    let mut c = standing(&t, 900.0, FLOOR);
    run(&mut c, &t, press_jump(1.0), 1);
    run(&mut c, &t, hold_jump(1.0), secs(0.7));
    assert!(matches!(c.state, MoveState::WallSlide { side: Side::Right }));
    let ev = charge_and_launch(&mut c, &t, DT);
    assert!(ev.contains(&MoveEvent::SuperdashLaunched), "{ev:?}");
    assert_eq!(c.facing, Side::Left);
    run(&mut c, &t, idle(), 5);
    assert!(c.vel.x < 0.0);
}

// ------------------------------------------------------------------ moving platforms

fn moved(id: WindowId, r: RectF) -> Terrain {
    with_window(id, r)
}

#[test]
fn a_window_moving_up_or_down_drops_him_into_a_fall() {
    let old = moved(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    for new_top in [570.0, 640.0] {
        let mut c = standing(&old, 1000.0, 500.0);
        let new = moved(1, RectF::new(850.0, new_top, 1250.0, new_top + 300.0));
        let mut ev = vec![];
        c.on_terrain_rebuilt(&old, &new, &mut ev);
        assert_eq!(ev, vec![MoveEvent::PlatformMoved { owner: 1 }, MoveEvent::LostSupport], "top {new_top}");
        assert_eq!(c.pos, Vec2::new(1000.0, 600.0), "not carried");
        assert_eq!(c.state, MoveState::Airborne);
    }
}

#[test]
fn a_window_sliding_sideways_slides_out_from_under_him() {
    let old = moved(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    let mut c = standing(&old, 1000.0, 500.0);
    let new = moved(1, RectF::new(850.0, 600.0, 1250.0, 900.0));
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(ev, vec![MoveEvent::PlatformMoved { owner: 1 }]);
    assert_eq!(c.pos, Vec2::new(1000.0, 600.0), "stays where he is");
    assert!(c.is_grounded());
    // Slid far enough that he is past its left end: he falls.
    let far = moved(1, RectF::new(1100.0, 600.0, 1500.0, 900.0));
    let mut ev = vec![];
    c.on_terrain_rebuilt(&new, &far, &mut ev);
    assert!(ev.contains(&MoveEvent::LostSupport), "{ev:?}");
}

#[test]
fn loses_support_when_window_closes() {
    let old = moved(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    let mut c = standing(&old, 1000.0, 500.0);
    let new = world();
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert!(ev.contains(&MoveEvent::LostSupport));
    assert_eq!(c.state, MoveState::Airborne);
    run_until_grounded(&mut c, &new, idle(), 240);
    assert_eq!(c.pos.y, FLOOR);
}

#[test]
fn loses_support_when_ledge_gets_occluded() {
    let old = moved(1, RectF::new(800.0, 600.0, 1200.0, 900.0));
    let mut c = standing(&old, 1000.0, 500.0);
    // Same window, but its top edge is now hidden around the knight.
    let mut new = old.clone();
    new.ledges.retain(|l| l.owner != Some(1));
    new.ledges.push(Ledge { x0: 800.0, x1: 900.0, y: 600.0, kind: LedgeKind::OneWay, owner: Some(1) });
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(ev, vec![MoveEvent::LostSupport]);
}

#[test]
fn window_rising_a_little_under_body_pops_it_up() {
    let old = world();
    let mut c = standing(&old, 1000.0, FLOOR);
    let new = moved(5, RectF::new(900.0, FLOOR - 20.0, 1200.0, 1040.0));
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(c.pos.y, FLOOR - 20.0);
    assert_eq!(c.support.and_then(|s| s.owner), Some(5));

    // A deep overlap leaves the knight where he is (overlap-ignore rule).
    let mut c = standing(&old, 1000.0, FLOOR);
    let new = moved(6, RectF::new(900.0, FLOOR - 200.0, 1200.0, 1040.0));
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(c.pos.y, FLOOR);
    assert_eq!(c.support.and_then(|s| s.owner), None);
    assert!(ev.is_empty());
    let ev = run(&mut c, &new, right(), secs(1.0));
    assert!(c.pos.x > 1200.0);
    assert!(!ev.contains(&MoveEvent::LostSupport));
}

// ------------------------------------------------------------------ determinism & facing

#[test]
fn forward_simulation_is_deterministic() {
    let t = with_window(1, RectF::new(1000.0, 300.0, 1400.0, 1040.0));
    let script = |i: usize| match i % 97 {
        0 => press_jump(1.0),
        1..=40 => hold_jump(1.0),
        50 => MoveInput { dash_pressed: true, x: -1.0, ..idle() },
        _ => MoveInput { x: if i % 200 < 100 { 1.0 } else { -0.5 }, ..idle() },
    };
    let mut a = standing(&t, 500.0, FLOOR);
    let mut b = a.clone();
    let (mut ea, mut eb) = (vec![], vec![]);
    for i in 0..2000 {
        a.step(&script(i), &t, DT, &mut ea);
    }
    for i in 0..2000 {
        b.step(&script(i), &t, DT, &mut eb);
    }
    assert_eq!(a, b);
    assert_eq!(ea, eb);
}

#[test]
fn turning_emits_event() {
    let t = world();
    let mut c = standing(&t, 960.0, FLOOR);
    let ev = run(&mut c, &t, left(), 1);
    assert_eq!(ev, vec![MoveEvent::TurnedAround]);
    let ev = run(&mut c, &t, left(), 5);
    assert!(ev.is_empty());
}

// ------------------------------------------------------------------ reachability (documented in params.rs)

/// Running jump on flat ground; returns (apex rise, horizontal distance) in px.
fn flat_jump(gait: Gait, double_at_apex: bool) -> (f32, f32) {
    let t = world_sized(20000.0, 3000.0);
    let floor = 2960.0;
    let mut c = standing(&t, 100.0, floor);
    c.vel.x = c.params.gait_speed_px(gait);
    let x0 = c.pos.x;
    let input = |i: usize| MoveInput { x: 1.0, gait, jump_pressed: i == 0, jump_held: true, ..idle() };
    let mut ev = vec![];
    let mut top = c.pos.y;
    let mut doubled = false;
    for i in 0..secs(4.0) {
        let mut inp = input(i);
        if double_at_apex && !doubled && i > 0 && c.vel.y >= 0.0 {
            inp.jump_pressed = true;
            doubled = true;
        }
        c.step(&inp, &t, DT, &mut ev);
        top = top.min(c.pos.y);
        if ev.iter().any(|e| matches!(e, MoveEvent::Landed { .. })) {
            break;
        }
    }
    assert!(c.is_grounded());
    (floor - top, c.pos.x - x0)
}

#[test]
fn reachability_numbers_match_docs() {
    let p = CharacterParams::hollow_knight(PPU);
    let close = |got: f32, want: f32, tol: f32, what: &str| {
        assert!((got - want).abs() <= tol, "{what}: got {got:.1}, documented {want}");
    };
    // Analytic values used in the docs.
    close(p.px(p.jump_height()), 379.0, 1.0, "analytic jump height");
    close(p.px(p.min_jump_height()), 92.0, 1.0, "analytic tap height");
    close(p.px(p.dash_distance()), 320.0, 0.01, "dash distance");

    let (h, walk) = flat_jump(Gait::Walk, false);
    close(h, 379.0, 4.0, "jump apex");
    close(walk, 400.0, 6.0, "walk jump distance");
    let (_, run_d) = flat_jump(Gait::Run, false);
    close(run_d, 553.0, 6.0, "run jump distance");
    let (_, sprint) = flat_jump(Gait::Sprint, false);
    close(sprint, 667.0, 6.0, "sprint jump distance");
    let (h2, run2) = flat_jump(Gait::Run, true);
    close(h2, 758.0, 6.0, "double jump apex");
    close(run2, 996.0, 8.0, "run double jump distance");
}

// ------------------------------------------------------------------ content ledges

/// A tall window (id 1) with a content ledge (a drawn line) inside it at `y`.
fn window_with_content(win: RectF, x0: f32, x1: f32, y: f32) -> Terrain {
    let mut t = with_window(1, win);
    t.ledges.push(Ledge { x0, x1, y, kind: LedgeKind::Content, owner: Some(1) });
    t
}

const PAINT: RectF = RectF::new(600.0, 200.0, 1400.0, 1040.0);

#[test]
fn content_ledge_inside_owner_window_is_solid() {
    let t = window_with_content(PAINT, 800.0, 1200.0, 700.0);
    // Spawned inside the window: its frame is ignored, the drawn line is not.
    let mut c = knight(1000.0, 400.0);
    let ev = run_until_grounded(&mut c, &t, idle(), 240);
    assert!(ev.contains(&MoveEvent::Landed { hard: false }), "{ev:?}");
    assert_eq!(c.pos.y, 700.0);
    let s = c.support.expect("support");
    assert_eq!((s.kind, s.owner), (LedgeKind::Content, Some(1)));
    // Walk off its end: fall to the floor (the window's walls stay ignored inside it).
    run(&mut c, &t, right(), secs(1.5));
    run_until_grounded(&mut c, &t, right(), 240);
    assert_eq!(c.pos.y, FLOOR);
    assert!(c.pos.x > 1200.0);
}

#[test]
fn jumps_up_through_content_and_drops_through_it() {
    let t = window_with_content(PAINT, 800.0, 1200.0, 900.0);
    let mut c = standing(&t, 1000.0, FLOOR);
    assert_eq!(c.pos.y, FLOOR);
    // Full jump (~379 px) rises through the line 140 px above and lands on it.
    run(&mut c, &t, press_jump(0.0), 1);
    run(&mut c, &t, hold_jump(0.0), secs(0.4));
    run_until_grounded(&mut c, &t, idle(), 240);
    assert_eq!(c.pos.y, 900.0);
    assert_eq!(c.support.map(|s| s.kind), Some(LedgeKind::Content));

    let ev = run(&mut c, &t, MoveInput { down: true, ..press_jump(0.0) }, 1);
    assert!(ev.contains(&MoveEvent::DroppedThrough), "{ev:?}");
    run_until_grounded(&mut c, &t, MoveInput { down: true, ..idle() }, 240);
    assert_eq!(c.pos.y, FLOOR);
}

#[test]
fn falls_when_the_window_showing_his_content_ledge_moves() {
    let old = window_with_content(PAINT, 800.0, 1200.0, 700.0);
    let mut c = standing(&old, 1000.0, 400.0);
    assert_eq!(c.pos.y, 700.0);
    let moved = RectF::new(PAINT.left + 50.0, PAINT.top - 30.0, PAINT.right + 50.0, PAINT.bottom - 30.0);
    let new = window_with_content(moved, 850.0, 1250.0, 670.0);
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(ev, vec![MoveEvent::PlatformMoved { owner: 1 }, MoveEvent::LostSupport]);
    assert_eq!(c.pos, Vec2::new(1000.0, 700.0));
    assert_eq!(c.state, MoveState::Airborne);
}

#[test]
fn content_vanishing_underfoot_drops_the_knight() {
    let old = window_with_content(PAINT, 800.0, 1200.0, 700.0);
    let mut c = standing(&old, 1000.0, 400.0);
    let new = with_window(1, PAINT); // the line was erased
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(ev, vec![MoveEvent::LostSupport]);
    run_until_grounded(&mut c, &new, idle(), 240);
    assert_eq!(c.pos.y, FLOOR);
}

#[test]
fn content_appearing_above_feet_does_not_pop_up() {
    let old = with_window(1, PAINT);
    let mut c = standing(&old, 1000.0, FLOOR);
    // A line drawn 20 px above the feet (a window edge there would lift the knight).
    let new = window_with_content(PAINT, 800.0, 1200.0, FLOOR - 20.0);
    let mut ev = vec![];
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert!(ev.is_empty(), "{ev:?}");
    assert_eq!(c.pos.y, FLOOR);
    // But one drawn right at the feet is stood on.
    let new = window_with_content(PAINT, 800.0, 1200.0, FLOOR - 3.0);
    c.on_terrain_rebuilt(&old, &new, &mut ev);
    assert_eq!(c.pos.y, FLOOR - 3.0);
    assert_eq!(c.support.map(|s| s.kind), Some(LedgeKind::Content));
}

// ------------------------------------------------------------------ content walls

/// A vertical line drawn inside the big window (id 1), `x..x + 2`, rows 300..FLOOR: walls
/// on both of its sides, as vision reports them.
fn window_with_vertical_line(x: f32) -> Terrain {
    let mut t = with_window(1, PAINT);
    t.walls.push(Wall { x, y0: 300.0, y1: FLOOR, face: Facing::Left, owner: Some(1), kind: WallKind::Content });
    t.walls.push(Wall { x: x + 2.0, y0: 300.0, y1: FLOOR, face: Facing::Right, owner: Some(1), kind: WallKind::Content });
    t
}

#[test]
fn content_wall_blocks_from_both_sides_inside_its_window() {
    let t = window_with_vertical_line(1100.0);
    // Inside the window (its own walls are ignored there), walking right: stopped.
    let mut c = standing(&t, 900.0, FLOOR);
    run(&mut c, &t, right(), secs(1.5));
    assert_eq!(c.body().right, 1100.0);
    // From the other side, walking left: stopped at the line's right edge.
    let mut c = standing(&t, 1300.0, FLOOR);
    run(&mut c, &t, left(), secs(1.5));
    assert_eq!(c.body().left, 1102.0);
}

#[test]
fn wall_slide_on_content_wall() {
    let t = window_with_vertical_line(1100.0);
    let mut c = standing(&t, 1000.0, FLOOR);
    let mut ev = run(&mut c, &t, press_jump(1.0), 1);
    for _ in 0..secs(1.0) {
        c.step(&hold_jump(1.0), &t, DT, &mut ev);
        if matches!(c.state, MoveState::WallSlide { .. }) {
            break;
        }
    }
    assert_eq!(c.state, MoveState::WallSlide { side: Side::Right });
    assert_eq!(c.body().right, 1100.0);
}

#[test]
fn shadow_dash_passes_content_walls() {
    let t = window_with_vertical_line(1100.0);
    let mut c = standing(&t, 1050.0, FLOOR);
    let ev = run(&mut c, &t, MoveInput { dash_pressed: true, ..right() }, 1);
    assert!(ev.contains(&MoveEvent::DashStarted { shadow: true }));
    run(&mut c, &t, idle(), secs(0.3));
    assert!(c.pos.x > 1102.0 + 16.0, "shadow dash stopped at {}", c.pos.x);
}

#[test]
fn stepping_behind_passes_content_walls_only() {
    let t = window_with_vertical_line(1100.0);
    let behind = |x: f32| MoveInput { behind: true, x, ..MoveInput::default() };
    // Turning in first: no movement while the knight faces into the screen.
    let mut c = standing(&t, 1050.0, FLOOR);
    run(&mut c, &t, behind(1.0), secs(0.2));
    assert_eq!(c.behind_phase(), Some(BehindPhase::TurningIn));
    assert!((c.pos.x - 1050.0).abs() < 0.5, "moved while turning: {}", c.pos.x);
    // Then walks behind the drawn line.
    run(&mut c, &t, behind(1.0), secs(1.0));
    assert_eq!(c.behind_phase(), Some(BehindPhase::Behind));
    assert!(c.body().left > 1102.0, "stopped at {}", c.pos.x);
    // Released: turns back out in place, then is done.
    let x = c.pos.x;
    run(&mut c, &t, MoveInput { x: 1.0, ..MoveInput::default() }, secs(0.1));
    assert_eq!(c.behind_phase(), Some(BehindPhase::TurningOut));
    assert!((c.pos.x - x).abs() < 1.0);
    run(&mut c, &t, idle(), secs(0.2));
    assert_eq!(c.behind_phase(), None);
    // Monitor edges still stop him.
    let mut c = standing(&t, t.bounds.right - 60.0, FLOOR);
    run(&mut c, &t, behind(1.0), secs(2.0));
    assert!(c.body().right <= t.bounds.right + 0.5);
}

// ------------------------------------------------------------------ pogo

/// Mid-air at `y`, falling, with the double jump and air dash spent.
fn falling_spent(t: &Terrain, y: f32) -> Character {
    let mut c = standing(t, 960.0, FLOOR);
    run(&mut c, t, press_jump(0.0), 1);
    run(&mut c, t, hold_jump(0.0), secs(0.3));
    run(&mut c, t, press_jump(0.0), 1);
    run(&mut c, t, MoveInput { dash_pressed: true, ..idle() }, 1);
    run(&mut c, t, idle(), secs(0.3));
    assert!(!c.can_double_jump() && !c.can_dash(), "spent: {c:?}");
    c.pos.y = y;
    c.vel.y = 300.0;
    c
}

#[test]
fn pogo_bounces_up_at_the_pogo_speed_and_refills_air_moves() {
    let t = world();
    let mut c = falling_spent(&t, 600.0);
    assert_eq!(c.state, MoveState::Airborne);
    let ev = run(&mut c, &t, MoveInput { pogo: true, ..idle() }, 1);
    assert_eq!(count(&ev, MoveEvent::Pogo), 1, "{ev:?}");
    let p = c.params;
    assert!((c.vel.y + p.px(p.pogo_speed)).abs() < 1e-3, "rising at the pogo speed: {}", c.vel.y);
    assert!(c.can_double_jump(), "double jump back");
    // The air dash is back too (once its cooldown is over).
    run(&mut c, &t, idle(), secs(p.dash_cooldown));
    assert!(c.can_dash(), "air dash back");
}

#[test]
fn pogo_rise_matches_its_analytic_height_and_ignores_the_jump_button() {
    let t = world();
    let p = CharacterParams::hollow_knight(PPU);
    for held in [false, true] {
        let mut c = falling_spent(&t, 600.0);
        let y0 = c.pos.y;
        c.step(&MoveInput { pogo: true, ..idle() }, &t, DT, &mut vec![]);
        let mut top = c.pos.y;
        for _ in 0..secs(1.0) {
            c.step(&MoveInput { jump_held: held, ..idle() }, &t, DT, &mut vec![]);
            top = top.min(c.pos.y);
        }
        let rise = y0 - top;
        assert!((rise - p.px(p.pogo_height())).abs() < 6.0, "rise {rise} vs {}", p.px(p.pogo_height()));
    }
}

#[test]
fn pogo_chains_and_does_nothing_on_the_ground() {
    let t = world();
    let mut c = falling_spent(&t, 600.0);
    let mut ev = vec![];
    for _ in 0..3 {
        c.step(&MoveInput { pogo: true, ..idle() }, &t, DT, &mut ev);
        // Back down to where he bounced.
        for _ in 0..secs(2.0) {
            c.step(&idle(), &t, DT, &mut ev);
            if c.vel.y > 0.0 && c.pos.y >= 600.0 {
                break;
            }
        }
    }
    assert_eq!(count(&ev, MoveEvent::Pogo), 3);
    // Standing: no bounce.
    let mut g = standing(&t, 960.0, FLOOR);
    let ev = run(&mut g, &t, MoveInput { pogo: true, ..idle() }, 1);
    assert!(!ev.contains(&MoveEvent::Pogo) && g.is_grounded());
    assert!(!g.pogo(&mut vec![]));
}
