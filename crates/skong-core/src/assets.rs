//! tk2d metadata (from `HK_TK2D_Exporter`) + atlas PNGs -> rasterized, ready-to-blit frames.
//!
//! Each tk2d sprite is a quad given as local `positions` (world units, y up, origin =
//! sprite anchor) with matching atlas `uvs`. Rasterizing that quad reproduces exactly what
//! the game draws: trimming, pivot and atlas rotation are all encoded in the vertices.
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Folder name of the vanilla atlases written by `HK_TK2D_Exporter --textures`.
pub const VANILLA: &str = "Vanilla";

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

    /// Atlas file for a collection, if the skin provides it (case-insensitive). Custom
    /// Knight names first (`Sprint.png` …), then the exporter's `<Collection_Name>.png`.
    pub fn atlas_for(&self, collection: &str) -> Option<PathBuf> {
        if let Some((_, file)) = SKIN_FILES.iter().find(|(c, _)| *c == collection) {
            return find_ci(&self.dir, file);
        }
        find_ci(&self.dir, &format!("{}.png", collection.replace(' ', "_")))
    }

    /// Skins under `root`: every folder (up to two levels deep) containing a Knight.png,
    /// sorted by name, with the vanilla export first.
    pub fn discover(root: &Path) -> Vec<Skin> {
        let subdirs = |d: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
        };
        let mut out: Vec<Skin> = Vec::new();
        for d in subdirs(root) {
            match Skin::from_dir(&d) {
                Some(s) => out.push(s),
                None => out.extend(subdirs(&d).iter().filter_map(|d2| Skin::from_dir(d2))),
            }
        }
        out.sort_by_key(|s| (!s.name.eq_ignore_ascii_case(VANILLA), s.name.to_lowercase()));
        out
    }

    /// [`Skin::discover`] over several roots (the dev `skins/` folder, the art extracted
    /// into the user's data folder): a name found in an earlier root wins.
    pub fn discover_all(roots: &[PathBuf]) -> Vec<Skin> {
        let mut out: Vec<Skin> = Vec::new();
        for root in roots {
            for s in Skin::discover(root) {
                if !out.iter().any(|o| o.name.eq_ignore_ascii_case(&s.name)) {
                    out.push(s);
                }
            }
        }
        out.sort_by_key(|s| (!s.name.eq_ignore_ascii_case(VANILLA), s.name.to_lowercase()));
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

// ---------------------------------------------------------------- frame store

/// Index of a frame: (collection slot, sprite id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteKey {
    pub collection: u16,
    pub sprite: u32,
}

/// A rasterized frame kept losslessly compressed (PNG bytes of its premultiplied pixels)
/// until it is first drawn.
struct Packed {
    width: u32,
    height: u32,
    anchor: [f32; 2],
    /// Empty for fully transparent (or empty) frames.
    png: Box<[u8]>,
}

impl Packed {
    fn pack(f: &Frame) -> Packed {
        let (width, height) = (f.image.width, f.image.height);
        let png = if f.image.px.iter().all(|p| p >> 24 == 0) { Box::default() } else { encode_px(&f.image) };
        Packed { width, height, anchor: f.anchor, png }
    }

    fn unpack(&self) -> Frame {
        let mut image = Image::new(self.width, self.height);
        if !self.png.is_empty() {
            match decode_px(&self.png, &mut image.px) {
                Ok(()) => {}
                Err(e) => log::warn!("packed frame {}x{}: {e:#}", self.width, self.height),
            }
        }
        Frame { image, anchor: self.anchor }
    }
}

fn encode_px(img: &Image) -> Box<[u8]> {
    let bytes: Vec<u8> = img.px.iter().flat_map(|p| p.to_le_bytes()).collect();
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    // Fast: ~25 ms for every Knight frame at x0.6 and ~4x smaller than raw pixels.
    enc.set_compression(png::Compression::Fast);
    let written = enc.write_header().and_then(|mut w| w.write_image_data(&bytes));
    match written {
        Ok(()) => out.into_boxed_slice(),
        Err(e) => {
            // Not expected for in-memory output of a non-empty image.
            log::warn!("pack frame {}x{}: {e}", img.width, img.height);
            Box::default()
        }
    }
}

fn decode_px(data: &[u8], px: &mut [u32]) -> Result<()> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(data)).read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().context("frame too large")?];
    let info = reader.next_frame(&mut buf)?;
    if info.buffer_size() != px.len() * 4 {
        bail!("size mismatch: {} bytes for {} pixels", info.buffer_size(), px.len());
    }
    for (p, b) in px.iter_mut().zip(buf.chunks_exact(4)) {
        *p = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    }
    Ok(())
}

