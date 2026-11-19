//! Hero effects (superdash charge/burst/trail, slash arcs, wings, puffs…): definitions
//! from `data/effects.json` with the offsets and scales of the Knight prefab, plus the
//! geometry to place them around the knight.
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::anim::{ClipId, ClipLibrary};
use crate::assets::{Frame, SpriteBank, SpriteKey, read_json};
use crate::fxraster::transform_frame;
use crate::geom::Vec2;

#[derive(Debug, Deserialize)]
struct EffectsFile {
    effects: Vec<EffectDef>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EffectDef {
    pub name: String,
    #[serde(default = "knight_lib")]
    pub library: String,
    pub clip: String,
    /// Local position in game units (y up) relative to the hero origin; +x is behind a
    /// left-facing knight (HK sprites face left).
    pub pos: [f32; 2],
    /// Local scale; negative x mirrors the effect.
    pub scale: [f32; 2],
    /// Local z rotation in degrees (counter-clockwise, y up), applied after the scale.
    #[serde(default)]
    pub rot: f32,
    /// Follows the knight (true) or stays where it spawned.
    #[serde(default = "yes")]
    pub attached: bool,
}

fn knight_lib() -> String {
    "Knight".into()
}
fn yes() -> bool {
    true
}

impl EffectDef {
    /// Rotated or clearly non-uniform effects get their local transform baked into the
    /// frames ([`Self::bake`]); the others are drawn at a uniform scale.
    pub fn baked(&self) -> bool {
        let (sx, sy) = (self.scale[0].abs(), self.scale[1].abs());
        self.rot != 0.0 || (sx - sy).abs() > 0.05 * sx.max(sy)
    }

    /// Uniform rasterization scale (baked effects: the larger axis, then [`Self::bake`]).
    pub fn raster_scale(&self) -> f32 {
        let (sx, sy) = (self.scale[0].abs(), self.scale[1].abs());
        if self.baked() { sx.max(sy) } else { (sx + sy) * 0.5 }
    }

    /// Apply the remaining local transform (per-axis scale incl. mirroring, rotation) to a
    /// frame rasterized at [`Self::raster_scale`]. Identity for non-baked effects.
    pub fn bake(&self, f: &Frame) -> Frame {
        if !self.baked() {
            return f.clone();
        }
        let s = self.raster_scale().max(1e-6);
        transform_frame(f, self.scale[0] / s, self.scale[1] / s, self.rot)
    }

    /// Screen position of the effect's origin for a knight whose sprite origin is at
    /// `origin` (screen px, y down), at `px_per_unit`.
    pub fn place(&self, origin: Vec2, facing_right: bool, px_per_unit: f32) -> Vec2 {
        let dir = if facing_right { -1.0 } else { 1.0 };
        origin + Vec2::new(self.pos[0] * dir, -self.pos[1]) * px_per_unit
    }

    /// Whether the effect's (left-facing) art must be mirrored.
    pub fn mirrored(&self, facing_right: bool) -> bool {
        (self.scale[0] < 0.0 && !self.baked()) != facing_right
    }
}

/// All effect definitions with their clips resolved.
pub struct EffectSet {
    pub defs: Vec<EffectDef>,
    pub libs: Vec<ClipLibrary>,
    /// Per def: (library index, clip).
    pub clips: Vec<(usize, ClipId)>,
    by_name: HashMap<String, usize>,
}

impl EffectSet {
    /// Load `effects.json`, reading each referenced library from `<anim_dir>/<Library>.json`.
    pub fn load(effects_json: &Path, anim_dir: &Path, bank: &SpriteBank) -> Result<EffectSet> {
        let defs = read_json::<EffectsFile>(effects_json)?.effects;
        let mut libs: Vec<ClipLibrary> = Vec::new();
        let mut lib_idx: HashMap<String, usize> = HashMap::new();
        let mut clips = Vec::new();
        for d in &defs {
            let li = match lib_idx.get(&d.library) {
                Some(&i) => i,
                None => {
                    let path = anim_dir.join(format!("{}.json", d.library.replace(' ', "_")));
                    libs.push(ClipLibrary::load(&path, bank)?);
                    lib_idx.insert(d.library.clone(), libs.len() - 1);
                    libs.len() - 1
                }
            };
            let clip = libs[li].id(&d.clip).with_context(|| format!("effect '{}': no clip '{}' in {}", d.name, d.clip, d.library))?;
            clips.push((li, clip));
        }
        let by_name = defs.iter().enumerate().map(|(i, d)| (d.name.clone(), i)).collect();
        Ok(EffectSet { defs, libs, clips, by_name })
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    pub fn lib(&self, def: usize) -> &ClipLibrary {
        &self.libs[self.clips[def].0]
    }

    /// Sprites needed by one effect (for rasterizing at its scale).
    pub fn sprite_keys(&self, def: usize) -> Vec<SpriteKey> {
        let (li, clip) = self.clips[def];
        let mut v: Vec<SpriteKey> = self.libs[li].get(clip).frames.iter().filter_map(|f| f.sprite).collect();
        v.sort_by_key(|k| (k.collection, k.sprite));
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_effects_resolve() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let bank = SpriteBank::load_metadata(&data.join("hk")).expect("bank");
        let fx = EffectSet::load(&data.join("effects.json"), &data.join("hk/animations"), &bank).expect("effects.json must resolve");
        assert!(fx.id("sd_burst").is_some() && fx.id("wall_puff").is_some());
        assert!(!fx.sprite_keys(fx.id("sd_trail").expect("trail")).is_empty());
    }

    #[test]
    fn movement_effects_resolve_and_bake_where_needed() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let bank = SpriteBank::load_metadata(&data.join("hk")).expect("bank");
        let fx = EffectSet::load(&data.join("effects.json"), &data.join("hk/animations"), &bank).expect("effects.json must resolve");
        let def = |n: &str| &fx.defs[fx.id(n).unwrap_or_else(|| panic!("no effect {n}"))];
        for n in ["dash_burst", "dash_puff", "jump_puff", "wall_slash", "q_charge", "q_slam", "q_trail_2", "q_mega"] {
            assert!(!def(n).baked(), "{n}");
            assert!(!fx.sprite_keys(fx.id(n).unwrap()).is_empty(), "{n}");
        }
        for n in ["dash_burst_down", "shadow_burst_down", "hard_land_lines", "q_trail", "q_pillar_2", "sd_charge_wall"] {
            assert!(def(n).baked(), "{n}");
        }
        // Baked mirrors live in the frame: only the knight's facing flips the art.
        let d = def("dash_burst_down");
        assert!(!d.mirrored(false) && d.mirrored(true));
        assert_eq!(d.raster_scale(), 1.509);
        // The side-dash burst is left behind the knight (+x local = behind).
        let o = Vec2::new(0.0, 0.0);
        assert!(def("dash_burst").place(o, true, 64.0).x < 0.0, "behind a right-facing knight");
        assert!(def("dash_burst_down").place(o, true, 64.0).y < 0.0, "above the knight, pointing up the dash");
    }

    #[test]
    fn placement_mirrors_with_facing() {
        let d = EffectDef { name: "t".into(), library: "Knight".into(), clip: "c".into(), pos: [5.0, -0.5], scale: [3.0, 3.0], rot: 0.0, attached: false };
        let o = Vec2::new(100.0, 100.0);
        // Facing left: +x local is to the right on screen (behind him).
        assert_eq!(d.place(o, false, 10.0), Vec2::new(150.0, 105.0));
        assert_eq!(d.place(o, true, 10.0), Vec2::new(50.0, 105.0));
        assert!(!d.mirrored(false) && d.mirrored(true));
    }
}
