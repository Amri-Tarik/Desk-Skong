//! A small always-on-top, per-pixel-alpha window that shows one composited image.
//!
//! Instead of a full-screen overlay (which makes DWM recompose the whole screen every
//! frame), the window is only as big as the sprite and moves with it. Layered windows
//! hit-test per pixel: clicks on transparent pixels fall through to whatever is below,
//! clicks on the sprite reach us (petting / grabbing later).
use anyhow::{Context, Result};
use skong_core::canvas::Image;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST,
    IDC_ARROW, LoadCursorW, RegisterClassExW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WNDCLASSEXW, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, GetCursorPos,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

pub struct SpriteWindow {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u32,
    cap: (u32, u32),
    /// Last presented geometry, to skip redundant updates.
    shown: Option<(i32, i32, u32, u32)>,
    /// Whole-window opacity applied on top of the per-pixel alpha (255: as drawn).
    opacity: u8,
}

/// Mouse activity on a sprite window (screen coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEvent {
    Down { x: i32, y: i32 },
    Move { x: i32, y: i32 },
    Up { x: i32, y: i32 },
    RightClick { x: i32, y: i32 },
}

thread_local! {
    static MOUSE: std::cell::RefCell<Vec<(isize, MouseEvent)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    // SAFETY: writes a local POINT.
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ev = match msg {
        WM_LBUTTONDOWN => {
            // SAFETY: capture the mouse for our own window so a drag keeps reporting.
            unsafe { SetCapture(hwnd) };
            let (x, y) = cursor();
            Some(MouseEvent::Down { x, y })
        }
        WM_MOUSEMOVE => {
            let (x, y) = cursor();
            Some(MouseEvent::Move { x, y })
        }
        WM_LBUTTONUP => {
            // SAFETY: releasing our own capture.
            let _ = unsafe { ReleaseCapture() };
            let (x, y) = cursor();
            Some(MouseEvent::Up { x, y })
        }
        WM_RBUTTONUP => {
            let (x, y) = cursor();
            Some(MouseEvent::RightClick { x, y })
        }
        _ => None,
    };
    if let Some(ev) = ev {
        MOUSE.with(|q| q.borrow_mut().push((hwnd.0 as isize, ev)));
        return LRESULT(0);
    }
    // Display layout changes raise no WinEvents; tell the desktop watcher directly.
    if matches!(msg, WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED) {
        crate::desktop::mark_dirty();
    }
    // SAFETY: plain forwarding to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

impl SpriteWindow {
    pub fn new() -> Result<Self> {
        // SAFETY: standard Win32 window creation; all handles are owned by `Self` and
        // released in `Drop`.
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = w!("DeskSkongSprite");
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: class,
                ..Default::default()
            };
            // Re-registering an existing class fails harmlessly; CreateWindow reports real errors.
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("Desk-Skong"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .context("CreateWindowExW")?;
            // Keep ourselves out of screenshots / screen capture (and out of our own vision),
            // unless the user wants the pet visible in screen shares.
            let aff = if share_visible() { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
            if let Err(e) = SetWindowDisplayAffinity(hwnd, aff) {
                log::warn!("SetWindowDisplayAffinity failed: {e}");
            }
            live_windows().lock().map(|mut l| l.push(hwnd.0 as isize)).ok();
            let dc = CreateCompatibleDC(None);
            let mut win = Self { hwnd, dc, bitmap: HBITMAP::default(), old_bitmap: HGDIOBJ::default(), bits: std::ptr::null_mut(), cap: (0, 0), shown: None, opacity: 255 };
            win.ensure_capacity(256, 256)?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(win)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Mouse events received by this window since the last call.
    pub fn take_mouse(&self) -> Vec<MouseEvent> {
        let me = self.hwnd.0 as isize;
        MOUSE.with(|q| {
            let mut q = q.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = q.drain(..).partition(|(h, _)| *h == me);
            *q = rest;
            mine.into_iter().map(|(_, e)| e).collect()
        })
    }

    /// Grow the backing DIB if needed (never shrinks, so steady-state presents don't allocate).
    fn ensure_capacity(&mut self, w: u32, h: u32) -> Result<()> {
        if w <= self.cap.0 && h <= self.cap.1 {
            return Ok(());
        }
        let (w, h) = (w.max(self.cap.0).next_power_of_two(), h.max(self.cap.1).next_power_of_two());
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: bmi is a valid 32bpp header; `bits` receives the DIB memory owned by the bitmap.
        unsafe {
            let bmp = CreateDIBSection(Some(self.dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).context("CreateDIBSection")?;
            let prev = SelectObject(self.dc, bmp.into());
            if self.bitmap.is_invalid() {
                self.old_bitmap = prev;
            } else {
                let _ = DeleteObject(self.bitmap.into());
            }
            self.bitmap = bmp;
        }
        self.bits = bits as *mut u32;
        self.cap = (w, h);
        Ok(())
    }

    /// Show `img` with its top-left at screen (x, y).
    pub fn present(&mut self, img: &Image, x: i32, y: i32) -> Result<()> {
        self.present_flipped(img, x, y, false)
    }

    /// Like [`Self::present`], optionally mirrored horizontally (no extra allocation).
    pub fn present_flipped(&mut self, img: &Image, x: i32, y: i32, flip_x: bool) -> Result<()> {
        let (w, h) = (img.width.max(1), img.height.max(1));
        self.ensure_capacity(w, h)?;
        let stride = self.cap.0 as usize;
        // SAFETY: the DIB is cap.0 * cap.1 u32s and (w, h) fit inside it.
        let dib = unsafe { std::slice::from_raw_parts_mut(self.bits, stride * self.cap.1 as usize) };
        for row in 0..img.height as usize {
            let src = &img.px[row * img.width as usize..(row + 1) * img.width as usize];
            let dst = &mut dib[row * stride..row * stride + src.len()];
            dst.copy_from_slice(src);
            if flip_x {
                dst.reverse();
            }
        }
        self.update_layered(Some((x, y)), Some((w, h)))
    }

    /// Move without re-uploading pixels.
    pub fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        if self.shown.is_some_and(|s| (s.0, s.1) == (x, y)) {
            return Ok(());
        }
        self.update_layered(Some((x, y)), None)
    }

    /// Fade the whole window (0..=255) without re-uploading pixels (after a `present`).
    pub fn set_opacity(&mut self, opacity: u8) -> Result<()> {
        if opacity == self.opacity {
            return Ok(());
        }
        self.opacity = opacity;
        if self.shown.is_none() {
            return Ok(());
        }
        self.update_layered(None, None)
    }

    fn update_layered(&mut self, pos: Option<(i32, i32)>, size: Option<(u32, u32)>) -> Result<()> {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: self.opacity, AlphaFormat: AC_SRC_ALPHA as u8 };
        let pt = pos.map(|(x, y)| POINT { x, y });
        let sz = size.map(|(w, h)| SIZE { cx: w as i32, cy: h as i32 });
        let src = POINT { x: 0, y: 0 };
        // SAFETY: dc has our DIB selected; pointers reference locals valid for the call.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                pt.as_ref().map(|p| p as *const _),
                sz.as_ref().map(|s| s as *const _),
                size.map(|_| self.dc),
                size.map(|_| &src as *const _),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .context("UpdateLayeredWindow")?;
        }
        let prev = self.shown.unwrap_or_default();
        let (x, y) = pos.unwrap_or((prev.0, prev.1));
        let (w, h) = size.unwrap_or((prev.2, prev.3));
        self.shown = Some((x, y, w, h));
        Ok(())
    }

    /// Debug: allow screenshots / capture to see the sprite (excluded by default).
    pub fn set_capturable(&self, on: bool) {
        let aff = if on || share_visible() { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
        // SAFETY: display affinity of our own window.
        if let Err(e) = unsafe { SetWindowDisplayAffinity(self.hwnd, aff) } {
            log::warn!("SetWindowDisplayAffinity: {e}");
        }
    }

    /// Let mouse input pass through the sprite entirely.
    pub fn set_click_through(&self, on: bool) {
        // SAFETY: toggling an extended style bit on our own window.
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let t = WS_EX_TRANSPARENT.0 as isize;
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, if on { ex | t } else { ex & !t });
        }
    }

    /// Hide or show the sprite (pause).
    pub fn set_visible(&self, on: bool) {
        // SAFETY: show state of our own window.
        unsafe {
            let _ = ShowWindow(self.hwnd, if on { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }

    /// Re-assert topmost (other topmost windows may have been raised above us).
    pub fn raise(&self) {
        // SAFETY: z-order change on our own window.
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

impl Drop for SpriteWindow {
    fn drop(&mut self) {
        // SAFETY: releasing handles created in `new`/`ensure_capacity`.
        unsafe {
            if !self.old_bitmap.is_invalid() {
                SelectObject(self.dc, self.old_bitmap);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
        }
        let me = self.hwnd.0 as isize;
        live_windows().lock().map(|mut l| l.retain(|h| *h != me)).ok();
    }
}

/// "Visible in screen shares": every sprite window of ours shows up in screen capture
/// (Discord, OBS, screenshots). Off (default): they are excluded, which also keeps them
/// out of the pet's own vision.
static SHARE_VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn share_visible() -> bool {
    SHARE_VISIBLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Every live sprite window (raw handles), to re-apply the share setting.
fn live_windows() -> &'static std::sync::Mutex<Vec<isize>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<Vec<isize>>> = std::sync::OnceLock::new();
    LIVE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Show (or hide) all of our sprite windows in screen capture, now and for new ones.
pub fn set_share_visible(on: bool) {
    SHARE_VISIBLE.store(on, std::sync::atomic::Ordering::Relaxed);
    let aff = if on { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
    if let Ok(list) = live_windows().lock() {
        for &h in list.iter() {
            // SAFETY: display affinity of our own (live) windows; a stale handle just fails.
            let _ = unsafe { SetWindowDisplayAffinity(HWND(h as *mut _), aff) };
        }
    }
}