struct Slot {
    packed: Packed,
    frame: OnceLock<Frame>,
    /// Requested since the last [`FrameStore::evict_unused`].
    used: AtomicBool,
}

/// Rasterized frames kept compressed and unpacked lazily, the first time each is asked
/// for (about 30–70 µs per frame), then cached until [`Self::evict_unused`]. Rarely played
/// clips cost only their compressed bytes (~1/4 of the pixels).
#[derive(Default)]
pub struct FrameStore {
    slots: HashMap<SpriteKey, Slot>,
}

impl FrameStore {
    pub fn pack(frames: HashMap<SpriteKey, Frame>) -> FrameStore {
        let mut s = FrameStore::default();
        for (k, f) in frames {
            s.insert(k, &f);
        }
        s
    }

    /// Compress and store `frame` (replacing any previous one for `key`).
    pub fn insert(&mut self, key: SpriteKey, frame: &Frame) {
        self.slots.insert(key, Slot { packed: Packed::pack(frame), frame: OnceLock::new(), used: AtomicBool::new(false) });
    }

    /// The frame for `key`, unpacked on first use.
    pub fn get(&self, key: &SpriteKey) -> Option<&Frame> {
        let slot = self.slots.get(key)?;
        if !slot.used.load(Ordering::Relaxed) {
            slot.used.store(true, Ordering::Relaxed);
        }
        Some(slot.frame.get_or_init(|| slot.packed.unpack()))
    }

    pub fn contains(&self, key: &SpriteKey) -> bool {
        self.slots.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Frames currently unpacked.
    pub fn unpacked(&self) -> usize {
        self.slots.values().filter(|s| s.frame.get().is_some()).count()
    }

    /// Drop the unpacked pixels of frames not asked for since the previous call (they
    /// stay packed and unpack again when next needed). Returns how many were dropped.
    pub fn evict_unused(&mut self) -> usize {
        let mut n = 0;
        for s in self.slots.values_mut() {
            if !*s.used.get_mut() && s.frame.take().is_some() {
                n += 1;
            }
            *s.used.get_mut() = false;
        }
        n
    }

    /// Bytes held: compressed frames plus unpacked pixels.
    pub fn bytes(&self) -> usize {
        self.slots.values().map(|s| s.packed.png.len() + s.frame.get().map_or(0, |f| f.image.px.len() * 4)).sum()
    }

    /// Bytes the frames would take fully unpacked (what an eager bank holds).
    pub fn unpacked_bytes(&self) -> usize {
        self.slots.values().map(|s| s.packed.width as usize * s.packed.height as usize * 4).sum()
    }
}

// ---------------------------------------------------------------- sprite bank

/// All collections' metadata plus the rasterized frames for one skin at one scale.
pub struct SpriteBank {
    collections: Vec<CollectionJson>,
    by_name: HashMap<String, u16>,
    frames: FrameStore,
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
        Ok(SpriteBank { collections, by_name, frames: FrameStore::default(), px_per_unit: 64.0 })
    }

