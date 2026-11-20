//! Placeholder README GIFs, rendered offline from the sprite clips on a drawn desktop (no
//! screen capture). Needs the extracted game art, and its output is game art: write it to
//! a folder that is never committed.
//! cargo run -p skong-core --example readme_gifs --release -- <data/hk> <Vanilla dir> <out dir>
use std::path::Path;

use anyhow::{Context, Result};
use skong_core::anim::{ClipLibrary, WrapMode};
use skong_core::assets::{Skin, SpriteBank};
use skong_core::canvas::Image;

const PPU: f32 = 64.0;
const FPS: f32 = 25.0;

struct Libs {
    bank: SpriteBank,
    /// (name, library, sprite origin above the feet: the bottom of its standing frame).
    libs: Vec<(String, ClipLibrary, f32)>,
}

impl Libs {
    fn lib(&self, name: &str) -> &ClipLibrary {
        &self.libs.iter().find(|(n, _, _)| n == name).expect(name).1
    }

    /// Draws `clip` of `lib` at `t` seconds into it, feet at (x, y). Art faces left;
    /// `right` mirrors it.
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, img: &mut Image, lib: &str, clip: &str, t: f32, x: f32, y: f32, right: bool) {
        let l = self.lib(lib);
        let Some(id) = l.id(clip) else { panic!("no clip {clip} in {lib}") };
        let c = l.get(id);
        let n = c.frames.len().max(1);
        let i = (t.max(0.0) * c.fps) as usize;
        let i = match c.wrap {
            WrapMode::Loop | WrapMode::RandomLoop | WrapMode::PingPong => i % n,
            WrapMode::LoopSection if i >= n => c.loop_start + (i - n) % (n - c.loop_start).max(1),
            _ => i.min(n - 1),
        };
        let Some(fr) = c.frames[i].sprite.and_then(|k| self.bank.frame(k)) else { return };
        let ax = if right { fr.image.width as f32 - fr.anchor[0] } else { fr.anchor[0] };
        let feet = self.libs.iter().find(|(n, _, _)| n == lib).map_or(0.0, |e| e.2);
        img.blit_over(&fr.image, (x - ax).round() as i32, (y - feet - fr.anchor[1]).round() as i32, right);
    }
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

fn rect(img: &mut Image, x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
    for y in y0.max(0)..y1.min(img.height as i32) {
        for x in x0.max(0)..x1.min(img.width as i32) {
            img.px[(y as u32 * img.width + x as u32) as usize] = c;
        }
    }
}

/// A dusky desktop with a few windows: (x0, top, x1, bottom).
fn desktop(w: u32, h: u32, windows: &[(i32, i32, i32, i32)]) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        let k = y as f32 / h as f32;
        let c = rgb((18.0 + 14.0 * k) as u8, (22.0 + 16.0 * k) as u8, (38.0 + 22.0 * k) as u8);
        rect(&mut img, 0, y as i32, w as i32, y as i32 + 1, c);
    }
    for &(x0, top, x1, bottom) in windows {
        rect(&mut img, x0, top, x1, bottom, rgb(46, 52, 68));
        rect(&mut img, x0, top, x1, top + 18, rgb(70, 78, 98));
        rect(&mut img, x0, top, x1, top + 1, rgb(120, 128, 150));
        for (i, c) in [rgb(200, 90, 80), rgb(200, 170, 80), rgb(90, 170, 100)].into_iter().enumerate() {
            let cx = x1 - 14 - 14 * i as i32;
            rect(&mut img, cx - 4, top + 5, cx + 4, top + 13, c);
        }
        for line in 0..((bottom - top - 30) / 14).max(0) {
            let len = (x1 - x0 - 40) * (3 + (line * 7) % 5) / 8;
            rect(&mut img, x0 + 16, top + 30 + line * 14, x0 + 16 + len, top + 34 + line * 14, rgb(64, 72, 92));
        }
    }
    rect(&mut img, 0, h as i32 - 22, w as i32, h as i32, rgb(28, 32, 44));
    img
}

