//! Soak test for the planner + follower: random desktops in a realistic two-monitor layout,
//! random starts and window goals, driven through `Character::step` exactly like the app.
//! Reports arrival rates per style and which move kinds the failures were executing.
//!
//! cargo run -p skong-core --example nav_soak --release -- [desktops=40] [scale=0.6] [windows=8]
use std::collections::BTreeMap;

use skong_core::controller::{Abilities, Character, CharacterParams, MoveInput, MoveState};
use skong_core::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use skong_core::geom::{RectI, Vec2};
use skong_core::nav::{FollowStatus, MoveKind, NavGoal, NavGraph, PathFollower, TravelStyle};
use skong_core::terrain::{Terrain, TerrainOptions};

const DT: f32 = 1.0 / 120.0;
/// Point goals on the monitor floors per desktop (on top of every window top).
const FLOOR_GOALS: usize = 3;

struct Rng(u64);

impl Rng {
    fn next(&mut self, lo: i32, hi: i32) -> i32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        lo + ((self.0 >> 33) % (hi - lo).max(1) as u64) as i32
    }
}

fn window(id: u64, rect: RectI) -> WindowInfo {
    WindowInfo {
        id,
        rect,
        title: String::new(),
        class: String::new(),
        process: String::new(),
        pid: 0,
        maximized: false,
        fullscreen: false,
        foreground: false,
        topmost: false,
    }
}

/// 2560x1440 primary (taskbar 48 px) with 1920x1080 to its left, tops aligned.
fn desktop(rng: &mut Rng, n: usize) -> DesktopSnapshot {
    let monitors = vec![
        Monitor { rect: RectI::new(0, 0, 2560, 1440), work: RectI::new(0, 0, 2560, 1392), primary: true, scale_milli: 1000 },
        Monitor { rect: RectI::new(-1920, 0, 0, 1080), work: RectI::new(-1920, 0, 0, 1080), primary: false, scale_milli: 1000 },
    ];
    let windows = (0..n)
        .map(|i| {
            let (w, h) = (rng.next(300, 1300), rng.next(250, 1100));
            let (x, y) = (rng.next(-1920, 2560 - w), rng.next(0, 1392 - h + 1));
            window(i as u64 + 1, RectI::new(x, y, x + w, y + h))
        })
        .collect();
    DesktopSnapshot { generation: 1, monitors, windows }
}

