//! BC7 block decoder (the D3D11 / Khronos BPTC spec), RGBA8 out.

/// Per mode: subsets, partition bits, rotation bits, index-selection bits, colour bits,
/// alpha bits, per-endpoint P-bits, shared P-bits, index bits, secondary index bits.
const MODES: [[u8; 10]; 8] = [
    [3, 4, 0, 0, 4, 0, 1, 0, 3, 0],
    [2, 6, 0, 0, 6, 0, 0, 1, 3, 0],
    [3, 6, 0, 0, 5, 0, 0, 0, 2, 0],
    [2, 6, 0, 0, 7, 0, 1, 0, 2, 0],
    [1, 0, 2, 1, 5, 6, 0, 0, 2, 3],
    [1, 0, 2, 0, 7, 8, 0, 0, 2, 2],
    [1, 0, 0, 0, 7, 7, 1, 0, 4, 0],
    [2, 6, 0, 0, 5, 5, 1, 0, 2, 0],
];

/// Two-subset partitions: bit `i` is pixel `i`'s subset.
const P2: [u16; 64] = [
    0xCCCC, 0x8888, 0xEEEE, 0xECC8, 0xC880, 0xFEEC, 0xFEC8, 0xEC80, 0xC800, 0xFFEC, 0xFE80, 0xE800, 0xFFE8, 0xFF00, 0xFFF0, 0xF000, //
    0xF710, 0x008E, 0x7100, 0x08CE, 0x008C, 0x7310, 0x3100, 0x8CCE, 0x088C, 0x3110, 0x6666, 0x366C, 0x17E8, 0x0FF0, 0x718E, 0x399C, //
    0xAAAA, 0xF0F0, 0x5A5A, 0x33CC, 0x3C3C, 0x55AA, 0x9696, 0xA55A, 0x73CE, 0x13C8, 0x324C, 0x3BDC, 0x6996, 0xC33C, 0x9966, 0x0660, //
    0x0272, 0x04E4, 0x4E40, 0x2720, 0xC936, 0x936C, 0x39C6, 0x639C, 0x9336, 0x9CC6, 0x817E, 0xE718, 0xCCF0, 0x0FCC, 0x7744, 0xEE22,
];

/// Three-subset partitions: two bits per pixel, pixel 0 in the low bits.
const P3: [[u8; 16]; 64] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 1, 2, 2, 2, 2],
    [0, 0, 0, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 2, 1],
    [0, 0, 0, 0, 2, 0, 0, 1, 2, 2, 1, 1, 2, 2, 1, 1],
    [0, 2, 2, 2, 0, 0, 2, 2, 0, 0, 1, 1, 0, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2],
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 2, 2],
    [0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2],
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2],
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2],
    [0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2],
    [0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2],
    [0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 2, 1, 2, 2, 2],
    [0, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 0, 2, 2, 2, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 2],
    [0, 1, 1, 1, 0, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 0],
    [0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2],
    [0, 0, 2, 2, 0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 1],
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 2, 2, 2, 0, 2, 2, 2],
    [0, 0, 0, 1, 0, 0, 0, 1, 2, 2, 2, 1, 2, 2, 2, 1],
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 2, 2, 0, 1, 2, 2],
    [0, 0, 0, 0, 1, 1, 0, 0, 2, 2, 1, 0, 2, 2, 1, 0],
    [0, 1, 2, 2, 0, 1, 2, 2, 0, 0, 1, 1, 0, 0, 0, 0],
    [0, 0, 1, 2, 0, 0, 1, 2, 1, 1, 2, 2, 2, 2, 2, 2],
    [0, 1, 1, 0, 1, 2, 2, 1, 1, 2, 2, 1, 0, 1, 1, 0],
    [0, 0, 0, 0, 0, 1, 1, 0, 1, 2, 2, 1, 1, 2, 2, 1],
    [0, 0, 2, 2, 1, 1, 0, 2, 1, 1, 0, 2, 0, 0, 2, 2],
    [0, 1, 1, 0, 0, 1, 1, 0, 2, 0, 0, 2, 2, 2, 2, 2],
    [0, 0, 1, 1, 0, 1, 2, 2, 0, 1, 2, 2, 0, 0, 1, 1],
    [0, 0, 0, 0, 2, 0, 0, 0, 2, 2, 1, 1, 2, 2, 2, 1],
    [0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 2, 2, 2],
    [0, 2, 2, 2, 0, 0, 2, 2, 0, 0, 1, 2, 0, 0, 1, 1],
    [0, 0, 1, 1, 0, 0, 1, 2, 0, 0, 2, 2, 0, 2, 2, 2],
    [0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0],
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0],
    [0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0],
    [0, 1, 2, 0, 2, 0, 1, 2, 1, 2, 0, 1, 0, 1, 2, 0],
    [0, 0, 1, 1, 2, 2, 0, 0, 1, 1, 2, 2, 0, 0, 1, 1],
    [0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 2, 1, 2, 1, 2, 1],
    [0, 0, 2, 2, 1, 1, 2, 2, 0, 0, 2, 2, 1, 1, 2, 2],
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 1, 1],
    [0, 2, 2, 0, 1, 2, 2, 1, 0, 2, 2, 0, 1, 2, 2, 1],
    [0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 0, 1, 0, 1],
    [0, 0, 0, 0, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2],
    [0, 2, 2, 2, 0, 1, 1, 1, 0, 2, 2, 2, 0, 1, 1, 1],
    [0, 0, 0, 2, 1, 1, 1, 2, 0, 0, 0, 2, 1, 1, 1, 2],
    [0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2],
    [0, 2, 2, 2, 0, 1, 1, 1, 0, 1, 1, 1, 0, 2, 2, 2],
    [0, 0, 0, 2, 1, 1, 1, 2, 1, 1, 1, 2, 0, 0, 0, 2],
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 2, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 1, 2],
    [0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2],
    [0, 0, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2, 0, 0, 2, 2],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2],
    [0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1],
    [0, 2, 2, 2, 1, 2, 2, 2, 0, 2, 2, 2, 1, 2, 2, 2],
    [0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 1, 1, 1, 2, 0, 1, 1, 2, 2, 0, 1, 2, 2, 2, 0],
];

