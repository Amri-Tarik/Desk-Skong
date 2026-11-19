//! Live content terrain: DXGI Desktop Duplication → [`skong_core::vision`] → terrain.
//!
//! # Design
//!
//! * One background thread per monitor (DXGI output), each with its own D3D11 device
//!   and `IDXGIOutputDuplication`. The thread blocks in `AcquireNextFrame`, so an idle
//!   desktop costs nothing but a wake-up every [`IDLE_WAKE`] to check for shutdown.
//! * **Paced acquisition**: after a frame (or a pointer-only wake-up) the thread sleeps
//!   until [`MIN_INTERVAL`] has passed before acquiring again. DXGI accumulates the
//!   updates meanwhile, so 60–144 Hz screen activity costs ≤ 30 acquisitions per second.
//!   The first change after a quiet period is still picked up immediately.
//! * Each frame's move + dirty rects are copied GPU-side into a persistent shadow
//!   texture that therefore always mirrors the whole screen.
//! * [`tiles::Tiles`] decides what to analyse: changed 32 px tiles, except that
//!   animated tiles (changed in consecutive passes) are analysed at most every
//!   [`tiles::SLOW_INTERVAL`], and tiles under fullscreen windows not at all.
//! * Content under **our own windows** (knight, benches, effects) is **frozen**: those
//!   tiles are not analysed and [`SurfaceMap::set_frozen`] keeps their masks, so the
//!   ledge the knight sits on can never change under him, even on systems where capture
//!   would see our windows. When a window moves away, the area it left is rescanned once.
//!   (Our windows are `WDA_EXCLUDEFROMCAPTURE` and, measured on Windows 11, invisible to
//!   duplication; moving them still produces dirty rects.)
//! * Only the selected regions (grown by the [`MARGIN`] px the edge tests look around) are read
//!   back, shelf-packed into a small CPU-readable staging band ([`BAND`] rows), mapped
//!   once per band and scanned in place: no full-screen CPU copy is kept.
//! * A CPU budget ([`BUDGET_FACTOR`]) further spaces out expensive analyses.
//! * Surface hysteresis needs a wake-up even when the screen stays still: the wait
//!   timeout is shortened to the next promotion deadline.
//! * `DXGI_ERROR_ACCESS_LOST` (mode change, secure desktop, fullscreen app switch) and
//!   creation failures (RDP, unsupported driver, too many duplicators) recreate the
//!   session with backoff. Failure is logged once per output and never fatal; the
//!   output's content is cleared while it is unavailable.
//! * Rotated outputs are skipped (their duplicated image is not in desktop orientation).
//!
//! [`ContentFeed`] is the UI-thread side: it drains updates, anchors them to windows
//! with a [`ContentLayer`] and builds terrain. [`set_stats_logging`] makes every thread
//! log per-second counters.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use skong_core::desktop::{DesktopSnapshot, Monitor};
use skong_core::geom::RectI;
use skong_core::terrain::{ContentLayer, ContentLedge, ContentWallSeg, Terrain, TerrainOptions};
use skong_core::vision::{Activity, ActivityGrid, ActivityParams, ActivitySpot, ContentWall, MARGIN, PixelView, Surface, SurfaceMap, VisionParams};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_ERROR_WAS_STILL_DRAWING, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_MOVE_RECT,
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible};
use windows::core::{BOOL, Interface};

mod tiles;

use tiles::Tiles;

/// Minimum time between two acquisitions (and analyses) of one output (≤ 30 Hz).
pub const MIN_INTERVAL: Duration = Duration::from_millis(33);
/// After an analysis taking `t`, the next waits at least `t * BUDGET_FACTOR`: bounds the
/// analysis duty cycle to ~5 % of a core per output during full-screen churn.
pub const BUDGET_FACTOR: u32 = 20;
/// Longest blocking wait, so threads notice shutdown.
pub const IDLE_WAKE: Duration = Duration::from_millis(250);
/// With animated work pending, wake up this soon to notice that it stopped.
const QUIET_WAKE: Duration = Duration::from_millis(50);
/// Rows of the CPU-readable staging band regions are packed into.
pub const BAND: u32 = 512;
/// Own windows are grown by this much (their position may lag a frame or two behind a
/// dashing knight).
const OWN_MARGIN: i32 = 16;
/// Activity hotspots are recomputed and sent at most this often (≤ 10 Hz).
pub const ACTIVITY_INTERVAL: Duration = Duration::from_millis(100);
/// How often [`ContentFeed`] re-lists our own windows.
const OWN_REFRESH: Duration = Duration::from_secs(1);
/// Own windows larger than this (px²) are not treated as such: they are full-screen
/// debug overlays, not the knight or his props.
const MAX_OWN_AREA: i64 = 1024 * 1024;
/// D3D11 + driver initialisation and the first full-screen scans leave ~30 MB of pages
/// resident that are never touched again. This long after capture starts, the process
/// working set is trimmed once: cold pages go to the standby list (still cached, freed
/// under pressure), hot ones fault straight back in.
const TRIM_AFTER: Duration = Duration::from_secs(5);

static STATS_LOG: AtomicBool = AtomicBool::new(false);

/// Log per-second capture counters (frames, skipped frames, analysed area, copy and
/// scan time) from every capture thread at `info` level.
pub fn set_stats_logging(on: bool) {
    STATS_LOG.store(on, Ordering::Relaxed);
}

/// Timings of one analysis, for diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureStats {
    pub source: u32,
    /// Frames folded into this analysis.
    pub frames: u32,
    /// Area of the analysed regions (px).
    pub dirty_px: u64,
    /// Read-back + detection + hysteresis.
    pub analysis: Duration,
    /// From the oldest included frame's present time until the update was sent
    /// (zero for timer-driven hysteresis updates).
    pub latency: Duration,
    pub surfaces: usize,
    /// Analyses (including ones that changed nothing) since the previous update.
    pub analyses: u32,
    /// Total analysis time of those.
    pub busy: Duration,
}

