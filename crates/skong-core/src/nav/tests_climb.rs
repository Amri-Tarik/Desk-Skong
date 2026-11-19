//! Wall-climbing scenarios on realistic desktops at the user's render scale (0.6: 38.4 px per
//! unit). Reference numbers at this scale: body 19.2 x 49.2 px, single jump ~227 px, jump +
//! double jump ~455 px, dash 192 px.
use super::*;
use crate::controller::{Abilities, CharacterParams, MoveInput, MoveState};
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::RectI;
use crate::terrain::{ContentLedge, ContentWallSeg, Facing, TerrainOptions};

const DT: f32 = 1.0 / 120.0;
const PPU: f32 = 38.4;

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

/// 2560x1440 primary with a 40 px taskbar (floor at y = 1400).
fn primary() -> Monitor {
    Monitor { rect: RectI::new(0, 0, 2560, 1440), work: RectI::new(0, 0, 2560, 1400), primary: true, scale_milli: 1000 }
}

const FLOOR: f32 = 1400.0;

fn desktop(monitors: Vec<Monitor>, windows: Vec<WindowInfo>) -> Terrain {
    Terrain::from_desktop(&DesktopSnapshot { generation: 1, monitors, windows }, &TerrainOptions::default())
}

fn knight(t: &Terrain, x: f32, y: f32) -> Character {
    let mut c = Character::new(CharacterParams::hollow_knight(PPU), Abilities::default(), Vec2::new(x, y));
    let mut ev = vec![];
    for _ in 0..600 {
        c.step(&MoveInput::default(), t, DT, &mut ev);
        if c.state == MoveState::Grounded {
            return c;
        }
    }
    panic!("never landed: {c:?}");
}

/// Plan, then drive `Character::step` with the follower until it reports an end state.
fn follow(t: &Terrain, ch: &mut Character, goal: NavGoal, style: TravelStyle) -> (FollowStatus, Vec<MoveKind>) {
    let g = NavGraph::build(t, ch);
    let route = g.plan(ch, goal, style).unwrap_or_else(|| panic!("no {style:?} route to {goal:?} from {:?}", ch.pos));
    let moves = route.moves.clone();
    let mut f = PathFollower::new(route, goal, style);
    let mut ev = vec![];
    for _ in 0..(40.0 / DT) as usize {
        let (input, status) = f.next_input(ch, &g, t, DT);
        if status != FollowStatus::Moving {
            return (status, moves);
        }
        ev.clear();
        ch.step(&input, t, DT, &mut ev);
    }
    (FollowStatus::Moving, moves)
}

fn assert_reaches_window(t: &Terrain, from: Vec2, id: WindowId) -> Vec<MoveKind> {
    let mut ch = knight(t, from.x, from.y);
    let (status, moves) = follow(t, &mut ch, NavGoal::OnWindow(id), TravelStyle::Normal);
    assert_eq!(status, FollowStatus::Arrived, "route {moves:?} ended at {:?} ({:?})", ch.pos, ch.state);
    assert_eq!(ch.support.and_then(|s| s.owner), Some(id), "ended at {:?}", ch.pos);
    moves
}

#[test]
fn classic_climb_up_a_tall_windows_own_side() {
    // 1100 px tall window standing on the taskbar: more than twice a double jump.
    let t = desktop(vec![primary()], vec![win(1, 1000, 300, 1400, 1400)]);
    let moves = assert_reaches_window(&t, Vec2::new(300.0, FLOOR), 1);
    assert!(moves.contains(&MoveKind::WallJump), "{moves:?}");
    // And from the other side.
    assert_reaches_window(&t, Vec2::new(2200.0, FLOOR), 1);
}

#[test]
fn climb_a_wall_whose_top_is_hidden_by_a_front_window() {
    // A small front window hides the target's top-left corner: the wall climbed is not at the
    // end of the target ledge.
    let t = desktop(vec![primary()], vec![win(2, 900, 200, 1150, 380), win(1, 1000, 300, 1400, 1400)]);
    assert_reaches_window(&t, Vec2::new(300.0, FLOOR), 1);
}

#[test]
fn climb_a_shaft_between_two_windows() {
    // Two tall windows 100 px apart; the knight starts at the bottom of the shaft.
    let t = desktop(vec![primary()], vec![win(1, 600, 300, 900, 1400), win(2, 1000, 300, 1300, 1400)]);
    let moves = assert_reaches_window(&t, Vec2::new(950.0, FLOOR), 1);
    assert!(moves.contains(&MoveKind::WallJump), "{moves:?}");
    assert_reaches_window(&t, Vec2::new(950.0, FLOOR), 2);
}

#[test]
fn climb_onto_a_tall_window_then_a_higher_one_behind_it() {
    // A (front) stands on the taskbar, 700 px tall; B (behind) rises another 600 px above A's
    // top, its left wall standing on A.
    let t = desktop(vec![primary()], vec![win(1, 800, 700, 1400, 1400), win(2, 1200, 100, 1800, 1300)]);
    assert_reaches_window(&t, Vec2::new(300.0, FLOOR), 1);
    let moves = assert_reaches_window(&t, Vec2::new(300.0, FLOOR), 2);
    assert!(moves.iter().filter(|m| **m == MoveKind::WallJump).count() >= 2, "{moves:?}");
}

