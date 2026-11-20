//! The whole job: what to extract (from Desk-Skong's committed metadata), finding it in
//! the game files, decoding, and writing the Vanilla skin folder.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::catalog::{self, Found, WantedAtlas};
use crate::{sprite, texture};

/// Marker written last into a complete extraction.
pub const MARKER: &str = "extracted.json";

/// What Desk-Skong draws from the game.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub atlases: Vec<WantedAtlas>,
    /// Unity sprite names, written to `props/<name>.png` + `.json`.
    pub sprites: Vec<String>,
}

#[derive(Deserialize)]
struct CollectionHead {
    name: String,
    #[serde(default)]
    atlas_name: String,
    #[serde(default)]
    atlas_width: u32,
    #[serde(default)]
    atlas_height: u32,
}

/// The plan from `data/hk/collections/*.json` and `data/props.json`.
pub fn plan(hk_data: &Path, props_json: &Path) -> Result<Plan> {
    let dir = hk_data.join("collections");
    let mut atlases = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("read {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        let c: CollectionHead = serde_json::from_str(&text).with_context(|| format!("parse {}", p.display()))?;
        if c.atlas_name.is_empty() || c.atlas_width == 0 {
            continue;
        }
        let file = match skong_core::assets::SKIN_FILES.iter().find(|(n, _)| *n == c.name) {
            Some((_, f)) => f.to_string(),
            None => format!("{}.png", c.name.replace(' ', "_")),
        };
        atlases.push(WantedAtlas { collection: c.name, atlas_name: c.atlas_name, width: c.atlas_width, height: c.atlas_height, file });
    }
    let sprites = match std::fs::read_to_string(props_json) {
        Ok(text) => prop_sprites(&serde_json::from_str(&text).with_context(|| format!("parse {}", props_json.display()))?),
        Err(_) => Vec::new(),
    };
    Ok(Plan { atlases, sprites })
}

/// Sprite names in `props.json`: every bench's `sprite` and the discoveries' art.
fn prop_sprites(v: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in v["benches"].as_array().into_iter().flatten() {
        if let Some(s) = b["sprite"].as_str() {
            out.push(s.to_string());
        }
    }
    for (k, s) in v["discoveries"].as_object().into_iter().flatten() {
        if let (false, Some(s)) = (k.starts_with('$'), s.as_str()) {
            out.push(s.to_string());
        }
    }
    out.dedup();
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// Looking through the game's files.
    Searching { file: usize, files: usize },
    /// Decoding and saving images.
    Writing { done: usize, total: usize },
}

