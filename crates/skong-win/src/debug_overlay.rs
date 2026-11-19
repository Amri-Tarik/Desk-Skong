//! Full-virtual-desktop debug overlay: terrain (ledges, walls), navigation surfaces and
//! edges, plus free-form lines, drawn on a topmost, click-through, never-activated layered
//! window that is excluded from screen capture (content vision never sees it).
//!
//! The caller redraws only when something changed: [`DebugOverlay::draw`] rasterizes into
//! a premultiplied DIB and [`DebugOverlay::present`] hands it to the compositor once. Nothing
//! runs between changes; dropping the overlay frees the window and its pixels.
//!
//! Colours: Ground ledges blue, OneWay green, Content magenta, walls orange, content walls purple; nav surfaces
//! (standable spans) cyan; edges per [`MoveKind`] (see [`move_color`]), the current route
//! thicker and opaque. A legend sits in the primary monitor's top-right corner.
use anyhow::{Context, Result, bail};
use skong_core::canvas::premul;
use skong_core::geom::{RectF, Vec2};
use skong_core::nav::{DebugPath, DebugSpan, MoveKind};
use skong_core::terrain::{Facing, LedgeKind, Terrain, WallKind};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
    DEFAULT_GUI_FONT, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetStockObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SW_SHOWNOACTIVATE, SetWindowDisplayAffinity, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::desktop::mark_dirty;
use crate::monitor;

/// Terrain line thickness (px).
const LINE: i32 = 3;

/// A free-form segment for callers' own debug data (screen px, straight RGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugLine {
    pub a: Vec2,
    pub b: Vec2,
    pub rgb: (u8, u8, u8),
    pub width: u8,
}

/// What to draw. Empty slices / `None` skip a layer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scene<'a> {
    pub terrain: Option<&'a Terrain>,
    pub spans: &'a [DebugSpan],
    pub paths: &'a [DebugPath],
    pub lines: &'a [DebugLine],
    /// Move-kind / terrain colour legend.
    pub legend: bool,
}

/// Edge colour per move kind.
pub fn move_color(kind: MoveKind) -> (u8, u8, u8) {
    match kind {
        MoveKind::Walk => (190, 190, 190),
        MoveKind::Drop => (255, 220, 0),
        MoveKind::DropThrough => (200, 140, 40),
        MoveKind::Jump => (90, 170, 255),
        MoveKind::DoubleJump => (170, 110, 255),
        MoveKind::WallJump => (255, 80, 80),
        MoveKind::Dash => (170, 255, 90),
        MoveKind::ShadowDash => (255, 150, 210),
        MoveKind::Superdash => (255, 255, 255),
    }
}

const GROUND: (u8, u8, u8) = (40, 120, 255);
const ONE_WAY: (u8, u8, u8) = (40, 230, 60);
const CONTENT: (u8, u8, u8) = (255, 40, 220);
const WALL: (u8, u8, u8) = (255, 140, 20);
const CONTENT_WALL: (u8, u8, u8) = (150, 60, 255);
const SURFACE: (u8, u8, u8) = (0, 230, 230);

/// Display changes re-trigger desktop enumeration; everything else is default.
extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if matches!(msg, WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED) {
        mark_dirty();
    }
    // SAFETY: forwarding the original arguments.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The overlay window and its backing DIB.
pub struct DebugOverlay {
    hwnd: HWND,
    bounds: RectF,
    origin: POINT,
    size: SIZE,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    old_font: HGDIOBJ,
    bits: *mut u32,
}

