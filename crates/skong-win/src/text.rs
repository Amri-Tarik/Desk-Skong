//! Text for speech bubbles: GDI renders grayscale-antialiased glyphs (white on black) into
//! a small DIB; the coverage becomes alpha, tinted and composited premultiplied onto an
//! [`Image`]. Layered windows need real per-pixel alpha, which plain GDI text lacks.
use anyhow::{Context, Result, bail};
use skong_core::canvas::{Image, over, premul};
use windows::Win32::Foundation::{COLORREF, SIZE};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, FF_ROMAN, FW_NORMAL, GetTextExtentPoint32W, GetTextFaceW, GetTextMetricsW, HBITMAP, HDC, HFONT, HGDIOBJ,
    OUT_TT_PRECIS, SelectObject, SetBkMode, SetTextColor, TEXTMETRICW, TRANSPARENT, TextOutW, VARIABLE_PITCH,
};
use windows::core::PCWSTR;

/// One font, ready to measure and draw lines.
pub struct TextPainter {
    dc: HDC,
    font: HFONT,
    old_font: HGDIOBJ,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u32,
    cap: (u32, u32),
    line_height: i32,
    /// The face actually selected.
    pub face: String,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl TextPainter {
    /// The first installed face of `faces` (serif fallback), `px` pixels tall.
    pub fn new(faces: &[&str], px: i32, italic: bool) -> Result<Self> {
        // SAFETY: a memory DC owned by `Self`, released in Drop.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            bail!("CreateCompatibleDC");
        }
        let mut chosen: Option<(HFONT, String)> = None;
        for face in faces.iter().copied().chain(["Georgia"]) {
            let name: Vec<u16> = wide(face).into_iter().chain([0]).collect();
            // SAFETY: plain font creation; the name buffer outlives the call.
            let font = unsafe {
                CreateFontW(
                    -px,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    italic as u32,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_TT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    ANTIALIASED_QUALITY,
                    (VARIABLE_PITCH.0 | FF_ROMAN.0) as u32,
                    PCWSTR(name.as_ptr()),
                )
            };
            if font.is_invalid() {
                continue;
            }
            // SAFETY: select into our DC and read back the face GDI really matched.
            let actual = unsafe {
                let prev = SelectObject(dc, font.into());
                let mut buf = [0u16; 64];
                let n = GetTextFaceW(dc, Some(&mut buf)).max(1) as usize - 1;
                SelectObject(dc, prev);
                String::from_utf16_lossy(&buf[..n.min(63)])
            };
            if actual.eq_ignore_ascii_case(face) {
                chosen = Some((font, actual));
                break;
            }
            // SAFETY: font created above, not selected anywhere.
            unsafe {
                let _ = DeleteObject(font.into());
            }
        }
        let (font, face) = match chosen {
            Some(c) => c,
            None => {
                // SAFETY: release the DC on failure.
                unsafe {
                    let _ = DeleteDC(dc);
                }
                bail!("no usable font among {faces:?}");
            }
        };
        // SAFETY: select the font for the DC's lifetime; read its metrics.
        let (old_font, line_height) = unsafe {
            let old = SelectObject(dc, font.into());
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut tm);
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00ff_ffff));
            (old, tm.tmHeight + tm.tmExternalLeading)
        };
        let mut p = Self { dc, font, old_font, bitmap: HBITMAP::default(), old_bitmap: HGDIOBJ::default(), bits: std::ptr::null_mut(), cap: (0, 0), line_height, face };
        p.ensure(256, 64)?;
        Ok(p)
    }

    pub fn line_height(&self) -> f32 {
        self.line_height as f32
    }

    /// Width of `s` in pixels.
    pub fn measure(&self, s: &str) -> f32 {
        let w = wide(s);
        let mut size = SIZE::default();
        // SAFETY: our DC with the font selected; `w` outlives the call.
        unsafe {
            let _ = GetTextExtentPoint32W(self.dc, &w, &mut size);
        }
        size.cx as f32
    }

    fn ensure(&mut self, w: u32, h: u32) -> Result<()> {
        if w <= self.cap.0 && h <= self.cap.1 {
            return Ok(());
        }
        let (w, h) = (w.max(self.cap.0).next_power_of_two(), h.max(self.cap.1).next_power_of_two());
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: valid 32bpp header; the bitmap owns `bits`.
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

    /// Draw `text` with its top-left at (x, y) on `dst`, in `rgb` at `opacity` (0..1).
    pub fn draw(&mut self, dst: &mut Image, x: i32, y: i32, text: &str, rgb: [u8; 3], opacity: f32) -> Result<()> {
        let w = self.measure(text).ceil() as u32 + 4;
        let h = self.line_height.max(1) as u32 + 4;
        self.ensure(w, h)?;
        let stride = self.cap.0 as usize;
        // SAFETY: the DIB holds cap.0 * cap.1 pixels; we only touch the (w, h) corner.
        let dib = unsafe { std::slice::from_raw_parts_mut(self.bits, stride * self.cap.1 as usize) };
        for row in 0..h as usize {
            dib[row * stride..row * stride + w as usize].fill(0);
        }
        let t = wide(text);
        // SAFETY: drawing into our own DIB.
        unsafe {
            let _ = TextOutW(self.dc, 2, 2, &t);
        }
        let op = (opacity.clamp(0.0, 1.0) * 255.0) as u32;
        for row in 0..h as i32 {
            let dy = y + row - 2;
            if dy < 0 || dy >= dst.height as i32 {
                continue;
            }
            for col in 0..w as i32 {
                let dx = x + col - 2;
                if dx < 0 || dx >= dst.width as i32 {
                    continue;
                }
                let p = dib[row as usize * stride + col as usize];
                let cov = ((p >> 16) & 0xff).max((p >> 8) & 0xff).max(p & 0xff);
                if cov == 0 {
                    continue;
                }
                let a = (cov * op / 255) as u8;
                let i = dy as usize * dst.width as usize + dx as usize;
                dst.px[i] = over(premul(rgb[0], rgb[1], rgb[2], a), dst.px[i]);
            }
        }
        Ok(())
    }
}

impl Drop for TextPainter {
    fn drop(&mut self) {
        // SAFETY: releasing the objects created in `new` / `ensure`.
        unsafe {
            SelectObject(self.dc, self.old_font);
            if !self.old_bitmap.is_invalid() {
                SelectObject(self.dc, self.old_bitmap);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
            let _ = DeleteObject(self.font.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
