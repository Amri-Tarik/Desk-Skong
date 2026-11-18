//! Planner + follower tests on synthetic terrains (render scale 1: 64 px/unit).
//!
//! Reference numbers at this scale (see `controller::params`): body 32 x 82 px, single jump
//! 379 px, jump + double jump 758 px, dash 320 px, Run jump + double jump flat ~996 px.
use super::*;
use crate::controller::{Abilities, CharacterParams, MoveInput, MoveState};
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::{RectF, RectI};
use crate::terrain::{Facing, Ledge, LedgeKind, TerrainOptions, Wall, WindowBox};

const DT: f32 = 1.0 / 120.0;
const PPU: f32 = 64.0;
const FLOOR: f32 = 1040.0;

/// One monitor `w` x `h`, taskbar 40 px.
fn world_sized(w: f32, h: f32) -> Terrain {
    let floor = h - 40.0;
    Terrain {
        generation: 1,
        windows: vec![],
        ledges: vec![Ledge { x0: 0.0, x1: w, y: floor, kind: LedgeKind::Ground, owner: None }],
        walls: vec![
            Wall { x: 0.0, y0: 0.0, y1: floor, face: Facing::Right, owner: None },
            Wall { x: w, y0: 0.0, y1: floor, face: Facing::Left, owner: None },
        ],
        bounds: RectF::new(0.0, 0.0, w, h),
    }
}

fn world() -> Terrain {
    world_sized(1920.0, 1080.0)
}

/// Window platform box: one-way top, outward-facing side walls (no occlusion clipping).
fn add_window(t: &mut Terrain, id: WindowId, r: RectF) {
    t.windows.push(WindowBox { id, rect: r });
    t.ledges.push(Ledge { x0: r.left, x1: r.right, y: r.top, kind: LedgeKind::OneWay, owner: Some(id) });
    let y1 = r.bottom.min(FLOOR);
    t.walls.push(Wall { x: r.left, y0: r.top, y1, face: Facing::Left, owner: Some(id) });
    t.walls.push(Wall { x: r.right, y0: r.top, y1, face: Facing::Right, owner: Some(id) });
}

fn with_windows(mut t: Terrain, windows: &[(WindowId, RectF)]) -> Terrain {
    for &(id, r) in windows {
        add_window(&mut t, id, r);
    }
    t
}

fn knight_with(t: &Terrain, x: f32, y: f32, abilities: Abilities) -> Character {
    let mut c = Character::new(CharacterParams::hollow_knight(PPU), abilities, Vec2::new(x, y));
    let mut ev = vec![];
    for _ in 0..600 {
        c.step(&MoveInput::default(), t, DT, &mut ev);
        if c.state == MoveState::Grounded {
            return c;
        }
    }
    panic!("never landed: {c:?}");
}

fn knight(t: &Terrain, x: f32, y: f32) -> Character {
    knight_with(t, x, y, Abilities::default())
}

/// Plan and follow with real `Character::step`s; returns the final status and the route.
fn follow(graph: &NavGraph, t: &Terrain, ch: &mut Character, goal: NavGoal, style: TravelStyle, secs: f32) -> (FollowStatus, Route) {
    let route = graph.plan(ch, goal, style).expect("route");
    let mut f = PathFollower::new(route.clone(), goal, style);
    let mut ev = vec![];
    for _ in 0..(secs / DT) as usize {
        let (input, status) = f.next_input(ch, graph, t, DT);
        if status != FollowStatus::Moving {
            return (status, route);
        }
        ev.clear();
        ch.step(&input, t, DT, &mut ev);
    }
    (FollowStatus::Moving, route)
}

fn owner_under(ch: &Character) -> Option<WindowId> {
    ch.support.and_then(|s| s.owner)
}

/// Plan + follow to `OnWindow(id)` and assert arrival on it; returns the planned moves.
fn reach_window(t: &Terrain, ch: &mut Character, id: WindowId, style: TravelStyle) -> Vec<MoveKind> {
    let g = NavGraph::build(t, ch);
    let (status, route) = follow(&g, t, ch, NavGoal::OnWindow(id), style, 30.0);
    assert_eq!(status, FollowStatus::Arrived, "{style:?} route {:?} ended at {:?} ({:?})", route.moves, ch.pos, ch.state);
    assert_eq!(owner_under(ch), Some(id), "{style:?}: ended at {:?}", ch.pos);
    route.moves
}

