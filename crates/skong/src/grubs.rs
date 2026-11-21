//! Trapped grubs' look: the game's grub jar (`brk_bottle`, a prop sprite) with the grub
//! (`Grub2`: `Idle`, and `Cry` while the knight is near) inside it, behind the glass. The
//! find itself (where jars are, breaking them) lives in `skong_core::discovery`; the app
//! draws it through `discoveries.rs` and plays the jar breaking (glass, the grub's
//! `Freed` / a mimic's `Freed Fake`) in `app/discover.rs`.
use std::path::Path;

use skong_core::anim::{Clip, ClipLibrary, WrapMode};
use skong_core::assets::{Skin, SpriteBank};
use skong_core::canvas::{Image, premul};
use skong_core::geom::Vec2;

/// The grub's origin inside the jar image, as a fraction of its size: the prefab's `Grub`
/// sits at (0.04, -0.22) units from the jar's pivot, which is (104, 121.5) px into the
/// 208 x 227 px jar art (its sprite rectangle is 366 x 265 with the art at (79, 27)).
const GRUB_IN_JAR: [f32; 2] = [106.6 / 208.0, 135.6 / 227.0];
/// Jar size drawn without the game's art (px at scale 1).
const PLAIN_JAR: [f32; 2] = [104.0, 114.0];

struct Clipped {
    frames: Vec<(Image, [f32; 2])>,
    fps: f32,
    loop_start: usize,
    wrap: WrapMode,
}

impl Clipped {
    fn from(clip: &Clip, frames: &std::collections::HashMap<skong_core::assets::SpriteKey, skong_core::assets::Frame>) -> Option<Clipped> {
        let images: Vec<(Image, [f32; 2])> = clip.frames.iter().filter_map(|f| f.sprite.and_then(|k| frames.get(&k))).map(|f| (f.image.clone(), f.anchor)).collect();
        (!images.is_empty()).then(|| Clipped { frames: images, fps: clip.fps.max(1.0), loop_start: clip.loop_start, wrap: clip.wrap })
    }

    /// Frame shown `t` seconds into the clip (loops as the clip says).
    fn index(&self, t: f32) -> usize {
        let n = self.frames.len();
        let i = (t.max(0.0) * self.fps) as usize;
        match self.wrap {
            WrapMode::LoopSection if i >= n => {
                let start = self.loop_start.min(n - 1);
                start + (i - n) % (n - start).max(1)
            }
            WrapMode::Loop | WrapMode::RandomLoop | WrapMode::PingPong => i % n,
            _ => i.min(n - 1),
        }
    }
}

/// The jar and its grub at the current scale.
pub struct GrubArt {
    jar: Image,
    idle: Option<Clipped>,
    cry: Option<Clipped>,
}

impl GrubArt {
    /// `jar`: the jar sprite at 64 px per unit (None: a drawn jar); the grub's clips come
    /// from `data/hk` and the vanilla skin (missing: a drawn grub).
    pub fn load(data: &Path, vanilla: Option<&Path>, jar: Option<&Image>, scale: f32) -> GrubArt {
        let jar = match jar {
            Some(j) => j.scaled(scale),
            None => plain_jar(scale),
        };
        let (idle, cry) = match vanilla.map(|v| grub_clips(data, v, 64.0 * scale)) {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                log::warn!("grub art: {e:#}; drawing a plain grub");
                (None, None)
            }
            None => (None, None),
        };
        GrubArt { jar, idle, cry }
    }

    /// Size of the jar on screen (px).
    pub fn size(&self) -> (f32, f32) {
        (self.jar.width as f32, self.jar.height as f32)
    }

    /// Which frame shows at `t` (s): (crying, frame index).
    pub fn frame(&self, crying: bool, t: f64) -> (bool, usize) {
        let clip = if crying { self.cry.as_ref().or(self.idle.as_ref()) } else { self.idle.as_ref() };
        (crying, clip.map_or(0, |c| c.index((t % 600.0) as f32)))
    }

    /// The jar with the grub in it (`right`: the grub turned to face right).
    pub fn compose(&self, (crying, i): (bool, usize), right: bool) -> Image {
        let mut img = Image::new(self.jar.width, self.jar.height);
        let origin = Vec2::new(GRUB_IN_JAR[0] * self.jar.width as f32, GRUB_IN_JAR[1] * self.jar.height as f32);
        let clip = if crying { self.cry.as_ref().or(self.idle.as_ref()) } else { self.idle.as_ref() };
        match clip.and_then(|c| c.frames.get(i.min(c.frames.len().saturating_sub(1)))) {
            Some((f, anchor)) => {
                // The art faces left; mirrored, its anchor mirrors too.
                let ax = if right { f.width as f32 - anchor[0] } else { anchor[0] };
                img.blit_over(f, (origin.x - ax).round() as i32, (origin.y - anchor[1]).round() as i32, right);
            }
            None => plain_grub(&mut img, origin),
        }
        // The glass (and its lid) in front.
        img.blit_over(&self.jar, 0, 0, false);
        img
    }

    /// Where the grub is in a jar standing with its bottom centre at `foot` (screen px).
    pub fn grub_at(&self, foot: Vec2) -> Vec2 {
        Vec2::new(foot.x + (GRUB_IN_JAR[0] - 0.5) * self.jar.width as f32, foot.y - (1.0 - GRUB_IN_JAR[1]) * self.jar.height as f32)
    }
}

