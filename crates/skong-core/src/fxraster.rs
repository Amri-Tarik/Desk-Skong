//! Affine resampling of rasterized frames around their anchor: per-axis scale (negative =
//! mirror) followed by a rotation, as Unity applies a child transform (`R · S`). Used for
//! prefab effects that are rotated (Dash Burst going down, SD Charge Wall, Impact Lines) or
//! squashed (Q Trail, Q Pillar).
use crate::assets::Frame;
use crate::canvas::Image;

/// Supersampling per axis (output quality when a bake also shrinks an axis).
const SS: u32 = 2;

/// `f` scaled by (`kx`, `ky`) then rotated `rot_deg` counter-clockwise (y up), around its
/// anchor. The anchor of the result is the same point of the art.
pub fn transform_frame(f: &Frame, kx: f32, ky: f32, rot_deg: f32) -> Frame {
    let (w, h) = (f.image.width as f32, f.image.height as f32);
    if w < 1.0 || h < 1.0 || kx.abs() < 1e-6 || ky.abs() < 1e-6 {
        return f.clone();
    }
    let (s, c) = rot_deg.to_radians().sin_cos();
    let [ax, ay] = f.anchor;
    // Local y-up coordinates relative to the anchor.
    let fwd = |u: f32, v: f32| {
        let (u, v) = (u * kx, v * ky);
        (c * u - s * v, s * u + c * v)
    };
    let inv = |u: f32, v: f32| ((c * u + s * v) / kx, (-s * u + c * v) / ky);
    // Snap float noise (cos 90° != 0) so exact quarter turns keep exact sizes.
    let snap = |x: f32| if (x - x.round()).abs() < 1e-3 { x.round() } else { x };
    let corners = [fwd(-ax, ay), fwd(w - ax, ay), fwd(-ax, ay - h), fwd(w - ax, ay - h)].map(|(u, v)| (snap(u), snap(v)));
    let min_u = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor();
    let max_u = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil();
    let min_v = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor();
    let max_v = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil();
    let (ow, oh) = ((max_u - min_u).max(1.0) as u32, (max_v - min_v).max(1.0) as u32);
    let mut out = Image::new(ow, oh);
    let norm = 1.0 / (SS * SS) as f32;
    for j in 0..oh {
        for i in 0..ow {
            let mut acc = [0.0f32; 4];
            for sj in 0..SS {
                for si in 0..SS {
                    let u = min_u + i as f32 + (si as f32 + 0.5) / SS as f32;
                    let v = max_v - (j as f32 + (sj as f32 + 0.5) / SS as f32);
                    let (su, sv) = inv(u, v);
                    let p = bilinear(&f.image, ax + su - 0.5, ay - sv - 0.5);
                    for k in 0..4 {
                        acc[k] += p[k];
                    }
                }
            }
            let a = (acc[0] * norm).round().clamp(0.0, 255.0) as u32;
            if a == 0 {
                continue;
            }
            let ch = |x: f32| (x * norm).round().clamp(0.0, a as f32) as u32;
            out.px[(j * ow + i) as usize] = (a << 24) | (ch(acc[1]) << 16) | (ch(acc[2]) << 8) | ch(acc[3]);
        }
    }
    Frame { image: out, anchor: [-min_u, max_v] }
}

/// Premultiplied ARGB channels at a continuous pixel position (transparent outside).
fn bilinear(img: &Image, x: f32, y: f32) -> [f32; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let mut acc = [0.0f32; 4];
    for (dx, dy, wgt) in [(0.0, 0.0, (1.0 - fx) * (1.0 - fy)), (1.0, 0.0, fx * (1.0 - fy)), (0.0, 1.0, (1.0 - fx) * fy), (1.0, 1.0, fx * fy)] {
        let (px, py) = (x0 + dx, y0 + dy);
        if wgt <= 0.0 || px < 0.0 || py < 0.0 || px >= img.width as f32 || py >= img.height as f32 {
            continue;
        }
        let p = img.px[py as usize * img.width as usize + px as usize];
        for (k, sh) in [24u32, 16, 8, 0].into_iter().enumerate() {
            acc[k] += ((p >> sh) & 0xff) as f32 * wgt;
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x2 opaque bar with the anchor at its left end, mid-height.
    fn bar() -> Frame {
        let mut img = Image::new(4, 2);
        img.px.fill(0xffff_ffff);
        Frame { image: img, anchor: [0.0, 1.0] }
    }

    fn opaque(f: &Frame, x: i32, y: i32) -> bool {
        // Pixel at offset (x, y) from the anchor (y down).
        let (px, py) = (f.anchor[0] as i32 + x, f.anchor[1] as i32 + y);
        px >= 0 && py >= 0 && (px as u32) < f.image.width && (py as u32) < f.image.height && f.image.px[(py as u32 * f.image.width + px as u32) as usize] >> 24 > 128
    }

    #[test]
    fn rotating_90_turns_a_bar_upward() {
        let r = transform_frame(&bar(), 1.0, 1.0, 90.0);
        assert_eq!((r.image.width, r.image.height), (2, 4));
        // Counter-clockwise with y up: the bar now points up from the anchor.
        assert!(opaque(&r, 0, -3) && opaque(&r, -1, -1) && !opaque(&r, 0, 1));
    }

    #[test]
    fn negative_scale_mirrors_before_rotating() {
        // Mirror (bar points left), then rotate 90° CCW: points down.
        let r = transform_frame(&bar(), -1.0, 1.0, 90.0);
        assert!(opaque(&r, 0, 2) && !opaque(&r, 0, -3), "{:?}", r.anchor);
    }

    #[test]
    fn identity_and_squash() {
        let r = transform_frame(&bar(), 1.0, 1.0, 0.0);
        assert_eq!((r.image.width, r.image.height, r.anchor), (4, 2, [0.0, 1.0]));
        let r = transform_frame(&bar(), 0.5, 1.0, 0.0);
        assert_eq!((r.image.width, r.image.height), (2, 2));
    }
}