fn plan_to(t: &Terrain, ch: &Character, goal: NavGoal, style: TravelStyle) -> Option<Route> {
    NavGraph::build(t, ch).plan(ch, goal, style)
}

// ------------------------------------------------------------------ surfaces

#[test]
fn ground_is_split_by_a_window_standing_on_it() {
    let t = with_windows(world(), &[(1, RectF::new(800.0, 600.0, 1000.0, 1040.0))]);
    let g = NavGraph::build(&t, &knight(&t, 300.0, FLOOR));
    let mut ground: Vec<(f32, f32)> = g.surfaces().iter().filter(|s| s.y == FLOOR).map(|s| (s.x0, s.x1)).collect();
    ground.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(ground, vec![(0.0, 800.0), (800.0, 1000.0), (1000.0, 1920.0)]);
    let left = g.surfaces().iter().find(|s| s.y == FLOOR && s.x0 == 0.0).expect("left");
    // Monitor wall on the left, window wall on the right: the body stays clear of both.
    assert_eq!((left.lo, left.hi), (16.5, 783.5));
}

// ------------------------------------------------------------------ single moves

#[test]
fn walks_across_touching_ledges() {
    let t = with_windows(world(), &[(1, RectF::new(200.0, 600.0, 600.0, 800.0)), (2, RectF::new(600.0, 604.0, 1000.0, 800.0))]);
    let mut ch = knight(&t, 300.0, 0.0);
    assert_eq!(owner_under(&ch), Some(1));
    let moves = reach_window(&t, &mut ch, 2, TravelStyle::Stroll);
    assert_eq!(moves, vec![MoveKind::Walk]);
}

#[test]
fn drops_down_to_the_ground() {
    let t = with_windows(world(), &[(1, RectF::new(300.0, 500.0, 700.0, 800.0))]);
    let mut ch = knight(&t, 400.0, 0.0);
    let g = NavGraph::build(&t, &ch);
    let goal = NavGoal::Point(Vec2::new(1200.0, FLOOR));
    let (status, route) = follow(&g, &t, &mut ch, goal, TravelStyle::Stroll, 20.0);
    assert_eq!(status, FollowStatus::Arrived);
    assert_eq!(route.moves, vec![MoveKind::Drop]);
    assert!((ch.pos.x - 1200.0).abs() <= 6.0 && ch.pos.y == FLOOR, "{:?}", ch.pos);
}

#[test]
fn drops_through_a_one_way_ledge_when_normal() {
    // Wide window: walking off an end is slower than dropping through the middle.
    let t = with_windows(world(), &[(1, RectF::new(200.0, 600.0, 1700.0, 700.0))]);
    let mut ch = knight(&t, 950.0, 0.0);
    let g = NavGraph::build(&t, &ch);
    let (status, route) = follow(&g, &t, &mut ch, NavGoal::Point(Vec2::new(950.0, FLOOR)), TravelStyle::Normal, 20.0);
    assert_eq!(status, FollowStatus::Arrived);
    assert_eq!(route.moves, vec![MoveKind::DropThrough]);
    assert_eq!(ch.pos.y, FLOOR);
}

#[test]
fn jumps_up_to_a_ledge_within_single_jump_height() {
    let t = with_windows(world(), &[(1, RectF::new(800.0, 740.0, 1200.0, 1040.0))]);
    let mut ch = knight(&t, 300.0, FLOOR);
    assert_eq!(reach_window(&t, &mut ch, 1, TravelStyle::Stroll), vec![MoveKind::Jump]);
    let mut ch = knight(&t, 300.0, FLOOR);
    assert_eq!(reach_window(&t, &mut ch, 1, TravelStyle::Normal), vec![MoveKind::Jump]);
}

#[test]
fn double_jump_ledge_needs_more_than_a_stroll() {
    // Floating 600 px above the ground: beyond a single jump, within a double jump.
    let t = with_windows(world(), &[(1, RectF::new(800.0, 440.0, 1100.0, 480.0))]);
    let ch = knight(&t, 300.0, FLOOR);
    assert!(plan_to(&t, &ch, NavGoal::OnWindow(1), TravelStyle::Stroll).is_none());
    let mut ch = knight(&t, 300.0, FLOOR);
    assert_eq!(reach_window(&t, &mut ch, 1, TravelStyle::Normal), vec![MoveKind::DoubleJump]);
}

