//! Render clips of a skin into one PNG contact sheet (one row per clip).
//! cargo run -p skong-core --example contact_sheet --release -- <data/hk> <skin dir> <out.png> <scale> "Clip A" "Clip B" ...
use std::path::Path;

use anyhow::{Context, Result};
use skong_core::anim::ClipLibrary;
use skong_core::assets::{SpriteBank, Skin};
use skong_core::canvas::Image;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data, skin, out, scale, clips @ ..] = args.as_slice() else {
        anyhow::bail!("usage: <data> <skin> <out.png> <scale> clips...");
    };
    let data = Path::new(data);
    let skin = Skin::from_dir(Path::new(skin)).context("not a skin dir")?;
    let mut bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Knight.json"), &bank)?;
    let ppu = 64.0 * scale.parse::<f32>()?;
    let t = std::time::Instant::now();
    bank.rasterize_skin(&skin, None, &lib.sprite_keys(), ppu)?;
    eprintln!("rasterized all Knight clip frames in {:?}, {} KiB", t.elapsed(), bank.frame_bytes() / 1024);

    let cell = (3.2 * ppu) as i32;
    let cols = clips.iter().filter_map(|c| lib.id(c)).map(|id| lib.get(id).frames.len()).max().unwrap_or(1) as i32;
    let mut sheet = Image::new((cols * cell) as u32, (clips.len() as i32 * cell) as u32);
    // checker bg so alpha is visible
    for (i, p) in sheet.px.iter_mut().enumerate() {
        let (x, y) = (i as u32 % sheet.width / 8, i as u32 / sheet.width / 8);
        *p = if (x + y) % 2 == 0 { 0xff50_5058 } else { 0xff60_6068 };
    }
    for (row, name) in clips.iter().enumerate() {
        let Some(id) = lib.id(name) else { eprintln!("no clip {name}"); continue };
        for (col, f) in lib.get(id).frames.iter().enumerate() {
            let Some(fr) = f.sprite.and_then(|k| bank.frame(k)) else { continue };
            // anchor at cell (center x, 70% down)
            let ax = col as i32 * cell + cell / 2;
            let ay = row as i32 * cell + cell * 7 / 10;
            // Seat-line corrections (seated clips) applied, as when the knight sits.
            sheet.blit_over(&fr.image, ax - fr.anchor[0] as i32, ay - fr.anchor[1] as i32 + (f.dy * ppu).round() as i32, false);
            sheet.px[(ay * sheet.width as i32 + ax) as usize] = 0xffff_0000;
        }
    }
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(out)?), sheet.width, sheet.height);
    enc.set_color(png::ColorType::Rgba);
    let rgba: Vec<u8> = sheet.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(())
}
