//! Nav graph build / rebuild / plan timings on synthetic busy desktops.
//! cargo run -p skong-core --example nav_bench --release -- [windows=30] [seeds=5]
use std::time::Instant;

use skong_core::controller::{Abilities, Character, CharacterParams};
use skong_core::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use skong_core::geom::{RectI, Vec2};
use skong_core::nav::{MoveKind, NavGoal, NavGraph, TravelStyle};
use skong_core::terrain::{Terrain, TerrainOptions};

/// Tiny deterministic LCG so runs are comparable.
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

/// 1920x1080 + 2560x1440 side by side (taskbar on the first), `n` overlapping windows.
fn desktop(n: usize, seed: u64) -> DesktopSnapshot {
    let monitors = vec![
        Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 },
        Monitor { rect: RectI::new(1920, -200, 4480, 1240), work: RectI::new(1920, -200, 4480, 1240), primary: false, scale_milli: 1000 },
    ];
    let mut rng = Rng(seed);
    let windows = (0..n)
        .map(|i| {
            let (w, h) = (rng.next(300, 1400), rng.next(200, 900));
            let (x, y) = (rng.next(-100, 4480 - w + 100), rng.next(-200, 1240 - h));
            window(i as u64 + 1, RectI::new(x, y, x + w, y + h))
        })
        .collect();
    DesktopSnapshot { generation: 1, monitors, windows }
}

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let n = args.first().copied().unwrap_or(30);
    let seeds = args.get(1).copied().unwrap_or(5) as u64;
    let opts = TerrainOptions::default();
    let template = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(960.0, 1040.0));
    let mut kinds = [0usize; 9];
    for seed in 1..=seeds {
        let mut desk = desktop(n, seed);
        let terrain = Terrain::from_desktop(&desk, &opts);

        // Cold build (best of 3).
        let mut best = f32::INFINITY;
        let mut graph = NavGraph::default();
        for _ in 0..3 {
            let t = Instant::now();
            graph = NavGraph::build(&terrain, &template);
            best = best.min(t.elapsed().as_secs_f32() * 1000.0);
        }
        let s = graph.stats();
        for (k, c) in kinds.iter_mut().zip(graph.edge_counts()) {
            *k += c;
        }

        // Drag one window by 40 px and rebuild incrementally.
        desk.generation += 1;
        if let Some(w) = desk.windows.get_mut(n / 2) {
            w.rect = RectI::new(w.rect.left + 40, w.rect.top + 25, w.rect.right + 40, w.rect.bottom + 25);
        }
        let moved = Terrain::from_desktop(&desk, &opts);
        let t = Instant::now();
        let rebuilt = graph.rebuild(&moved, &template);
        let inc = t.elapsed().as_secs_f32() * 1000.0;
        let r = rebuilt.stats();

        // Plan from the ground to every window, all styles.
        let mut ch = template.clone();
        let mut ev = Vec::new();
        for _ in 0..600 {
            ch.step(&Default::default(), &moved, 1.0 / 120.0, &mut ev);
        }
        let (mut plans, mut found, mut plan_ms) = (0, [0; 3], 0.0f32);
        for w in &desk.windows {
            for (i, style) in [TravelStyle::Stroll, TravelStyle::Normal, TravelStyle::Hurry].into_iter().enumerate() {
                let t = Instant::now();
                let route = rebuilt.plan(&ch, NavGoal::OnWindow(w.id), style);
                plan_ms += t.elapsed().as_secs_f32() * 1000.0;
                plans += 1;
                found[i] += usize::from(route.is_some());
            }
        }
        println!(
            "seed {seed}: {} surfaces, {} edges, {} pairs, {} sims ({} ticks) | build {best:.2} ms | rebuild after drag {inc:.2} ms ({} sims, {} cache hits) | plan {:.1} us avg; windows reachable stroll/normal/hurry {}/{}/{} of {}",
            s.surfaces,
            s.edges,
            s.pairs,
            s.sims,
            s.ticks,
            r.sims,
            r.cache_hits,
            plan_ms * 1000.0 / plans as f32,
            found[0],
            found[1],
            found[2],
            desk.windows.len(),
        );
    }
    let total: Vec<String> = MoveKind::ALL.iter().zip(kinds).map(|(k, c)| format!("{k:?} {c}")).collect();
    println!("edge kinds over all seeds: {}", total.join(", "));
}