fn save(path: &Path, frames: &[Image]) -> Result<()> {
    let (w, h) = (frames[0].width as u16, frames[0].height as u16);
    let mut enc = gif::Encoder::new(std::fs::File::create(path)?, w, h, &[])?;
    enc.set_repeat(gif::Repeat::Infinite)?;
    for img in frames {
        let mut rgba: Vec<u8> = img.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
        let mut f = gif::Frame::from_rgba_speed(w, h, &mut rgba, 10);
        f.delay = (100.0 / FPS) as u16;
        enc.write_frame(&f)?;
    }
    eprintln!("wrote {} ({} frames)", path.display(), frames.len());
    if std::env::var_os("STRIP").is_some() {
        strip(&path.with_extension("strip.png"), frames)?;
    }
    Ok(())
}

/// Every 12th frame in a grid of 3 columns, to check a scene at a glance (STRIP=1).
fn strip(path: &Path, frames: &[Image]) -> Result<()> {
    let picks: Vec<&Image> = frames.iter().step_by(12).collect();
    let (w, h) = (frames[0].width, frames[0].height);
    let rows = picks.len().div_ceil(3) as u32;
    let mut sheet = Image::new(w * 3, h * rows);
    for (i, f) in picks.iter().enumerate() {
        sheet.blit_over(f, (i as u32 % 3 * w) as i32, (i as u32 / 3 * h) as i32, false);
    }
    let mut enc = png::Encoder::new(std::fs::File::create(path)?, sheet.width, sheet.height);
    enc.set_color(png::ColorType::Rgba);
    let rgba: Vec<u8> = sheet.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(())
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k.clamp(0.0, 1.0)
}

/// He runs along a window top, jumps up to a higher one, dashes across it and stops.
fn hero(l: &Libs) -> Vec<Image> {
    let wins = [(20, 250, 330, 340), (300, 180, 700, 320)];
    let (low, high) = (250.0, 180.0);
    let mut out = vec![];
    for i in 0..(5.2 * FPS) as usize {
        let t = i as f32 / FPS;
        let mut img = desktop(720, 360, &wins);
        let k = "Knight";
        match t {
            t if t < 1.6 => l.draw(&mut img, k, "Run", t, lerp(50.0, 255.0, t / 1.6), low, true),
            t if t < 2.15 => {
                let u = (t - 1.6) / 0.55;
                let y = lerp(low, high, u) - 60.0 * (std::f32::consts::PI * u).sin();
                l.draw(&mut img, k, "Airborne", t - 1.6, lerp(255.0, 340.0, u), y, true);
            }
            t if t < 2.35 => l.draw(&mut img, k, "Land", t - 2.15, 340.0, high, true),
            t if t < 2.75 => {
                let u = (t - 2.35) / 0.4;
                l.draw(&mut img, k, "Dash Effect", t - 2.35, lerp(330.0, 500.0, u) - 30.0, high, true);
                l.draw(&mut img, k, "Dash", t - 2.35, lerp(340.0, 560.0, u), high, true);
            }
            t if t < 3.05 => l.draw(&mut img, k, "Dash To Idle", t - 2.75, 560.0, high, true),
            t => l.draw(&mut img, k, "Idle", t - 3.05, 560.0, high, true),
        }
        out.push(img);
    }
    out
}