impl DebugOverlay {
    /// Create (shown, empty) covering `bounds` (virtual-desktop px; e.g. `Terrain::bounds`).
    pub fn new(bounds: RectF) -> Result<Self> {
        let origin = POINT { x: bounds.left as i32, y: bounds.top as i32 };
        let size = SIZE { cx: (bounds.width() as i32).max(1), cy: (bounds.height() as i32).max(1) };
        let hwnd = create_window(origin, size)?;
        // SAFETY: our own window; affinity is best-effort (keeps vision from seeing lines).
        if let Err(e) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
            log::warn!("debug overlay: SetWindowDisplayAffinity: {e}");
        }
        let (dc, bitmap, bits) = match create_dib(size) {
            Ok(d) => d,
            Err(e) => {
                // SAFETY: destroying the window we just created.
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err(e);
            }
        };
        // SAFETY: select our bitmap and a stock font into our DC; restored in Drop.
        let (old, old_font) = unsafe { (SelectObject(dc, bitmap.into()), SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT))) };
        let o = Self { hwnd, bounds, origin, size, dc, bitmap, old, old_font, bits };
        o.present()?;
        // SAFETY: valid window; SW_SHOWNOACTIVATE never steals focus.
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Ok(o)
    }

    /// Area covered; recreate the overlay when the terrain bounds change.
    pub fn bounds(&self) -> RectF {
        self.bounds
    }

    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `bits` points to cx*cy u32s owned by `bitmap`, alive as long as self;
        // `&mut self` makes this the only view.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.size.cx * self.size.cy) as usize) }
    }

    /// Rasterize `scene` (replacing the previous picture). Call [`Self::present`] after.
    pub fn draw(&mut self, scene: &Scene) {
        let mut c = Canvas { ox: self.origin.x, oy: self.origin.y, w: self.size.cx, h: self.size.cy, px: self.pixels() };
        c.px.fill(0);
        if let Some(t) = scene.terrain {
            let half = (LINE / 2) as f32;
            for l in &t.ledges {
                let rgb = match l.kind {
                    LedgeKind::Ground => GROUND,
                    LedgeKind::OneWay => ONE_WAY,
                    LedgeKind::Content => CONTENT,
                };
                c.rect(l.x0, l.y - half, l.x1, l.y - half + LINE as f32, argb(rgb, 230));
            }
            for wall in &t.walls {
                // Inside the solid side, so adjacent window walls stay distinguishable.
                let x0 = match wall.face {
                    Facing::Left => wall.x,
                    Facing::Right => wall.x - LINE as f32,
                };
                let rgb = if wall.kind == WallKind::Content { CONTENT_WALL } else { WALL };
                c.rect(x0, wall.y0, x0 + LINE as f32, wall.y1, argb(rgb, 230));
            }
        }
        for s in scene.spans {
            // Just above the ledge line so both stay visible.
            c.rect(s.x0, s.y - 6.0, s.x1, s.y - 4.0, argb(SURFACE, 220));
            c.rect(s.x0, s.y - 9.0, s.x0 + 1.0, s.y - 1.0, argb(SURFACE, 220));
            c.rect(s.x1 - 1.0, s.y - 9.0, s.x1, s.y - 1.0, argb(SURFACE, 220));
        }
        // Background edges first, the route on top.
        for p in scene.paths.iter().filter(|p| !p.highlighted).chain(scene.paths.iter().filter(|p| p.highlighted)) {
            let (alpha, width) = if p.highlighted { (255, 4) } else { (150, 1) };
            let color = argb(move_color(p.kind), alpha);
            for seg in p.points.windows(2) {
                c.line(seg[0], seg[1], width, color);
            }
            if p.highlighted && let Some(&end) = p.points.last() {
                c.rect(end.x - 4.0, end.y - 4.0, end.x + 4.0, end.y + 4.0, color);
            }
        }
        for l in scene.lines {
            c.line(l.a, l.b, l.width.max(1) as i32, argb(l.rgb, 255));
        }
        if scene.legend {
            self.legend();
        }
    }

    /// Colour legend in the primary monitor's top-right corner (GDI text on an opaque box).
    fn legend(&mut self) {
        let mut rows: Vec<(String, (u8, u8, u8))> = MoveKind::ALL.iter().map(|k| (format!("{k:?}"), move_color(*k))).collect();
        rows.extend([
            ("nav surface".to_string(), SURFACE),
            ("ground".to_string(), GROUND),
            ("one-way".to_string(), ONE_WAY),
            ("content".to_string(), CONTENT),
            ("wall".to_string(), WALL),
            ("content wall".to_string(), CONTENT_WALL),
        ]);
        let work = monitor::primary_work_area();
        let (row_h, box_w) = (16, 150);
        let box_h = rows.len() as i32 * row_h + 12;
        let (x0, y0) = (work.right - box_w - 12 - self.origin.x, work.top + 12 - self.origin.y);
        if x0 < 0 || y0 < 0 || x0 + box_w > self.size.cx || y0 + box_h > self.size.cy {
            return;
        }
        let (w, bg) = (self.size.cx, premul(20, 20, 24, 255));
        {
            let px = self.pixels();
            for y in y0..y0 + box_h {
                px[(y * w + x0) as usize..(y * w + x0 + box_w) as usize].fill(bg);
            }
            for (i, (_, rgb)) in rows.iter().enumerate() {
                let y = y0 + 6 + i as i32 * row_h + row_h / 2;
                for yy in y - 1..y + 2 {
                    px[(yy * w + x0 + 8) as usize..(yy * w + x0 + 36) as usize].fill(argb(*rgb, 255));
                }
            }
        }
        // SAFETY: drawing text into our own memory DC; GdiFlush before touching bits again.
        unsafe {
            SetBkMode(self.dc, TRANSPARENT);
            SetTextColor(self.dc, COLORREF(0x00e0e0e0));
            for (i, (label, _)) in rows.iter().enumerate() {
                let text: Vec<u16> = label.encode_utf16().collect();
                let _ = TextOutW(self.dc, x0 + 44, y0 + 6 + i as i32 * row_h, &text);
            }
            let _ = GdiFlush();
        }
        // GDI clears alpha where it draws: the box is opaque, so restore it.
        let px = self.pixels();
        for y in y0..y0 + box_h {
            for p in &mut px[(y * w + x0) as usize..(y * w + x0 + box_w) as usize] {
                *p |= 0xff00_0000;
            }
        }
    }

    /// Hand the current picture to the compositor.
    pub fn present(&self) -> Result<()> {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let src = POINT { x: 0, y: 0 };
        // SAFETY: all pointers reference locals/fields alive for the call.
        unsafe {
            UpdateLayeredWindow(self.hwnd, None, Some(&self.origin), Some(&self.size), Some(self.dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA)
        }
        .context("UpdateLayeredWindow")
    }
}