/// Content surfaces and walls of one output, in virtual-desktop pixels.
#[derive(Clone, Debug)]
pub struct ContentUpdate {
    pub source: u32,
    pub surfaces: Vec<Surface>,
    pub walls: Vec<ContentWall>,
    /// When the newest frame included in this analysis was acquired.
    pub captured_at: Instant,
    pub stats: CaptureStats,
}

/// What capture threads send to the UI thread.
#[derive(Clone, Debug)]
pub enum CaptureMsg {
    Content(ContentUpdate),
    /// Current activity hotspots of one output (virtual-desktop px, no owners yet).
    Activity { source: u32, spots: Vec<ActivitySpot> },
}

/// Running capture threads (one per output). Dropping it signals them to stop; they
/// exit within [`IDLE_WAKE`] without being joined.
pub struct ContentCapture {
    shared: Arc<Shared>,
    rx: Receiver<CaptureMsg>,
    threads: Vec<thread::JoinHandle<()>>,
    outputs: Vec<RectI>,
}

impl ContentCapture {
    /// Enumerate desktop outputs and start one capture thread per output. Fails only if
    /// DXGI itself is unusable; per-output failures are logged by the threads.
    pub fn start(params: VisionParams) -> Result<Self> {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel();
        let mut threads = Vec::new();
        let mut outputs = Vec::new();
        for (source, (name, rect)) in enumerate_outputs()?.into_iter().enumerate() {
            let source = source as u32;
            let (shared, tx, params) = (shared.clone(), tx.clone(), params.clone());
            let label = String::from_utf16_lossy(&name).trim_end_matches('\0').to_string();
            log::info!("content capture: output {source} {label} at {rect:?}");
            let handle = thread::Builder::new()
                .name(format!("skong-capture-{source}"))
                .spawn(move || output_thread(source, name, label, params, shared, tx))
                .context("spawn capture thread")?;
            threads.push(handle);
            outputs.push(rect);
        }
        if threads.is_empty() {
            bail!("no desktop outputs to duplicate");
        }
        Ok(Self { shared, rx, threads, outputs })
    }

    /// Desktop rects of the captured outputs, indexed by `source`.
    pub fn outputs(&self) -> &[RectI] {
        &self.outputs
    }

    /// Next pending update, if any (never blocks).
    pub fn try_recv(&self) -> Option<CaptureMsg> {
        self.rx.try_recv().ok()
    }

    /// Virtual-desktop regions not worth analysing (fullscreen windows: their content
    /// never becomes terrain). Previously excluded regions are re-analysed.
    pub fn set_excluded(&self, rects: &[RectI]) {
        self.shared.excluded.set(rects);
    }

    /// Virtual-desktop rects of our own (capture-excluded) windows: changes there are
    /// mostly our own movement and are analysed lazily.
    pub fn set_own(&self, rects: &[RectI]) {
        self.shared.own.set(rects);
    }

