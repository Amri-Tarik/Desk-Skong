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
    (drive(&g, t, ch, &mut f, 40.0).0, moves)
}

/// Step the follower like the app does, including its automatic Desolate Dive on big straight
/// drops (only where the follower allows it). Returns the end status and the dives made.
fn drive(g: &NavGraph, t: &Terrain, ch: &mut Character, f: &mut PathFollower, secs: f32) -> (FollowStatus, usize) {
    let (mut ev, mut dives) = (vec![], 0);
    for _ in 0..(secs / DT) as usize {
        let (mut input, status) = f.next_input(ch, g, t, DT);
        if status != FollowStatus::Moving {
            return (status, dives);
        }
        if f.dive_ok(ch)
            && let Some(dark) = ch.auto_dive(t, &input, false)
        {
            input.dive_pressed = true;
            input.dive_dark = dark;
            dives += 1;
        }
        ev.clear();
        ch.step(&input, t, DT, &mut ev);
    }
    (FollowStatus::Moving, dives)
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
fn climbs_hold_from_many_starting_points() {
    // The follower reaches the take-off from anywhere (near, far, either side, walking in
    // or turning around); the climb script must not depend on one exact arrival.
    let t = desktop(vec![primary()], vec![win(1, 1000, 300, 1400, 1400), win(2, 1700, 500, 1900, 1400)]);
    for x in [60.0, 400.0, 900.0, 975.0, 1450.0, 1600.0, 1950.0, 2500.0] {
        let mut ch = knight(&t, x, FLOOR);
        let (status, moves) = follow(&t, &mut ch, NavGoal::OnWindow(1), TravelStyle::Normal);
        assert_eq!(status, FollowStatus::Arrived, "from x {x}: {moves:?} ended at {:?}", ch.pos);
        assert_eq!(ch.support.and_then(|s| s.owner), Some(1), "from x {x}");
    }
}

#[test]
fn dives_during_a_drop_land_where_the_drop_would() {
    // Off the top of a tall window down to the taskbar: the app's auto-dive kicks in on the
    // way down and the follower carries on.
    let t = desktop(vec![primary()], vec![win(1, 1000, 300, 1400, 1400)]);
    let mut ch = knight(&t, 1200.0, 0.0);
    assert_eq!(ch.pos.y, 300.0);
    let goal = NavGoal::Point(Vec2::new(500.0, FLOOR));
    let g = NavGraph::build(&t, &ch);
    let route = g.plan(&ch, goal, TravelStyle::Normal).expect("route");
    assert!(route.moves.iter().any(|m| matches!(m, MoveKind::Drop | MoveKind::DropThrough)), "{:?}", route.moves);
    let mut f = PathFollower::new(route, goal, TravelStyle::Normal);
    let (status, dives) = drive(&g, &t, &mut ch, &mut f, 30.0);
    assert_eq!(status, FollowStatus::Arrived, "at {:?}", ch.pos);
    assert!(dives > 0, "expected an auto-dive on the 1100 px drop");
    assert!((ch.pos.x - 500.0).abs() <= 6.0 && ch.pos.y == FLOOR, "{:?}", ch.pos);
}

#[test]
fn no_dive_mid_climb() {
    let t = desktop(vec![primary()], vec![win(1, 1000, 300, 1400, 1400)]);
    let ch = knight(&t, 300.0, FLOOR);
    let g = NavGraph::build(&t, &ch);
    let goal = NavGoal::OnWindow(1);
    let f = PathFollower::new(g.plan(&ch, goal, TravelStyle::Normal).expect("route"), goal, TravelStyle::Normal);
    assert_eq!(f.current_move(), Some(MoveKind::WallJump));
    assert!(!f.dive_ok(&ch));
}

#[test]
fn a_hop_that_keeps_slipping_is_retried_then_given_up() {
    // Plan with the window's top in place, then follow on a terrain where it vanished (same
    // generation, so only the landing checks notice): every climb ends back on the floor.
    // The follower retries, then avoids and excludes the failing hops and reports Failed
    // instead of looping forever.
    let planned = desktop(vec![primary()], vec![win(1, 1000, 300, 1400, 1400)]);
    let mut real = planned.clone();
    real.ledges.retain(|l| l.owner != Some(1));
    let mut ch = knight(&planned, 300.0, FLOOR);
    let g = NavGraph::build(&planned, &ch);
    let goal = NavGoal::OnWindow(1);
    let mut f = PathFollower::new(g.plan(&ch, goal, TravelStyle::Normal).expect("route"), goal, TravelStyle::Normal);
    let (status, _) = drive(&g, &real, &mut ch, &mut f, 120.0);
    assert_eq!(status, FollowStatus::Failed, "at {:?}", ch.pos);
    assert!(f.replans() >= 3, "a slip is retried before giving up ({} re-plans)", f.replans());
}

#[test]
fn climb_edges_have_their_walls_and_draw_distinctly() {
    let t = desktop(vec![primary()], vec![win(1, 600, 300, 900, 1400), win(2, 1000, 300, 1300, 1400)]);
    let ch = knight(&t, 950.0, FLOOR);
    let g = NavGraph::build(&t, &ch);
    let climbs: Vec<&Edge> = g.edges().iter().filter(|e| e.kind == MoveKind::WallJump).collect();
    assert!(!climbs.is_empty());
    for e in &climbs {
        let c = e.climb.expect("climb edges carry their wall");
        let p = edge_path(e, g.surfaces()[e.from].y, false);
        assert!(p.climb);
        assert_eq!(p.points.first().map(|q| q.x), Some(e.takeoff));
        assert_eq!(p.points.last().copied(), Some(e.land));
        let contacts: Vec<Vec2> = p.wall_contacts().collect();
        assert!(contacts.len() >= 2, "{contacts:?}");
        assert!(contacts.iter().all(|q| q.x == c.wall.x && q.y >= c.wall.y0 - 0.5));
    }
    // Ordinary edges are not drawn as climbs.
    assert!(g.edges().iter().filter(|e| e.climb.is_none()).all(|e| !edge_path(e, g.surfaces()[e.from].y, false).climb));
}

#[test]
fn step_behind_a_drawn_line_instead_of_dashing() {
    // A tall divider drawn in a window (a prompt box border) from near the top to the floor:
    // too tall to hop. The knight turns into the screen and walks behind it, in every style.
    let desk = DesktopSnapshot { generation: 1, monitors: vec![primary()], windows: vec![win(1, 200, 100, 1800, 1400)] };
    let walls = [
        ContentWallSeg { x: 1000, y0: 200, y1: 1400, face: Facing::Left, owner: Some(1) },
        ContentWallSeg { x: 1002, y0: 200, y1: 1400, face: Facing::Right, owner: Some(1) },
    ];
    let t = Terrain::from_desktop_and_content(&desk, &[], &walls, &TerrainOptions::default());
    for style in [TravelStyle::Stroll, TravelStyle::Normal] {
        for (from, to) in [(700.0, 1300.0), (1300.0, 700.0)] {
            let mut ch = knight(&t, from, FLOOR);
            let (status, moves) = follow(&t, &mut ch, NavGoal::Point(Vec2::new(to, FLOOR)), style);
            assert_eq!(status, FollowStatus::Arrived, "{style:?} {from}->{to}: {moves:?} at {:?}", ch.pos);
            assert!(moves.contains(&MoveKind::Behind), "{style:?} {from}->{to}: {moves:?}");
            assert!(!moves.iter().any(|m| matches!(m, MoveKind::ShadowDash | MoveKind::Superdash)), "{moves:?}");
            assert!((ch.pos.x - to).abs() < 6.0, "{:?}", ch.pos);
        }
    }
}

#[test]
fn hanging_walls_that_stop_short_of_the_floor() {
    // Walls whose bottom ends above the floor but below the knight's head still block him.
    // Content lines (a box border ending just above the taskbar) and a window floating a
    // little above the floor.
    let mut fails = vec![];
    for gap in [4, 12, 25, 40] {
        let desk = DesktopSnapshot { generation: 1, monitors: vec![primary()], windows: vec![win(1, 200, 100, 1800, 1400)] };
        let walls = [
            ContentWallSeg { x: 1000, y0: 900, y1: 1400 - gap, face: Facing::Left, owner: Some(1) },
            ContentWallSeg { x: 1002, y0: 900, y1: 1400 - gap, face: Facing::Right, owner: Some(1) },
            // A short prompt box (120 px tall) floating `gap` above the floor.
            ContentWallSeg { x: 1200, y0: 1280 - gap, y1: 1400 - gap, face: Facing::Left, owner: Some(1) },
            ContentWallSeg { x: 1202, y0: 1280 - gap, y1: 1400 - gap, face: Facing::Right, owner: Some(1) },
        ];
        let t = Terrain::from_desktop_and_content(&desk, &[], &walls, &TerrainOptions::default());
        let floating = desktop(vec![primary()], vec![win(2, 900, 700, 1300, 1400 - gap)]);
        for (name, t) in [("content", &t), ("window", &floating)] {
            for style in [TravelStyle::Stroll, TravelStyle::Normal, TravelStyle::Hurry] {
                // Strolls never climb: past a tall floating window they stop at the nearest point.
                if name == "window" && style == TravelStyle::Stroll {
                    continue;
                }
                for (from, to) in [(700.0, 1500.0), (1500.0, 700.0)] {
                    let mut ch = knight(t, from, FLOOR);
                    let g = NavGraph::build(t, &ch);
                    let goal = NavGoal::Point(Vec2::new(to, FLOOR));
                    let Some(route) = g.plan(&ch, goal, style) else {
                        fails.push(format!("{name} gap {gap} {style:?} {from}->{to}: no route"));
                        continue;
                    };
                    let moves = route.moves.clone();
                    let mut f = PathFollower::new(route, goal, style);
                    let (status, _) = drive(&g, t, &mut ch, &mut f, 40.0);
                    if status != FollowStatus::Arrived || (ch.pos.x - to).abs() > 6.0 {
                        fails.push(format!("{name} gap {gap} {style:?} {from}->{to}: {status:?} {moves:?} at {:?} replans {}", ch.pos, f.replans()));
                    }
                }
            }
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
