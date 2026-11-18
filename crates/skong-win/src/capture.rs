//! Live content terrain: DXGI Desktop Duplication → [`skong_core::vision`] → terrain.
//!
//! # Design
//!
//! * One background thread per monitor (DXGI output), each with its own D3D11 device
//!   and `IDXGIOutputDuplication`. The thread blocks in `AcquireNextFrame`, so an idle
//!   desktop costs nothing but a wake-up every [`IDLE_WAKE`] to check for shutdown.
//! * Each new frame's move + dirty rects are copied GPU-side into a persistent shadow
//!   texture that therefore always mirrors the whole screen. Only the accumulated dirty
//!   rects are read back (shadow → CPU-readable staging texture) and analysed, at most once per [`MIN_INTERVAL`]
//!   (≤ 30 Hz) and within a CPU budget ([`BUDGET_FACTOR`]); the first change after a
//!   quiet period is analysed immediately.
//! * The cursor is not part of duplicated frames, and our own windows are excluded
//!   with `WDA_EXCLUDEFROMCAPTURE`, so the knight never sees himself.
//! * Surface hysteresis needs a wake-up even when the screen stays still: the wait
//!   timeout is shortened to the next promotion deadline.
//! * `DXGI_ERROR_ACCESS_LOST` (mode change, secure desktop, fullscreen app switch) and
//!   creation failures (RDP, unsupported driver, too many duplicators) recreate the
//!   session with backoff. Failure is logged once per output and never fatal; the
//!   output's content is cleared while it is unavailable.
//! * Rotated outputs are skipped (their duplicated image is not in desktop orientation).
//!
//! [`ContentFeed`] is the UI-thread side: it drains updates, anchors them to windows
//! with a [`ContentLayer`] and builds terrain.
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use skong_core::desktop::{DesktopSnapshot, Monitor};
use skong_core::geom::RectI;
use skong_core::terrain::{ContentLayer, ContentLedge, Terrain, TerrainOptions};
use skong_core::vision::{PixelView, Surface, SurfaceMap, VisionParams};
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_MOVE_RECT,
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
use windows::core::Interface;

/// Minimum time between two analyses of one output (≤ 30 Hz).
pub const MIN_INTERVAL: Duration = Duration::from_millis(33);
/// After an analysis taking `t`, the next waits at least `t * BUDGET_FACTOR`: bounds the
/// analysis duty cycle to ~5 % of a core per output during full-screen churn (video,
/// scrolling a maximized window) while small updates still run at 30 Hz.
pub const BUDGET_FACTOR: u32 = 20;
/// Longest blocking wait, so threads notice shutdown.
pub const IDLE_WAKE: Duration = Duration::from_millis(250);
/// More dirty rects than this are merged into their bounding box.
const MAX_DIRTY_RECTS: usize = 64;

/// Timings of one analysis, for diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureStats {
    pub source: u32,
    /// Frames folded into this analysis.
    pub frames: u32,
    /// Area of the analysed dirty rects (px).
    pub dirty_px: u64,
    /// Staging map + detection + hysteresis.
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

/// Content surfaces of one output, in virtual-desktop pixels.
#[derive(Clone, Debug)]
pub struct ContentUpdate {
    pub source: u32,
    pub surfaces: Vec<Surface>,
    /// When the newest frame included in this analysis was acquired.
    pub captured_at: Instant,
    pub stats: CaptureStats,
}

/// Running capture threads (one per output). Dropping it signals them to stop; they
/// exit within [`IDLE_WAKE`] without being joined.
pub struct ContentCapture {
    shared: Arc<Shared>,
    rx: Receiver<ContentUpdate>,
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
    pub fn try_recv(&self) -> Option<ContentUpdate> {
        self.rx.try_recv().ok()
    }

    /// Virtual-desktop regions not worth analysing (fullscreen windows: their content
    /// never becomes terrain). Previously excluded regions are re-analysed.
    pub fn set_excluded(&self, rects: &[RectI]) {
        self.shared.set_excluded(rects);
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

/// State shared between [`ContentCapture`] and its threads.
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    /// Regions not to analyse, with a generation bumped on every change.
    excluded: Mutex<(u64, Vec<RectI>)>,
}

impl Shared {
    fn set_excluded(&self, rects: &[RectI]) {
        if let Ok(mut g) = self.excluded.lock()
            && g.1 != rects
        {
            g.0 += 1;
            g.1 = rects.to_vec();
        }
    }