    /// Signal the threads and wait for them (up to about [`IDLE_WAKE`]).
    pub fn stop_and_join(mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for ContentCapture {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// A rect list with a generation bumped on every change.
#[derive(Default)]
struct Versioned(Mutex<(u64, Vec<RectI>)>);

impl Versioned {
    fn set(&self, rects: &[RectI]) {
        if let Ok(mut g) = self.0.lock()
            && g.1 != rects
        {
            g.0 += 1;
            g.1 = rects.to_vec();
        }
    }

    /// The list, if its generation differs from `seen`.
    fn if_changed(&self, seen: u64) -> Option<(u64, Vec<RectI>)> {
        let g = self.0.lock().ok()?;
        (g.0 != seen).then(|| (g.0, g.1.clone()))
    }
}

/// State shared between [`ContentCapture`] and its threads.
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    /// Regions not to analyse (fullscreen windows).
    excluded: Versioned,
    /// Our own windows.
    own: Versioned,
}

/// `(device name, desktop rect)` of every output attached to the desktop.
fn enumerate_outputs() -> Result<Vec<([u16; 32], RectI)>> {
    let mut out = Vec::new();
    for_each_output(|_, output| {
        // SAFETY: plain COM getter on a live output.
        if let Ok(d) = unsafe { output.GetDesc() }
            && d.AttachedToDesktop.as_bool()
        {
            out.push((d.DeviceName, rect_i(&d.DesktopCoordinates)));
        }
        false
    })?;
    Ok(out)
}

/// Visit outputs of all adapters until `f` returns `true`.
fn for_each_output(mut f: impl FnMut(&IDXGIAdapter1, &IDXGIOutput) -> bool) -> Result<()> {
    // SAFETY: COM factory creation and enumeration; enumeration stops at the first error
    // (DXGI_ERROR_NOT_FOUND past the last item).
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;
        let mut ai = 0;
        while let Ok(adapter) = factory.EnumAdapters1(ai) {
            let mut oi = 0;
            while let Ok(output) = adapter.EnumOutputs(oi) {
                if f(&adapter, &output) {
                    return Ok(());
                }
                oi += 1;
            }
            ai += 1;
        }
    }
    Ok(())
}

fn rect_i(r: &RECT) -> RectI {
    RectI::new(r.left, r.top, r.right, r.bottom)
}

enum End {
    Stopped,
    Lost,
}

fn output_thread(source: u32, name: [u16; 32], label: String, params: VisionParams, shared: Arc<Shared>, tx: Sender<CaptureMsg>) {
    // SAFETY: lowering this thread's own priority; failure is harmless.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
    let mut backoff = Duration::from_millis(250);
    let mut warned = false;
    while !shared.stop.load(Ordering::Relaxed) {
        match Session::open(&name) {
            Ok(mut s) => {
                log::debug!("content capture {label}: {}x{} at {:?}", s.width, s.height, s.origin);
                backoff = Duration::from_millis(250);
                let res = s.run(source, &params, &shared, &tx);
                // Whatever we reported is stale now.
                let clear = ContentUpdate { source, surfaces: Vec::new(), walls: Vec::new(), captured_at: Instant::now(), stats: CaptureStats { source, ..Default::default() } };
                match res {
                    Ok(End::Stopped) => return,
                    Ok(End::Lost) => {
                        log::debug!("content capture {label}: access lost, recreating");
                        backoff = Duration::from_millis(50);
                    }
                    Err(e) => log::debug!("content capture {label}: {e:#}"),
                }
                if s.sent_any && (tx.send(CaptureMsg::Content(clear)).is_err() || tx.send(CaptureMsg::Activity { source, spots: Vec::new() }).is_err()) {
                    return;
                }
            }
            Err(e) => {
                if !warned {
                    log::warn!("content capture unavailable on {label} ({e:#}); running without content terrain there, retrying quietly");
                    warned = true;
                }
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
        }
        let until = Instant::now() + backoff;
        while Instant::now() < until && !shared.stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// What one `AcquireNextFrame` call produced.
enum Acquired {
    Timeout,
    /// Only the pointer changed.
    Pointer,
    /// A new desktop image; its changed rects were copied to the shadow texture.
    Frame { present: i64 },
}

/// Per-second counters for [`set_stats_logging`].
struct PerSecond {
    since: Instant,
    frames: u32,
    /// Frames whose changes were all under our own windows or excluded regions.
    self_only: u32,
    pointer: u32,
    analyses: u32,
    px: u64,
    copy: Duration,
    scan: Duration,
}

impl PerSecond {
    fn new() -> Self {
        Self { since: Instant::now(), frames: 0, self_only: 0, pointer: 0, analyses: 0, px: 0, copy: Duration::ZERO, scan: Duration::ZERO }
    }

    fn maybe_log(&mut self, source: u32) {
        if self.since.elapsed() < Duration::from_secs(1) {
            return;
        }
        if STATS_LOG.load(Ordering::Relaxed) {
            log::info!(
                "capture {source}: {} frames ({} self/excluded-only), {} pointer-only, {} analyses, {:.2} Mpx, copy {:.2} ms, scan {:.2} ms",
                self.frames,
                self.self_only,
                self.pointer,
                self.analyses,
                self.px as f64 / 1e6,
                self.copy.as_secs_f64() * 1e3,
                self.scan.as_secs_f64() * 1e3
            );
        }
        *self = Self::new();
    }
}

/// One output duplication with its device, textures and detector.
struct Session {
    ctx: ID3D11DeviceContext,
    dupl: IDXGIOutputDuplication,
    /// GPU copy of the current screen, updated every frame.
    shadow: ID3D11Texture2D,
    /// CPU-readable band (`width` x `band`) regions are packed into for read-back.
    staging: ID3D11Texture2D,
    band: u32,
    width: u32,
    height: u32,
    /// Top-left of the output in virtual-desktop pixels.
    origin: (i32, i32),
    sent_any: bool,
}

impl Session {
    fn open(name: &[u16; 32]) -> Result<Self> {
        let mut found = None;
        for_each_output(|adapter, output| {
            // SAFETY: plain COM getter.
            let hit = unsafe { output.GetDesc() }.is_ok_and(|d| d.DeviceName == *name && d.AttachedToDesktop.as_bool());
            if hit {
                found = Some((adapter.clone(), output.clone()));
            }
            hit
        })?;
        let (adapter, output) = found.context("output no longer attached")?;
        // SAFETY: COM calls on live objects; out-pointers reference locals.
        unsafe {
            let desc = output.GetDesc()?;
            let mut device: Option<ID3D11Device> = None;
            let mut ctx: Option<ID3D11DeviceContext> = None;
            let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .context("D3D11CreateDevice")?;
            let (device, ctx) = (device.context("no device")?, ctx.context("no context")?);
            // Per-monitor-aware processes must use DuplicateOutput1 (Windows 10+).
            let dupl = match output.cast::<IDXGIOutput5>() {
                Ok(o5) => o5.DuplicateOutput1(&device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
                Err(_) => output.cast::<IDXGIOutput1>()?.DuplicateOutput(&device),
            }
            .context("DuplicateOutput")?;
            let dd = dupl.GetDesc();
            if dd.Rotation != DXGI_MODE_ROTATION_IDENTITY && dd.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED {
                bail!("rotated output ({:?}) not supported", dd.Rotation);
            }
            if dd.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
                bail!("unexpected desktop format {:?}", dd.ModeDesc.Format);
            }
            let (width, height) = (dd.ModeDesc.Width, dd.ModeDesc.Height);
            let band = BAND.min(height);
            let shadow_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                ..Default::default()
            };
            let mut shadow = None;
            device.CreateTexture2D(&shadow_desc, None, Some(&mut shadow)).context("CreateTexture2D (shadow)")?;
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Height: band,
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..shadow_desc
            };
            let mut staging = None;
            device.CreateTexture2D(&staging_desc, None, Some(&mut staging)).context("CreateTexture2D (staging)")?;
            let origin = (desc.DesktopCoordinates.left, desc.DesktopCoordinates.top);
            Ok(Self {
                ctx,
                dupl,
                shadow: shadow.context("no shadow texture")?,
                staging: staging.context("no staging texture")?,
                band,
                width,
                height,
                origin,
                sent_any: false,
            })
        }
    }

    fn full(&self) -> RectI {
        RectI::new(0, 0, self.width as i32, self.height as i32)
    }

    /// Virtual-desktop rects → this output's local coords, grown by `margin`, clipped.
    fn to_local(&self, rects: &[RectI], margin: i32) -> Vec<RectI> {
        let (ox, oy) = self.origin;
        let full = self.full();
        rects
            .iter()
            .map(|r| RectI::new(r.left - ox - margin, r.top - oy - margin, r.right - ox + margin, r.bottom - oy + margin).intersect(&full))
            .filter(|r| !r.is_empty())
            .collect()
    }

    fn run(&mut self, source: u32, params: &VisionParams, shared: &Shared, tx: &Sender<CaptureMsg>) -> Result<End> {
        let epoch = Instant::now();
        let secs = |t: Instant| t.saturating_duration_since(epoch).as_secs_f64();
        let mut map = SurfaceMap::new(self.width as usize, self.height as usize, params.clone());
        let mut tiles = Tiles::new(self.width as i32, self.height as i32);
        let mut need_full = true;
        let mut frames = 0u32;
        let mut oldest_present: Option<i64> = None;
        let mut last_frame = Instant::now();
        let mut next_acquire = Instant::now();
        let mut rects: Vec<RectI> = Vec::new();
        let (mut excluded_gen, mut own_gen) = (u64::MAX, u64::MAX);
        let (ox, oy) = self.origin;
        let mut grid = ActivityGrid::new(RectI::new(ox, oy, ox + self.width as i32, oy + self.height as i32), ActivityParams::default());
        let mut own_virtual: Vec<RectI> = Vec::new();
        let (mut activity_sent, mut activity_shown) = (None::<Instant>, false);
        // Work since the last sent update.
        let (mut analyses, mut busy) = (0u32, Duration::ZERO);
        let mut per_sec = PerSecond::new();
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return Ok(End::Stopped);
            }
            let now = Instant::now();
            if now < next_acquire {
                // Paced: let DXGI accumulate updates instead of waking for every one.
                thread::sleep(next_acquire - now);
                continue;
            }
            let mut wait = IDLE_WAKE;
            if tiles.has_pending() {
                wait = wait.min(QUIET_WAKE);
            }
            if !grid.is_quiet() {
                wait = wait.min(ACTIVITY_INTERVAL);
            }
            if let Some(d) = map.next_deadline() {
                wait = wait.min((epoch + Duration::from_secs_f64(d.max(0.0))).saturating_duration_since(now));
            }
            let timeout = wait.as_micros().div_ceil(1000) as u32;

            match self.acquire(timeout, need_full, &mut rects) {
                Ok(Acquired::Frame { present }) => {
                    // Activity: what changed, minus our own windows (the first frame is a full
                    // copy, not a change).
                    if !need_full {
                        let mut changed: Vec<RectI> = rects.iter().map(|r| RectI::new(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy)).collect();
                        for cut in &own_virtual {
                            changed = changed.into_iter().flat_map(|r| subtract_rect(r, cut)).collect();
                        }
                        grid.add_frame(&changed, secs(Instant::now()));
                    }
                    if need_full {
                        tiles.mark_all();
                        rects.clear();
                        need_full = false;
                    }
                    let mut real = false;
                    for r in rects.drain(..) {
                        real |= tiles.mark(&r);
                    }
                    per_sec.frames += 1;
                    per_sec.self_only += u32::from(!real);
                    frames += 1;
                    last_frame = Instant::now();
                    oldest_present.get_or_insert(present);
                    next_acquire = last_frame + MIN_INTERVAL;
                }
                Ok(Acquired::Pointer) => {
                    per_sec.pointer += 1;
                    next_acquire = Instant::now() + MIN_INTERVAL;
                }
                Ok(Acquired::Timeout) => {}
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => return Ok(End::Lost),
                Err(e) => return Err(e).context("AcquireNextFrame"),
            }

            if let Some((g, list)) = shared.excluded.if_changed(excluded_gen) {
                excluded_gen = g;
                tiles.set_excluded(&self.to_local(&list, 0));
            }
            if let Some((g, list)) = shared.own.if_changed(own_gen) {
                own_gen = g;
                // Freeze what is detected under our own windows; areas they left are
                // marked for one rescan (Tiles::set_own) and unfrozen here.
                let local = self.to_local(&list, OWN_MARGIN);
                tiles.set_own(&local);
                map.set_frozen(&local);
                own_virtual = list.iter().map(|r| RectI::new(r.left - OWN_MARGIN, r.top - OWN_MARGIN, r.right + OWN_MARGIN, r.bottom + OWN_MARGIN)).collect();
            }

            let now = Instant::now();
            if activity_sent.is_none_or(|t| now >= t + ACTIVITY_INTERVAL) && (activity_shown || !grid.is_quiet()) {
                let spots = grid.spots(secs(now));
                activity_shown = !spots.is_empty();
                activity_sent = Some(now);
                if tx.send(CaptureMsg::Activity { source, spots }).is_err() {
                    return Ok(End::Stopped);
                }
            }

            let now = Instant::now();
            let work = tiles.tick(now);
            let promote = map.next_deadline().is_some_and(|d| secs(now) >= d);
            if work.is_empty() && !promote {
                per_sec.maybe_log(source);
                continue;
            }
            let start = Instant::now();
            let mut dirty_px = 0;
            let (mut batch_frames, mut latency) = (0, Duration::ZERO);
            if !work.is_empty() {
                dirty_px = work.iter().map(|r| r.width() as u64 * r.height() as u64).sum();
                let (copy, scan) = self.analyse(&mut map, &work)?;
                per_sec.copy += copy;
                per_sec.scan += scan;
                per_sec.px += dirty_px;
                per_sec.analyses += 1;
                batch_frames = std::mem::take(&mut frames);
                latency = oldest_present.take().map_or(Duration::ZERO, qpc_age);
                next_acquire = next_acquire.max(now + start.elapsed() * BUDGET_FACTOR);
            }
            let changed = map.update(secs(Instant::now()));
            let analysis = start.elapsed();
            analyses += 1;
            busy += analysis;
            per_sec.maybe_log(source);
            if changed || !self.sent_any {
                let surfaces: Vec<Surface> = map.surfaces().iter().map(|s| s.translated(ox, oy)).collect();
                let walls: Vec<ContentWall> = map.walls().iter().map(|w| w.translated(ox, oy)).collect();
                let stats = CaptureStats { source, frames: batch_frames, dirty_px, analysis, latency, surfaces: surfaces.len() + walls.len(), analyses, busy };
                if tx.send(CaptureMsg::Content(ContentUpdate { source, surfaces, walls, captured_at: last_frame, stats })).is_err() {
                    return Ok(End::Stopped); // receiver gone
                }
                self.sent_any = true;
                (analyses, busy) = (0, Duration::ZERO);
            }
        }
    }

