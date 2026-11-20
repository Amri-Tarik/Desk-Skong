//! Extract from a real Hollow Knight install into a temp folder and compare every image
//! with a reference export (the .NET exporter's `skins/Vanilla`). Needs the game, so it is
//! ignored by default:
//!
//! ```text
//! cargo test -p skong-extract --release -- --ignored --nocapture
//! ```
//! `SKONG_HK` (game folder) and `SKONG_REF` (reference skin folder) override the defaults.
use std::path::{Path, PathBuf};

use skong_core::assets::Atlas;

fn env_or(key: &str, default: &str) -> PathBuf {
    std::env::var_os(key).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "needs a Hollow Knight install"]
fn extracts_the_real_game_like_the_dotnet_exporter() {
    let game = env_or("SKONG_HK", "E:/SteamLibrary/steamapps/common/Hollow Knight");
    let reference = env_or("SKONG_REF", "E:/Data/Desk-Skong/skins/Vanilla");
    let data = skong_extract::install::data_dir(&game).expect("a Hollow Knight install");
    // What a player's first run would find on its own (not asserted: machine-specific).
    println!("auto-detected install: {:?}", skong_extract::install::find());
    let plan = skong_extract::plan(&repo().join("data/hk"), &repo().join("data/props.json")).unwrap();
    let out = std::env::temp_dir().join(format!("skong-extract-real-{}", std::process::id())).join("Vanilla");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();

    let report = skong_extract::extract(&data, &plan, &out, &|_| {}).unwrap();
    println!("Unity {}: {} files in {:.1} s; missing: {:?}", report.unity_version, report.written.len(), report.seconds, report.missing);
    println!("texture formats: {:?}", report.formats);
    assert!(report.missing.is_empty(), "missing {:?}", report.missing);

    let mut worst = 0u8;
    let mut compared = 0;
    for file in &report.written {
        let (ours, theirs) = (out.join(file), reference.join(file));
        if !theirs.exists() {
            println!("  {file}: no reference");
            continue;
        }
        let (a, b) = (Atlas::load_png(&ours).unwrap(), Atlas::load_png(&theirs).unwrap());
        assert_eq!((a.width, a.height), (b.width, b.height), "{file}: size");
        let mut diff_px = 0usize;
        let mut max = 0u8;
        for (p, q) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
            // Fully transparent pixels may carry any colour.
            if p[3] == 0 && q[3] == 0 {
                continue;
            }
            let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            if d > 0 {
                diff_px += 1;
                max = max.max(d);
            }
        }
        println!("  {file:<40} {}x{}: {} differing px, max channel diff {max}", a.width, a.height, diff_px);
        worst = worst.max(max);
        compared += 1;
    }
    println!("compared {compared} images; worst channel difference {worst}");
    assert!(compared > 0);
    assert!(worst <= 1, "pixels differ from the reference by up to {worst}");
    std::fs::remove_dir_all(out.parent().unwrap()).ok();
}
