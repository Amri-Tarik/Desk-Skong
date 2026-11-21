//! Finding what Desk-Skong needs inside the game files: each tk2d sprite collection's atlas
//! texture, and the few Unity sprites used as props.
//!
//! A tk2d collection (`tk2dSpriteCollectionData`, a MonoBehaviour) is identified by its
//! GameObject's name, which is the `name` in `data/hk/collections/*.json`. Its atlas is
//! the Texture2D its `textures` array points to; rather than decoding the whole
//! MonoBehaviour (its layout lives in the game's scripts), the extractor looks for a
//! `PPtr<Texture2D>` array in its bytes that points at a texture with the expected name
//! and size, both known from the committed metadata.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::reader::{PPtr, Reader};
use crate::serialized::*;
use crate::sprite::{self, SpriteRect};
use crate::texture::{self, TextureInfo};

/// An atlas to extract.
#[derive(Clone, Debug)]
pub struct WantedAtlas {
    /// The collection's GameObject name ("Knight", "Grimm Cln").
    pub collection: String,
    /// Index in the collection's `textures` (0: the main atlas; sprites with tk2d
    /// `materialId` k use page k).
    pub page: u32,
    pub atlas_name: String,
    pub width: u32,
    pub height: u32,
    /// Output file name in the skin folder ("Knight.png", "Sprint.png").
    pub file: String,
}

/// A texture found in the game files.
pub struct Found {
    pub file: Arc<SerializedFile>,
    pub obj: ObjectInfo,
    pub tex: TextureInfo,
}

/// A prop sprite found in the game files: a rectangle of a texture.
pub struct FoundSprite {
    pub name: String,
    pub texture: Found,
    pub rect: SpriteRect,
}

#[derive(Default)]
pub struct Catalog {
    /// By output file ([`WantedAtlas::file`]).
    pub atlases: HashMap<String, Found>,
    pub sprites: HashMap<String, FoundSprite>,
}

/// The serialized files to search, most likely first: `resources.assets`, then every
/// `sharedassetsN.assets` in numeric order.
pub fn game_files(data: &Path) -> Result<Vec<PathBuf>> {
    let mut shared: Vec<(u32, PathBuf)> = std::fs::read_dir(data)
        .with_context(|| format!("read {}", data.display()))?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let n = name.strip_prefix("sharedassets")?.strip_suffix(".assets")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    shared.sort();
    let mut files = vec![data.join("resources.assets")];
    files.extend(shared.into_iter().map(|(_, p)| p));
    Ok(files)
}

/// Opened serialized files and resolved script names, shared across the search.
#[derive(Default)]
struct Files {
    open: HashMap<PathBuf, Option<Arc<SerializedFile>>>,
    scripts: HashMap<(PathBuf, i64), String>,
}

impl Files {
    fn get(&mut self, path: &Path) -> Option<Arc<SerializedFile>> {
        let key = path.to_path_buf();
        self.open
            .entry(key)
            .or_insert_with(|| match SerializedFile::open(path) {
                Ok(f) => Some(Arc::new(f)),
                Err(e) => {
                    log::debug!("skip {}: {e:#}", path.display());
                    None
                }
            })
            .clone()
    }

    /// The file a pointer leads into, and the object.
    fn resolve(&mut self, from: &Arc<SerializedFile>, p: PPtr) -> Option<(Arc<SerializedFile>, ObjectInfo)> {
        if p.is_null() {
            return None;
        }
        let file = if p.file == 0 { from.clone() } else { self.get(&from.external_path(p.file)?)? };
        let obj = file.object(p.path_id)?.clone();
        Some((file, obj))
    }