impl Progress {
    /// Overall 0..1 (searching is the first half).
    pub fn fraction(self) -> f32 {
        match self {
            Progress::Searching { file, files } => 0.5 * file as f32 / files.max(1) as f32,
            Progress::Writing { done, total } => 0.5 + 0.5 * done as f32 / total.max(1) as f32,
        }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub unity_version: String,
    /// Files written into the skin folder.
    pub written: Vec<String>,
    /// Things the game files didn't have (Desk-Skong draws stand-ins for them).
    pub missing: Vec<String>,
    pub seconds: f32,
    /// Texture formats read, with how many textures used each.
    pub formats: BTreeMap<String, usize>,
}

/// Extract everything in `plan` from the game at `game_data` (`…/hollow_knight_Data`)
/// into `out` (the Vanilla skin folder). Writes into `<out>.partial` and swaps it in at
/// the end, so an interrupted run never leaves a half-made skin behind. Reads the game
/// files only.
pub fn extract(game_data: &Path, plan: &Plan, out: &Path, progress: &(dyn Fn(Progress) + Sync)) -> Result<Report> {
    let t0 = Instant::now();
    let cat = catalog::search(game_data, &plan.atlases, &plan.sprites, &mut |file, files| progress(Progress::Searching { file, files }))?;
    let Some(knight) = cat.atlases.get("Knight") else {
        bail!("the knight's sprites weren't found in {}", game_data.display())
    };
    let unity_version = knight.file.unity_version.clone();
    let mut formats = BTreeMap::new();
    for t in cat.atlases.values().map(|f| &f.tex).chain(cat.sprites.values().map(|s| &s.texture.tex)) {
        *formats.entry(texture::format_name(t.format)).or_insert(0) += 1;
    }

    let mut missing: Vec<String> = plan.atlases.iter().filter(|a| !cat.atlases.contains_key(&a.collection)).map(|a| a.collection.clone()).collect();
    missing.extend(plan.sprites.iter().filter(|s| !cat.sprites.contains_key(*s)).cloned());

    enum Job<'a> {
        Atlas(&'a Found, &'a str),
        Sprite(&'a catalog::FoundSprite),
    }
    let mut jobs: Vec<Job> = plan.atlases.iter().filter_map(|a| cat.atlases.get(&a.collection).map(|f| Job::Atlas(f, a.file.as_str()))).collect();
    jobs.extend(plan.sprites.iter().filter_map(|s| cat.sprites.get(s)).map(Job::Sprite));
    // Biggest first, so the threads finish together.
    jobs.sort_by_key(|j| std::cmp::Reverse(match j {
        Job::Atlas(f, _) => f.tex.image_size,
        Job::Sprite(s) => s.texture.tex.image_size / 64,
    }));

    let partial = sibling(out, ".partial");
    if partial.exists() {
        std::fs::remove_dir_all(&partial).with_context(|| format!("remove {}", partial.display()))?;
    }
    std::fs::create_dir_all(partial.join("props")).with_context(|| format!("create {}", partial.display()))?;

    let total = jobs.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let written = Mutex::new(Vec::new());
    let failure: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    progress(Progress::Writing { done: 0, total });
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total || failure.lock().is_ok_and(|f| f.is_some()) {
                        break;
                    }
                    let result = match &jobs[i] {
                        Job::Atlas(f, file) => write_atlas(f, &partial.join(file)).map(|_| file.to_string()),
                        Job::Sprite(sp) => write_sprite(sp, &partial.join("props")).map(|_| format!("props/{}.png", sp.name)),
                    };
                    match result {
                        Ok(name) => written.lock().unwrap_or_else(|e| e.into_inner()).push(name),
                        Err(e) => {
                            failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(e);
                        }
                    }
                    progress(Progress::Writing { done: done.fetch_add(1, Ordering::Relaxed) + 1, total });
                }
            });
        }
    });
    if let Some(e) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        std::fs::remove_dir_all(&partial).ok();
        return Err(e);
    }
    let mut written = written.into_inner().unwrap_or_else(|e| e.into_inner());
    written.sort();

    let marker = serde_json::json!({
        "game_data": game_data.display().to_string(),
        "game_unity_version": unity_version,
        "extractor_version": env!("CARGO_PKG_VERSION"),
        "files": written.len(),
        "missing": missing,
        "note": "Extracted from your own copy of Hollow Knight, for Desk-Skong on this PC only. Do not redistribute.",
    });
    std::fs::write(partial.join(MARKER), serde_json::to_string_pretty(&marker)?)?;
    replace_dir(&partial, out)?;
    Ok(Report { unity_version, written, missing, seconds: t0.elapsed().as_secs_f32(), formats })
}

/// `out` with `suffix` appended to its last component.
fn sibling(out: &Path, suffix: &str) -> PathBuf {
    let mut name = out.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    out.with_file_name(name)
}

/// Swap `new` in for `out`: the old folder is renamed aside first and removed after.
fn replace_dir(new: &Path, out: &Path) -> Result<()> {
    let old = sibling(out, ".old");
    if old.exists() {
        std::fs::remove_dir_all(&old).with_context(|| format!("remove {}", old.display()))?;
    }
    if out.exists() {
        std::fs::rename(out, &old).with_context(|| format!("move aside {}", out.display()))?;
    }
    if let Err(e) = std::fs::rename(new, out) {
        if old.exists() {
            std::fs::rename(&old, out).ok();
        }
        return Err(e).with_context(|| format!("move {} into place", new.display()));
    }
    if old.exists() {
        std::fs::remove_dir_all(&old).ok();
    }
    Ok(())
}

