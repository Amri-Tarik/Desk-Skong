//! Tiny software compositor. Pixels are `u32` **premultiplied ARGB** (`0xAARRGGBB`),
//! which in memory (little-endian) is B,G,R,A — exactly a Win32 32-bpp top-down DIB.

#[derive(Clone, Debug, Default)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub px: Vec<u32>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, px: vec![0; (width * height) as usize] }
    }

    pub fn clear(&mut self) {
        self.px.fill(0);
    }

    /// Resize, discarding contents. Keeps the allocation when shrinking.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.px.clear();
        self.px.resize((width * height) as usize, 0);
    }

    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: u32 slice reinterpreted as bytes, alignment of u8 is 1.
        unsafe { std::slice::from_raw_parts(self.px.as_ptr() as *const u8, self.px.len() * 4) }
    }

    /// Source-over composite `src` with its top-left at (x, y). `flip_x` mirrors `src`.
    pub fn blit_over(&mut self, src: &Image, x: i32, y: i32, flip_x: bool) {
        let (dw, dh) = (self.width as i32, self.height as i32);
        let (sw, sh) = (src.width as i32, src.height as i32);
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + sw).min(dw);
        let y1 = (y + sh).min(dh);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for dy in y0..y1 {
            let sy = dy - y;
            let drow = (dy * dw) as usize;
            let srow = (sy * sw) as usize;
            for dx in x0..x1 {
                let sx = if flip_x { sw - 1 - (dx - x) } else { dx - x };
                let s = src.px[srow + sx as usize];
                let sa = s >> 24;
                if sa == 0 {
                    continue;
                }
                let d = &mut self.px[drow + dx as usize];
                *d = if sa == 255 { s } else { over(s, *d) };
            }
        }
    }
}

/// Premultiplied source-over.
#[inline]
pub fn over(s: u32, d: u32) -> u32 {
    let inv = 255 - (s >> 24);
    let rb = (((d & 0x00ff_00ff) * inv + 0x0080_0080) >> 8) & 0x00ff_00ff;
    let ag = ((((d >> 8) & 0x00ff_00ff) * inv + 0x0080_0080) >> 8) & 0x00ff_00ff;
    s.wrapping_add(rb | (ag << 8))
}

/// Pack straight-alpha RGBA bytes into premultiplied ARGB.
#[inline]
pub fn premul(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let a32 = a as u32;
    let m = |c: u8| (c as u32 * a32 + 127) / 255;
    (a32 << 24) | (m(r) << 16) | (m(g) << 8) | m(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_clips_and_flips() {
        let mut dst = Image::new(4, 1);
        let src = Image { width: 2, height: 1, px: vec![0xff00_0001, 0xff00_0002] };
        dst.blit_over(&src, 3, 0, false);
        assert_eq!(dst.px, vec![0, 0, 0, 0xff00_0001]);
        dst.clear();
        dst.blit_over(&src, 0, 0, true);
        assert_eq!(&dst.px[..2], &[0xff00_0002, 0xff00_0001]);
    }

    #[test]
    fn over_half_alpha() {
        let s = premul(255, 0, 0, 128);
        let d = premul(0, 0, 255, 255);
        let o = over(s, d);
        assert_eq!(o >> 24, 255);
        assert!(((o >> 16) & 0xff) >= 127 && (o & 0xff) >= 126);
    }
}
