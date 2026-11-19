//! Run content detection on a screenshot and report what would become terrain.
//!
//! ```text
//! cargo run -p skong-core --example vision_png --release -- shot.png [annotated.png]
//! ```
//!
//! Prints every detected surface (ledge) and content wall. With a second path, writes the
//! screenshot dimmed, with ledges drawn in magenta and walls in cyan.
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result};
use skong_core::assets::Atlas;
use skong_core::geom::RectI;
use skong_core::vision::{PixelView, SurfaceMap, VisionParams};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args.next().context("usage: vision_png <shot.png> [annotated.png]")?;
    let output = args.next();
    let img = Atlas::load_png(Path::new(&input))?;
    let (w, h) = (img.width as usize, img.height as usize);
    // RGBA bytes -> BGRA u32 (what desktop duplication delivers).
    let px: Vec<u32> = img.rgba.chunks_exact(4).map(|p| 0xff00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
    let view = PixelView::new(&px, w, h, w).context("view")?;
    let mut map = SurfaceMap::new(w, h, VisionParams::default());
    let start = std::time::Instant::now();
    map.scan(&view, &[RectI::new(0, 0, w as i32, h as i32)]);
    let mid = std::time::Instant::now();
    map.finish();
    let scan = start.elapsed();
    let finish = mid.elapsed();
    let surfaces = map.raw_surfaces().to_vec();
    let walls = map.raw_walls().to_vec();
    println!(
        "{w}x{h}: {} ledges, {} walls (full scan {:.2} ms, of which finish {:.2} ms)",
        surfaces.len(),
        walls.len(),
        scan.as_secs_f64() * 1e3,
        finish.as_secs_f64() * 1e3
    );
    for s in &surfaces {
        println!("  ledge y={:<4} x={}..{} ({} px)", s.y, s.x0, s.x1, s.len());
    }
    for v in &walls {
        println!("  wall  x={:<4} y={}..{} ({} px) facing {:?}", v.x, v.y0, v.y1, v.len(), v.face);
    }
    if let Some(out) = output {
        let mut rgba: Vec<u8> = img.rgba.chunks_exact(4).flat_map(|p| [p[0] / 3, p[1] / 3, p[2] / 3, 255]).collect();
        let mut put = |x: i32, y: i32, c: [u8; 4]| {
            if (0..w as i32).contains(&x) && (0..h as i32).contains(&y) {
                let i = (y as usize * w + x as usize) * 4;
                rgba[i..i + 4].copy_from_slice(&c);
            }
        };
        for s in &surfaces {
            for x in s.x0..s.x1 {
                put(x, s.y, [255, 0, 255, 255]);
            }
        }
        for v in &walls {
            for y in v.y0..v.y1 {
                put(v.x, y, [0, 255, 255, 255]);
            }
        }
        let f = BufWriter::new(File::create(&out).with_context(|| format!("create {out}"))?);
        let mut enc = png::Encoder::new(f, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&rgba)?;
        println!("annotated image written to {out}");
    }
    Ok(())
}