    /// Class name of a MonoBehaviour's script (`MonoScript.m_ClassName`).
    fn script_class(&mut self, from: &Arc<SerializedFile>, script: usize) -> Option<String> {
        let p = *from.scripts.get(script)?;
        let key = (if p.file == 0 { from.path.clone() } else { from.external_path(p.file)? }, p.path_id);
        if let Some(s) = self.scripts.get(&key) {
            return Some(s.clone());
        }
        let (file, obj) = self.resolve(from, p)?;
        let name = (obj.class_id == CLASS_MONO_SCRIPT).then(|| read_script_class(&file, &obj).ok()).flatten().unwrap_or_default();
        self.scripts.insert(key, name.clone());
        Some(name)
    }
}

/// `MonoScript`: m_Name, m_ExecutionOrder, m_PropertiesHash (16 bytes), m_ClassName.
fn read_script_class(sf: &SerializedFile, obj: &ObjectInfo) -> Result<String> {
    let bytes = sf.read(obj)?;
    let mut r = sf.reader(&bytes);
    r.string()?;
    r.skip(4 + 16)?;
    r.string()
}

/// `GameObject.m_Name`: after the component list and the layer.
fn game_object_name(sf: &SerializedFile, obj: &ObjectInfo) -> Result<String> {
    let bytes = sf.read(obj)?;
    let mut r = sf.reader(&bytes);
    let n = r.len(12)?;
    r.skip(n * 12 + 4)?;
    r.string()
}

/// Search the game files until everything wanted is found (or the files run out).
/// `progress(i, n)` is called per file.
pub fn search(data: &Path, atlases: &[WantedAtlas], sprites: &[String], progress: &mut dyn FnMut(usize, usize)) -> Result<Catalog> {
    let files = game_files(data)?;
    let mut fs = Files::default();
    let mut out = Catalog::default();
    let mut want_atlas: HashMap<&str, Vec<&WantedAtlas>> = HashMap::new();
    for a in atlases {
        want_atlas.entry(a.collection.as_str()).or_default().push(a);
    }
    for (i, path) in files.iter().enumerate() {
        progress(i, files.len());
        if out.atlases.len() == atlases.len() && sprites.iter().all(|s| out.sprites.contains_key(s)) {
            break;
        }
        let Some(sf) = fs.get(path) else { continue };
        for obj in &sf.objects {
            match obj.class_id {
                CLASS_MONO_BEHAVIOUR if out.atlases.len() < atlases.len() => {
                    let Some(script) = obj.script else { continue };
                    if fs.script_class(&sf, script).as_deref() != Some("tk2dSpriteCollectionData") {
                        continue;
                    }
                    if let Err(e) = find_atlas(&mut fs, &sf, obj, &want_atlas, &mut out) {
                        log::debug!("{} #{}: {e:#}", path.display(), obj.path_id);
                    }
                }
                CLASS_SPRITE if sprites.len() > out.sprites.len() => {
                    let Ok(name) = sf.object_name(obj) else { continue };
                    if !sprites.contains(&name) || out.sprites.contains_key(&name) {
                        continue;
                    }
                    match find_sprite(&mut fs, &sf, obj, &name) {
                        Ok(s) => {
                            out.sprites.insert(name, s);
                        }
                        Err(e) => log::warn!("sprite {name} in {}: {e:#}", path.display()),
                    }
                }
                _ => {}
            }
        }
        // Keep only what's needed open (each file holds a handle).
        let keep: Vec<PathBuf> = out.atlases.values().map(|f| f.file.path.clone()).chain(out.sprites.values().map(|s| s.texture.file.path.clone())).collect();
        fs.open.retain(|p, _| keep.contains(p) || p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("globalgamemanagers")));
    }
    progress(files.len(), files.len());
    Ok(out)
}

