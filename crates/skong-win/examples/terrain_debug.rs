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
use skong_core::canvas::premul;
use skong_core::desktop::DesktopSnapshot;
use skong_core::geom::RectF;
use skong_core::terrain::{Facing, LedgeKind, Terrain, TerrainOptions};
use skong_win::capture::{CaptureStats, ContentFeed};
use skong_win::desktop::{DesktopWatcher, enable_per_monitor_dpi, mark_dirty};
use windows::Win32::Foundation::{COLORREF, FILETIME, HWND, LPARAM, LRESULT, POINT, SIZE, WAIT_FAILED, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE,
    PeekMessageW, QS_ALLINPUT, RegisterClassW, SW_SHOWNOACTIVATE, SetWindowDisplayAffinity, ShowWindow,
    TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_HOTKEY, WM_QUIT, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

const HOTKEY_QUIT: i32 = 1;
const LINE: i32 = 3;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    enable_per_monitor_dpi();

    let mut watcher = DesktopWatcher::new()?;
    let opts = TerrainOptions { include_wallpaper_content: args.wallpaper, ..TerrainOptions::default() };
    print_snapshot(watcher.snapshot());
    println!("enumerate: {:?}", watcher.last_enumerate_cost());
    let mut feed = ContentFeed::new(args.vision, watcher.snapshot());
    println!("content capture: {}", if feed.is_running() { "running" } else { "off" });
    let mut terrain = build_timed(watcher.snapshot(), &feed, &opts, true);
    print_terrain(&terrain, args.verbose);

    let mut overlay = if args.overlay {
        let mut o = Overlay::new(terrain.bounds)?;
        o.draw(&terrain);
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
            if terrain.bounds != o.bounds {
                *o = Overlay::new(terrain.bounds)?;
            }
            o.draw(&terrain);
            o.present()?;
        }
    }
    feed.shutdown();
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

/// Window procedure: forward display changes to the watcher, default for the rest.
extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if matches!(msg, WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED) {
        mark_dirty();
    }
    // SAFETY: forwarding the original arguments.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Full-desktop layered window backed by a top-down 32-bpp premultiplied DIB.
struct Overlay {
    hwnd: HWND,
    bounds: RectF,
    origin: POINT,
    size: SIZE,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}

impl Overlay {
    fn new(bounds: RectF) -> Result<Self> {
        let origin = POINT { x: bounds.left as i32, y: bounds.top as i32 };
        let size = SIZE { cx: (bounds.width() as i32).max(1), cy: (bounds.height() as i32).max(1) };
        let hwnd = create_window(origin, size)?;
        // SAFETY: valid window owned by this thread. Affinity is best-effort.
        unsafe {
            if let Err(e) = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
                eprintln!("SetWindowDisplayAffinity failed: {e}");
            }
            RegisterHotKey(Some(hwnd), HOTKEY_QUIT, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, u32::from(b'Q'))
                .context("RegisterHotKey Ctrl+Alt+Q")?;
        }
        let (dc, bitmap, bits) = create_dib(size)?;
        // SAFETY: select our bitmap into our DC; restored in Drop.
        let old = unsafe { SelectObject(dc, bitmap.into()) };
        // SAFETY: valid window; SW_SHOWNOACTIVATE never steals focus.
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Ok(Self { hwnd, bounds, origin, size, dc, bitmap, old, bits })
    }

    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `bits` points to cx*cy u32s owned by `bitmap`, alive as long as self.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.size.cx * self.size.cy) as usize) }
    }

    fn draw(&mut self, t: &Terrain) {
        let (ox, oy, w, h) = (self.origin.x, self.origin.y, self.size.cx, self.size.cy);
        let green = premul(40, 230, 60, 230);
        let blue = premul(40, 120, 255, 230);
        let orange = premul(255, 140, 20, 230);
        let magenta = premul(255, 40, 220, 230);
        let px = self.pixels();
        px.fill(0);
        let mut fill = |x0: f32, y0: f32, x1: f32, y1: f32, c: u32| {
            let (x0, x1) = ((x0 as i32 - ox).clamp(0, w), (x1 as i32 - ox).clamp(0, w));
            let (y0, y1) = ((y0 as i32 - oy).clamp(0, h), (y1 as i32 - oy).clamp(0, h));
            for y in y0..y1 {
                px[(y * w + x0) as usize..(y * w + x1) as usize].fill(c);
            }
        };
        let half = (LINE / 2) as f32;
        for l in &t.ledges {
            let c = match l.kind {
                LedgeKind::Ground => blue,
                LedgeKind::OneWay => green,
                LedgeKind::Content => magenta,
            };
            fill(l.x0, l.y - half, l.x1, l.y - half + LINE as f32, c);
        }
        for wall in &t.walls {
            // Draw on the solid side's inside so adjacent window walls stay distinguishable.
            let x0 = match wall.face {
                Facing::Left => wall.x,
                Facing::Right => wall.x - LINE as f32,
            };
            fill(x0, wall.y0, x0 + LINE as f32, wall.y1, orange);
        }
    }

    fn present(&self) -> Result<()> {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let src = POINT { x: 0, y: 0 };
        // SAFETY: all pointers reference locals/fields alive for the call.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&self.origin),
                Some(&self.size),
                Some(self.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .context("UpdateLayeredWindow")
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // SAFETY: tearing down resources created in `new`, in reverse order.
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_QUIT);
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn create_window(origin: POINT, size: SIZE) -> Result<HWND> {
    // SAFETY: registering a class with a 'static wndproc and name; creating a popup
    // window on this thread.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("SkongTerrainDebug");
        let wc =
            WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
        // Re-registration (overlay recreated after a display change) fails harmlessly.
        RegisterClassW(&wc);
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
        CreateWindowExW(
            ex,
            class,
            w!("skong terrain debug"),
            WS_POPUP,
            origin.x,
            origin.y,
            size.cx,
            size.cy,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .context("CreateWindowExW")
    }
}

fn create_dib(size: SIZE) -> Result<(HDC, HBITMAP, *mut u32)> {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size.cx,
            biHeight: -size.cy, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: memory DC compatible with the screen; DIB section with a valid header.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            anyhow::bail!("CreateCompatibleDC failed");
        }
        let mut bits = std::ptr::null_mut();
        match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => Ok((dc, bitmap, bits.cast())),
            Err(e) => {
                let _ = DeleteDC(dc);
                Err(e).context("CreateDIBSection")
            }
        }
    }
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