/// Two thin platforms 1060 px above the ground of a tall monitor (out of reach from it, and
/// far enough below the top that jumps are not capped) with a gap between.
fn high_platforms(width: f32, gap: f32) -> Terrain {
    let a = RectF::new(100.0, 900.0, 500.0, 940.0);
    let b = RectF::new(500.0 + gap, 900.0, 500.0 + gap + 200.0, 940.0);
    with_windows(world_sized(width, 2000.0), &[(1, a), (2, b)])
}

#[test]
fn gap_only_crossable_with_dash() {
    let t = high_platforms(1920.0, 1150.0);
    let ch = knight(&t, 300.0, 0.0);
    assert!(plan_to(&t, &ch, NavGoal::OnWindow(2), TravelStyle::Stroll).is_none());
    let mut ch = knight(&t, 300.0, 0.0);
    let moves = reach_window(&t, &mut ch, 2, TravelStyle::Normal);
    assert_eq!(moves, vec![MoveKind::Dash]);
    let no_dash = Abilities { dash: false, shadow_dash: false, superdash: false, ..Abilities::default() };
    let ch = knight_with(&t, 300.0, 0.0, no_dash);
    assert!(plan_to(&t, &ch, NavGoal::OnWindow(2), TravelStyle::Hurry).is_none());
}

#[test]
fn tall_wall_climbed_with_wall_jumps() {
    // 900 px tall window standing on the ground: higher than jump + double jump.
    let t = with_windows(world(), &[(1, RectF::new(800.0, 140.0, 1200.0, 1040.0))]);
    let ch = knight(&t, 300.0, FLOOR);
    assert!(plan_to(&t, &ch, NavGoal::OnWindow(1), TravelStyle::Stroll).is_none());
    let mut ch = knight(&t, 300.0, FLOOR);
    assert_eq!(reach_window(&t, &mut ch, 1, TravelStyle::Normal), vec![MoveKind::WallJump]);
    let no_claw = Abilities { wall_jump: false, ..Abilities::default() };
    let ch = knight_with(&t, 300.0, FLOOR, no_claw);
    assert!(plan_to(&t, &ch, NavGoal::OnWindow(1), TravelStyle::Hurry).is_none());
}

#[test]
fn shadow_dash_through_a_window_wall() {
    // Full-height window: no way over it, only through.
    let t = with_windows(world(), &[(1, RectF::new(800.0, 0.0, 1000.0, 1040.0))]);
    let goal = NavGoal::Point(Vec2::new(1500.0, FLOOR));
    let mut ch = knight(&t, 300.0, FLOOR);
    let g = NavGraph::build(&t, &ch);
    let (status, route) = follow(&g, &t, &mut ch, goal, TravelStyle::Normal, 20.0);
    assert_eq!(status, FollowStatus::Arrived);
    assert_eq!(route.moves, vec![MoveKind::ShadowDash]);
    assert!((ch.pos.x - 1500.0).abs() <= 6.0, "{:?}", ch.pos);
    // Without the shade cloak the closest reachable point is on this side of the window.
    let plain = Abilities { shadow_dash: false, ..Abilities::default() };
    let ch = knight_with(&t, 300.0, FLOOR, plain);
    let r = plan_to(&t, &ch, goal, TravelStyle::Hurry).expect("route");
    assert!(r.end.x < 800.0, "{:?}", r.end);
}

#[test]
fn superdash_only_when_hurrying() {
    let t = high_platforms(2560.0, 1500.0);
    let ch = knight(&t, 300.0, 0.0);
    assert!(plan_to(&t, &ch, NavGoal::OnWindow(2), TravelStyle::Normal).is_none());
    let mut ch = knight(&t, 300.0, 0.0);
    assert_eq!(reach_window(&t, &mut ch, 2, TravelStyle::Hurry), vec![MoveKind::Superdash]);
}