impl Drop for DebugOverlay {
    fn drop(&mut self) {
        // SAFETY: tearing down resources created in `new`, in reverse order.
        unsafe {
            SelectObject(self.dc, self.old_font);
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn argb((r, g, b): (u8, u8, u8), a: u8) -> u32 {
    premul(r, g, b, a)
}

/// Clipped rasterization into the overlay's pixels (screen coordinates in).
struct Canvas<'a> {
    ox: i32,
    oy: i32,
    w: i32,
    h: i32,
    px: &'a mut [u32],
}

impl Canvas<'_> {
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: u32) {
        let (x0, x1) = ((x0 as i32 - self.ox).clamp(0, self.w), (x1 as i32 - self.ox).clamp(0, self.w));
        let (y0, y1) = ((y0 as i32 - self.oy).clamp(0, self.h), (y1 as i32 - self.oy).clamp(0, self.h));
        if x0 >= x1 {
            return;
        }
        for y in y0..y1 {
            self.px[(y * self.w + x0) as usize..(y * self.w + x1) as usize].fill(c);
        }
    }

    /// Thick line: `width`-px squares stamped every pixel along the segment.
    fn line(&mut self, a: Vec2, b: Vec2, width: i32, c: u32) {
        let n = (b - a).abs().max_element().ceil().clamp(1.0, 20_000.0) as i32;
        let (lo, hi) = (-(width / 2) as f32, (width - width / 2) as f32);
        for i in 0..=n {
            let p = a + (b - a) * (i as f32 / n as f32);
            self.rect(p.x + lo, p.y + lo, p.x + hi, p.y + hi, c);
        }
    }
}

fn create_window(origin: POINT, size: SIZE) -> Result<HWND> {
    // SAFETY: registering a class with a 'static wndproc and name; creating a popup window
    // on this thread.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("SkongDebugOverlay");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
        // Re-registration (overlay recreated after a display change) fails harmlessly.
        RegisterClassW(&wc);
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
        CreateWindowExW(ex, class, w!("skong debug overlay"), WS_POPUP, origin.x, origin.y, size.cx, size.cy, None, None, Some(instance.into()), None)
            .context("CreateWindowExW (debug overlay)")
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
            bail!("CreateCompatibleDC failed");
        }
        let mut bits = std::ptr::null_mut();
        match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => Ok((dc, bitmap, bits.cast())),
            Ok(bitmap) => {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(dc);
                bail!("CreateDIBSection returned no pixels")
            }
            Err(e) => {
                let _ = DeleteDC(dc);
                Err(e).context("CreateDIBSection")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_clips_and_draws_thick_lines() {
        let mut px = vec![0u32; 100 * 50];
        let mut c = Canvas { ox: -10, oy: 0, w: 100, h: 50, px: &mut px };
        c.rect(-100.0, -5.0, 1000.0, 2.0, 1); // clipped to the top two rows
        c.line(Vec2::new(0.0, 25.0), Vec2::new(40.0, 25.0), 3, 2);
        c.line(Vec2::new(500.0, 500.0), Vec2::new(600.0, 600.0), 3, 3); // fully outside
        assert!(px[..200].iter().all(|&p| p == 1));
        assert_eq!(px[25 * 100 + 10], 2); // x=0 screen → column 10
        assert_eq!(px[24 * 100 + 50], 2);
        assert_eq!(px[26 * 100 + 50], 2);
        assert_eq!(px[25 * 100 + 52], 0);
        assert!(!px.contains(&3));
    }
}
