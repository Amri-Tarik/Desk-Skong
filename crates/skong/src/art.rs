//! Where Desk-Skong's files are, and the first-run art setup.
//!
//! - Metadata (`data/hk`) and the dev `skins/` folder are found next to `skong.exe` (or a
//!   few folders above it, for `target/release` builds), then in the working directory,
//!   so a shortcut started from anywhere works.
//! - The game art a player extracts lives in `%APPDATA%\Desk-Skong\skins\Vanilla`, beside
//!   the settings: it survives unzipping a new version elsewhere, and the folder the zip
//!   was unpacked into may not be writable. Skins are looked up in the dev folder first,
//!   then there.
//! - No Vanilla art anywhere: the setup window (`skong-panel --setup`) runs first and the
//!   pet waits for it.
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use skong_core::assets::{Skin, VANILLA};

pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// A default folder such as `data/hk`: next to the exe or up to three folders above it,
/// then the working directory; if none exists, the one next to the exe.
pub fn locate(rel: &str) -> PathBuf {
    locate_in(rel, exe_dir().as_deref(), std::env::current_dir().ok().as_deref())
}

fn locate_in(rel: &str, exe_dir: Option<&Path>, cwd: Option<&Path>) -> PathBuf {
    let near_exe = exe_dir.into_iter().flat_map(|d| d.ancestors().take(4)).map(|d| d.join(rel));
    let in_cwd = cwd.map(|d| d.join(rel));
    near_exe.clone().chain(in_cwd).find(|p| p.exists()).or_else(|| exe_dir.map(|d| d.join(rel))).unwrap_or_else(|| PathBuf::from(rel))
}

/// Where extracted game art goes: `%APPDATA%\Desk-Skong\skins`.
pub fn user_skins() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("skins"))
}

/// Skin folders to search, in order: the dev / `--skins` folder, then the user's.
pub fn skin_roots(dev: &Path) -> Vec<PathBuf> {
    let mut roots = vec![dev.to_path_buf()];
    roots.extend(user_skins());
    roots
}

pub fn has_vanilla(roots: &[PathBuf]) -> bool {
    Skin::discover_all(roots).iter().any(|s| s.name.eq_ignore_ascii_case(VANILLA))
}

/// The Vanilla folder the setup writes.
fn setup_out() -> Option<PathBuf> {
    user_skins().map(|d| d.join(VANILLA))
}

/// The game install the art was last read from (recorded with the art).
pub fn recorded_game() -> Option<PathBuf> {
    skong_extract::recorded_game(&setup_out()?)
}

fn setup_command(data: &Path, game: Option<&Path>, again: bool) -> Result<std::process::Command> {
    let Some(exe) = exe_dir().map(|d| d.join("skong-panel.exe")).filter(|p| p.is_file()) else {
        bail!("skong-panel.exe (the art setup window) is missing next to skong.exe");
    };
    let Some(out) = setup_out() else { bail!("no %APPDATA% folder to keep the art in") };
    let mut c = std::process::Command::new(exe);
    c.arg("--setup").arg("--data").arg(std::path::absolute(data)?).arg("--out").arg(out);
    if let Some(g) = game {
        c.arg("--game").arg(g);
    }
    if again {
        c.arg("--again");
    }
    Ok(c)
}

/// First run: without Vanilla art, show the setup window and wait for it. `Ok(true)`: the
/// art is there, carry on; `Ok(false)`: the player closed the window first (quit quietly).
pub fn ensure(data: &Path, roots: &[PathBuf]) -> Result<bool> {
    if has_vanilla(roots) {
        return Ok(true);
    }
    log::info!("no Hollow Knight art yet: opening the setup window");
    let status = match setup_command(data, recorded_game().as_deref(), false) {
        Ok(mut c) => c.status()?,
        Err(e) => bail!(
            "the Hollow Knight art isn't set up yet ({e:#}). Desk-Skong reads it from your own copy of the game on first run; \
             developers can also export it into skins/Vanilla (see docs/DEVELOPING.md)"
        ),
    };
    if !status.success() {
        log::info!("art setup closed without extracting; exiting");
        return Ok(false);
    }
    if !has_vanilla(roots) {
        bail!("the art setup finished but no Vanilla art was found in {}", roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", "));
    }
    Ok(true)
}

/// "Re-extract game art": open the setup window without waiting (the pet keeps running).
pub fn reextract(data: &Path, game: Option<&Path>) {
    match setup_command(data, game, true).and_then(|mut c| Ok(c.spawn()?)) {
        Ok(child) => log::info!("art setup opened (pid {})", child.id()),
        Err(e) => log::warn!("could not open the art setup: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_data_next_to_the_exe_above_it_or_in_the_working_directory() {
        let d = std::env::temp_dir().join(format!("skong-art-locate-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        let (repo, other) = (d.join("repo"), d.join("elsewhere"));
        let exe = repo.join("target").join("release");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(other.join("data/hk")).unwrap();
        // Nothing near the exe: the working directory's.
        assert_eq!(locate_in("data/hk", Some(&exe), Some(&other)), other.join("data/hk"));
        // In the repo two levels up (a dev build started from a shortcut).
        std::fs::create_dir_all(repo.join("data/hk")).unwrap();
        assert_eq!(locate_in("data/hk", Some(&exe), Some(&other)), repo.join("data/hk"));
        // Next to the exe (the portable zip) wins.
        std::fs::create_dir_all(exe.join("data/hk")).unwrap();
        assert_eq!(locate_in("data/hk", Some(&exe), Some(&other)), exe.join("data/hk"));
        // Nowhere: next to the exe, for a clear "not found" message.
        assert_eq!(locate_in("skins", Some(&exe), Some(&other)), exe.join("skins"));
        assert_eq!(locate_in("skins", None, None), PathBuf::from("skins"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn vanilla_counts_in_any_root() {
        let d = std::env::temp_dir().join(format!("skong-art-vanilla-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(d.join("dev/Other")).unwrap();
        std::fs::write(d.join("dev/Other/Knight.png"), b"").unwrap();
        let roots = [d.join("dev"), d.join("user")];
        assert!(!has_vanilla(&roots), "a community skin alone isn't enough");
        std::fs::create_dir_all(d.join("user/Vanilla")).unwrap();
        std::fs::write(d.join("user/Vanilla/Knight.png"), b"").unwrap();
        assert!(has_vanilla(&roots));
        std::fs::remove_dir_all(&d).ok();
    }
}