    /// Wait up to `timeout_ms` for a new desktop image and copy its changed regions
    /// (everything when `full`) into the shadow texture, appending them to `rects`.
    fn acquire(&self, timeout_ms: u32, full: bool, rects: &mut Vec<RectI>) -> windows::core::Result<Acquired> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        // SAFETY: out-pointers reference locals; a successful acquire is paired with
        // ReleaseFrame by `Release` below.
        match unsafe { self.dupl.AcquireNextFrame(timeout_ms, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(Acquired::Timeout),
            Err(e) => return Err(e),
        }
        let _release = Release(&self.dupl);
        let Some(res) = res.filter(|_| info.LastPresentTime != 0) else { return Ok(Acquired::Pointer) };
        let present = info.LastPresentTime;
        let tex: ID3D11Texture2D = res.cast()?;
        if full || info.TotalMetadataBufferSize == 0 {
            // SAFETY: same-size, same-format textures on our device.
            unsafe { self.ctx.CopyResource(&self.shadow, &tex) };
            rects.push(self.full());
            return Ok(Acquired::Frame { present });
        }
        let start = rects.len();
        self.frame_rects(info.TotalMetadataBufferSize, rects)?;
        let bounds = self.full();
        for r in &mut rects[start..] {
            *r = r.intersect(&bounds);
            if r.is_empty() {
                continue;
            }
            let b = d3d_box(r);
            // SAFETY: box clipped to both textures' bounds.
            unsafe { self.ctx.CopySubresourceRegion(&self.shadow, 0, b.left, b.top, 0, &tex, 0, Some(&b)) };
        }
        rects.retain(|r| !r.is_empty());
        Ok(Acquired::Frame { present })
    }