    /// The exclusion list, if its generation differs from `seen`.
    fn excluded_if_changed(&self, seen: u64) -> Option<(u64, Vec<RectI>)> {
        let g = self.excluded.lock().ok()?;
        (g.0 != seen).then(|| (g.0, g.1.clone()))
    }
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

fn output_thread(source: u32, name: [u16; 32], label: String, params: VisionParams, shared: Arc<Shared>, tx: Sender<ContentUpdate>) {
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
                let clear = ContentUpdate { source, surfaces: Vec::new(), captured_at: Instant::now(), stats: CaptureStats { source, ..Default::default() } };
                match res {
                    Ok(End::Stopped) => return,
                    Ok(End::Lost) => {
                        log::debug!("content capture {label}: access lost, recreating");
                        backoff = Duration::from_millis(50);
                    }
                    Err(e) => log::debug!("content capture {label}: {e:#}"),
                }
                if s.sent_any && tx.send(clear).is_err() {
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

/// One output duplication with its device, staging copy and detector.
struct Session {
    ctx: ID3D11DeviceContext,
    dupl: IDXGIOutputDuplication,
    /// GPU copy of the current screen, updated every frame.
    shadow: ID3D11Texture2D,
    /// CPU-readable copy, updated from `shadow` only where analysed.
    staging: ID3D11Texture2D,
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
            let tex = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut staging = None;
            device.CreateTexture2D(&tex, None, Some(&mut staging)).context("CreateTexture2D (staging)")?;
            let staging = staging.context("no staging texture")?;
            let shadow_desc = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_DEFAULT, CPUAccessFlags: 0, ..tex };
            let mut shadow = None;
            device.CreateTexture2D(&shadow_desc, None, Some(&mut shadow)).context("CreateTexture2D (shadow)")?;
            let shadow = shadow.context("no shadow texture")?;
            let origin = (desc.DesktopCoordinates.left, desc.DesktopCoordinates.top);
            Ok(Self { ctx, dupl, shadow, staging, width, height, origin, sent_any: false })
        }
    }

    fn full(&self) -> RectI {
        RectI::new(0, 0, self.width as i32, self.height as i32)
    }

    fn run(&mut self, source: u32, params: &VisionParams, shared: &Shared, tx: &Sender<ContentUpdate>) -> Result<End> {
        let epoch = Instant::now();
        let secs = |t: Instant| t.saturating_duration_since(epoch).as_secs_f64();
        let mut map = SurfaceMap::new(self.width as usize, self.height as usize, params.clone());
        let mut dirty: Vec<RectI> = Vec::new();
        let mut need_full = true;
        let mut frames = 0u32;
        let mut oldest_present: Option<i64> = None;
        let mut last_frame = Instant::now();
        let mut next_allowed = Instant::now();
        let mut rects: Vec<RectI> = Vec::new();
        // Local-coordinate regions we don't analyse (fullscreen windows).
        let mut excluded: Vec<RectI> = Vec::new();
        let mut excluded_gen = u64::MAX;
        // Work since the last sent update, and since the last periodic log.
        let (mut analyses, mut busy) = (0u32, Duration::ZERO);
        let mut log_window = (Instant::now(), 0u32, 0u32, Duration::ZERO, 0u64);
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return Ok(End::Stopped);
            }
            let now = Instant::now();
            let mut wake = now + IDLE_WAKE;
            if !dirty.is_empty() {
                wake = wake.min(next_allowed.max(now));
            }
            if let Some(d) = map.next_deadline() {
                wake = wake.min(epoch + Duration::from_secs_f64(d.max(0.0)));
            }
            let timeout = wake.saturating_duration_since(now).as_micros().div_ceil(1000) as u32;

            match self.acquire(timeout, need_full, &mut rects) {
                Ok(Some(present)) => {
                    dirty.append(&mut rects);
                    need_full = false;
                    frames += 1;
                    last_frame = Instant::now();
                    oldest_present.get_or_insert(present);
                }
                Ok(None) => {}
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => return Ok(End::Lost),
                Err(e) => return Err(e).context("AcquireNextFrame"),
            }

            // Exclusions changed: what was hidden from analysis must be looked at again.
            if let Some((generation, list)) = shared.excluded_if_changed(excluded_gen) {
                excluded_gen = generation;
                dirty.append(&mut excluded);
                let (ox, oy) = self.origin;
                let full = self.full();
                excluded = list.iter().map(|r| RectI::new(r.left - ox, r.top - oy, r.right - ox, r.bottom - oy).intersect(&full)).filter(|r| !r.is_empty()).collect();
            }

