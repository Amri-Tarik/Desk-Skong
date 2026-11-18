//! tk2d metadata (from `HK_TK2D_Exporter`) + atlas PNGs -> rasterized, ready-to-blit frames.
//!
//! Each tk2d sprite is a quad given as local `positions` (world units, y up, origin =
//! sprite anchor) with matching atlas `uvs`. Rasterizing that quad reproduces exactly what
//! the game draws: trimming, pivot and atlas rotation are all encoded in the vertices.
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::canvas::Image;

// ---------------------------------------------------------------- JSON schema

#[derive(Debug, Deserialize)]
pub struct CollectionJson {
    pub name: String,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub sprites: Vec<SpriteJson>,
}

#[derive(Debug, Deserialize)]
pub struct SpriteJson {
    pub id: u32,
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    #[serde(default)]
    pub texel_size: Vec<f32>,
}

#[derive(Debug, Deserialize)]
pub struct AnimLibraryJson {
    pub name: String,
    pub clips: Vec<ClipJson>,
}

#[derive(Debug, Deserialize)]
pub struct ClipJson {
    pub name: String,
    pub fps: f32,
    #[serde(default)]
    pub loop_start: u32,
    pub wrap_mode: String,
    pub frames: Vec<FrameJson>,
}

#[derive(Debug, Deserialize)]
pub struct FrameJson {
    pub collection: String,
    pub sprite_id: u32,
    #[serde(default)]
    pub event: Option<EventJson>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct EventJson {
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub int: i32,
    #[serde(default)]
    pub float: f32,
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    serde_json::from_reader(BufReader::new(f)).with_context(|| format!("parse {}", path.display()))
}

// ---------------------------------------------------------------- skins

/// Custom-Knight-style skin file name for each tk2d collection.
pub const SKIN_FILES: &[(&str, &str)] = &[
    ("Knight", "Knight.png"),
    ("Knight Dream Gate Cln", "Sprint.png"),
    ("Knight Slug Cln", "Unn.png"),
    ("Grimmchild Cln", "Grimm.png"),
    ("Weaverling Cln", "Weaver.png"),
    ("Knight Hatchling Cln", "Hatchling.png"),
    ("Shade", "Shade.png"),
];

/// A skin is a folder with `Knight.png` (required) and optionally the other atlases.
#[derive(Clone, Debug)]
pub struct Skin {
    pub name: String,
    pub dir: PathBuf,
}

impl Skin {
    pub fn from_dir(dir: &Path) -> Option<Skin> {
        find_ci(dir, "Knight.png")?;
        let name = dir.file_name()?.to_string_lossy().into_owned();
        Some(Skin { name, dir: dir.to_path_buf() })
    }

    /// Atlas file for a collection, if the skin provides it (case-insensitive).
    pub fn atlas_for(&self, collection: &str) -> Option<PathBuf> {
        let file = SKIN_FILES.iter().find(|(c, _)| *c == collection)?.1;
        find_ci(&self.dir, file)
    }

    /// All skins directly under `root` (each subfolder containing a Knight.png), sorted by name.
    pub fn discover(root: &Path) -> Vec<Skin> {
        let mut out: Vec<Skin> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| Skin::from_dir(&e.path()))
            .collect();
        out.sort_by_key(|s| s.name.to_lowercase());
        out
    }
}

fn find_ci(dir: &Path, file: &str) -> Option<PathBuf> {
    let exact = dir.join(file);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(file)))
}

// ---------------------------------------------------------------- atlas

