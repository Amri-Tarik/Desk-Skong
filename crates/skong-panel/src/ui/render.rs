//! Headless rendering of the panel for layout checks: the real `Panel::ui` against a
//! made-up pet (never a real one: the link points at a pipe nobody serves), tessellated by
//! egui and drawn by a tiny software rasterizer. No window opens.
//!
//! The test always renders the Home tab at a few window sizes (so a layout that panics or
//! loops shows up in `cargo test`); with `SKONG_RENDER_DIR` set it also writes each frame
//! there as `home-<w>x<h>.png` to look at.
use std::collections::HashMap;

use eframe::egui::{self, Color32, Rect, TextureId, epaint};
use skong_ipc::{AchievementEntry, AchievementsInfo, Hello, StakesInfo, Status, UnlockedAchievement};

use super::Panel;
use crate::link::Link;

/// A connected-looking panel with a well-played journal.
fn panel() -> Panel {
    let link = Link::start(format!(r"\\.\pipe\desk-skong-render-nobody-{}", std::process::id()), || {});
    {
        let mut s = link.shared();
        s.connected = true;
        let named = |v: &[(&str, f32)]| v.iter().map(|(n, x)| (n.to_string(), *x)).collect::<Vec<_>>();
        s.hello = Some(Hello {
            version: "0.2.0".into(),
            pid: 1,
            skins: ["Vanilla", "Hornet-ish", "Pale King"].map(String::from).to_vec(),
            bench_styles: ["Classic", "Ornate"].map(String::from).to_vec(),
            scales: named(&[("Tiny", 0.5), ("Small", 0.75), ("Normal", 1.0), ("Large", 1.25), ("Huge", 1.5)]),
            bother_levels: named(&[("Off", 0.0), ("Low", 0.5), ("Normal", 1.0), ("High", 2.0)]),
            ..Default::default()
        });
        s.status = Some(Status {
            share_visible: false,
            lantern: true,
            text_curiosity: true,
            vision: true,
            scale: 1.0,
            bother: 1.0,
            benches_available: vec![true, true],
            stakes: Some(StakesInfo { geo: 6072, ..Default::default() }),
            played_secs: 8 * 3600 + 60,
            ..Default::default()
        });
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let entries = (0..40)
            .map(|i| {
                if i < 27 {
                    let unlocked = UnlockedAchievement { id: format!("a{i}"), title: format!("Achievement number {i}"), description: "Something the knight did, told in a line or two of journal prose.".into(), at: now - 600 - i as u64 * 3600 };
                    AchievementEntry { hint: None, unlocked: Some(unlocked) }
                } else {
                    AchievementEntry { hint: Some("a visitor".into()), unlocked: None }
                }
            })
            .collect();
        s.achievements = Some(AchievementsInfo { entries });
    }
    Panel::new(link, None)
}

/// Run the panel for a few frames at `w`×`h` points (1 px each) and rasterize the last.
fn render(w: usize, h: usize) -> Vec<Color32> {
    let ctx = egui::Context::default();
    super::style::apply(&ctx);
    let mut p = panel();
    let mut frame = eframe::Frame::_new_kittest();
    let mut textures: HashMap<TextureId, (usize, usize, Vec<Color32>)> = HashMap::new();
    let mut last = None;
    for i in 0..4 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))), time: Some(i as f64 * 0.1), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| eframe::App::ui(&mut p, ui, &mut frame));
        for (&id, delta) in out.textures_delta.set.iter().flat_map(|(id, ds)| ds.iter().map(move |d| (id, d))) {
            let egui::ImageData::Color(img) = &delta.image;
            let [iw, ih] = img.size;
            match delta.pos {
                None => {
                    textures.insert(id, (iw, ih, img.pixels.clone()));
                }
                Some([x0, y0]) => {
                    let t = textures.get_mut(&id).expect("patch of an unknown texture");
                    for y in 0..ih {
                        let row = (y0 + y) * t.0 + x0;
                        t.2[row..row + iw].copy_from_slice(&img.pixels[y * iw..(y + 1) * iw]);
                    }
                }
            }
        }
        out.textures_delta.clear();
        last = Some((out.shapes, out.pixels_per_point));
    }
    let (shapes, ppp) = last.expect("ran at least once");
    let mut px = vec![[13.0f32 / 255.0, 16.0 / 255.0, 21.0 / 255.0, 1.0]; w * h];
    for prim in ctx.tessellate(shapes, ppp) {
        let epaint::Primitive::Mesh(mesh) = prim.primitive else { continue };
        let Some(tex) = textures.get(&mesh.texture_id) else { continue };
        let clip = prim.clip_rect;
        let (cx0, cy0) = (clip.min.x.max(0.0) as usize, clip.min.y.max(0.0) as usize);
        let (cx1, cy1) = ((clip.max.x.ceil().max(0.0) as usize).min(w), (clip.max.y.ceil().max(0.0) as usize).min(h));
        for t in mesh.indices.chunks_exact(3) {
            let v = [mesh.vertices[t[0] as usize], mesh.vertices[t[1] as usize], mesh.vertices[t[2] as usize]];
            triangle(&mut px, w, (cx0, cy0, cx1, cy1), &v, tex);
        }
    }
    px.iter().map(|c| Color32::from_rgba_premultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255)).collect()
}