#[test]
fn point_goal_ends_at_closest_reachable_point() {
    // The point is on top of an unreachable (too tall, no claw) window: end below it.
    let t = with_windows(world(), &[(1, RectF::new(800.0, 140.0, 1200.0, 1040.0))]);
    let ch = knight_with(&t, 300.0, FLOOR, Abilities::none());
    let r = plan_to(&t, &ch, NavGoal::Point(Vec2::new(900.0, 100.0)), TravelStyle::Normal).expect("route");
    assert!(r.moves.is_empty());
    assert!((r.end.x - 783.5).abs() < 0.01 && r.end.y == FLOOR, "{:?}", r.end);
}

// ------------------------------------------------------------------ multi-step + re-plan

#[test]
fn staircase_of_windows() {
    let t = with_windows(
        world(),
        &[
            (1, RectF::new(300.0, 760.0, 600.0, 1040.0)),
            (2, RectF::new(700.0, 480.0, 1000.0, 600.0)),
            (3, RectF::new(1150.0, 200.0, 1500.0, 320.0)),
        ],
    );
    for style in [TravelStyle::Stroll, TravelStyle::Normal, TravelStyle::Hurry] {
        let mut ch = knight(&t, 100.0, FLOOR);
        let moves = reach_window(&t, &mut ch, 3, style);
        assert!(moves.len() >= 2, "{style:?}: {moves:?}");
    }
}

#[test]
fn replans_after_target_window_moves() {
    let t1 = with_windows(world(), &[(1, RectF::new(1200.0, 740.0, 1500.0, 1040.0))]);
    let mut ch = knight(&t1, 300.0, FLOOR);
    let g1 = NavGraph::build(&t1, &ch);
    let goal = NavGoal::OnWindow(1);
    let mut f = PathFollower::new(g1.plan(&ch, goal, TravelStyle::Normal).expect("route"), goal, TravelStyle::Normal);
    let mut ev = vec![];
    for _ in 0..30 {
        let (input, status) = f.next_input(&ch, &g1, &t1, DT);
        assert_eq!(status, FollowStatus::Moving);
        ch.step(&input, &t1, DT, &mut ev);
    }
    // The target is dragged further away and raised.
    let mut t2 = with_windows(world(), &[(1, RectF::new(1300.0, 600.0, 1600.0, 1040.0))]);
    t2.generation = 2;
    ch.on_terrain_rebuilt(&t1, &t2, &mut ev);
    let g2 = g1.rebuild(&t2, &ch);
    assert_eq!(g2.generation, 2);
    let mut arrived = false;
    for _ in 0..(20.0 / DT) as usize {
        let (input, status) = f.next_input(&ch, &g2, &t2, DT);
        if status == FollowStatus::Arrived {
            arrived = true;
            break;
        }
        assert_eq!(status, FollowStatus::Moving);
        ch.step(&input, &t2, DT, &mut ev);
    }
    assert!(arrived, "at {:?}", ch.pos);
    assert_eq!(owner_under(&ch), Some(1));
    assert!(f.route().end.x >= 1300.0);
}

#[test]
fn replans_after_landing_on_the_wrong_surface() {
    // Plan on terrain without the middle window, then follow on terrain that has it (same
    // generation, so only the landing check notices): the drop lands early and the follower
    // re-plans from there.
    let base = [(1, RectF::new(300.0, 300.0, 700.0, 400.0))];
    let t_plan = with_windows(world(), &base);
    let t_real = with_windows(world(), &[base[0], (2, RectF::new(600.0, 700.0, 1100.0, 800.0))]);
    let mut ch = knight(&t_plan, 400.0, 0.0);
    let g_plan = NavGraph::build(&t_plan, &ch);
    let goal = NavGoal::Point(Vec2::new(1500.0, FLOOR));
    let mut f = PathFollower::new(g_plan.plan(&ch, goal, TravelStyle::Normal).expect("route"), goal, TravelStyle::Normal);
    let g_real = g_plan.rebuild(&t_real, &ch);
    let mut ev = vec![];
    let mut status = FollowStatus::Moving;
    for _ in 0..(20.0 / DT) as usize {
        let (input, s) = f.next_input(&ch, &g_real, &t_real, DT);
        status = s;
        if status != FollowStatus::Moving {
            break;
        }
        ch.step(&input, &t_real, DT, &mut ev);
    }
    assert_eq!(status, FollowStatus::Arrived, "at {:?}", ch.pos);
    assert!((ch.pos.x - 1500.0).abs() <= 6.0 && ch.pos.y == FLOOR);
}

