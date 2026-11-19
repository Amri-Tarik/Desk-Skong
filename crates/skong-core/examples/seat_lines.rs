//! Print, for every frame of the given clips, the opaque bounds of the sprite relative to
//! its anchor (px at scale 1, y down) — to line up seated frames on one seat line.
//! cargo run -p skong-core --example seat_lines --release -- <data/hk> <skin dir> "Clip A" ...
use std::path::Path;

use anyhow::{Context, Result};
use skong_core::anim::ClipLibrary;
use skong_core::assets::{Skin, SpriteBank};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data, skin, clips @ ..] = args.as_slice() else {
        anyhow::bail!("usage: <data> <skin> clips...");
    };
    let data = Path::new(data);
    let skin = Skin::from_dir(Path::new(skin)).context("not a skin dir")?;
    let mut bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Knight.json"), &bank)?;
    bank.rasterize_skin(&skin, None, &lib.sprite_keys(), 64.0)?;
    for name in clips {
        let Some(id) = lib.id(name) else { eprintln!("no clip {name}"); continue };
        let mut seen = Vec::new();
        for f in &lib.get(id).frames {
            let Some(k) = f.sprite else { continue };
            if seen.contains(&k) {
                continue;
            }
            seen.push(k);
            let Some(fr) = bank.frame(k) else { continue };
            let (w, h) = (fr.image.width as usize, fr.image.height as usize);
            let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| fr.image.px[y * w + x] >> 24 > 128)).collect();
            let (top, bottom) = (rows.first().copied().unwrap_or(0), rows.last().copied().unwrap_or(0));
            // Widest row in the lower half ~ where the body meets the seat (cloak hem).
            let width = |y: usize| (0..w).filter(|&x| fr.image.px[y * w + x] >> 24 > 128).count();
            let hem = rows.iter().copied().filter(|&y| y > (top + bottom) / 2).max_by_key(|&y| (width(y), y)).unwrap_or(bottom);
            println!(
                "{name:24} {:20} top {:6.1} bottom {:6.1} hem {:6.1} seated bottom {:6.1}",
                bank.sprite_name(k).unwrap_or("?"),
                top as f32 - fr.anchor[1],
                bottom as f32 - fr.anchor[1],
                hem as f32 - fr.anchor[1],
                bottom as f32 - fr.anchor[1] + f.dy * 64.0
            );
        }
    }
    Ok(())
}