/// One textured, vertex-coloured triangle, premultiplied-alpha blended.
fn triangle(px: &mut [[f32; 4]], w: usize, (cx0, cy0, cx1, cy1): (usize, usize, usize, usize), v: &[epaint::Vertex; 3], tex: &(usize, usize, Vec<Color32>)) {
    let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < 1e-6 {
        return;
    }
    let x0 = (a.x.min(b.x).min(c.x).floor().max(0.0) as usize).max(cx0);
    let y0 = (a.y.min(b.y).min(c.y).floor().max(0.0) as usize).max(cy0);
    let x1 = (a.x.max(b.x).max(c.x).ceil().max(0.0) as usize).min(cx1);
    let y1 = (a.y.max(b.y).max(c.y).ceil().max(0.0) as usize).min(cy1);
    let rgba = |c: Color32| [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0];
    let cols = v.map(|v| rgba(v.color));
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b.x - p) * (c.y - q) - (b.y - q) * (c.x - p)) / area;
            let w1 = ((c.x - p) * (a.y - q) - (c.y - q) * (a.x - p)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let u = v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2;
            let t = v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2;
            let s = rgba(sample(tex, u, t));
            let dst = &mut px[y * w + x];
            let src: [f32; 4] = std::array::from_fn(|k| (cols[0][k] * w0 + cols[1][k] * w1 + cols[2][k] * w2) * s[k]);
            for k in 0..4 {
                dst[k] = src[k] + dst[k] * (1.0 - src[3]);
            }
        }
    }
}

/// Bilinear texture lookup (premultiplied texels).
fn sample((tw, th, texels): &(usize, usize, Vec<Color32>), u: f32, v: f32) -> Color32 {
    let (fx, fy) = (u * *tw as f32 - 0.5, v * *th as f32 - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (ax, ay) = (fx - x0, fy - y0);
    let at = |x: f32, y: f32| texels[(y.clamp(0.0, *th as f32 - 1.0) as usize) * tw + x.clamp(0.0, *tw as f32 - 1.0) as usize];
    let (p00, p10, p01, p11) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    let mix = |k: usize| {
        let ch = |c: Color32| c.to_array()[k] as f32;
        let top = ch(p00) * (1.0 - ax) + ch(p10) * ax;
        let bottom = ch(p01) * (1.0 - ax) + ch(p11) * ax;
        (top * (1.0 - ay) + bottom * ay).round() as u8
    };
    Color32::from_rgba_premultiplied(mix(0), mix(1), mix(2), mix(3))
}

fn write_png(path: &std::path::Path, w: usize, h: usize, px: &[Color32]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create png"));
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = px.iter().flat_map(|c| c.to_array()).collect();
    enc.write_header().and_then(|mut wr| wr.write_image_data(&data)).expect("write png");
}

#[test]
fn home_renders_at_every_size() {
    let dir = std::env::var_os("SKONG_RENDER_DIR").map(std::path::PathBuf::from);
    // The default size, the narrowest two-column one, the smallest; then tall ones that
    // show everything without scrolling (two columns, stacked).
    for (w, h) in [(1180, 820), (920, 700), (640, 520), (1180, 1400), (760, 1700)] {
        let px = render(w, h);
        assert!(px.iter().any(|c| c.r() > 200), "nothing bright drawn at {w}x{h}");
        if let Some(dir) = &dir {
            write_png(&dir.join(format!("home-{w}x{h}.png")), w, h, &px);
        }
    }
}
