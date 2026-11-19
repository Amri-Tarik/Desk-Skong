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

/// Held for the process lifetime; dropping it releases the single-instance lock.
pub struct InstanceGuard(windows::Win32::Foundation::HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: closing our own mutex handle.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// `Some(guard)` if no other Desk-Skong pet is running in this session, `None` otherwise.
pub fn single_instance() -> Option<InstanceGuard> {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    // SAFETY: named mutex creation; the last-error check must directly follow the call.
    unsafe {
        let h = CreateMutexW(None, true, windows::core::w!(r"Local\DeskSkongPet")).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = windows::Win32::Foundation::CloseHandle(h);
            return None;
        }
        Some(InstanceGuard(h))
    }
}

/// Local wall-clock hour (0..24) with fraction, e.g. 21.5 = 9:30 pm.
pub fn local_hour() -> f32 {
    // SAFETY: plain query into a returned struct.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    t.wHour as f32 + t.wMinute as f32 / 60.0
}