    /// Move destinations and dirty rects of the acquired frame.
    fn frame_rects(&self, meta_bytes: u32, out: &mut Vec<RectI>) -> windows::core::Result<()> {
        let mut need = 0u32;
        let mut moves = vec![DXGI_OUTDUPL_MOVE_RECT::default(); meta_bytes as usize / size_of::<DXGI_OUTDUPL_MOVE_RECT>() + 1];
        // SAFETY: buffer size in bytes matches the vector; `need` receives the used size.
        unsafe { self.dupl.GetFrameMoveRects((moves.len() * size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32, moves.as_mut_ptr(), &mut need) }?;
        moves.truncate(need as usize / size_of::<DXGI_OUTDUPL_MOVE_RECT>());
        out.extend(moves.iter().map(|m| rect_i(&m.DestinationRect)));
        let mut dirty = vec![RECT::default(); meta_bytes as usize / size_of::<RECT>() + 1];
        // SAFETY: as above.
        unsafe { self.dupl.GetFrameDirtyRects((dirty.len() * size_of::<RECT>()) as u32, dirty.as_mut_ptr(), &mut need) }?;
        dirty.truncate(need as usize / size_of::<RECT>());
        out.extend(dirty.iter().map(rect_i));
        Ok(())
    }

    /// Read `work` back from the shadow texture (each rect grown by the [`MARGIN`] px the edge
    /// tests need, shelf-packed into the staging band, split if taller than it), scan it and
    /// re-merge surfaces. Returns (copy + map time, scan time).
    fn analyse(&self, map: &mut SurfaceMap, work: &[RectI]) -> Result<(Duration, Duration)> {
        let full = self.full();
        let (sw, band) = (self.width as i32, self.band as i32);
        let mut timing = (Duration::ZERO, Duration::ZERO);
        let mut batch: Vec<Placed> = Vec::new();
        let (mut x, mut y, mut shelf) = (0, 0, 0);
        for r in work {
            let mut top = r.top;
            while top < r.bottom {
                let bottom = (top + band - 2 * MARGIN).min(r.bottom);
                let scan = RectI::new(r.left, top, r.right, bottom);
                let grown = RectI::new(scan.left - MARGIN, scan.top - MARGIN, scan.right + MARGIN, scan.bottom + MARGIN).intersect(&full);
                top = bottom;
                if x + grown.width() > sw {
                    (x, y, shelf) = (0, y + shelf, 0);
                }
                if y + grown.height() > band {
                    self.scan_batch(map, &batch, &mut timing)?;
                    batch.clear();
                    (x, y, shelf) = (0, 0, 0);
                }
                let b = d3d_box(&grown);
                let t = Instant::now();
                // SAFETY: `grown` lies inside the shadow texture and the destination
                // (x, y) + size inside the staging band (checked by the packing above).
                unsafe { self.ctx.CopySubresourceRegion(&self.staging, 0, x as u32, y as u32, 0, &self.shadow, 0, Some(&b)) };
                timing.0 += t.elapsed();
                batch.push(Placed { scan, grown, at: (x, y) });
                x += grown.width();
                shelf = shelf.max(grown.height());
            }
        }
        self.scan_batch(map, &batch, &mut timing)?;
        let t = Instant::now();
        map.finish();
        timing.1 += t.elapsed();
        Ok(timing)
    }

    /// Map the staging band and scan every placed region in place.
    fn scan_batch(&self, map: &mut SurfaceMap, batch: &[Placed], timing: &mut (Duration, Duration)) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let t = Instant::now();
        let mut scan_t = Duration::ZERO;
        self.with_mapped(|px, stride| {
            timing.0 += t.elapsed();
            let t = Instant::now();
            for p in batch {
                let off = p.at.1 as usize * stride + p.at.0 as usize;
                let view = PixelView::new(&px[off..], p.grown.width() as usize, p.grown.height() as usize, stride);
                if let Some(view) = view {
                    map.scan(&view.at(p.grown.left as usize, p.grown.top as usize), &[p.scan]);
                }
            }
            scan_t = t.elapsed();
        })?;
        timing.1 += scan_t;
        Ok(())
    }