#[test]
fn fails_when_the_target_window_closes() {
    let t1 = with_windows(world(), &[(1, RectF::new(1200.0, 740.0, 1500.0, 1040.0))]);
    let ch = knight(&t1, 300.0, FLOOR);
    let g1 = NavGraph::build(&t1, &ch);
    let goal = NavGoal::OnWindow(1);
    let mut f = PathFollower::new(g1.plan(&ch, goal, TravelStyle::Normal).expect("route"), goal, TravelStyle::Normal);
    let mut t2 = world();
    t2.generation = 2;
    let g2 = g1.rebuild(&t2, &ch);
    assert_eq!(f.next_input(&ch, &g2, &t2, DT).1, FollowStatus::Failed);
}

// ------------------------------------------------------------------ realistic desktops

fn win(id: u64, l: i32, t: i32, r: i32, b: i32) -> WindowInfo {
    WindowInfo {
        id,
        rect: RectI::new(l, t, r, b),
        title: format!("w{id}"),
        class: String::new(),
        process: String::new(),
        pid: 0,
        maximized: false,
        fullscreen: false,
        foreground: false,
        topmost: false,
    }
}

fn desktop(windows: Vec<WindowInfo>) -> Terrain {
    let monitors = vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }];
    Terrain::from_desktop(&DesktopSnapshot { generation: 1, monitors, windows }, &TerrainOptions::default())
}

fn office_windows() -> Vec<WindowInfo> {
    vec![
        win(1, 1300, 700, 1800, 980), // terminal, front
        win(2, 900, 300, 1500, 760),  // editor
        win(3, 120, 160, 1000, 900),  // browser, back
        win(4, 1550, 120, 1850, 420), // chat
        win(5, 300, 940, 700, 1040),  // small dialog on the taskbar
    ]
}

fn office() -> Terrain {
    desktop(office_windows())
}

#[test]
fn realistic_desktop_every_window_reachable_and_followed() {
    let t = office();
    for style in [TravelStyle::Normal, TravelStyle::Hurry] {
        for id in 1..=5 {
            let mut ch = knight(&t, 60.0, FLOOR);
            let moves = reach_window(&t, &mut ch, id, style);
            assert!(!moves.is_empty(), "{style:?} window {id}");
        }
    }
}

#[test]
fn realistic_desktop_stroll_reaches_low_windows() {
    let t = office();
    let ch = knight(&t, 60.0, FLOOR);
    let g = NavGraph::build(&t, &ch);
    for id in [1, 5] {
        let mut ch = knight(&t, 60.0, FLOOR);
        let (status, route) = follow(&g, &t, &mut ch, NavGoal::OnWindow(id), TravelStyle::Stroll, 30.0);
        assert_eq!(status, FollowStatus::Arrived, "window {id}: {:?}", route.moves);
        assert!(route.moves.iter().all(|m| matches!(m, MoveKind::Walk | MoveKind::Drop | MoveKind::Jump)));
    }
}

#[test]
fn round_trip_between_windows() {
    let t = office();
    let mut ch = knight(&t, 60.0, FLOOR);
    let g = NavGraph::build(&t, &ch);
    for id in [4, 5, 3, 1, 2] {
        let (status, route) = follow(&g, &t, &mut ch, NavGoal::OnWindow(id), TravelStyle::Normal, 30.0);
        assert_eq!(status, FollowStatus::Arrived, "window {id}: {:?} at {:?}", route.moves, ch.pos);
        assert_eq!(owner_under(&ch), Some(id));
    }
}

// ------------------------------------------------------------------ determinism + cache

#[test]
fn build_and_follow_are_deterministic() {
    let t = office();
    let ch0 = knight(&t, 60.0, FLOOR);
    let (a, b) = (NavGraph::build(&t, &ch0), NavGraph::build(&t, &ch0));
    assert_eq!(a.edges(), b.edges());
    assert_eq!(a.surfaces(), b.surfaces());
    let run = || {
        let mut ch = ch0.clone();
        follow(&a, &t, &mut ch, NavGoal::OnWindow(4), TravelStyle::Hurry, 30.0);
        ch
    };
    assert_eq!(run(), run());
}