            let now = Instant::now();
            let analyse = !dirty.is_empty() && now >= next_allowed;
            let promote = map.next_deadline().is_some_and(|d| secs(now) >= d);
            if !analyse && !promote {
                continue;
            }
            let start = Instant::now();
            let mut dirty_px = 0;
            let (mut batch_frames, mut latency) = (0, Duration::ZERO);
            if analyse {
                for cut in &excluded {
                    dirty = dirty.into_iter().flat_map(|r| subtract_rect(r, cut)).collect();
                }
                coalesce(&mut dirty, self.full());
                dirty_px = dirty.iter().map(|r| r.width() as u64 * r.height() as u64).sum();
                if !dirty.is_empty() {
                    self.rescan(&mut map, &dirty)?;
                }
                dirty.clear();
                next_allowed = now + MIN_INTERVAL.max(start.elapsed() * BUDGET_FACTOR);
                batch_frames = std::mem::take(&mut frames);
                latency = oldest_present.take().map_or(Duration::ZERO, qpc_age);
            }
            let changed = map.update(secs(Instant::now()));
            let analysis = start.elapsed();
            analyses += 1;
            busy += analysis;
            log_window.1 += batch_frames;
            log_window.2 += 1;
            log_window.3 += analysis;
            log_window.4 += dirty_px;
            if log_window.0.elapsed() >= Duration::from_secs(10) {
                let (t, f, a, b, px) = log_window;
                log::debug!(
                    "content capture {source}: {f} frames, {a} analyses, {:.1} Mpx, {:.1} ms busy in the last {:.0?}",
                    px as f64 / 1e6,
                    b.as_secs_f64() * 1e3,
                    t.elapsed()
                );
                log_window = (Instant::now(), 0, 0, Duration::ZERO, 0);
            }
            if changed || !self.sent_any {
                let (ox, oy) = self.origin;
                let surfaces: Vec<Surface> = map.surfaces().iter().map(|s| s.translated(ox, oy)).collect();
                let stats = CaptureStats { source, frames: batch_frames, dirty_px, analysis, latency, surfaces: surfaces.len(), analyses, busy };
                if tx.send(ContentUpdate { source, surfaces, captured_at: last_frame, stats }).is_err() {
                    return Ok(End::Stopped); // receiver gone
                }
                self.sent_any = true;
                (analyses, busy) = (0, Duration::ZERO);
            }
        }
    }

    /// Wait up to `timeout_ms` for a new desktop image and copy its changed regions
    /// (everything when `full`) into the shadow texture. Returns the frame's present
    /// time (QPC ticks), or `None` on timeout / pointer-only updates.
    fn acquire(&self, timeout_ms: u32, full: bool, rects: &mut Vec<RectI>) -> windows::core::Result<Option<i64>> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        // SAFETY: out-pointers reference locals; a successful acquire is paired with
        // ReleaseFrame by `Release` below.
        match unsafe { self.dupl.AcquireNextFrame(timeout_ms, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(e) => return Err(e),
        }
        let _release = Release(&self.dupl);
        let Some(res) = res.filter(|_| info.LastPresentTime != 0) else { return Ok(None) };
        let tex: ID3D11Texture2D = res.cast()?;
        if full || info.TotalMetadataBufferSize == 0 {
            // SAFETY: same-size, same-format textures on our device.
            unsafe { self.ctx.CopyResource(&self.shadow, &tex) };
            rects.push(self.full());
            return Ok(Some(info.LastPresentTime));
        }
        let start = rects.len();
        self.frame_rects(info.TotalMetadataBufferSize, rects)?;
        let bounds = self.full();
        for r in &mut rects[start..] {
            *r = r.intersect(&bounds);
            if r.is_empty() {
                continue;
            }
            let b = D3D11_BOX { left: r.left as u32, top: r.top as u32, front: 0, right: r.right as u32, bottom: r.bottom as u32, back: 1 };
            // SAFETY: box clipped to both textures' bounds.
            unsafe { self.ctx.CopySubresourceRegion(&self.shadow, 0, b.left, b.top, 0, &tex, 0, Some(&b)) };
        }
        rects.retain(|r| !r.is_empty());
        Ok(Some(info.LastPresentTime))
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

    /// Read `dirty` back from the shadow texture and re-detect inside it. Rows just
    /// above each rect are read too: edges depend on the two rows above.
    fn rescan(&self, map: &mut SurfaceMap, dirty: &[RectI]) -> Result<()> {
        let full = self.full();
        for r in dirty {
            let r = RectI::new(r.left - 1, r.top - 2, r.right + 1, r.bottom).intersect(&full);
            if r.is_empty() {
                continue;
            }
            let b = D3D11_BOX { left: r.left as u32, top: r.top as u32, front: 0, right: r.right as u32, bottom: r.bottom as u32, back: 1 };
            // SAFETY: box clipped to both (same-size) textures.
            unsafe { self.ctx.CopySubresourceRegion(&self.staging, 0, b.left, b.top, 0, &self.shadow, 0, Some(&b)) };
        }
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging texture created with CPU read access; unmapped below.
        unsafe { self.ctx.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut m)) }.context("Map staging")?;
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = m.RowPitch as usize / 4;
        let ok = h > 0 && !m.pData.is_null() && m.RowPitch.is_multiple_of(4) && stride >= w && (m.pData as usize).is_multiple_of(4);
        if ok {
            // SAFETY: while mapped, pData points to `h` rows of RowPitch bytes (the last row
            // at least `w` pixels), 4-byte aligned (checked), and nothing writes to it.
            let px = unsafe { std::slice::from_raw_parts(m.pData as *const u32, stride * (h - 1) + w) };
            if let Some(view) = PixelView::new(px, w, h, stride) {
                map.rescan(&view, dirty);
            }
        }
        // SAFETY: matches the Map above; `px` is not used past this point.
        unsafe { self.ctx.Unmap(&self.staging, 0) };
        if !ok {
            bail!("unexpected staging layout (pitch {})", m.RowPitch);
        }
        Ok(())
    }
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