/// A crawlid walks up; three slashes and it's done.
fn fight(l: &Libs) -> Vec<Image> {
    let wins = [(60, 170, 660, 300)];
    let y = 170.0;
    let mut out = vec![];
    let slashes = [1.2, 1.75, 2.3];
    for i in 0..(4.6 * FPS) as usize {
        let t = i as f32 / FPS;
        let mut img = desktop(720, 320, &wins);
        let kx = 280.0;
        // Crawlid: walks in from the right, recoils from each blow, dies on the third.
        let hit = slashes.iter().filter(|&&s| t >= s + 0.08).count();
        let cx = lerp(560.0, 360.0, t / 1.2) + 14.0 * hit as f32;
        if hit < 3 {
            l.draw(&mut img, "Crawler", "walk", t, cx, y, false);
        } else {
            let d = t - slashes[2] - 0.08;
            let lift = if d < 0.35 { -60.0 * (std::f32::consts::PI * d / 0.35).sin() } else { 0.0 };
            let clip = if d < 0.35 { "Death Air" } else { "Death Land" };
            l.draw(&mut img, "Crawler", clip, if d < 0.35 { d } else { d - 0.35 }, cx + 40.0 * d.min(0.35), y + lift, false);
        }
        // Knight: walks in, three slashes, then looks down at it.
        if t < 1.1 {
            l.draw(&mut img, "Knight", "Run", t, lerp(120.0, kx, t / 1.1), y, true);
        } else if let Some((n, s)) = slashes.iter().enumerate().find(|(_, s)| t >= **s && t < **s + 0.4) {
            let clip = if n % 2 == 0 { "Slash" } else { "SlashAlt" };
            let fx = if n % 2 == 0 { "SlashEffect" } else { "SlashEffectAlt" };
            l.draw(&mut img, "Knight", clip, t - s, kx, y, true);
            l.draw(&mut img, "Knight", fx, t - s, kx, y, true);
        } else if t > slashes[2] + 0.9 {
            l.draw(&mut img, "Knight", "LookDown", t - slashes[2] - 0.9, kx, y, true);
        } else {
            l.draw(&mut img, "Knight", "Idle", t, kx, y, true);
        }
        out.push(img);
    }
    out
}

/// Cornifer on a window top, humming over his map; the knight walks up and listens.
fn visitor(l: &Libs) -> Vec<Image> {
    let wins = [(40, 250, 680, 350)];
    let y = 250.0;
    let mut out = vec![];
    for i in 0..(4.0 * FPS) as usize {
        let t = i as f32 / FPS;
        let mut img = desktop(720, 360, &wins);
        let talking = t > 1.6;
        l.draw(&mut img, "Mapper", if talking { "Talk L" } else { "Idle" }, if talking { t - 1.6 } else { t }, 470.0, y, false);
        if t < 1.4 {
            l.draw(&mut img, "Knight", "Walk", t, lerp(130.0, 330.0, t / 1.4), y, true);
        } else {
            l.draw(&mut img, "Knight", "Idle", t - 1.4, 330.0, y, true);
        }
        if talking {
            // A speech box over Cornifer (the app writes his line in it).
            rect(&mut img, 380, 14, 640, 70, rgb(10, 10, 14));
            rect(&mut img, 380, 14, 640, 15, rgb(200, 200, 210));
            rect(&mut img, 380, 69, 640, 70, rgb(200, 200, 210));
            for row in 0..2 {
                let len = ((t - 1.6) * 260.0).min(if row == 0 { 220.0 } else { 140.0 });
                if row == 0 || t > 2.6 {
                    rect(&mut img, 396, 30 + row * 20, 396 + len as i32, 36 + row * 20, rgb(170, 170, 180));
                }
            }
        }
        out.push(img);
    }
    out
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data, skin, out] = args.as_slice() else {
        anyhow::bail!("usage: <data/hk> <Vanilla dir> <out dir>");
    };
    let data = Path::new(data);
    let skin = Skin::from_dir(Path::new(skin)).context("not an art dir (no Knight.png)")?;
    let mut bank = SpriteBank::load_metadata(data)?;
    let mut libs = vec![];
    let mut keys = vec![];
    let mut standing = vec![];
    for (name, stand) in [("Knight", "Idle"), ("Crawler", "walk"), ("Mapper", "Idle")] {
        let lib = ClipLibrary::load(&data.join(format!("animations/{name}.json")), &bank)?;
        keys.extend(lib.sprite_keys());
        standing.push((name.to_string(), lib, stand));
    }
    bank.rasterize_skin(&skin, None, &keys, PPU)?;
    // As the app does: the sprite origin sits this far above the feet (its standing frame's
    // bottom edge is the ground line).
    for (name, lib, stand) in standing {
        let feet = lib.id(stand).and_then(|id| lib.get(id).frames.first()).and_then(|f| f.sprite).and_then(|k| bank.frame(k)).map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
        libs.push((name, lib, feet));
    }
    let l = Libs { bank, libs };
    let out = Path::new(out);
    std::fs::create_dir_all(out)?;
    save(&out.join("hero.gif"), &hero(&l))?;
    save(&out.join("fight.gif"), &fight(&l))?;
    save(&out.join("visitor.gif"), &visitor(&l))?;
    Ok(())
}