#[test]
fn rebuild_reuses_unaffected_pairs() {
    let t1 = office();
    let ch = knight(&t1, 60.0, FLOOR);
    let g1 = NavGraph::build(&t1, &ch);
    // Same terrain: everything comes from the cache, identical result.
    let same = g1.rebuild(&t1, &ch);
    assert_eq!(same.stats().sims, 0);
    assert_eq!(same.edges(), g1.edges());
    // Nudge the chat window: only nearby pairs are re-simulated.
    let mut windows = office_windows();
    windows[3] = win(4, 1560, 130, 1860, 430);
    let t2 = desktop(windows);
    let g2 = g1.rebuild(&t2, &ch);
    let fresh = NavGraph::build(&t2, &ch);
    assert_eq!(g2.edges(), fresh.edges());
    assert!(g2.stats().cache_hits > 0 && g2.stats().sims < fresh.stats().sims, "{:?} vs {:?}", g2.stats(), fresh.stats());
}

#[test]
fn edges_are_consistent() {
    let t = office();
    let g = NavGraph::build(&t, &knight(&t, 60.0, FLOOR));
    assert_eq!(g.edge_counts().iter().sum::<usize>(), g.edges().len());
    assert_eq!(g.stats().edges, g.edges().len());
    for (s, _) in g.surfaces().iter().enumerate() {
        assert!(g.edges_from(s).iter().all(|e| e.from == s));
    }
    for e in g.edges() {
        assert!(e.to < g.surfaces().len() && e.from != e.to && e.duration > 0.0);
    }
}

fn dump(name: &str, t: &Terrain, ch: &Character) {
    let g = NavGraph::build(t, ch);
    println!("== {name}: {:?}", g.stats());
    for (i, s) in g.surfaces().iter().enumerate() {
        println!("  S{i}: x {}..{} y {} stand {}..{} owner {:?}", s.x0, s.x1, s.y, s.lo, s.hi, s.owner);
    }
    for e in g.edges() {
        println!("  E {}->{} {:?} {:?} takeoff {} land {:?} dur {:.2} {:?}", e.from, e.to, e.kind, e.gait, e.takeoff, e.land, e.duration, e.script.recipe);
    }
}

#[test]
fn zz_debug() {
    let t = with_windows(world(), &[(1, RectF::new(200.0, 600.0, 1700.0, 700.0))]);
    dump("dropthrough", &t, &knight(&t, 950.0, 0.0));
    let t = high_platforms(1920.0, 1150.0);
    dump("dash", &t, &knight(&t, 300.0, 0.0));
    let t = with_windows(world(), &[(1, RectF::new(800.0, 0.0, 1000.0, 1040.0))]);
    dump("shadow", &t, &knight(&t, 300.0, FLOOR));
    let t = office();
    dump("office", &t, &knight(&t, 60.0, FLOOR));
}

#[test]
fn zz_trace() {
    use super::script::*;
    use crate::controller::{Gait, Side};
    let t = office();
    let ch0 = knight(&t, 361.0, FLOOR);
    let g = NavGraph::build(&t, &ch0);
    let (s, tt) = (g.surfaces()[1], g.surfaces()[0]);
    let sc = Script { recipe: Recipe::Walk, start: Start::Pass, dir: Side::Left, gait: Gait::Walk, takeoff: 301.0, back: 361.0, tx: 259.5, ty: FLOOR, needs_shadow: false };
    let mut ch = ch0.clone();
    let mut ex = Exec::new(sc);
    let mut ev = vec![];
    for i in 0..80 {
        let inp = ex.input(&ch, Gait::Walk, DT);
        ev.clear();
        ch.step(&inp, &t, DT, &mut ev);
        let o = ex.outcome(&ch, &s, &tt);
        println!("{i} stage {:?} x {:.2} y {:.2} vx {:.1} in.x {:.2} {:?} {:?} {:?}", ex.stage, ch.pos.x, ch.pos.y, ch.vel.x, inp.x, ch.state, o, ev);
        if o != Outcome::Continue { break; }
    }
}