    /// Map the staging band (without blocking in the driver) and hand `f` its pixels and
    /// row stride (in pixels).
    fn with_mapped(&self, f: impl FnOnce(&[u32], usize)) -> Result<()> {
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        // Don't block in Map (drivers may spin until the GPU copy is done): submit, then
        // poll with DO_NOT_WAIT and sleep between attempts.
        // SAFETY: plain submission of queued commands.
        unsafe { self.ctx.Flush() };
        loop {
            // SAFETY: staging texture created with CPU read access; unmapped below.
            match unsafe { self.ctx.Map(&self.staging, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32, Some(&mut m)) } {
                Ok(()) => break,
                Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => thread::sleep(Duration::from_micros(500)),
                Err(e) => return Err(e).context("Map staging"),
            }
        }
        let (w, h) = (self.width as usize, self.band as usize);
        let stride = m.RowPitch as usize / 4;
        let ok = h > 0 && !m.pData.is_null() && m.RowPitch.is_multiple_of(4) && stride >= w && (m.pData as usize).is_multiple_of(4);
        if ok {
            // SAFETY: while mapped, pData points to `h` rows of RowPitch bytes (the last row
            // at least `w` pixels), 4-byte aligned (checked), and nothing writes to it.
            let px = unsafe { std::slice::from_raw_parts(m.pData as *const u32, stride * (h - 1) + w) };
            f(px, stride);
        }
        // SAFETY: matches the Map above; `px` is not used past this point.
        unsafe { self.ctx.Unmap(&self.staging, 0) };
        if !ok {
            bail!("unexpected staging layout (pitch {})", m.RowPitch);
        }
        Ok(())
    }
}

/// A region placed in the staging band: `scan` is what to re-detect, `grown` the pixels
/// copied (at `at` in the band).
struct Placed {
    scan: RectI,
    grown: RectI,
    at: (i32, i32),
}

fn d3d_box(r: &RectI) -> D3D11_BOX {
    D3D11_BOX { left: r.left as u32, top: r.top as u32, front: 0, right: r.right as u32, bottom: r.bottom as u32, back: 1 }
}

/// Releases an acquired duplication frame on drop.
struct Release<'a>(&'a IDXGIOutputDuplication);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        // SAFETY: only constructed right after a successful AcquireNextFrame. Errors
        // (access lost) resurface on the next acquire.
        let _ = unsafe { self.0.ReleaseFrame() };
    }
}

