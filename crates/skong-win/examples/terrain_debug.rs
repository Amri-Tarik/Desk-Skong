//! Terrain debug overlay: draws the live window + content terrain over the whole virtual
//! desktop.
//!
//! OneWay ledges green, Ground blue, content ledges magenta, walls orange. Click-through,
//! never takes focus, excluded from screen capture (so it never detects itself). Redraws
//! when the desktop snapshot or the detected content changes, and prints per-update
//! content timings (frames, dirty area, analysis time, frame-to-ledges latency).
//!
//! Quit with **Ctrl+Alt+Q**, or pass `--seconds N` to exit automatically.
//! `--verbose` prints every ledge/wall on each change, `--no-vision` disables content
//! capture, `--wallpaper` keeps content ledges on the desktop wallpaper, `--no-overlay`
//! runs headless (timings and CPU only), `--repaint-hz N` re-presents the overlay N times
//! a second.
//!
//! ```text
//! cargo run -p skong-win --example terrain_debug --release
//! ```
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use skong_core::desktop::DesktopSnapshot;
use skong_core::terrain::{LedgeKind, Terrain, TerrainOptions};
use skong_win::capture::{CaptureStats, ContentFeed};
use skong_win::debug_overlay::{DebugOverlay, Scene};
use skong_win::desktop::{DesktopWatcher, enable_per_monitor_dpi};
use windows::Win32::Foundation::{FILETIME, WAIT_FAILED};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage, WM_HOTKEY, WM_QUIT,
};

const HOTKEY_QUIT: i32 = 1;

/// Terrain layer of the shared debug overlay (the pet can also draw the nav graph).
fn scene(terrain: &Terrain) -> Scene<'_> {
    Scene { terrain: Some(terrain), legend: true, ..Default::default() }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    enable_per_monitor_dpi();
    // SAFETY: thread-level hotkey; WM_HOTKEY arrives in this thread's queue.
    unsafe { RegisterHotKey(None, HOTKEY_QUIT, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, u32::from(b'Q')) }.context("RegisterHotKey Ctrl+Alt+Q")?;

    let mut watcher = DesktopWatcher::new()?;
    let opts = TerrainOptions { include_wallpaper_content: args.wallpaper, ..TerrainOptions::default() };
    print_snapshot(watcher.snapshot());
    println!("enumerate: {:?}", watcher.last_enumerate_cost());
    let mut feed = ContentFeed::new(args.vision, watcher.snapshot());
    println!("content capture: {}", if feed.is_running() { "running" } else { "off" });
    let mut terrain = build_timed(watcher.snapshot(), &feed, &opts, true);
    print_terrain(&terrain, args.verbose);

    let mut overlay = if args.overlay {
        let mut o = DebugOverlay::new(terrain.bounds)?;
        o.draw(&scene(&terrain));
        o.present()?;
        println!("overlay up; Ctrl+Alt+Q to quit");
        Some(o)
    } else {
        println!("headless (--no-overlay): stop with --seconds or Ctrl+C");
        None
    };
    let mut summary = Summary::default();
    let started = Instant::now();
    let mut last_repaint = Instant::now();

    let deadline = args.seconds.map(|s| Instant::now() + Duration::from_secs_f64(s));
    let (mut changes, mut updates) = (0u32, 0u32);
    while pump_messages() {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            println!("--seconds elapsed");
            break;
        }
        if let (Some(hz), Some(o)) = (args.repaint_hz, overlay.as_mut())
            && last_repaint.elapsed().as_secs_f64() >= 1.0 / hz
        {
            last_repaint = Instant::now();
            o.present()?;
        }
        let desk_changed = watcher.poll();
        let content_changed = feed.poll(watcher.snapshot(), desk_changed);
        for s in feed.stats() {
            updates += 1;
            summary.add(s);
            println!(
                "content update #{updates}: output {} frames {} dirty {} px, analysis {:.2} ms, frame->ledges latency {:.1} ms, {} surfaces",
                s.source,
                s.frames,
                s.dirty_px,
                s.analysis.as_secs_f64() * 1e3,
                s.latency.as_secs_f64() * 1e3,
                s.surfaces
            );
        }
        if !desk_changed && !content_changed {
            continue;
        }
        if desk_changed {
            changes += 1;
            println!("change #{changes}: gen {} enumerate {:?}", watcher.snapshot().generation, watcher.last_enumerate_cost());
        }
        terrain = build_timed(watcher.snapshot(), &feed, &opts, desk_changed);
        if content_changed {
            let n = terrain.ledges.iter().filter(|l| l.kind == LedgeKind::Content).count();
            println!("  -> {n} content ledges in terrain");
        }
        print_terrain(&terrain, args.verbose);
        if let Some(o) = overlay.as_mut() {
            if terrain.bounds != o.bounds() {
                *o = DebugOverlay::new(terrain.bounds)?;
            }
            o.draw(&scene(&terrain));
            o.present()?;
        }
    }
    feed.shutdown();
    // SAFETY: unregistering the hotkey registered in main.
    let _ = unsafe { UnregisterHotKey(None, HOTKEY_QUIT) };
    println!("exiting after {changes} desktop changes, {updates} content updates");
    summary.print(started.elapsed());
    Ok(())
}