fn decode(f: &Found) -> Result<Vec<u8>> {
    let raw = texture::read_pixels(&f.file, &f.obj, &f.tex)?;
    texture::decode_top_down(&f.tex, &raw)
}

fn write_atlas(f: &Found, path: &Path) -> Result<()> {
    let rgba = decode(f)?;
    write_png(path, f.tex.width, f.tex.height, &rgba)
}

fn write_sprite(s: &catalog::FoundSprite, dir: &Path) -> Result<()> {
    let tex = &s.texture.tex;
    let rgba = decode(&s.texture)?;
    let px = sprite::crop(&rgba, tex.width, tex.height, &s.rect).with_context(|| format!("sprite {}", s.name))?;
    write_png(&dir.join(format!("{}.png", s.name)), s.rect.width as u32, s.rect.height as u32, &px)?;
    let meta = serde_json::json!({
        "name": s.name, "width": s.rect.width, "height": s.rect.height,
        "pivot": s.rect.pivot, "pixels_per_unit": s.rect.pixels_per_unit,
    });
    std::fs::write(dir.join(format!("{}.json", s.name)), meta.to_string())?;
    Ok(())
}

pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut wr = enc.write_header()?;
    wr.write_image_data(rgba).with_context(|| format!("write {}", path.display()))?;
    wr.finish()?;
    Ok(())
}

/// What a finished extraction recorded, if `dir` holds one.
pub fn marker(dir: &Path) -> Option<BTreeMap<String, serde_json::Value>> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(MARKER)).ok()?).ok()
}

/// The game data folder a finished extraction in `dir` was read from.
pub fn recorded_game(dir: &Path) -> Option<PathBuf> {
    marker(dir)?.get("game_data")?.as_str().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_reads_collections_and_prop_sprites() {
        let dir = std::env::temp_dir().join(format!("skong-extract-plan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("hk/collections")).unwrap();
        std::fs::write(dir.join("hk/collections/Knight.json"), r#"{"name":"Knight","atlas_name":"atlas0","atlas_width":4096,"atlas_height":4096,"sprites":[]}"#).unwrap();
        std::fs::write(dir.join("hk/collections/Grimm_Cln.json"), r#"{"name":"Grimm Cln","atlas_name":"atlas0","atlas_width":64,"atlas_height":32}"#).unwrap();
        std::fs::write(dir.join("hk/collections/Knight_Slug_Cln.json"), r#"{"name":"Knight Slug Cln","atlas_name":"atlas0","atlas_width":64,"atlas_height":32}"#).unwrap();
        std::fs::write(dir.join("props.json"), r#"{"benches":[{"sprite":"town_bench"},{"name":"x"}],"discoveries":{"$comment":"no","deposit_rock":"mines_small_rock"}}"#).unwrap();
        let p = plan(&dir.join("hk"), &dir.join("props.json")).unwrap();
        let files: Vec<(&str, &str)> = p.atlases.iter().map(|a| (a.collection.as_str(), a.file.as_str())).collect();
        assert_eq!(files, [("Grimm Cln", "Grimm_Cln.png"), ("Knight", "Knight.png"), ("Knight Slug Cln", "Unn.png")]);
        assert_eq!(p.sprites, ["town_bench", "mines_small_rock"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replacing_a_folder_keeps_only_the_new_one() {
        let dir = std::env::temp_dir().join(format!("skong-extract-swap-{}", std::process::id()));
        let (out, new) = (dir.join("Vanilla"), dir.join("Vanilla.partial"));
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(out.join("old.png"), b"o").unwrap();
        std::fs::write(new.join("Knight.png"), b"n").unwrap();
        replace_dir(&new, &out).unwrap();
        assert!(out.join("Knight.png").exists() && !out.join("old.png").exists());
        assert!(!new.exists() && !dir.join("Vanilla.old").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn progress_fraction_is_monotonic_across_stages() {
        let a = Progress::Searching { file: 9, files: 10 }.fraction();
        let b = Progress::Writing { done: 0, total: 3 }.fraction();
        let c = Progress::Writing { done: 3, total: 3 }.fraction();
        assert!(a < b && b < c && (c - 1.0).abs() < 1e-6);
    }
}
