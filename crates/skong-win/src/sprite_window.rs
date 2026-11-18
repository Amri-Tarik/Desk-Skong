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
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST,
    IDC_ARROW, LoadCursorW, RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WNDCLASSEXW, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE,
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
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
            // Keep ourselves out of screenshots / screen capture (and out of our own vision).
            if let Err(e) = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
                log::warn!("SetWindowDisplayAffinity failed: {e}");
            }
            let dc = CreateCompatibleDC(None);
            let mut win = Self { hwnd, dc, bitmap: HBITMAP::default(), old_bitmap: HGDIOBJ::default(), bits: std::ptr::null_mut(), cap: (0, 0), shown: None };
            win.ensure_capacity(256, 256)?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(win)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
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

    fn update_layered(&mut self, pos: Option<(i32, i32)>, size: Option<(u32, u32)>) -> Result<()> {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
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
        let aff = if on { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
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
    }
}