/// Anchor pixel of subset 1 in two-subset partitions.
const A2: [u8; 64] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, //
    15, 2, 8, 2, 2, 8, 8, 15, 2, 8, 2, 2, 8, 8, 2, 2, //
    15, 15, 6, 8, 2, 8, 15, 15, 2, 8, 2, 2, 2, 15, 15, 6, //
    6, 2, 6, 8, 15, 15, 2, 2, 15, 15, 15, 15, 15, 2, 2, 15,
];

/// Anchor pixels of subsets 1 and 2 in three-subset partitions.
const A3A: [u8; 64] = [
    3, 3, 15, 15, 8, 3, 15, 15, 8, 8, 6, 6, 6, 5, 3, 3, //
    3, 3, 8, 15, 3, 3, 6, 10, 5, 8, 8, 6, 8, 5, 15, 15, //
    8, 15, 3, 5, 6, 10, 8, 15, 15, 3, 15, 5, 15, 15, 15, 15, //
    3, 15, 5, 5, 5, 8, 5, 10, 5, 10, 8, 13, 15, 12, 3, 3,
];
const A3B: [u8; 64] = [
    15, 8, 8, 3, 15, 15, 3, 8, 15, 15, 15, 15, 15, 15, 15, 8, //
    15, 8, 15, 3, 15, 8, 15, 8, 3, 15, 6, 10, 15, 15, 10, 8, //
    15, 3, 15, 10, 10, 8, 9, 10, 6, 15, 8, 15, 3, 6, 6, 8, //
    15, 3, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 3, 15, 15, 8,
];

const W2: [u32; 4] = [0, 21, 43, 64];
const W3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const W4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

struct Bits {
    v: u128,
}

impl Bits {
    fn take(&mut self, n: u8) -> u32 {
        let r = (self.v & ((1u128 << n) - 1)) as u32;
        self.v >>= n;
        r
    }
}

fn subset_of(subsets: u8, partition: usize, pixel: usize) -> usize {
    match subsets {
        2 => (P2[partition] >> pixel & 1) as usize,
        3 => P3[partition][pixel] as usize,
        _ => 0,
    }
}

fn is_anchor(subsets: u8, partition: usize, pixel: usize) -> bool {
    pixel == 0
        || match subsets {
            2 => pixel == A2[partition] as usize,
            3 => pixel == A3A[partition] as usize || pixel == A3B[partition] as usize,
            _ => false,
        }
}

fn interp(e0: u8, e1: u8, w: u32) -> u8 {
    (((64 - w) * e0 as u32 + w * e1 as u32 + 32) >> 6) as u8
}

fn weights(bits: u8) -> &'static [u32] {
    match bits {
        2 => &W2,
        3 => &W3,
        _ => &W4,
    }
}