/// The grub's `Idle` and `Cry` clips, rasterized.
fn grub_clips(data: &Path, vanilla: &Path, px_per_unit: f32) -> anyhow::Result<(Option<Clipped>, Option<Clipped>)> {
    let skin = Skin { name: "Vanilla".into(), dir: vanilla.to_path_buf() };
    let bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Grub2.json"), &bank)?;
    let clip = |name: &str| lib.id(name).map(|id| lib.get(id));
    let mut keys: Vec<_> = ["Idle", "Cry"].iter().filter_map(|n| clip(n)).flat_map(|c| c.frames.iter().filter_map(|f| f.sprite)).collect();
    keys.sort_by_key(|k| (k.collection, k.sprite));
    keys.dedup();
    let frames = bank.rasterize_sets(&skin, None, &[(&keys, px_per_unit)])?.pop().unwrap_or_default();
    let idle = clip("Idle").and_then(|c| Clipped::from(c, &frames));
    anyhow::ensure!(idle.is_some(), "no Grub2 Idle frames (Grub.png missing from the skin?)");
    Ok((idle, clip("Cry").and_then(|c| Clipped::from(c, &frames))))
}

/// A drawn jar: a pale glass bulb under a dark lid.
fn plain_jar(scale: f32) -> Image {
    let (w, h) = ((PLAIN_JAR[0] * scale).round().max(8.0) as u32, (PLAIN_JAR[1] * scale).round().max(8.0) as u32);
    let mut img = Image::new(w, h);
    let (cx, cy, rx, ry) = (w as f32 * 0.5, h as f32 * 0.62, w as f32 * 0.48, h as f32 * 0.37);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
            let d = dx * dx + dy * dy;
            let c = if y as f32 > h as f32 * 0.16 && y as f32 <= h as f32 * 0.25 && dx.abs() < 0.75 {
                premul(40, 42, 50, 255) // the lid
            } else if d <= 1.0 {
                let rim = d > 0.82;
                if rim { premul(200, 210, 225, 200) } else { premul(210, 220, 235, 70) }
            } else {
                continue;
            };
            img.px[(y * w + x) as usize] = c;
        }
    }
    img
}

/// A drawn grub: a soft green blob with two dark eyes.
fn plain_grub(img: &mut Image, at: Vec2) {
    let r = (img.width as f32 * 0.16).max(3.0);
    for y in 0..img.height {
        for x in 0..img.width {
            let (dx, dy) = ((x as f32 - at.x) / r, (y as f32 - at.y) / (1.3 * r));
            if dx * dx + dy * dy <= 1.0 {
                let eye = (dy + 0.35).abs() < 0.14 && ((dx - 0.3).abs() < 0.12 || (dx + 0.3).abs() < 0.12);
                img.px[(y * img.width + x) as usize] = if eye { premul(20, 24, 22, 255) } else { premul(132, 168, 140, 255) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_art_a_drawn_jar_and_grub_still_show() {
        let art = GrubArt::load(Path::new("nowhere"), None, None, 1.0);
        let (w, h) = art.size();
        assert!(w > 50.0 && h > 50.0);
        let img = art.compose(art.frame(true, 3.0), false);
        let at = art.grub_at(Vec2::new(w * 0.5, h));
        let px = img.px[(at.y as u32 * img.width + at.x as u32) as usize];
        assert!(px >> 24 > 0, "the grub shows through the glass");
    }
}