/// Decoded atlas, straight RGBA8, rows **top-down**.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Atlas {
    pub fn load_png(path: &Path) -> Result<Atlas> {
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut dec = png::Decoder::new(BufReader::new(f));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().with_context(|| format!("png {}", path.display()))?;
        let mut buf = vec![0; reader.output_buffer_size().context("png too large")?];
        let info = reader.next_frame(&mut buf)?;
        buf.truncate(info.buffer_size());
        let (w, h) = (info.width, info.height);
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => bail!("indexed png not expanded: {}", path.display()),
        };
        Ok(Atlas { width: w, height: h, rgba })
    }

    /// Bilinear sample at continuous texel coords (top-down), premultiplied ARGB as f32x4.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = x - 0.5;
        let y = y - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let mut acc = [0.0f32; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                let w = wx * wy;
                if w <= 0.0 {
                    continue;
                }
                let px = (x0 as i32 + dx).clamp(0, self.width as i32 - 1) as usize;
                let py = (y0 as i32 + dy).clamp(0, self.height as i32 - 1) as usize;
                let i = (py * self.width as usize + px) * 4;
                let a = self.rgba[i + 3] as f32 / 255.0;
                acc[0] += w * a * 255.0;
                acc[1] += w * self.rgba[i] as f32 * a;
                acc[2] += w * self.rgba[i + 1] as f32 * a;
                acc[3] += w * self.rgba[i + 2] as f32 * a;
            }
        }
        acc
    }
}

// ---------------------------------------------------------------- frames

/// A rasterized sprite. `anchor` is the sprite origin (tk2d transform position) in image
/// pixels, y down. Draw at `screen_anchor - anchor`.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub image: Image,
    pub anchor: [f32; 2],
}

impl Frame {
    /// Horizontal mirror around the anchor (sprites face left natively in HK).
    pub fn anchor_x(&self, flip: bool) -> f32 {
        if flip { self.image.width as f32 - self.anchor[0] } else { self.anchor[0] }
    }
}

/// Rasterize one tk2d sprite at `px_per_unit` output pixels per world unit.
pub fn rasterize(sprite: &SpriteJson, atlas: &Atlas, px_per_unit: f32) -> Frame {
    if sprite.positions.len() < 4 || sprite.uvs.len() < 4 {
        return Frame::default();
    }
    let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in &sprite.positions {
        minx = minx.min(p[0]);
        maxx = maxx.max(p[0]);
        miny = miny.min(p[1]);
        maxy = maxy.max(p[1]);
    }
    let (qw, qh) = (maxx - minx, maxy - miny);
    if qw <= 0.0 || qh <= 0.0 {
        return Frame::default();
    }
    // UV at each quad corner (tk2d quads are axis-aligned rectangles in local space).
    let corner = |cx: f32, cy: f32| -> [f32; 2] {
        let i = (0..sprite.positions.len())
            .min_by(|&a, &b| {
                let d = |k: usize| (sprite.positions[k][0] - cx).abs() + (sprite.positions[k][1] - cy).abs();
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0);
        sprite.uvs[i]
    };
    let (uv_bl, uv_br, uv_tl, uv_tr) = (corner(minx, miny), corner(maxx, miny), corner(minx, maxy), corner(maxx, maxy));

    // Output covers the quad snapped outward to whole pixels around the anchor so the
    // anchor lands on an integer pixel (avoids sub-pixel wobble between frames).
    let left = (minx * px_per_unit).floor();
    let right = (maxx * px_per_unit).ceil();
    let top = (maxy * px_per_unit).ceil();
    let bottom = (miny * px_per_unit).floor();
    let w = (right - left).max(1.0) as u32;
    let h = (top - bottom).max(1.0) as u32;
    let mut img = Image::new(w, h);

    // Supersample when downscaling so thin details average instead of aliasing.
    let src_ppu = sprite.texel_size.first().map_or(64.0, |t| 1.0 / t.max(1e-6));
    let ss = (src_ppu / px_per_unit).ceil().clamp(1.0, 4.0) as u32;
    let inv_ss = 1.0 / ss as f32;
    let norm = 1.0 / (ss * ss) as f32;
    let (aw, ah) = (atlas.width as f32, atlas.height as f32);

    for j in 0..h {
        for i in 0..w {
            let mut acc = [0.0f32; 4];
            for sj in 0..ss {
                for si in 0..ss {
                    let xw = (left + i as f32 + (si as f32 + 0.5) * inv_ss) / px_per_unit;
                    let yw = (top - (j as f32 + (sj as f32 + 0.5) * inv_ss)) / px_per_unit;
                    let s = (xw - minx) / qw;
                    let t = (yw - miny) / qh;
                    if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
                        continue;
                    }
                    let lerp = |a: [f32; 2], b: [f32; 2], k: f32| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
                    let uv = lerp(lerp(uv_bl, uv_br, s), lerp(uv_tl, uv_tr, s), t);
                    let c = atlas.sample(uv[0] * aw, (1.0 - uv[1]) * ah);
                    for k in 0..4 {
                        acc[k] += c[k];
                    }
                }
            }
            let a = (acc[0] * norm).round().clamp(0.0, 255.0) as u32;
            if a == 0 {
                continue;
            }
            let ch = |v: f32| (v * norm).round().clamp(0.0, a as f32) as u32;
            img.px[(j * w + i) as usize] = (a << 24) | (ch(acc[1]) << 16) | (ch(acc[2]) << 8) | ch(acc[3]);
        }
    }
    trim(Frame { image: img, anchor: [-left, top] })
}