/// `r` minus `cut`: up to four rects.
fn subtract_rect(r: RectI, cut: &RectI) -> Vec<RectI> {
    let i = r.intersect(cut);
    if i.is_empty() {
        return vec![r];
    }
    [
        RectI::new(r.left, r.top, r.right, i.top),
        RectI::new(r.left, i.bottom, r.right, r.bottom),
        RectI::new(r.left, i.top, i.left, i.bottom),
        RectI::new(i.right, i.top, r.right, i.bottom),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect()
}


/// One-shot capture of `r` (virtual-desktop px; on a single output, at most [`BAND`] rows
/// tall) exactly as Desktop Duplication sees it, BGRA row-major. Diagnostics only: opens
/// its own duplication, so it competes with a running [`ContentCapture`].
pub fn grab_region(r: RectI) -> Result<Vec<u32>> {
    let (name, out) = enumerate_outputs()?
        .into_iter()
        .find(|(_, o)| o.left <= r.left && r.right <= o.right && o.top <= r.top && r.bottom <= o.bottom)
        .context("rect is not inside a single output")?;
    let s = Session::open(&name)?;
    let local = RectI::new(r.left - out.left, r.top - out.top, r.right - out.left, r.bottom - out.top);
    if local.is_empty() || local.height() as u32 > s.band {
        bail!("rect must be non-empty and at most {} rows", s.band);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut rects = Vec::new();
    loop {
        if let Acquired::Frame { .. } = s.acquire(200, true, &mut rects)? {
            break;
        }
        if Instant::now() > deadline {
            bail!("no desktop frame within 2 s");
        }
    }
    let b = d3d_box(&local);
    // SAFETY: `local` lies inside the shadow texture and fits the staging band at (0, 0).
    unsafe { s.ctx.CopySubresourceRegion(&s.staging, 0, 0, 0, 0, &s.shadow, 0, Some(&b)) };
    let (w, h) = (local.width() as usize, local.height() as usize);
    let mut out = Vec::with_capacity(w * h);
    s.with_mapped(|px, stride| {
        for y in 0..h {
            out.extend_from_slice(&px[y * stride..y * stride + w]);
        }
    })?;
    Ok(out)
}
/// Time since a QPC timestamp.
fn qpc_age(then: i64) -> Duration {
    let (mut now, mut freq) = (0i64, 0i64);
    // SAFETY: out-pointers reference locals; both calls cannot fail on XP+.
    let ok = unsafe { QueryPerformanceCounter(&mut now).is_ok() && QueryPerformanceFrequency(&mut freq).is_ok() };
    if !ok || freq <= 0 || now < then {
        return Duration::ZERO;
    }
    Duration::from_secs_f64((now - then) as f64 / freq as f64)
}

/// UI-thread side of live content terrain: owns the capture threads, drains their
/// updates without blocking, anchors surfaces to windows and builds terrain.
///
/// ```ignore
/// let changed_desk = watcher.poll();
/// if feed.poll(watcher.snapshot(), changed_desk) || changed_desk {
///     terrain = feed.terrain(watcher.snapshot(), &opts);
/// }
/// ```
pub struct ContentFeed {
    capture: Option<ContentCapture>,
    params: VisionParams,
    layer: ContentLayer,
    epoch: Instant,
    /// Monitors the capture was started for; restarts when they change.
    monitors: Vec<Monitor>,
    enabled: bool,
    stats: Vec<CaptureStats>,
    /// Our own visible top-level windows (sprite, benches...): capture-excluded, so their
    /// movement isn't content. Re-enumerated every [`OWN_REFRESH`].
    own: Vec<HWND>,
    own_listed: Option<Instant>,
    /// Own windows not to treat as such (e.g. a full-screen debug overlay).
    not_own: Vec<HWND>,
    /// Only these windows of ours freeze content under them (the knight: his own sprite
    /// must never erase the ledge he stands on). `None`: all of our sprite windows.
    /// Coins, props and visitors must not: a line that went away stayed "alive" under
    /// them and they floated on it.
    freeze_only: Option<Vec<HWND>>,
    own_rects: Vec<RectI>,
    /// Latest hotspots per output, and the merged view with owners.
    spots: Vec<(u32, Vec<ActivitySpot>)>,
    activity: Activity,
    /// When to release the cold pages capture start-up left resident (see [`TRIM_AFTER`]).
    trim_at: Option<Instant>,
    totals: CaptureTotals,
    /// Content was switched off: the next poll reports a change (terrain loses it).
    cleared: bool,
}

/// Running totals over the feed's lifetime (status displays).
#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureTotals {
    pub updates: u64,
    pub frames: u64,
    pub last: Option<CaptureStats>,
}

impl ContentFeed {
    /// Start capturing (unless `enabled` is false). Never fails: without duplication the
    /// feed simply stays empty.
    pub fn new(enabled: bool, desk: &DesktopSnapshot) -> Self {
        let mut f = Self {
            capture: None,
            params: VisionParams::default(),
            layer: ContentLayer::default(),
            epoch: Instant::now(),
            monitors: desk.monitors.clone(),
            enabled,
            stats: Vec::new(),
            own: Vec::new(),
            own_listed: None,
            not_own: Vec::new(),
            freeze_only: None,
            own_rects: Vec::new(),
            spots: Vec::new(),
            activity: Activity::default(),
            totals: CaptureTotals::default(),
            trim_at: None,
            cleared: false,
        };
        if enabled {
            f.restart(desk);
        }
        f
    }

    /// (Re)start capture threads for the current outputs, forgetting all content.
    fn restart(&mut self, desk: &DesktopSnapshot) {
        self.capture = None; // signals the old threads
        self.layer = ContentLayer::default();
        self.layer.observe_desktop(desk, self.secs(Instant::now()));
        self.spots.clear();
        self.activity = Activity::default();
        match ContentCapture::start(self.params.clone()) {
            Ok(c) => {
                c.set_excluded(&fullscreen_rects(desk));
                c.set_own(&self.own_rects);
                self.capture = Some(c);
                self.trim_at = Some(Instant::now() + TRIM_AFTER);
            }
            Err(e) => log::warn!("content capture disabled: {e:#}"),
        }
    }

    pub fn is_running(&self) -> bool {
        self.capture.is_some()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turn content capture on or off at runtime. Either way the next [`Self::poll`]
    /// reports a change so terrain is rebuilt with (or without) content.
    pub fn set_enabled(&mut self, on: bool, desk: &DesktopSnapshot) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        if on {
            self.monitors = desk.monitors.clone();
            self.restart(desk);
        } else {
            self.shutdown();
            self.layer = ContentLayer::default();
            self.spots.clear();
            self.activity = Activity::default();
        }
        self.cleared = true;
    }

    pub fn totals(&self) -> CaptureTotals {
        self.totals
    }

    fn secs(&self, t: Instant) -> f64 {
        t.saturating_duration_since(self.epoch).as_secs_f64()
    }

    /// Call every tick with the current snapshot (and whether it just changed). Returns
    /// `true` when content changed and terrain should be rebuilt.
    pub fn poll(&mut self, desk: &DesktopSnapshot, desk_changed: bool) -> bool {
        self.stats.clear();
        let cleared = std::mem::take(&mut self.cleared);
        if !self.enabled {
            return cleared;
        }
        if desk_changed {
            self.layer.observe_desktop(desk, self.secs(Instant::now()));
            if let Some(c) = &self.capture {
                c.set_excluded(&fullscreen_rects(desk));
            }
            if desk.monitors != self.monitors {
                log::info!("monitor layout changed: restarting content capture");
                self.monitors = desk.monitors.clone();
                self.restart(desk);
                return true;
            }
        }
        self.update_own_rects();
        if self.trim_at.is_some_and(|t| Instant::now() >= t) {
            self.trim_at = None;
            trim_working_set();
        }
        let Some(cap) = &self.capture else { return cleared };
        let (mut changed, mut activity) = (cleared, desk_changed);
        while let Some(msg) = cap.try_recv() {
            match msg {
                CaptureMsg::Content(u) => {
                    self.layer.ingest(u.source, &u.surfaces, &u.walls, desk, self.secs(u.captured_at));
                    self.totals.updates += 1;
                    self.totals.frames += u64::from(u.stats.frames);
                    self.totals.last = Some(u.stats);
                    self.stats.push(u.stats);
                    changed = true;
                }
                CaptureMsg::Activity { source, spots } => {
                    self.spots.retain(|(s, _)| *s != source);
                    self.spots.push((source, spots));
                    activity = true;
                }
            }
        }
        if activity {
            self.merge_activity(desk);
        }
        changed
    }

    fn update_own_rects(&mut self) {
        if self.own_listed.is_none_or(|t| t.elapsed() >= OWN_REFRESH) {
            self.own = match &self.freeze_only {
                Some(only) => only.clone(),
                None => own_windows(),
            };
            self.own.retain(|h| !self.not_own.contains(h));
            self.own_listed = Some(Instant::now());
        }
        let rects: Vec<RectI> = self
            .own
            .iter()
            .filter_map(|&h| {
                let mut r = RECT::default();
                // SAFETY: queries on a window handle; a stale handle just fails.
                let visible = unsafe { IsWindowVisible(h) }.as_bool();
                (visible && unsafe { GetWindowRect(h, &mut r) }.is_ok()).then(|| rect_i(&r))
            })
            // Debug overlays cover whole monitors; only sprite-sized windows are "us".
            .filter(|r| (r.width() as i64) * (r.height() as i64) <= MAX_OWN_AREA)
            .collect();
        if rects != self.own_rects {
            if let Some(c) = &self.capture {
                c.set_own(&rects);
            }
            self.own_rects = rects;
        }
    }

    /// Don't treat this window of ours as "own" (changes under our own windows are
    /// analysed lazily and excluded from activity): for overlays covering the screen.
    pub fn not_own_window(&mut self, hwnd: HWND) {
        self.not_own.push(hwnd);
        self.own_listed = None;
    }

    /// Freeze content only under these windows of ours (see the field).
    pub fn freeze_only(&mut self, hwnds: Vec<HWND>) {
        self.freeze_only = Some(hwnds);
        self.own_listed = None;
    }

    /// Where the screen is busy (all monitors, strongest first, at most 5 spots), with
    /// owners. Updated by [`Self::poll`] at most ~10 times a second; changes under our
    /// own windows and screen-wide flashes are ignored.
    pub fn activity(&self) -> &Activity {
        &self.activity
    }

    fn merge_activity(&mut self, desk: &DesktopSnapshot) {
        let mut spots: Vec<ActivitySpot> = self.spots.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        for s in &mut spots {
            let (x, y) = (s.center.x as i32, s.center.y as i32);
            s.owner = desk.windows.iter().find(|w| w.rect.contains(x, y)).map(|w| w.id);
        }
        spots.sort_by(|a, b| b.intensity.total_cmp(&a.intensity));
        spots.truncate(ActivityParams::default().max_spots);
        self.activity = Activity { spots };
    }

    /// Stats of the updates consumed by the last [`Self::poll`].
    pub fn stats(&self) -> &[CaptureStats] {
        &self.stats
    }

    /// Content ledges for the current desktop.
    pub fn ledges(&self, desk: &DesktopSnapshot) -> Vec<ContentLedge> {
        self.layer.ledges(desk)
    }

    /// Content walls for the current desktop.
    pub fn walls(&self, desk: &DesktopSnapshot) -> Vec<ContentWallSeg> {
        self.layer.walls(desk)
    }

    /// Window + content terrain for the current desktop.
    pub fn terrain(&self, desk: &DesktopSnapshot, opts: &TerrainOptions) -> Terrain {
        Terrain::from_desktop_and_content(desk, &self.ledges(desk), &self.walls(desk), opts)
    }

    /// Stop and join the capture threads.
    pub fn shutdown(&mut self) {
        if let Some(c) = self.capture.take() {
            c.stop_and_join();
        }
    }
}

/// Rects of fullscreen windows (games, video): their content never becomes terrain.
fn fullscreen_rects(desk: &DesktopSnapshot) -> Vec<RectI> {
    desk.windows.iter().filter(|w| w.fullscreen).map(|w| w.rect).collect()
}

/// Our process's visible top-level windows.
fn own_windows() -> Vec<HWND> {
    extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let mut pid = 0u32;
        // SAFETY: `lparam` is the `&mut Vec<HWND>` passed below, alive during EnumWindows;
        // the other calls are plain queries on the enumerated handle.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() {
                (*(lparam.0 as *mut Vec<HWND>)).push(hwnd);
            }
        }
        BOOL(1)
    }
    let mut out: Vec<HWND> = Vec::new();
    // SAFETY: the callback only uses `out` through the pointer during this call.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut out as *mut Vec<HWND> as isize)) };
    out
}

/// Release cold pages of this process to the standby list (one-shot, see [`TRIM_AFTER`]).
fn trim_working_set() {
    // SAFETY: plain call on our own process pseudo-handle.
    let ok = unsafe { K32EmptyWorkingSet(GetCurrentProcess()) }.as_bool();
    log::debug!("working set trimmed after capture start-up: {ok}");
}
