//! Render hero effects placed around the knight (prefab offsets, baked rotation / scale),
//! facing left and right, to check placement offline.
//! cargo run -p skong-core --example fx_sheet --release -- <data> <skin dir> <out.png> <scale> "effect@Knight Clip[:frame][@effect frame]" ...
use std::path::Path;

use anyhow::{Context, Result};
use skong_core::anim::ClipLibrary;
use skong_core::assets::{Skin, SpriteBank};
use skong_core::canvas::Image;
use skong_core::effects::EffectSet;
use skong_core::geom::Vec2;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data, skin, out, scale, specs @ ..] = args.as_slice() else {
        anyhow::bail!("usage: <data> <skin> <out.png> <scale> effect@Clip[:frame][@fxframe]...");
    };
    let data = Path::new(data);
    let skin = Skin::from_dir(Path::new(skin)).context("not a skin dir")?;
    let bank = SpriteBank::load_metadata(&data.join("hk"))?;
    let lib = ClipLibrary::load(&data.join("hk/animations/Knight.json"), &bank)?;
    let fx = EffectSet::load(&data.join("effects.json"), &data.join("hk/animations"), &bank)?;
    let ppu = 64.0 * scale.parse::<f32>()?;
    let keys = lib.sprite_keys();
    let mut sets: Vec<(&[skong_core::assets::SpriteKey], f32)> = vec![(&keys, ppu)];
    let fx_keys: Vec<_> = (0..fx.defs.len()).map(|i| fx.sprite_keys(i)).collect();
    sets.extend(fx_keys.iter().enumerate().map(|(i, k)| (k.as_slice(), ppu * fx.defs[i].raster_scale())));
    let frames = bank.rasterize_sets(&skin, None, &sets)?;

    let cell = (12.0 * ppu) as i32;
    let mut sheet = Image::new((cell * 2) as u32, (cell * specs.len() as i32) as u32);
    for (i, p) in sheet.px.iter_mut().enumerate() {
        let (x, y) = (i as u32 % sheet.width / 8, i as u32 / sheet.width / 8);
        *p = if (x + y) % 2 == 0 { 0xff50_5058 } else { 0xff60_6068 };
    }
    for (row, spec) in specs.iter().enumerate() {
        let mut parts = spec.split('@');
        let name = parts.next().unwrap_or_default();
        let (clip, kframe) = parts.next().map_or(("Idle", 0), |c| c.split_once(':').map_or((c, 0), |(c, f)| (c, f.parse().unwrap_or(0))));
        let def_i = fx.id(name).with_context(|| format!("no effect {name}"))?;
        let def = &fx.defs[def_i];
        let (li, fclip) = fx.clips[def_i];
        let ffr = fx.libs[li].get(fclip).frames.len();
        let fframe = parts.next().and_then(|f| f.parse().ok()).unwrap_or(ffr / 2);
        let kid = lib.id(clip).with_context(|| format!("no clip {clip}"))?;
        for (col, right) in [(0, false), (1, true)] {
            let origin = Vec2::new((col * cell + cell / 2) as f32, (row as i32 * cell + cell / 2) as f32);
            if let Some(f) = lib.get(kid).frames.get(kframe).and_then(|f| f.sprite).and_then(|k| frames[0].get(&k)) {
                sheet.blit_over(&f.image, (origin.x - f.anchor_x(right)) as i32, (origin.y - f.anchor[1]) as i32, right);
            }
            let Some(k) = fx.libs[li].get(fclip).frames.get(fframe).and_then(|f| f.sprite) else { continue };
            let Some(raw) = frames[1 + def_i].get(&k) else { continue };
            let f = def.bake(raw);
            let at = def.place(origin, right, ppu);
            let flip = def.mirrored(right);
            sheet.blit_over(&f.image, (at.x - f.anchor_x(flip)) as i32, (at.y - f.anchor[1]) as i32, flip);
            sheet.px[(origin.y as u32 * sheet.width + origin.x as u32) as usize] = 0xffff_0000;
        }
    }
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(out)?), sheet.width, sheet.height);
    enc.set_color(png::ColorType::Rgba);
    let rgba: Vec<u8> = sheet.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(())
}