/// Drop duplicates and contained rects; too many → bounding box.
fn coalesce(rects: &mut Vec<RectI>, bounds: RectI) {
    rects.retain(|r| !r.intersect(&bounds).is_empty());
    rects.sort_unstable_by_key(|r| (r.top, r.left, r.bottom, r.right));
    rects.dedup();
    let snapshot = rects.clone();
    rects.retain(|r| !snapshot.iter().any(|o| o != r && o.left <= r.left && o.top <= r.top && o.right >= r.right && o.bottom >= r.bottom));
    if rects.len() > MAX_DIRTY_RECTS
        && let Some(u) = rects.iter().copied().reduce(|a, b| RectI::new(a.left.min(b.left), a.top.min(b.top), a.right.max(b.right), a.bottom.max(b.bottom)))
    {
        *rects = vec![u.intersect(&bounds)];
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
        match ContentCapture::start(self.params.clone()) {
            Ok(c) => {
                c.set_excluded(&fullscreen_rects(desk));
                self.capture = Some(c);
            }
            Err(e) => log::warn!("content capture disabled: {e:#}"),
        }
    }

    pub fn is_running(&self) -> bool {
        self.capture.is_some()
    }

    fn secs(&self, t: Instant) -> f64 {
        t.saturating_duration_since(self.epoch).as_secs_f64()
    }

    /// Call every tick with the current snapshot (and whether it just changed). Returns
    /// `true` when content changed and terrain should be rebuilt.
    pub fn poll(&mut self, desk: &DesktopSnapshot, desk_changed: bool) -> bool {
        self.stats.clear();
        if !self.enabled {
            return false;
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
        let Some(cap) = &self.capture else { return false };
        let mut changed = false;
        while let Some(u) = cap.try_recv() {
            self.layer.ingest(u.source, &u.surfaces, desk, self.secs(u.captured_at));
            self.stats.push(u.stats);
            changed = true;
        }
        changed
    }

    /// Stats of the updates consumed by the last [`Self::poll`].
    pub fn stats(&self) -> &[CaptureStats] {
        &self.stats
    }

    /// Content ledges for the current desktop.
    pub fn ledges(&self, desk: &DesktopSnapshot) -> Vec<ContentLedge> {
        self.layer.ledges(desk)
    }

    /// Window + content terrain for the current desktop.
    pub fn terrain(&self, desk: &DesktopSnapshot, opts: &TerrainOptions) -> Terrain {
        Terrain::from_desktop_and_content(desk, &self.ledges(desk), opts)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_dedups_and_bounds() {
        let b = RectI::new(0, 0, 100, 100);
        let mut v = vec![RectI::new(0, 0, 10, 10), RectI::new(0, 0, 10, 10), RectI::new(2, 2, 5, 5), RectI::new(200, 0, 300, 10)];
        coalesce(&mut v, b);
        assert_eq!(v, vec![RectI::new(0, 0, 10, 10)]);
        let mut v: Vec<RectI> = (0..100).map(|i| RectI::new(i, i, i + 1, i + 1)).collect();
        coalesce(&mut v, b);
        assert_eq!(v, vec![RectI::new(0, 0, 100, 100)]);
    }

    #[test]
    fn subtract_rect_pieces() {
        let r = RectI::new(0, 0, 100, 100);
        assert_eq!(subtract_rect(r, &RectI::new(200, 0, 300, 10)), vec![r]);
        assert!(subtract_rect(r, &RectI::new(-10, -10, 200, 200)).is_empty());
        let parts = subtract_rect(r, &RectI::new(10, 20, 30, 40));
        assert_eq!(parts.iter().map(|p| p.width() * p.height()).sum::<i32>(), 100 * 100 - 20 * 20);
        assert!(parts.iter().all(|p| p.intersect(&RectI::new(10, 20, 30, 40)).is_empty()));
    }
}