fn find_atlas(fs: &mut Files, sf: &Arc<SerializedFile>, obj: &ObjectInfo, want: &HashMap<&str, Vec<&WantedAtlas>>, out: &mut Catalog) -> Result<()> {
    let bytes = sf.read(obj)?;
    let go = sf.reader(&bytes).pptr()?;
    let (gf, gobj) = fs.resolve(sf, go).context("no GameObject")?;
    let name = game_object_name(&gf, &gobj)?;
    let Some(pages) = want.get(name.as_str()) else { return Ok(()) };
    let Some(main) = pages.iter().find(|w| w.page == 0) else { return Ok(()) };
    if out.atlases.contains_key(&main.file) {
        return Ok(());
    }
    // The texture `p` points to, if it is `w`'s.
    let texture = |fs: &mut Files, p: PPtr, w: &WantedAtlas| -> Option<Found> {
        if p.file < 0 || p.path_id <= 0 {
            return None;
        }
        let (tf, tobj) = fs.resolve(sf, p)?;
        let tex = (tobj.class_id == CLASS_TEXTURE2D).then(|| tf.read(&tobj).and_then(|b| texture::parse(&tf, &b)).ok()).flatten()?;
        (tex.name == w.atlas_name && (tex.width, tex.height) == (w.width, w.height)).then_some(Found { file: tf, obj: tobj, tex })
    };
    // `textures`: i32 count (1–8), then that many PPtrs (12 bytes each); the first is the
    // main atlas, page k the k-th.
    let mut off = 4;
    while off + 16 <= bytes.len() {
        let mut r = Reader::new(&bytes[off..], sf.big_endian);
        let count = r.i32()?;
        if (1..=8).contains(&count)
            && let Ok(p) = r.pptr()
            && let Some(found) = texture(fs, p, main)
        {
            out.atlases.insert(main.file.clone(), found);
            for w in pages.iter().filter(|w| w.page > 0 && (w.page as i32) < count) {
                let at = off + 4 + 12 * w.page as usize;
                let p = bytes.get(at..at + 12).and_then(|b| Reader::new(b, sf.big_endian).pptr().ok());
                match p.and_then(|p| texture(fs, p, w)) {
                    Some(found) => {
                        out.atlases.insert(w.file.clone(), found);
                    }
                    None => log::warn!("collection '{name}': page {} ('{}' {}x{}) not found", w.page, w.atlas_name, w.width, w.height),
                }
            }
            return Ok(());
        }
        off += 4;
    }
    anyhow::bail!("collection '{name}': no {}x{} '{}' texture referenced", main.width, main.height, main.atlas_name)
}