#[test]
fn climb_a_content_wall_to_a_content_ledge() {
    // A big window showing a table: a column border (content wall) under the left end of a
    // horizontal border (content ledge) 800 px above the floor.
    let desk = DesktopSnapshot { generation: 1, monitors: vec![primary()], windows: vec![win(1, 200, 100, 1800, 1400)] };
    let ledges = [ContentLedge { x0: 700, x1: 1100, y: 600, owner: Some(1) }];
    let walls = [
        ContentWallSeg { x: 700, y0: 600, y1: 1400, face: Facing::Left, owner: Some(1) },
        ContentWallSeg { x: 702, y0: 600, y1: 1400, face: Facing::Right, owner: Some(1) },
    ];
    let t = Terrain::from_desktop_and_content(&desk, &ledges, &walls, &TerrainOptions::default());
    assert!(t.ledges.iter().any(|l| l.kind == crate::terrain::LedgeKind::Content));
    let goal = NavGoal::Point(Vec2::new(900.0, 600.0));
    let mut ch = knight(&t, 400.0, FLOOR);
    let (status, moves) = follow(&t, &mut ch, goal, TravelStyle::Normal);
    assert_eq!(status, FollowStatus::Arrived, "{moves:?} at {:?}", ch.pos);
    assert!((ch.pos.y - 600.0).abs() < 1.0, "{:?}", ch.pos);
}

#[test]
fn cross_the_seam_to_a_higher_neighbour_floor() {
    // The user's layout: 2560x1440 primary, 1920x1080 to its left, tops aligned, so the left
    // floor is 360 px higher.
    let left = Monitor { rect: RectI::new(-1920, 0, 0, 1080), work: RectI::new(-1920, 0, 0, 1080), primary: false, scale_milli: 1000 };
    let prim = Monitor { work: RectI::new(0, 0, 2560, 1440), ..primary() };
    let t = desktop(vec![left.clone(), prim.clone()], vec![]);
    let goal = NavGoal::Point(Vec2::new(-900.0, 1080.0));
    let mut ch = knight(&t, 300.0, 1440.0);
    let (status, moves) = follow(&t, &mut ch, goal, TravelStyle::Normal);
    assert_eq!(status, FollowStatus::Arrived, "{moves:?} at {:?}", ch.pos);
    assert!((ch.pos.x + 900.0).abs() < 6.0 && ch.pos.y == 1080.0, "{:?}", ch.pos);
    // With a maximized window on each monitor (the windows stop at the seam).
    let t = desktop(vec![left, prim], vec![win(1, -1920, 0, 0, 1080), win(2, 0, 0, 2560, 1440)]);
    let mut ch = knight(&t, 300.0, 1440.0);
    let (status, moves) = follow(&t, &mut ch, goal, TravelStyle::Normal);
    assert_eq!(status, FollowStatus::Arrived, "{moves:?} at {:?}", ch.pos);
}

#[test]
#[ignore]
fn zz_trace_climb() {
    use super::script::*;
    use crate::controller::{Gait, Side};
    let ws = vec![
        win(1, 388, 293, 963, 1300), win(2, -533, 640, 715, 1182), win(3, 814, 132, 1865, 899), win(4, 2082, 159, 2484, 1079),
        win(5, 1237, 44, 2305, 802), win(6, 1438, 434, 2299, 1213), win(7, 81, 46, 1186, 940), win(8, 511, 28, 1654, 992),
    ];
    let mon = Monitor { work: RectI::new(0, 0, 2560, 1392), ..primary() };
    let t = desktop(vec![mon], ws);
    let ch0 = knight(&t, 1945.0, 1392.0);
    let half = ch0.size.x * 0.5;
    let sc = Script { recipe: Recipe::Climb, start: Start::Run, dir: Side::Right, gait: Gait::Run, takeoff: 2082.0 - half - 48.0, back: 1950.0, tx: 2082.0 + half + 8.0, ty: 159.0, needs_shadow: false, climb: Some(Climb { wall: ClimbWall { x: 2082.0, y0: 159.0, y1: 1079.0 }, shaft: None, dash: false, over: true }) };
    let mut ch = ch0.clone();
    let mut ex = Exec::new(sc);
    let mut ev = vec![];
    for i in 0..1200 {
        let inp = ex.input(&ch, Gait::Run, DT);
        ev.clear();
        ch.step(&inp, &t, DT, &mut ev);
        if i % 6 == 0 || !ev.is_empty() {
            println!("{i} x {:.1} y {:.1} v {:?} in.x {:.2} j {} {:?} {:?}", ch.pos.x, ch.pos.y, ch.vel, inp.x, inp.jump_pressed, ch.state, ev);
        }
        if ch.is_grounded() && ex.stage == Stage::Script && ex.t > 0.3 { println!("grounded at {:?}", ch.pos); break; }
    }
}
