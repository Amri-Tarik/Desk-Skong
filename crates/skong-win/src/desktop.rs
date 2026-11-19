//! Event-driven observation of the desktop: monitors and top-level windows, turned
//! into a [`DesktopSnapshot`].
//!
//! # Threading
//!
//! [`DesktopWatcher`] installs out-of-context WinEvent hooks. Windows delivers those
//! callbacks **only while the installing thread pumps messages**, so the watcher must be
//! created and polled on the UI thread that runs the message loop. The type is `!Send`
//! to enforce this. Display-configuration changes (resolution, monitor plug, taskbar
//! moves) don't raise WinEvents: forward `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE` and
//! `WM_DPICHANGED` from your window procedure to [`mark_dirty`]. A slow safety refresh
//! catches anything else.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use skong_core::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use skong_core::geom::RectI;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, EnumWindows, GA_ROOT, GWL_EXSTYLE,
    GWL_STYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetLayeredWindowAttributes, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, MONITORINFOF_PRIMARY, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};
use windows::core::{BOOL, PWSTR};

/// Minimum spacing between two event-triggered enumerations.
pub const COALESCE_INTERVAL: Duration = Duration::from_millis(8);
/// Re-enumerate at least this often even without events.
pub const SAFETY_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// Windows smaller than this (either side, px) are not listed at all.
const MIN_LISTED_SIZE: i32 = 8;

/// Window classes belonging to the shell or system UI, never treated as app windows.
const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Windows.UI.Core.CoreWindow",
    "Windows.UI.Composition.DesktopWindowContentBridge",
    "XamlExplorerHostIslandWindow",
    "MultitaskingViewFrame",
    "TaskListThumbnailWnd",
    "ForegroundStaging",
    "Shell_InputSwitchTopLevelWindow",
    "EdgeUiInputTopWndClass",
    "ApplicationManager_ImmersiveShellWindow",
    "Xaml_WindowedPopupClass",
    "SysShadow",
    "tooltips_class32",
    "#32768", // popup menus
    "IME",
    "MSCTFIME UI",
];