    pub fn collection_index(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    /// Key of the sprite called `name` in `collection`.
    pub fn sprite_key(&self, collection: &str, name: &str) -> Option<SpriteKey> {
        let ci = self.collection_index(collection)?;
        let i = self.collections[ci as usize].sprites.iter().position(|s| s.name == name)?;
        Some(SpriteKey { collection: ci, sprite: i as u32 })
    }

    pub fn sprite_name(&self, key: SpriteKey) -> Option<&str> {
        let c = self.collections.get(key.collection as usize)?;
        c.sprites.get(key.sprite as usize).map(|s| s.name.as_str())
    }

    /// Rasterize `keys` from `skin` at `px_per_unit`, replacing any previous frames (they
    /// are stored packed, see [`FrameStore`]). Atlases are decoded one at a time and
    /// dropped right after, so peak memory is one atlas plus the packed frames. Collections
    /// the skin lacks come from `fallback` (normally the vanilla export) or are skipped.
    pub fn rasterize_skin(&mut self, skin: &Skin, fallback: Option<&Skin>, keys: &[SpriteKey], px_per_unit: f32) -> Result<()> {
        let mut store = FrameStore::default();
        self.rasterize_each(skin, fallback, &[(keys, px_per_unit)], |_, k, f| store.insert(k, &f))?;
        self.install_store(store, px_per_unit);
        Ok(())
    }

    /// Rasterize several key sets, each at its own scale, decoding every atlas only once.
    /// Returns one frame map per set (this bank's own frames are untouched).
    pub fn rasterize_sets(&self, skin: &Skin, fallback: Option<&Skin>, sets: &[(&[SpriteKey], f32)]) -> Result<Vec<HashMap<SpriteKey, Frame>>> {
        let mut out: Vec<HashMap<SpriteKey, Frame>> = vec![HashMap::new(); sets.len()];
        self.rasterize_each(skin, fallback, sets, |set, k, f| {
            out[set].insert(k, f);
        })?;
        Ok(out)
    }

    /// Like [`Self::rasterize_sets`], but hands each frame (with its set index) to `sink`
    /// as soon as it is rasterized, so callers can pack frames without holding them all.
    pub fn rasterize_each(&self, skin: &Skin, fallback: Option<&Skin>, sets: &[(&[SpriteKey], f32)], mut sink: impl FnMut(usize, SpriteKey, Frame)) -> Result<()> {
        // collection -> [(set index, sprite id)]
        let mut by_col: HashMap<u16, Vec<(usize, u32)>> = HashMap::new();
        for (i, (keys, _)) in sets.iter().enumerate() {
            for k in keys.iter() {
                by_col.entry(k.collection).or_default().push((i, k.sprite));
            }
        }
        for (col, mut wanted) in by_col {
            wanted.sort_unstable();
            wanted.dedup();
            let meta = &self.collections[col as usize];
            let Some(path) = skin.atlas_for(&meta.name).or_else(|| fallback?.atlas_for(&meta.name)) else {
                log::debug!("skin '{}' (and fallback) has no atlas for '{}'", skin.name, meta.name);
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
            for (set, id) in wanted {
                if let Some(s) = meta.sprites.get(id as usize) {
                    sink(set, SpriteKey { collection: col, sprite: id }, rasterize(s, &atlas, sets[set].1));
                }
            }
        }
        Ok(())
    }

    /// Replace this bank's frames (e.g. from [`Self::rasterize_sets`]).
    pub fn install_frames(&mut self, frames: HashMap<SpriteKey, Frame>, px_per_unit: f32) {
        self.install_store(FrameStore::pack(frames), px_per_unit);
    }

    /// Replace this bank's frames with already packed ones (the unpacked cache starts empty).
    pub fn install_store(&mut self, frames: FrameStore, px_per_unit: f32) {
        self.frames = frames;
        self.px_per_unit = px_per_unit;
    }

    /// The frame for `key`, unpacked on first use (see [`FrameStore`]).
    pub fn frame(&self, key: SpriteKey) -> Option<&Frame> {
        self.frames.get(&key)
    }

    pub fn frames(&self) -> &FrameStore {
        &self.frames
    }

    /// See [`FrameStore::evict_unused`].
    pub fn evict_unused(&mut self) -> usize {
        self.frames.evict_unused()
    }

    /// Bytes of frame memory held now (packed frames plus unpacked ones).
    pub fn frame_bytes(&self) -> usize {
        self.frames.bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skins_from_several_roots_first_root_wins_vanilla_first() {
        let d = std::env::temp_dir().join(format!("skong-core-skins-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        for (root, skin) in [("dev", "Zed"), ("dev", "Vanilla"), ("user", "vanilla"), ("user", "Abyss")] {
            std::fs::create_dir_all(d.join(root).join(skin)).unwrap();
            std::fs::write(d.join(root).join(skin).join("Knight.png"), b"").unwrap();
        }
        let skins = Skin::discover_all(&[d.join("dev"), d.join("user"), d.join("missing")]);
        let names: Vec<&str> = skins.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Vanilla", "Abyss", "Zed"]);
        assert_eq!(skins[0].dir, d.join("dev").join("Vanilla"));
        std::fs::remove_dir_all(&d).ok();
    }

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

    /// Pseudo-random atlas with varied alpha (including fully transparent texels).
    fn noise_atlas(w: u32, h: u32) -> Atlas {
        let mut s = 0x2545_f491u32;
        let rgba = (0..w * h * 4)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                if s.is_multiple_of(7) { 0 } else { (s >> 8) as u8 }
            })
            .collect();
        Atlas { width: w, height: h, rgba }
    }

    #[test]
    fn packed_frames_equal_eager_ones() {
        let atlas = noise_atlas(64, 64);
        let mut eager = HashMap::new();
        for (i, ppu) in [0.7f32, 1.0, 3.3, 9.0].into_iter().enumerate() {
            let f = rasterize(&quad([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]), &atlas, ppu * 16.0);
            eager.insert(SpriteKey { collection: 0, sprite: i as u32 }, f);
        }
        // Empty and fully transparent frames too.
        eager.insert(SpriteKey { collection: 1, sprite: 0 }, Frame::default());
        eager.insert(SpriteKey { collection: 1, sprite: 1 }, Frame { image: Image::new(3, 2), anchor: [1.5, 2.0] });
        let store = FrameStore::pack(eager.clone());
        assert_eq!(store.len(), eager.len());
        assert!(store.bytes() < store.unpacked_bytes(), "packing saves memory");
        for (k, f) in &eager {
            let g = store.get(k).expect("frame");
            assert_eq!((g.image.width, g.image.height, g.anchor), (f.image.width, f.image.height, f.anchor));
            assert_eq!(g.image.px, f.image.px, "{k:?}");
        }
        assert!(store.get(&SpriteKey { collection: 9, sprite: 0 }).is_none());
    }

    #[test]
    fn frames_unpack_once_and_evict_when_idle() {
        let atlas = noise_atlas(32, 32);
        let (a, b) = (SpriteKey { collection: 0, sprite: 0 }, SpriteKey { collection: 0, sprite: 1 });
        let mut store = FrameStore::default();
        store.insert(a, &rasterize(&quad([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]), &atlas, 20.0));
        store.insert(b, &rasterize(&quad([[1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [0.0, 1.0]]), &atlas, 20.0));
        let packed = store.bytes();
        assert_eq!(store.unpacked(), 0, "nothing unpacked up front");

        let p1 = store.get(&a).unwrap() as *const Frame;
        let p2 = store.get(&a).unwrap() as *const Frame;
        assert_eq!(p1, p2, "second request is a cache hit");
        assert_eq!(store.unpacked(), 1);
        assert!(store.bytes() > packed);

        // `a` was used since the last check: kept; then idle for a whole period: dropped.
        assert_eq!(store.evict_unused(), 0);
        assert_eq!(store.unpacked(), 1);
        store.get(&b);
        assert_eq!(store.evict_unused(), 1, "a idle for a period");
        assert_eq!(store.unpacked(), 1);
        assert_eq!(store.evict_unused(), 1, "b idle too");
        assert_eq!((store.unpacked(), store.bytes()), (0, packed));
        let eager = rasterize(&quad([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]), &atlas, 20.0);
        assert_eq!(store.get(&a).unwrap().image.px, eager.image.px, "unpacks again after eviction");
    }

    #[test]
    fn rescale_and_skin_change_reset_the_cache() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let mut bank = SpriteBank::load_metadata(&data).expect("load");
        // A tiny stand-in Knight atlas (UVs are normalized, so any size works).
        let dir = std::env::temp_dir().join(format!("skong-lazy-frames-{}", std::process::id()));
        let write_skin = |name: &str, seed_x: u32| -> Skin {
            let d = dir.join(name);
            std::fs::create_dir_all(&d).unwrap();
            let atlas = noise_atlas(128 + seed_x, 128);
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, atlas.width, atlas.height);
            e.set_color(png::ColorType::Rgba);
            e.write_header().unwrap().write_image_data(&atlas.rgba).unwrap();
            std::fs::write(d.join("Knight.png"), out).unwrap();
            Skin::from_dir(&d).unwrap()
        };
        let (s1, s2) = (write_skin("One", 0), write_skin("Two", 1));
        let ki = bank.collection_index("Knight").unwrap();
        let keys: Vec<SpriteKey> = (0..40).map(|i| SpriteKey { collection: ki, sprite: i }).collect();

        let eager = |skin: &Skin, ppu: f32| bank.rasterize_sets(skin, None, &[(&keys, ppu)]).unwrap().pop().unwrap();
        let (e1_small, e1_big, e2_big) = (eager(&s1, 38.4), eager(&s1, 64.0), eager(&s2, 64.0));

        bank.rasterize_skin(&s1, None, &keys, 38.4).unwrap();
        for k in &keys {
            assert_eq!(bank.frame(*k).unwrap().image.px, e1_small[k].image.px);
        }
        assert_eq!(bank.frames().unpacked(), keys.len());

        bank.rasterize_skin(&s1, None, &keys, 64.0).unwrap();
        assert_eq!((bank.frames().unpacked(), bank.px_per_unit), (0, 64.0), "rescale starts a fresh cache");
        for k in &keys {
            let f = bank.frame(*k).unwrap();
            assert_eq!((f.image.width, f.image.height), (e1_big[k].image.width, e1_big[k].image.height));
            assert_eq!(f.image.px, e1_big[k].image.px);
        }

        bank.rasterize_skin(&s2, None, &keys, 64.0).unwrap();
        assert_eq!(bank.frames().unpacked(), 0, "skin change starts a fresh cache");
        for k in &keys {
            assert_eq!(bank.frame(*k).unwrap().image.px, e2_big[k].image.px);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_knight_metadata_loads() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = SpriteBank::load_metadata(&data).expect("load");
        let k = bank.collection_index("Knight").expect("Knight collection");
        assert_eq!(bank.collections[k as usize].sprites.len(), 891);
    }
}
