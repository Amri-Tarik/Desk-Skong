//! Minimal monitor queries (the full desktop model lives in `desktop`).
use skong_core::geom::RectI;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};

/// Opt into per-monitor DPI awareness so all coordinates are physical pixels.
pub fn enable_dpi_awareness() {
    // SAFETY: process-wide setting; failure (already set by manifest) is harmless.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Work area (screen minus taskbar) of the primary monitor.
pub fn primary_work_area() -> RectI {
    // SAFETY: plain queries with a properly sized MONITORINFO.
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let r = mi.rcWork;
            RectI::new(r.left, r.top, r.right, r.bottom)
        } else {
            RectI::new(0, 0, 1920, 1040)
        }
    }
}