/// Crop fully transparent margins, keeping the anchor in place.
fn trim(f: Frame) -> Frame {
    let (w, h) = (f.image.width as usize, f.image.height as usize);
    let opaque = |x: usize, y: usize| f.image.px[y * w + x] >> 24 != 0;
    let Some(y0) = (0..h).find(|&y| (0..w).any(|x| opaque(x, y))) else {
        return Frame { image: Image::new(1, 1), anchor: [0.0, 0.0] };
    };
    let y1 = (0..h).rev().find(|&y| (0..w).any(|x| opaque(x, y))).unwrap_or(y0) + 1;
    let x0 = (0..w).find(|&x| (y0..y1).any(|y| opaque(x, y))).unwrap_or(0);
    let x1 = (0..w).rev().find(|&x| (y0..y1).any(|y| opaque(x, y))).unwrap_or(x0) + 1;
    if (x0, y0, x1, y1) == (0, 0, w, h) {
        return f;
    }
    let mut out = Image::new((x1 - x0) as u32, (y1 - y0) as u32);
    for y in y0..y1 {
        out.px[(y - y0) * (x1 - x0)..(y - y0 + 1) * (x1 - x0)].copy_from_slice(&f.image.px[y * w + x0..y * w + x1]);
    }
    Frame { image: out, anchor: [f.anchor[0] - x0 as f32, f.anchor[1] - y0 as f32] }
}

// ---------------------------------------------------------------- sprite bank

/// Index of a frame: (collection slot, sprite id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteKey {
    pub collection: u16,
    pub sprite: u32,
}

/// All collections' metadata plus the rasterized frames for one skin at one scale.
pub struct SpriteBank {
    collections: Vec<CollectionJson>,
    by_name: HashMap<String, u16>,
    frames: HashMap<SpriteKey, Frame>,
    pub px_per_unit: f32,
}