fn find_sprite(fs: &mut Files, sf: &Arc<SerializedFile>, obj: &ObjectInfo, name: &str) -> Result<FoundSprite> {
    let bytes = sf.read(obj)?;
    let s = sprite::parse(sf, &bytes)?;
    let (tex_ptr, rect) = match s.texture {
        Some(t) => (t, s.rect),
        None => {
            // Packed into a SpriteAtlas: its render data holds the texture and rectangle.
            let (af, aobj) = fs.resolve(sf, s.atlas).context("sprite has no texture and no atlas")?;
            let abytes = af.read(&aobj)?;
            let (t, rect) = sprite::atlas_render_data(&af, &abytes, &s.render_key)?.context("sprite not in its atlas")?;
            let (tf, tobj) = fs.resolve(&af, t).context("atlas texture missing")?;
            let tex = texture::parse(&tf, &tf.read(&tobj)?)?;
            return Ok(FoundSprite { name: name.to_string(), texture: Found { file: tf, obj: tobj, tex }, rect: SpriteRect { pivot: s.rect.pivot, pixels_per_unit: s.rect.pixels_per_unit, ..rect } });
        }
    };
    let (tf, tobj) = fs.resolve(sf, tex_ptr).context("sprite texture missing")?;
    let tex = texture::parse(&tf, &tf.read(&tobj)?)?;
    Ok(FoundSprite { name: name.to_string(), texture: Found { file: tf, obj: tobj, tex }, rect })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialized::build::{self, Obj};

    #[test]
    fn lists_resources_then_shared_assets_in_numeric_order() {
        let dir = std::env::temp_dir().join(format!("skong-extract-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["sharedassets10.assets", "sharedassets2.assets", "sharedassets2.assets.resS", "level1", "resources.assets"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let names: Vec<String> = game_files(&dir).unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["resources.assets", "sharedassets2.assets", "sharedassets10.assets"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A tiny game: a collection's GameObject, its MonoBehaviour (script in another file),
    /// a decoy texture with the same name but another size, and the real atlas.
    #[test]
    fn finds_a_collections_atlas_by_game_object_name_and_texture_pointer() {
        let dir = std::env::temp_dir().join(format!("skong-extract-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut script = build::string("tk2dSpriteCollectionData");
        script.extend(0i32.to_le_bytes());
        script.extend([0u8; 16]);
        script.extend(build::string("tk2dSpriteCollectionData"));
        let ggm = build::file("6000.0.61f1", &[Obj { path_id: 3, class_id: CLASS_MONO_SCRIPT, script: None, data: script }], &[], &[]);
        std::fs::write(dir.join("globalgamemanagers.assets"), ggm).unwrap();

        let mut go = 1i32.to_le_bytes().to_vec();
        go.extend(build::pptr(0, 20));
        go.extend(0u32.to_le_bytes());
        go.extend(build::string("Knight"));
        let mut mb = build::pptr(0, 10); // m_GameObject
        mb.extend([1, 0, 0, 0]);
        mb.extend(build::pptr(1, 3)); // m_Script
        mb.extend(build::string(""));
        mb.extend([7u8; 40]); // sprite definitions…
        mb.extend(1i32.to_le_bytes());
        mb.extend(build::pptr(0, 31)); // materials
        mb.extend(2i32.to_le_bytes());
        mb.extend(build::pptr(0, 30)); // textures: the atlas, then page 1 (the 512x512)
        mb.extend(build::pptr(0, 29));
        let body = |w, h| crate::texture::tests::texture_body("atlas0", w, h, 25, None, Some(("resources.assets.resS", 0, (w * h) as u32)));
        let objs = [
            Obj { path_id: 10, class_id: CLASS_GAME_OBJECT, script: None, data: go },
            Obj { path_id: 20, class_id: CLASS_MONO_BEHAVIOUR, script: Some(0), data: mb },
            Obj { path_id: 29, class_id: CLASS_TEXTURE2D, script: None, data: body(512, 512) },
            Obj { path_id: 30, class_id: CLASS_TEXTURE2D, script: None, data: body(1024, 512) },
        ];
        let res = build::file("6000.0.61f1", &objs, &[PPtr { file: 1, path_id: 3 }], &["globalgamemanagers.assets"]);
        std::fs::write(dir.join("resources.assets"), res).unwrap();

        let want = [WantedAtlas { collection: "Knight".into(), page: 0, atlas_name: "atlas0".into(), width: 1024, height: 512, file: "Knight.png".into() }];
        let mut calls = 0;
        let cat = search(&dir, &want, &[], &mut |_, _| calls += 1).unwrap();
        let found = &cat.atlases["Knight.png"];
        assert!(!cat.atlases.contains_key("Knight_1.png"), "page 1 only when wanted");
        assert_eq!(found.obj.path_id, 30);
        assert_eq!((found.tex.width, found.tex.height), (1024, 512));
        assert!(calls >= 2);

        // A second atlas page: the next pointer in `textures`.
        let paged = [want[0].clone(), WantedAtlas { page: 1, width: 512, height: 512, file: "Knight_1.png".into(), ..want[0].clone() }];
        let cat = search(&dir, &paged, &[], &mut |_, _| {}).unwrap();
        assert_eq!(cat.atlases["Knight.png"].obj.path_id, 30);
        assert_eq!(cat.atlases["Knight_1.png"].obj.path_id, 29);

        // Wrong size wanted: nothing found, no error.
        let want = [WantedAtlas { width: 2048, ..want[0].clone() }];
        assert!(search(&dir, &want, &[], &mut |_, _| {}).unwrap().atlases.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