pub fn decode_block(block: &[u8], px: &mut [[u8; 4]; 16]) {
    let mut b = Bits { v: u128::from_le_bytes(block[..16].try_into().expect("16-byte block")) };
    let Some(mode) = (0..8).find(|&m| b.v >> m & 1 == 1) else {
        *px = [[0; 4]; 16]; // reserved mode: transparent black
        return;
    };
    b.take(mode as u8 + 1);
    let [ns, pb, rb, isb, cb, ab, epb, spb, ib, ib2] = MODES[mode];
    let partition = b.take(pb) as usize;
    let rotation = b.take(rb);
    let index_sel = b.take(isb);

    // Endpoints: [subset][end][channel], channel-major in the stream.
    let mut ep = [[[0u8; 4]; 2]; 3];
    let n = ns as usize;
    for c in 0..3 {
        for e in ep.iter_mut().take(n) {
            for end in e.iter_mut() {
                end[c] = b.take(cb) as u8;
            }
        }
    }
    if ab > 0 {
        for e in ep.iter_mut().take(n) {
            for end in e.iter_mut() {
                end[3] = b.take(ab) as u8;
            }
        }
    }
    let (mut cbits, mut abits) = (cb, ab);
    if epb == 1 || spb == 1 {
        let mut p = [[0u8; 2]; 3];
        for s in p.iter_mut().take(n) {
            if epb == 1 {
                s[0] = b.take(1) as u8;
                s[1] = b.take(1) as u8;
            } else {
                let v = b.take(1) as u8;
                *s = [v, v];
            }
        }
        for (s, e) in ep.iter_mut().take(n).enumerate() {
            for (k, end) in e.iter_mut().enumerate() {
                for (c, v) in end.iter_mut().enumerate() {
                    if c < 3 || ab > 0 {
                        *v = *v << 1 | p[s][k];
                    }
                }
            }
        }
        cbits += 1;
        if ab > 0 {
            abits += 1;
        }
    }
    let expand = |v: u8, bits: u8| -> u8 {
        let v = (v as u32) << (8 - bits);
        (v | v >> bits) as u8
    };
    for e in ep.iter_mut().take(n) {
        for end in e.iter_mut() {
            for (c, v) in end.iter_mut().enumerate() {
                *v = if c < 3 {
                    expand(*v, cbits)
                } else if ab > 0 {
                    expand(*v, abits)
                } else {
                    255
                };
            }
        }
    }

    let mut idx = [0u8; 16];
    for (i, x) in idx.iter_mut().enumerate() {
        *x = b.take(if is_anchor(ns, partition, i) { ib - 1 } else { ib }) as u8;
    }
    let mut idx2 = [0u8; 16];
    if ib2 > 0 {
        for (i, x) in idx2.iter_mut().enumerate() {
            *x = b.take(if i == 0 { ib2 - 1 } else { ib2 }) as u8;
        }
    }

    for (i, p) in px.iter_mut().enumerate() {
        let s = subset_of(ns, partition, i);
        let [e0, e1] = ep[s];
        let (ci, cw, ai, aw) = if ib2 == 0 {
            (idx[i], weights(ib), idx[i], weights(ib))
        } else if index_sel == 0 {
            (idx[i], weights(ib), idx2[i], weights(ib2))
        } else {
            (idx2[i], weights(ib2), idx[i], weights(ib))
        };
        for c in 0..3 {
            p[c] = interp(e0[c], e1[c], cw[ci as usize]);
        }
        p[3] = interp(e0[3], e1[3], aw[ai as usize]);
        match rotation {
            1 => p.swap(0, 3),
            2 => p.swap(1, 3),
            3 => p.swap(2, 3),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_tables_agree_with_their_anchors() {
        for p in 0..64 {
            assert_eq!(subset_of(2, p, 0), 0, "2-subset partition {p}: pixel 0 is subset 0");
            assert_eq!(subset_of(2, p, A2[p] as usize), 1, "2-subset partition {p}: anchor");
            assert_eq!(subset_of(3, p, 0), 0, "3-subset partition {p}: pixel 0 is subset 0");
            assert_eq!(subset_of(3, p, A3A[p] as usize), 1, "3-subset partition {p}: anchor 1");
            assert_eq!(subset_of(3, p, A3B[p] as usize), 2, "3-subset partition {p}: anchor 2");
        }
    }

    #[test]
    fn mode6_solid_block() {
        // Mode 6 (bit 6 set): RGBA 7 bits + P-bit per endpoint, 4-bit indices, all zero
        // → every pixel is endpoint 0. Endpoint 0 = 0x7F in every channel, P-bit 1 → 255.
        let mut v: u128 = 1 << 6;
        let mut pos = 7;
        let mut put = |val: u128, bits: u32| {
            v |= val << pos;
            pos += bits;
        };
        for _ in 0..4 {
            put(0x7F, 7); // endpoint 0
            put(0, 7); // endpoint 1
        }
        put(1, 1);
        put(0, 1);
        let mut px = [[0u8; 4]; 16];
        decode_block(&v.to_le_bytes(), &mut px);
        assert!(px.iter().all(|p| *p == [255, 255, 255, 255]), "{px:?}");
    }

    #[test]
    fn reserved_mode_is_transparent() {
        let mut px = [[9u8; 4]; 16];
        decode_block(&[0u8; 16], &mut px);
        assert!(px.iter().all(|p| *p == [0; 4]));
    }
}