impl SpriteBank {
    /// Load every `*.json` in `<data>/collections`.
    pub fn load_metadata(data_dir: &Path) -> Result<SpriteBank> {
        let dir = data_dir.join("collections");
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .with_context(|| format!("read {}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();
        let mut collections = Vec::new();
        let mut by_name = HashMap::new();
        for p in paths {
            let c: CollectionJson = read_json(&p)?;
            by_name.insert(c.name.clone(), collections.len() as u16);
            collections.push(c);
        }
        Ok(SpriteBank { collections, by_name, frames: HashMap::new(), px_per_unit: 64.0 })
    }

    pub fn collection_index(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    pub fn sprite_name(&self, key: SpriteKey) -> Option<&str> {
        let c = self.collections.get(key.collection as usize)?;
        c.sprites.get(key.sprite as usize).map(|s| s.name.as_str())
    }

    /// Rasterize `keys` from `skin` at `px_per_unit`, replacing any previous frames.
    /// Atlases are decoded one at a time and dropped right after, so peak memory is one
    /// atlas plus the (small) rasterized frames. Collections the skin lacks are skipped.
    pub fn rasterize_skin(&mut self, skin: &Skin, keys: &[SpriteKey], px_per_unit: f32) -> Result<()> {
        self.frames.clear();
        self.px_per_unit = px_per_unit;
        let mut by_col: HashMap<u16, Vec<u32>> = HashMap::new();
        for k in keys {
            by_col.entry(k.collection).or_default().push(k.sprite);
        }
        for (col, mut ids) in by_col {
            ids.sort_unstable();
            ids.dedup();
            let meta = &self.collections[col as usize];
            let Some(path) = skin.atlas_for(&meta.name) else {
                log::debug!("skin '{}' has no atlas for '{}'", skin.name, meta.name);
                continue;
            };
            let atlas = Atlas::load_png(&path)?;
            if (atlas.width, atlas.height) != (meta.atlas_width, meta.atlas_height) {
                log::warn!(
                    "{}: atlas is {}x{}, expected {}x{} — UVs are normalized so it may still work",
                    path.display(),
                    atlas.width,
                    atlas.height,
                    meta.atlas_width,
                    meta.atlas_height
                );
            }
            for id in ids {
                if let Some(s) = meta.sprites.get(id as usize) {
                    self.frames.insert(SpriteKey { collection: col, sprite: id }, rasterize(s, &atlas, px_per_unit));
                }
            }
        }
        Ok(())
    }

    pub fn frame(&self, key: SpriteKey) -> Option<&Frame> {
        self.frames.get(&key)
    }

    pub fn frame_bytes(&self) -> usize {
        self.frames.values().map(|f| f.image.px.len() * 4).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x2 atlas: TL red, TR green, BL blue, BR white (top-down rows).
    fn atlas() -> Atlas {
        #[rustfmt::skip]
        let rgba = vec![
            255, 0, 0, 255,   0, 255, 0, 255,
            0, 0, 255, 255,   255, 255, 255, 255,
        ];
        Atlas { width: 2, height: 2, rgba }
    }

    fn quad(uvs: [[f32; 2]; 4]) -> SpriteJson {
        SpriteJson {
            id: 0,
            name: "t".into(),
            // BL, BR, TL, TR — 2x2 units centered on origin
            positions: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            uvs: uvs.to_vec(),
            texel_size: vec![1.0, 1.0],
        }
    }

    #[test]
    fn upright_quad_maps_corners() {
        // uv v is bottom-up: BL=(0,0) BR=(1,0) TL=(0,1) TR=(1,1)
        let f = rasterize(&quad([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]), &atlas(), 1.0);
        assert_eq!((f.image.width, f.image.height), (2, 2));
        assert_eq!(f.anchor, [1.0, 1.0]);
        let rgb = |p: u32| p & 0x00ff_ffff;
        assert_eq!(rgb(f.image.px[0]), 0xff0000); // TL red
        assert_eq!(rgb(f.image.px[1]), 0x00ff00); // TR green
        assert_eq!(rgb(f.image.px[2]), 0x0000ff); // BL blue
        assert_eq!(rgb(f.image.px[3]), 0xffffff); // BR white
    }

    #[test]
    fn rotated_uvs_rotate_image() {
        // Rotate 90°: quad TL samples atlas BL, etc.
        let f = rasterize(&quad([[1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [0.0, 1.0]]), &atlas(), 1.0);
        let rgb = |p: u32| p & 0x00ff_ffff;
        assert_eq!(rgb(f.image.px[0]), 0x0000ff); // TL <- atlas BL (blue)
        assert_eq!(rgb(f.image.px[1]), 0xff0000); // TR <- atlas TL (red)
    }

    #[test]
    fn real_knight_metadata_loads() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = SpriteBank::load_metadata(&data).expect("load");
        let k = bank.collection_index("Knight").expect("Knight collection");
        assert_eq!(bank.collections[k as usize].sprites.len(), 891);
    }
}