fn settle(t: &Terrain, mut c: Character) -> Option<Character> {
    let mut ev = Vec::new();
    for _ in 0..600 {
        c.step(&MoveInput::default(), t, DT, &mut ev);
        if c.state == MoveState::Grounded {
            return Some(c);
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let desktops: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(40);
    let scale: f32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0.6);
    let n: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(8);
    let params = CharacterParams::hollow_knight(64.0 * scale);
    let mut tally: BTreeMap<String, [usize; 4]> = BTreeMap::new(); // planned, arrived, failed, timeout
    let mut failing: BTreeMap<String, usize> = BTreeMap::new();
    if let Ok(spec) = std::env::var("SOAK_DUMP") {
        let mut it = spec.split(':').filter_map(|v| v.parse::<u64>().ok());
        if let (Some(seed), Some(w)) = (it.next(), it.next()) {
            dump_case(seed, w, n, params);
        }
        return;
    }
    let mut build_ms = Vec::new();
    let mut superdash_edges = Vec::new();
    let mut superdash_routes: BTreeMap<String, usize> = BTreeMap::new();
    // Total seconds to arrival per style (arrived routes only).
    let mut travel: BTreeMap<String, f32> = BTreeMap::new();
    let mut dives = 0usize;
    let mut hard_landings = 0usize;
    for seed in 1..=desktops {
        let mut rng = Rng(seed * 7919);
        let desk = desktop(&mut rng, n);
        let t = Terrain::from_desktop(&desk, &TerrainOptions::default());
        let start_x = rng.next(-1800, 2500) as f32;
        let Some(ch0) = settle(&t, Character::new(params, Abilities::default(), Vec2::new(start_x, 0.0))) else { continue };
        let g = NavGraph::build(&t, &ch0);
        build_ms.push(g.stats().millis);
        superdash_edges.push(g.edge_counts()[MoveKind::ALL.iter().position(|k| *k == MoveKind::Superdash).unwrap_or(0)]);
        // Window tops, plus a few points on the floors (long level runs: where a hurry
        // should crystal dash).
        let mut goals: Vec<(NavGoal, &str, Option<u64>)> = desk.windows.iter().map(|w| (NavGoal::OnWindow(w.id), "", Some(w.id))).collect();
        for _ in 0..FLOOR_GOALS {
            let (x, floor) = if rng.next(0, 2) == 0 { (rng.next(-1880, -40), 1080.0) } else { (rng.next(40, 2520), 1392.0) };
            goals.push((NavGoal::Point(Vec2::new(x as f32, floor)), " floor", None));
        }
        for (gi, &(goal, label, win)) in goals.iter().enumerate() {
            // Trace / tick dump id: the window id, or 100 + index for a floor point.
            let case = win.unwrap_or(100 + gi as u64);
            for style in [TravelStyle::Normal, TravelStyle::Hurry] {
                let visible = win.is_some_and(|id| t.ledges.iter().any(|l| l.owner == Some(id) && l.y - 50.0 * scale / 0.6 > t.bounds.top));
                let Some(route) = g.plan(&ch0, goal, style) else {
                    if visible {
                        *tally.entry(format!("{style:?} unreachable (visible top)")).or_insert([0; 4]).first_mut().unwrap_or(&mut 0) += 1;
                        if std::env::var("SOAK_UNREACH").is_ok() { println!("seed {seed} {style:?}: no route from {:?} to {goal:?}", ch0.pos); }
                    }
                    continue;
                };
                let key = format!("{style:?}{label}");
                if route.moves.contains(&MoveKind::Superdash) {
                    *superdash_routes.entry(key.clone()).or_default() += 1;
                }
                let entry = tally.entry(key).or_insert([0; 4]);
                entry[0] += 1;
                let mut ch = ch0.clone();
                let planned = route.moves.clone();
                let mut f = PathFollower::new(route, goal, style);
                let mut ev = Vec::new();
                let mut result = 3;
                let mut last_kind = None;
                let mut trace_moves: Option<Vec<MoveKind>> = None;
                for tick in 0..(60.0 / DT) as usize {
                    let (input, status) = f.next_input(&ch, &g, &t, DT);
                    if status == FollowStatus::Arrived {
                        *travel.entry(format!("{style:?}{label}")).or_default() += tick as f32 * DT;
                    }
                    if std::env::var("SOAK_TRACE").ok().as_deref() == Some(&format!("{seed}:{case}:{style:?}")) && (Some(f.route().moves.clone()) != trace_moves || f.current_move() != last_kind) {
                        trace_moves = Some(f.route().moves.clone());
                        println!("  t {:.2} at {:?} {:?} route {:?} step {:?} eta {:.2} end {:?} edges {:?}", 0.0, ch.pos, ch.state, f.route().moves, f.current_move(), f.route().eta, f.route().end, f.route().edges().map(|e| (e.takeoff, e.land, e.kind)).collect::<Vec<_>>());
                    }
                    if let Some(k) = f.current_move() {
                        last_kind = Some(k);
                    }
                    match status {
                        FollowStatus::Arrived => {
                            result = 1;
                            break;
                        }
                        FollowStatus::Failed => {
                            result = 2;
                            break;
                        }
                        FollowStatus::Moving => {}
                    }
                    // Like the app: dive on big straight drops (excited on odd seeds).
                    let mut input = input;
                    if ch.hard_landing_ahead(&t) && !(f.dive_ok(&ch) && ch.auto_dive(&t, &input, seed % 2 == 1).is_some()) {
                        input.dive_pressed = true;
                        input.dive_dark = true;
                        dives += 1;
                    } else if f.dive_ok(&ch) && let Some(dark) = ch.auto_dive(&t, &input, seed % 2 == 1) {
                        input.dive_pressed = true;
                        input.dive_dark = dark;
                        dives += 1;
                    }
                    ev.clear();
                    ch.step(&input, &t, DT, &mut ev);
                    hard_landings += ev.iter().filter(|e| matches!(e, skong_core::controller::MoveEvent::Landed { hard: true })).count();
                    if std::env::var("SOAK_TICKS").ok().as_deref() == Some(&format!("{seed}:{case}:{style:?}")) && f.route().moves.len() == 1 {
                        println!("    x {:.1} y {:.1} v {:.0},{:.0} in {:.2} j{} d{} dn{} {:?} {:?}", ch.pos.x, ch.pos.y, ch.vel.x, ch.vel.y, input.x, u8::from(input.jump_pressed), u8::from(input.dash_pressed), u8::from(input.down), ch.state, ev);
                    }
                }
                entry[result] += 1;
                if result != 1 {
                    let what = if result == 2 { "failed" } else { "timeout" };
                    if std::env::var("SOAK_VERBOSE").is_ok() { println!("  replan now: {:?} surface {:?}", g.plan(&ch, goal, style).map(|r| r.moves), g.surface_at(&ch)); }
                    *failing.entry(format!("{what} during {:?}", last_kind.unwrap_or(MoveKind::Walk))).or_default() += 1;
                    if std::env::var("SOAK_VERBOSE").is_ok() {
                        println!("seed {seed} {style:?} -> {goal:?} (case {case}): {what} at {:?} ({:?}) during {last_kind:?}, {} re-plans, route {:?}", ch.pos, ch.state, f.replans(), planned);
                    }
                }
                let _ = f.replans();
            }
        }
    }
    for (style, [planned, arrived, failed, timeout]) in &tally {
        let sd = superdash_routes.get(style).copied().unwrap_or(0);
        let mean = travel.get(style).copied().unwrap_or(0.0) / (*arrived).max(1) as f32;
        println!(
            "{style}: {arrived}/{planned} arrived ({:.1}%), {failed} failed, {timeout} timed out; {sd} routes crystal dash; mean arrival {mean:.2} s",
            100.0 * *arrived as f32 / (*planned).max(1) as f32
        );
    }
    superdash_edges.sort_unstable();
    if let (Some(med), Some(max)) = (superdash_edges.get(superdash_edges.len() / 2), superdash_edges.last()) {
        println!("superdash edges per graph: median {med}, max {max}");
    }
    for (what, n) in &failing {
        println!("  {what}: {n}");
    }
    println!("auto-dives during routes: {dives}");
    println!("hard landings during routes (fatal now): {hard_landings}");
    build_ms.sort_by(f32::total_cmp);
    if let (Some(med), Some(max)) = (build_ms.get(build_ms.len() / 2), build_ms.last()) {
        println!("graph build: median {med:.2} ms, max {max:.2} ms over {} desktops", build_ms.len());
    }
}

/// `SOAK_DUMP=seed:window`: print the terrain, graph and edges around one case.
#[allow(dead_code)]
fn dump_case(seed: u64, win_id: u64, n: usize, params: CharacterParams) {
    let mut rng = Rng(seed * 7919);
    let desk = desktop(&mut rng, n);
    let t = Terrain::from_desktop(&desk, &TerrainOptions::default());
    let start_x = rng.next(-1800, 2500) as f32;
    let Some(ch0) = settle(&t, Character::new(params, Abilities::default(), Vec2::new(start_x, 0.0))) else { return };
    let g = NavGraph::build(&t, &ch0);
    for w in &desk.windows {
        println!("window {} {:?}", w.id, w.rect);
    }
    for w in &t.walls {
        println!("wall x {} y {}..{} {:?} {:?} {:?}", w.x, w.y0, w.y1, w.face, w.owner, w.kind);
    }
    println!("knight {:?} on {:?}", ch0.pos, g.surface_at(&ch0));
    for (i, s) in g.surfaces().iter().enumerate() {
        let tag = if s.owner == Some(win_id) { " <== target" } else { "" };
        println!("S{i} {}..{} @{} stand {}..{} {:?}{tag}", s.x0, s.x1, s.y, s.lo, s.hi, s.owner);
        for e in g.edges_from(i) {
            println!("    -> S{} {:?} {:?} takeoff {} land {:?}", e.to, e.kind, e.gait, e.takeoff, e.land);
        }
    }
}
