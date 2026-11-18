//! Minimal monitor queries (the full desktop model lives in `desktop`).
use skong_core::geom::RectI;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Current mouse cursor position in virtual-desktop pixels.
pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: writes into a local POINT.
    unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
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