/// Bumped by the WinEvent callback (and [`mark_dirty`]); watchers compare against the
/// value they last consumed.
static EVENT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Opt the process into per-monitor-v2 DPI awareness so all window/monitor coordinates
/// are physical pixels. Call first thing, before creating any window. Failure (already
/// set, e.g. by a manifest) is ignored.
pub fn enable_per_monitor_dpi() {
    // SAFETY: plain Win32 call with a predefined context constant.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// A window that left the snapshot was really closed (destroyed, or hidden by its app
/// like a close-to-tray), not minimized or cloaked (virtual desktop switch).
pub fn window_closed(id: u64) -> bool {
    let hwnd = HWND(id as usize as *mut core::ffi::c_void);
    // SAFETY: plain queries on a window handle that may no longer exist.
    unsafe {
        if !windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool() {
            return true;
        }
        !IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() && !is_cloaked(hwnd)
    }
}

/// Request a re-enumeration on the next [`DesktopWatcher::poll`]. Cheap and thread-safe.
pub fn mark_dirty() {
    EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
}

/// Event ranges we hook, inclusive. Kept as separate ranges so noisy events in
/// between (menus, scrolling, focus, selection...) never reach our callback.
const HOOK_RANGES: &[(u32, u32)] = &[
    (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
    (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
    (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
    (EVENT_OBJECT_DESTROY, EVENT_OBJECT_REORDER), // destroy, show, hide, reorder
    (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
    (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
];

/// Keeps a [`DesktopSnapshot`] up to date from WinEvent notifications.
pub struct DesktopWatcher {
    snapshot: DesktopSnapshot,
    hooks: Vec<HWINEVENTHOOK>,
    seen_seq: u64,
    last_enum: Instant,
    last_cost: Duration,
    enumerator: Enumerator,
}

impl DesktopWatcher {
    /// Install the hooks and take the first snapshot (generation 1). Must be called on
    /// the thread that pumps messages; see the module docs.
    pub fn new() -> Result<Self> {
        let hooks = install_hooks()?;
        let seen_seq = EVENT_SEQ.load(Ordering::Relaxed);
        let mut enumerator = Enumerator::new();
        let start = Instant::now();
        let mut snapshot = enumerator.capture();
        let last_cost = start.elapsed();
        snapshot.generation = 1;
        Ok(Self { snapshot, hooks, seen_seq, last_enum: Instant::now(), last_cost, enumerator })
    }

    /// Re-enumerate if something changed (coalesced to one enumeration per
    /// [`COALESCE_INTERVAL`]) or the safety interval elapsed. Returns `true` and bumps
    /// the snapshot generation only if the snapshot content actually changed.
    pub fn poll(&mut self) -> bool {
        let since = self.last_enum.elapsed();
        let due = (self.pending() && since >= COALESCE_INTERVAL) || since >= SAFETY_REFRESH_INTERVAL;
        due && self.refresh()
    }

    /// Re-enumerate now, regardless of events. Returns `true` if the content changed.
    pub fn refresh(&mut self) -> bool {
        // Load before enumerating so events raised meanwhile trigger another pass.
        self.seen_seq = EVENT_SEQ.load(Ordering::Relaxed);
        let start = Instant::now();
        let next = self.enumerator.capture();
        self.last_cost = start.elapsed();
        self.last_enum = Instant::now();
        if next.monitors == self.snapshot.monitors && next.windows == self.snapshot.windows {
            return false;
        }
        self.snapshot = DesktopSnapshot { generation: self.snapshot.generation + 1, ..next };
        true
    }

    /// `true` if events arrived since the last enumeration.
    pub fn pending(&self) -> bool {
        EVENT_SEQ.load(Ordering::Relaxed) != self.seen_seq
    }

    pub fn snapshot(&self) -> &DesktopSnapshot {
        &self.snapshot
    }

    /// Wall time of the most recent enumeration (monitors + windows).
    pub fn last_enumerate_cost(&self) -> Duration {
        self.last_cost
    }
}

impl Drop for DesktopWatcher {
    fn drop(&mut self) {
        for &hook in &self.hooks {
            // SAFETY: each handle came from a successful SetWinEventHook on this thread.
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    }
}

fn install_hooks() -> Result<Vec<HWINEVENTHOOK>> {
    let mut hooks = Vec::with_capacity(HOOK_RANGES.len());
    for &(min, max) in HOOK_RANGES {
        // SAFETY: out-of-context hook with a 'static callback; no module handle needed.
        let hook = unsafe {
            SetWinEventHook(min, max, None, Some(on_win_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)
        };
        if hook.is_invalid() {
            for &h in &hooks {
                // SAFETY: handles from successful SetWinEventHook calls above.
                let _ = unsafe { UnhookWinEvent(h) };
            }
            bail!("SetWinEventHook({min:#x}..={max:#x}) failed");
        }
        hooks.push(hook);
    }
    Ok(hooks)
}

/// WinEvent callback: only decides relevance and bumps the sequence number.
unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if is_relevant(event, hwnd, id_object, id_child) {
        mark_dirty();
    }
}

fn is_relevant(event: u32, hwnd: HWND, id_object: i32, id_child: i32) -> bool {
    let window_itself = id_object == OBJID_WINDOW.0 && id_child == CHILDID_SELF as i32 && !hwnd.is_invalid();
    match event {
        // Fired at very high rates for child controls and the caret/cursor; only
        // top-level window moves matter. GetAncestor just reads user32's shared tables.
        EVENT_OBJECT_LOCATIONCHANGE | EVENT_OBJECT_SHOW | EVENT_OBJECT_HIDE => {
            // SAFETY: GetAncestor tolerates stale handles (returns null).
            window_itself && unsafe { GetAncestor(hwnd, GA_ROOT) } == hwnd
        }
        EVENT_OBJECT_DESTROY | EVENT_OBJECT_CLOAKED | EVENT_OBJECT_UNCLOAKED => window_itself,
        _ => true,
    }
}

/// Stateful enumerator: caches process names by pid and reuses buffers.
struct Enumerator {
    own_pid: u32,
    exe_names: HashMap<u32, String>,
    hwnds: Vec<HWND>,
}

impl Enumerator {
    fn new() -> Self {
        // SAFETY: no preconditions.
        let own_pid = unsafe { GetCurrentProcessId() };
        Self { own_pid, exe_names: HashMap::new(), hwnds: Vec::with_capacity(512) }
    }

    /// Full snapshot with generation 0 (the watcher assigns generations).
    fn capture(&mut self) -> DesktopSnapshot {
        let monitors = enumerate_monitors();
        // SAFETY: no preconditions.
        let foreground = unsafe { GetForegroundWindow() };
        self.hwnds.clear();
        // SAFETY: the callback only pushes into the Vec passed via lparam, which
        // outlives the synchronous EnumWindows call.
        let _ = unsafe { EnumWindows(Some(collect_hwnd), LPARAM(&mut self.hwnds as *mut Vec<HWND> as isize)) };

        let hwnds = std::mem::take(&mut self.hwnds);
        let windows: Vec<WindowInfo> = hwnds.iter().filter_map(|&h| self.inspect(h, &monitors, foreground)).collect();
        self.hwnds = hwnds;
        self.exe_names.retain(|pid, _| windows.iter().any(|w| w.pid == *pid));
        DesktopSnapshot { generation: 0, monitors, windows }
    }

    /// Describe `hwnd` if it is a real, visible app window on some monitor.
    /// Checks are ordered cheapest-first.
    fn inspect(&mut self, hwnd: HWND, monitors: &[Monitor], foreground: HWND) -> Option<WindowInfo> {
        // SAFETY (whole fn): read-only queries on a window handle; all tolerate the
        // window having been destroyed since enumeration.
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
            return None;
        }
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        let app_window = ex & WS_EX_APPWINDOW.0 != 0;
        if ex & WS_EX_TRANSPARENT.0 != 0 || (!app_window && ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0) {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == self.own_pid || is_cloaked(hwnd) {
            return None;
        }
        let class = class_name(hwnd);
        if SHELL_CLASSES.contains(&class.as_str()) {
            return None;
        }
        let title = window_text(hwnd);
        let has_caption = style & WS_CAPTION.0 == WS_CAPTION.0;
        if title.is_empty() && !app_window && !has_caption {
            return None;
        }
        let rect = frame_bounds(hwnd)?;
        if rect.width() < MIN_LISTED_SIZE || rect.height() < MIN_LISTED_SIZE {
            return None;
        }
        if !monitors.iter().any(|m| !m.rect.intersect(&rect).is_empty()) {
            return None;
        }
        if ex & WS_EX_LAYERED.0 != 0 && is_fully_transparent(hwnd) {
            return None;
        }
        let fullscreen = !has_caption && monitors.iter().any(|m| covers(&rect, &m.rect));
        Some(WindowInfo {
            id: hwnd.0 as usize as u64,
            rect,
            title,
            class,
            process: self.exe_name(pid).to_owned(),
            pid,
            maximized: unsafe { IsZoomed(hwnd) }.as_bool(),
            fullscreen,
            foreground: hwnd == foreground,
            topmost: ex & WS_EX_TOPMOST.0 != 0,
        })
    }

    /// Executable file name of `pid` (e.g. `notepad.exe`); empty if inaccessible.
    fn exe_name(&mut self, pid: u32) -> &str {
        self.exe_names.entry(pid).or_insert_with(|| query_exe_name(pid).unwrap_or_default())
    }
}

/// EnumWindows callback: collect handles, decide later.
unsafe extern "system" fn collect_hwnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the &mut Vec<HWND> passed by `Enumerator::capture`.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    out.push(hwnd);
    BOOL(1)
}

/// EnumDisplayMonitors callback: collect handles.
unsafe extern "system" fn collect_monitor(hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the &mut Vec<HMONITOR> passed by `enumerate_monitors`.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    out.push(hmon);
    BOOL(1)
}

fn enumerate_monitors() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::with_capacity(4);
    // SAFETY: synchronous enumeration; the callback writes only into `handles`.
    let _ = unsafe {
        EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(&mut handles as *mut Vec<HMONITOR> as isize))
    };
    handles.into_iter().filter_map(describe_monitor).collect()
}

fn describe_monitor(hmon: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `info` is a properly sized MONITORINFO.
    if !unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: out-pointers to locals.
    if unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        dpi_x = 96;
    }
    Some(Monitor {
        rect: to_rect(info.rcMonitor),
        work: to_rect(info.rcWork),
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        scale_milli: dpi_x * 1000 / 96,
    })
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: DWMWA_CLOAKED writes a DWORD into `cloaked`.
    let ok = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), size_of::<u32>() as u32)
    };
    ok.is_ok() && cloaked != 0
}

