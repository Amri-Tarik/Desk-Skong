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

/// Mean luminance (0..1) of the screen inside `r` (virtual-desktop px), sampled with a plain
/// GDI copy: layered windows (all of ours: the knight, props) aren't included, so this is
/// what is drawn *behind* him. `None` when the copy fails (secure desktop, off-screen).
/// Cheap for small rects (a few hundred µs); call every couple of seconds at most.
pub fn mean_luma(r: RectI) -> Option<f32> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    let (w, h) = (r.width().clamp(1, 256), r.height().clamp(1, 256));
    // SAFETY: standard GDI screen copy into a bitmap owned and released here; GetDIBits
    // writes w*h 32-bit pixels into a buffer of exactly that size.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let ok = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY).is_ok();
        let mut px = vec![0u32; (w * h) as usize];
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        SelectObject(mem, old);
        let lines = if ok { GetDIBits(mem, bmp, 0, h as u32, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS) } else { 0 };
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if lines == 0 {
            return None;
        }
        let sum: f32 = px
            .iter()
            .map(|p| {
                let (r, g, b) = (((p >> 16) & 0xff) as f32, ((p >> 8) & 0xff) as f32, (p & 0xff) as f32);
                (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0
            })
            .sum();
        Some(sum / px.len() as f32)
    }
}
