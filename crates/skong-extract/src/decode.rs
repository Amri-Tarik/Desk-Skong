//! Texture formats Desk-Skong's atlases use, decoded to straight RGBA8 (rows as stored).
use anyhow::{Result, bail, ensure};

/// Bytes of one `w`×`h` level in `format`, or `None` if the format isn't supported.
pub fn level_size(format: i32, w: u32, h: u32) -> Option<usize> {
    let (w, h) = (w as usize, h as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    Some(match format {
        1 => w * h,              // Alpha8
        3 => w * h * 3,          // RGB24
        4 | 5 | 14 => w * h * 4, // RGBA32, ARGB32, BGRA32
        10 => blocks * 8,        // DXT1 / BC1
        12 | 25 => blocks * 16,  // DXT5 / BC3, BC7
        _ => return None,
    })
}

pub fn decode(format: i32, w: u32, h: u32, raw: &[u8]) -> Result<Vec<u8>> {
    let Some(size) = level_size(format, w, h) else { bail!("unsupported texture format {format}") };
    ensure!(raw.len() >= size, "pixel data too short ({} < {size} bytes)", raw.len());
    let raw = &raw[..size];
    Ok(match format {
        1 => raw.iter().flat_map(|&a| [255, 255, 255, a]).collect(),
        3 => raw.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        4 => raw.to_vec(),
        5 => raw.chunks_exact(4).flat_map(|p| [p[1], p[2], p[3], p[0]]).collect(),
        14 => raw.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect(),
        10 => blocks(w, h, raw, 8, bc1_block),
        12 => blocks(w, h, raw, 16, bc3_block),
        25 => blocks(w, h, raw, 16, crate::bc7::decode_block),
        _ => unreachable!(),
    })
}

/// Decode a block-compressed image; `f` writes one 4×4 block as 16 RGBA pixels.
fn blocks(w: u32, h: u32, raw: &[u8], block_bytes: usize, f: fn(&[u8], &mut [[u8; 4]; 16])) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let bw = w.div_ceil(4);
    let mut out = vec![0u8; w * h * 4];
    let mut px = [[0u8; 4]; 16];
    for (i, block) in raw.chunks_exact(block_bytes).enumerate() {
        let (bx, by) = (i % bw * 4, i / bw * 4);
        f(block, &mut px);
        for y in 0..4.min(h - by) {
            for x in 0..4.min(w - bx) {
                let o = ((by + y) * w + bx + x) * 4;
                out[o..o + 4].copy_from_slice(&px[y * 4 + x]);
            }
        }
    }
    out
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = (c >> 11 & 31) as u32;
    let g = (c >> 5 & 63) as u32;
    let b = (c & 31) as u32;
    [((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8]
}

/// BC1 colour endpoints → 4-colour palette; `opaque_only` (BC3's colour half) always
/// uses the 4-colour mode.
fn bc1_palette(b: &[u8], opaque_only: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
    let mut pal = [[p0[0], p0[1], p0[2], 255], [p1[0], p1[1], p1[2], 255], [0; 4], [0; 4]];
    if c0 > c1 || opaque_only {
        for (i, (wa, wb)) in [(2, 1), (1, 2)].into_iter().enumerate() {
            pal[2 + i] = [mix(p0[0], p1[0], wa, wb), mix(p0[1], p1[1], wa, wb), mix(p0[2], p1[2], wa, wb), 255];
        }
    } else {
        pal[2] = [mix(p0[0], p1[0], 1, 1), mix(p0[1], p1[1], 1, 1), mix(p0[2], p1[2], 1, 1), 255];
        pal[3] = [0, 0, 0, 0];
    }
    pal
}

fn bc1_colors(b: &[u8], opaque_only: bool, px: &mut [[u8; 4]; 16]) {
    let pal = bc1_palette(b, opaque_only);
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (i, p) in px.iter_mut().enumerate() {
        *p = pal[(idx >> (2 * i) & 3) as usize];
    }
}

fn bc1_block(b: &[u8], px: &mut [[u8; 4]; 16]) {
    bc1_colors(b, false, px);
}

fn bc3_block(b: &[u8], px: &mut [[u8; 4]; 16]) {
    bc1_colors(&b[8..], true, px);
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut alpha = [a0, a1, 0, 0, 0, 0, 0, 0];
    if a0 > a1 {
        for i in 1..7 {
            alpha[i + 1] = ((7 - i) as u32 * a0 + i as u32 * a1) / 7;
        }
    } else {
        for i in 1..5 {
            alpha[i + 1] = ((5 - i) as u32 * a0 + i as u32 * a1) / 5;
        }
        alpha[6] = 0;
        alpha[7] = 255;
    }
    let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
    for (i, p) in px.iter_mut().enumerate() {
        p[3] = alpha[(bits >> (3 * i) & 7) as usize] as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncompressed_formats_reorder_channels() {
        assert_eq!(decode(5, 1, 1, &[9, 1, 2, 3]).unwrap(), vec![1, 2, 3, 9]); // ARGB
        assert_eq!(decode(14, 1, 1, &[3, 2, 1, 9]).unwrap(), vec![1, 2, 3, 9]); // BGRA
        assert_eq!(decode(3, 1, 1, &[1, 2, 3]).unwrap(), vec![1, 2, 3, 255]);
        assert_eq!(decode(1, 1, 1, &[7]).unwrap(), vec![255, 255, 255, 7]);
        assert!(decode(4, 2, 2, &[0; 15]).is_err(), "short data is an error");
        assert!(decode(29, 4, 4, &[0; 64]).is_err(), "crunched isn't supported");
    }

    #[test]
    fn dxt5_block_decodes_endpoints_and_alpha() {
        // Alpha 255 → 0 (8-step mode), all indices 0 (alpha0) except pixel 1 = index 1.
        // Colour: c0 = pure red (0xF800), c1 = black, indices: pixel 0 → c0, pixel 1 → c1.
        let mut b = [0u8; 16];
        b[0] = 255;
        b[1] = 0;
        b[2] = 0b0000_1000; // pixel 0 → 0, pixel 1 → 1
        b[8..10].copy_from_slice(&0xF800u16.to_le_bytes());
        b[12] = 0b0000_0100; // pixel 0 → c0, pixel 1 → c1
        let px = decode(12, 4, 4, &b).unwrap();
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(&px[4..8], &[0, 0, 0, 0]);
        // Non-multiple-of-4 sizes crop the block.
        assert_eq!(decode(12, 3, 2, &b).unwrap().len(), 3 * 2 * 4);
    }

    #[test]
    fn dxt1_three_colour_mode_has_transparent_black() {
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&0x0000u16.to_le_bytes()); // c0 <= c1
        b[2..4].copy_from_slice(&0xFFFFu16.to_le_bytes());
        b[4] = 0b11; // pixel 0 → index 3
        let px = decode(10, 4, 4, &b).unwrap();
        assert_eq!(&px[0..4], &[0, 0, 0, 0]);
        assert_eq!(&px[4..8], &[0, 0, 0, 255]);
    }
}
