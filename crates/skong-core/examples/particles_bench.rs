//! Cost of the procedural particles (step + draw per frame) and an optional preview sheet.
//! cargo run -p skong-core --example particles_bench --release -- [out.png] [scale]
use std::time::Instant;

use anyhow::Result;
use skong_core::canvas::{Image, over};
use skong_core::geom::Vec2;
use skong_core::particles::{self as pt, Burst, Spec};

const DT: f32 = 1.0 / 120.0;

fn presets() -> [(&'static str, Spec); 8] {
    [
        ("land", pt::LAND_DUST),
        ("hard land", pt::HARD_LAND_DUST),
        ("jump", pt::JUMP_DUST),
        ("dash", pt::DASH_DUST),
        ("wall", pt::WALL_DUST),
        ("sd burst", pt::SD_BURST_SHARDS),
        ("sd break", pt::SD_BREAK_SHARDS),
        ("geo", pt::GEO_SPARKLE),
    ]
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scale: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.6);
    let ppu = 64.0 * scale;

    // Worst case the app allows: 8 bursts alive (the hardest-hitting presets, re-fired as
    // they die) plus the two streams fed every tick; draw every other tick (60 fps).
    let mut img = Image::default();
    let frames = 120 * 20;
    let mut bursts: Vec<Burst> = Vec::new();
    let (mut step_ns, mut draw_ns, mut parts, mut px) = (0u128, 0u128, 0usize, 0usize);
    let mut run = Burst::new(pt::RUN_DUST, ppu, 1);
    let mut charge = Burst::new(pt::SD_CHARGE_SHARDS, ppu, 2);
    let mut seed = 10;
    for f in 0..frames {
        while bursts.len() < 6 {
            let (_, spec) = presets()[seed as usize % 8];
            let mut b = Burst::new(spec, ppu, seed);
            b.emit(Vec2::new(500.0 + seed as f32, 400.0), if seed % 2 == 0 { 1.0 } else { -1.0 });
            bursts.push(b);
            seed += 1;
        }
        let t0 = Instant::now();
        run.feed(DT, Vec2::new(300.0 + f as f32 * 2.7, 400.0), 1.0);
        charge.feed(DT, Vec2::new(800.0, 380.0), 1.0);
        for b in bursts.iter_mut().chain([&mut run, &mut charge]) {
            b.step(DT);
        }
        bursts.retain(|b| !b.is_empty());
        step_ns += t0.elapsed().as_nanos();
        if f % 2 == 0 {
            let t1 = Instant::now();
            for b in bursts.iter().chain([&run, &charge]) {
                parts += b.len();
                if b.draw(&mut img).is_some() {
                    px += img.px.len();
                }
            }
            draw_ns += t1.elapsed().as_nanos();
        }
    }
    let ticks = frames as f64;
    println!("scale {scale}: worst case 8 bursts (6 one-shots + run dust + crystal charge)");
    println!("  step  {:.2} us / tick (120 Hz)", step_ns as f64 / ticks / 1e3);
    println!("  draw  {:.2} us / drawn frame (60 Hz), {:.1} particles, {:.0} px of window per frame", draw_ns as f64 / (ticks / 2.0) / 1e3, parts as f64 / (ticks / 2.0), px as f64 / (ticks / 2.0));
    let per_sec = (step_ns as f64 + draw_ns as f64) / (frames as f64 / 120.0);
    println!("  => {:.3} ms of CPU per second ({:.3} % of one core)", per_sec / 1e6, per_sec / 1e9 * 100.0);

    if let Some(out) = args.first() {
        sheet(out, ppu)?;
    }
    Ok(())
}

/// One row per preset: the burst at a few moments of its life, on a dark checker.
fn sheet(out: &str, ppu: f32) -> Result<()> {
    let times = [0.03, 0.08, 0.15, 0.25, 0.35, 0.5];
    let cell = (4.0 * ppu) as i32;
    let rows = presets().len() + 2;
    let mut sheet = Image::new((cell * times.len() as i32) as u32, (cell * rows as i32) as u32);
    for (i, p) in sheet.px.iter_mut().enumerate() {
        let (x, y) = (i as u32 % sheet.width / 8, i as u32 / sheet.width / 8);
        *p = if (x + y) % 2 == 0 { 0xff28_2830 } else { 0xff30_3038 };
    }
    let mut img = Image::default();
    let mut put = |row: i32, col: i32, b: &Burst, sheet: &mut Image| {
        if let Some((x, y)) = b.draw(&mut img) {
            // Burst origin at (2/3 down, centre) of the cell: (0, 0) in burst space.
            let (ox, oy) = (col * cell + cell / 2, row * cell + cell * 2 / 3);
            for yy in 0..img.height as i32 {
                for xx in 0..img.width as i32 {
                    let (sx, sy) = (ox + x + xx, oy + y + yy);
                    if sx >= col * cell && sx < (col + 1) * cell && sy >= row * cell && sy < (row + 1) * cell {
                        let d = &mut sheet.px[(sy as u32 * sheet.width + sx as u32) as usize];
                        *d = over(img.px[(yy as u32 * img.width + xx as u32) as usize], *d);
                    }
                }
            }
        }
    };
    let all: Vec<(Spec, bool)> = presets().iter().map(|p| (p.1, false)).chain([(pt::RUN_DUST, true), (pt::SD_CHARGE_SHARDS, true)]).collect();
    for (row, (spec, stream)) in all.into_iter().enumerate() {
        for (col, &t) in times.iter().enumerate() {
            let mut b = Burst::new(spec, ppu, 7);
            let mut x = 0.0;
            b.emit(Vec2::ZERO, 1.0);
            let mut age = 0.0;
            while age < t {
                if stream {
                    // Running right at run speed (the stream follows the feet).
                    let v = if spec.shape == pt::Shape::Dust { 8.3 * ppu } else { 0.0 };
                    x += v * DT;
                    b.feed(DT, Vec2::new(x - v * t, 0.0), 1.0);
                }
                b.step(DT);
                age += DT;
            }
            put(row as i32, col as i32, &b, &mut sheet);
        }
    }
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(out)?), sheet.width, sheet.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    let rgba: Vec<u8> = sheet
        .px
        .iter()
        .flat_map(|&p| {
            let a = p >> 24;
            let un = |c: u32| (c * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
            [un((p >> 16) & 0xff), un((p >> 8) & 0xff), un(p & 0xff), a as u8]
        })
        .collect();
    w.write_image_data(&rgba)?;
    println!("wrote {out}");
    Ok(())
}
