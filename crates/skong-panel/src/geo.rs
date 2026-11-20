//! The Geo icon beside the wallet: the very coin sprite the pet drops (`Geo` collection,
//! `coin0000`, the first frame of the small coin's spin), rasterized at runtime from the
//! player's extracted art — the dev `skins/` folder first, then
//! `%APPDATA%\Desk-Skong\skins` — with the metadata in `data/hk`. No art is ever bundled:
//! without it the panel draws a plain, pale Geo-like stone instead (`home.rs`).
use std::path::{Path, PathBuf};

use skong_core::assets::{Skin, SpriteBank, VANILLA};
use skong_core::canvas::Image;

/// The sprite drawn for the wallet.
pub const COLLECTION: &str = "Geo";
pub const SPRITE: &str = "coin0000";
/// Rasterized this large (px per world unit), then shown smaller: crisp on high-DPI.
const PX_PER_UNIT: f32 = 160.0;

/// `rel` (e.g. `data/hk`) next to this exe or up to three folders above it, then in the
/// working directory (as the pet finds its files).
fn locate(rel: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let near_exe = exe.iter().flat_map(|e| e.ancestors().skip(1).take(4)).map(|d| d.join(rel));
    let cwd = std::env::current_dir().ok().map(|d| d.join(rel));
    near_exe.chain(cwd).find(|p| p.exists())
}

/// Where skins are looked for: the dev folder, then the user's extracted art.
fn skin_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = locate("skins").into_iter().collect();
    roots.extend(std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Desk-Skong").join("skins")));
    roots
}

/// The Geo coin from the extracted art, if it's there.
pub fn load() -> Option<Image> {
    load_from(&locate("data/hk")?, &skin_roots())
}

/// The Geo coin rasterized from `data` (metadata) and the Vanilla art under `roots`.
pub fn load_from(data: &Path, roots: &[PathBuf]) -> Option<Image> {
    let bank = SpriteBank::load_metadata(data).ok()?;
    let key = bank.sprite_key(COLLECTION, SPRITE)?;
    let vanilla = Skin::discover_all(roots).into_iter().find(|s| s.name.eq_ignore_ascii_case(VANILLA))?;
    let mut out = None;
    bank.rasterize_each(&vanilla, None, &[(&[key], PX_PER_UNIT)], |_, _, f| out = Some(f.image)).ok()?;
    out.filter(|i| i.width > 0 && i.height > 0 && i.px.iter().any(|p| p >> 24 > 0))
}

/// Premultiplied ARGB pixels → premultiplied RGBA bytes (what egui textures take).
pub fn rgba_premultiplied(img: &Image) -> Vec<u8> {
    img.px.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_art_means_no_icon_not_a_crash() {
        let nowhere = std::env::temp_dir().join("skong-panel-no-such-folder");
        assert!(load_from(&nowhere, std::slice::from_ref(&nowhere)).is_none());
        // Real metadata but no skins: still nothing.
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        assert!(load_from(&data, &[nowhere]).is_none());
    }

    #[test]
    fn the_coin_sprite_exists_in_the_metadata() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = SpriteBank::load_metadata(&data).unwrap();
        assert!(bank.sprite_key(COLLECTION, SPRITE).is_some());
    }

    #[test]
    fn the_real_coin_when_the_art_is_extracted() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let roots = [root.join("skins")];
        if !Skin::discover_all(&roots).iter().any(|s| s.name.eq_ignore_ascii_case(VANILLA)) {
            eprintln!("skipped: no skins/Vanilla art in this checkout");
            return;
        }
        let img = load_from(&root.join("data/hk"), &roots).expect("Geo coin");
        assert!(img.width >= 16 && img.height >= 16, "{}x{}", img.width, img.height);
    }

    #[test]
    fn pixels_convert_to_premultiplied_rgba() {
        let img = Image { width: 1, height: 1, px: vec![0x80_40_20_10] };
        assert_eq!(rgba_premultiplied(&img), [0x40, 0x20, 0x10, 0x80]);
    }
}