/// Visible frame (DWM extended frame bounds), falling back to the window rect.
fn frame_bounds(hwnd: HWND) -> Option<RectI> {
    let mut r = RECT::default();
    // SAFETY: DWMWA_EXTENDED_FRAME_BOUNDS writes a RECT into `r`.
    let dwm = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut r as *mut RECT).cast(), size_of::<RECT>() as u32)
    };
    if dwm.is_err() {
        // SAFETY: out-pointer to a local RECT.
        unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    }
    Some(to_rect(r))
}

/// Layered windows with constant alpha 0 (SetLayeredWindowAttributes) are invisible.
fn is_fully_transparent(hwnd: HWND) -> bool {
    let mut alpha = 255u8;
    let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
    // SAFETY: out-pointers to locals; fails harmlessly for UpdateLayeredWindow windows.
    let ok = unsafe { GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)) };
    ok.is_ok() && flags.0 & LWA_ALPHA.0 != 0 && alpha == 0
}

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: buffer length is passed by the wrapper. For windows of other processes
    // this reads the cached caption and never sends WM_GETTEXT, so it cannot hang.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: buffer length is passed by the wrapper.
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Full image path of a process (e.g. `C:\Games\x\x.exe`).
pub(crate) fn query_exe_path(pid: u32) -> Option<String> {
    // SAFETY: handle is closed below on every path after a successful open.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `len` holds the buffer capacity in characters.
    let res = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
    // SAFETY: valid handle from OpenProcess.
    let _ = unsafe { CloseHandle(handle) };
    res.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

fn query_exe_name(pid: u32) -> Option<String> {
    let path = query_exe_path(pid)?;
    Some(path.rsplit(['\\', '/']).next().unwrap_or_default().to_owned())
}

fn to_rect(r: RECT) -> RectI {
    RectI::new(r.left, r.top, r.right, r.bottom)
}

/// `outer` fully contains `inner`.
fn covers(outer: &RectI, inner: &RectI) -> bool {
    outer.left <= inner.left && outer.top <= inner.top && outer.right >= inner.right && outer.bottom >= inner.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_is_inclusive() {
        let m = RectI::new(0, 0, 100, 100);
        assert!(covers(&m, &m));
        assert!(covers(&RectI::new(-5, -5, 105, 105), &m));
        assert!(!covers(&RectI::new(0, 0, 100, 99), &m));
    }

    /// Smoke test against the live desktop: enumeration works and is reasonably fast.
    #[test]
    fn live_snapshot() {
        let mut e = Enumerator::new();
        let start = Instant::now();
        let snap = e.capture();
        let cost = start.elapsed();
        assert!(!snap.monitors.is_empty(), "no monitors enumerated");
        assert!(snap.monitors.iter().all(|m| !m.rect.is_empty() && m.scale_milli >= 1000));
        assert!(snap.windows.iter().all(|w| !SHELL_CLASSES.contains(&w.class.as_str())));
        assert!(cost < Duration::from_millis(500), "enumeration took {cost:?}");
    }

    #[test]
    fn watcher_installs_and_unhooks() -> Result<()> {
        let mut w = DesktopWatcher::new()?;
        assert_eq!(w.snapshot().generation, 1);
        // Without events and before the safety interval nothing happens.
        let gen_before = w.snapshot().generation;
        w.seen_seq = EVENT_SEQ.load(Ordering::Relaxed);
        assert!(!w.poll());
        assert_eq!(w.snapshot().generation, gen_before);
        mark_dirty();
        assert!(w.pending());
        Ok(())
    }
}