struct Args {
    seconds: Option<f64>,
    verbose: bool,
    vision: bool,
    wallpaper: bool,
    overlay: bool,
    /// Re-present the overlay this often (tests whether capture-excluded windows still
    /// cause dirty rects).
    repaint_hz: Option<f64>,
}

impl Args {
    fn parse() -> Self {
        let mut args = Args { seconds: None, verbose: false, vision: true, wallpaper: false, overlay: true, repaint_hz: None };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--seconds" => args.seconds = it.next().and_then(|s| s.parse().ok()),
                "--verbose" | "-v" => args.verbose = true,
                "--no-vision" => args.vision = false,
                "--wallpaper" => args.wallpaper = true,
                "--no-overlay" => args.overlay = false,
                "--repaint-hz" => args.repaint_hz = it.next().and_then(|s| s.parse().ok()),
                other => eprintln!("ignoring unknown argument {other}"),
            }
        }
        args
    }
}

/// Block until a message arrives or ~16 ms pass, then drain the queue.
/// Returns `false` when the app should quit.
fn pump_messages() -> bool {
    // SAFETY: standard message pump on this thread's queue.
    unsafe {
        if MsgWaitForMultipleObjects(None, false, 16, QS_ALLINPUT) == WAIT_FAILED {
            return false;
        }
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT || (msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_QUIT as usize) {
                println!("quit requested");
                return false;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    true
}

fn build_timed(desk: &DesktopSnapshot, feed: &ContentFeed, opts: &TerrainOptions, print: bool) -> Terrain {
    let start = Instant::now();
    let t = feed.terrain(desk, opts);
    if print {
        println!(
            "terrain build: {:?} ({} boxes, {} ledges, {} walls)",
            start.elapsed(),
            t.windows.len(),
            t.ledges.len(),
            t.walls.len()
        );
    }
    t
}

fn print_snapshot(desk: &DesktopSnapshot) {
    println!("{} monitor(s):", desk.monitors.len());
    for m in &desk.monitors {
        println!("  rect {:?} work {:?} primary={} scale={}", m.rect, m.work, m.primary, m.scale_milli);
    }
    println!("{} window(s), front to back:", desk.windows.len());
    for w in &desk.windows {
        let flags: String = [(w.foreground, 'F'), (w.topmost, 'T'), (w.maximized, 'M'), (w.fullscreen, 'X')]
            .iter()
            .map(|&(on, c)| if on { c } else { '.' })
            .collect();
        println!(
            "  {flags} {:>5},{:>5} {:>5}x{:<5} {:<20} {:<28} {:?}",
            w.rect.left,
            w.rect.top,
            w.rect.width(),
            w.rect.height(),
            truncate(&w.process, 20),
            truncate(&w.class, 28),
            truncate(&w.title, 50)
        );
    }
}

fn print_terrain(t: &Terrain, verbose: bool) {
    if !verbose {
        return;
    }
    for l in &t.ledges {
        println!("  ledge {:?} y={} x={}..{} owner={:?}", l.kind, l.y, l.x0, l.x1, l.owner);
    }
    for w in &t.walls {
        println!("  wall {:?} x={} y={}..{} owner={:?}", w.face, w.x, w.y0, w.y1, w.owner);
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Aggregate content-capture timings and process CPU use.
#[derive(Default)]
struct Summary {
    updates: u32,
    frame_updates: u32,
    frames: u64,
    dirty_px: u64,
    analysis: Duration,
    max_analysis: Duration,
    latency: Duration,
    max_latency: Duration,
    analyses: u64,
    busy: Duration,
}

impl Summary {
    fn add(&mut self, s: &CaptureStats) {
        self.updates += 1;
        self.analyses += u64::from(s.analyses);
        self.busy += s.busy;
        self.frames += u64::from(s.frames);
        self.dirty_px += s.dirty_px;
        self.analysis += s.analysis;
        self.max_analysis = self.max_analysis.max(s.analysis);
        if s.frames > 0 {
            self.frame_updates += 1;
            self.latency += s.latency;
            self.max_latency = self.max_latency.max(s.latency);
        }
    }

    fn print(&self, wall: Duration) {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        if self.updates > 0 {
            println!(
                "content: {} updates ({} frame-driven, {} frames), dirty {:.1} Mpx, analysis mean {:.2} ms max {:.2} ms",
                self.updates,
                self.frame_updates,
                self.frames,
                self.dirty_px as f64 / 1e6,
                ms(self.analysis) / f64::from(self.updates),
                ms(self.max_analysis)
            );
            println!("analyses incl. unchanged: {} totalling {:.1} ms", self.analyses, ms(self.busy));
        }
        if self.frame_updates > 0 {
            println!(
                "frame->ledges latency mean {:.1} ms max {:.1} ms",
                ms(self.latency) / f64::from(self.frame_updates),
                ms(self.max_latency)
            );
        }
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: querying our own process; out-pointers reference locals.
        if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.is_ok() {
            let secs = |f: FILETIME| ((u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)) as f64 * 1e-7;
            let cpu = secs(k) + secs(u);
            println!(
                "process CPU: {:.2} s (user {:.2} s, kernel {:.2} s) over {:.1} s wall = {:.1}% of one core",
                cpu,
                secs(u),
                secs(k),
                wall.as_secs_f64(),
                100.0 * cpu / wall.as_secs_f64().max(1e-9)
            );
        }
    }
}
