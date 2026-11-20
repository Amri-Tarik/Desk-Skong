//! Finding the player's Hollow Knight install: every Steam library, then GOG, Epic and
//! Xbox locations. Nothing here writes anywhere.
use std::path::{Path, PathBuf};

use crate::vdf;

pub const STEAM_APP_ID: &str = "367520";
pub const GOG_GAME_ID: &str = "1308320804";
const DATA: &str = "hollow_knight_Data";
const EXE: &str = "hollow_knight.exe";

/// The game's data folder (`…/hollow_knight_Data`) for a path the player may give: the
/// game folder, `hollow_knight.exe`, or the data folder itself. `None` unless it really
/// looks like Hollow Knight (its asset files and scripts are there).
pub fn data_dir(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_file() { path.parent()? } else { path };
    let candidates = [dir.join(DATA), dir.to_path_buf()];
    candidates.into_iter().find(|d| is_game_data(d))
}

fn is_game_data(d: &Path) -> bool {
    d.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(DATA))
        && d.join("resources.assets").is_file()
        && d.join("globalgamemanagers").is_file()
        && d.join("Managed").join("Assembly-CSharp.dll").is_file()
}

/// Where the game is, if it can be found without asking.
pub fn find() -> Option<PathBuf> {
    candidates().iter().find_map(|p| data_dir(p))
}

/// Every place worth checking, most likely first.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = steam_games(&steam_roots());
    out.extend(gog_dirs());
    out.extend(epic_dirs(&program_data().join("Epic/EpicGamesLauncher/Data/Manifests")));
    out.extend(drives().iter().map(|d| d.join("XboxGames/Hollow Knight/Content")));
    out.dedup();
    out
}

/// Steam install folders: the registry's, then the default.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for (hive, key, value) in [
        (Hive::CurrentUser, r"Software\Valve\Steam", "SteamPath"),
        (Hive::LocalMachine, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (Hive::LocalMachine, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ] {
        if let Some(p) = registry_string(hive, key, value) {
            roots.push(PathBuf::from(p));
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    let mut seen: Vec<String> = Vec::new();
    roots.retain(|r| {
        let k = r.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
        let new = !seen.contains(&k);
        seen.push(k);
        new
    });
    roots
}

/// The game folder in each Steam library listed by the given Steam installs
/// (`steamapps/libraryfolders.vdf`), libraries that list the game first.
pub fn steam_games(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut with_game = Vec::new();
    let mut others = Vec::new();
    for root in roots {
        others.push(root.clone());
        for vdf_path in [root.join("steamapps/libraryfolders.vdf"), root.join("config/libraryfolders.vdf")] {
            let Some(doc) = std::fs::read_to_string(&vdf_path).ok().and_then(|t| vdf::parse(&t).ok()) else { continue };
            let Some(libs) = doc.get("libraryfolders") else { continue };
            for (key, lib) in libs.entries() {
                let (path, has_game) = match lib {
                    vdf::Vdf::Str(p) if key.chars().all(|c| c.is_ascii_digit()) => (p.clone(), false),
                    vdf::Vdf::Obj(_) => {
                        let Some(p) = lib.get("path").and_then(|p| p.str()) else { continue };
                        (p.to_string(), lib.get("apps").and_then(|a| a.get(STEAM_APP_ID)).is_some())
                    }
                    _ => continue,
                };
                if has_game { &mut with_game } else { &mut others }.push(PathBuf::from(path));
            }
        }
    }
    with_game.extend(others);
    let mut games = Vec::new();
    for lib in with_game {
        let apps = lib.join("steamapps");
        let dir = std::fs::read_to_string(apps.join(format!("appmanifest_{STEAM_APP_ID}.acf")))
            .ok()
            .and_then(|t| vdf::parse(&t).ok())
            .and_then(|m| m.get("AppState")?.get("installdir")?.str().map(str::to_string))
            .unwrap_or_else(|| "Hollow Knight".to_string());
        let game = apps.join("common").join(dir);
        if !games.contains(&game) {
            games.push(game);
        }
    }
    games
}

fn gog_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for key in [format!(r"SOFTWARE\WOW6432Node\GOG.com\Games\{GOG_GAME_ID}"), format!(r"SOFTWARE\GOG.com\Games\{GOG_GAME_ID}")] {
        if let Some(p) = registry_string(Hive::LocalMachine, &key, "path") {
            out.push(PathBuf::from(p));
        }
    }
    out.push(PathBuf::from(r"C:\GOG Games\Hollow Knight"));
    out.push(PathBuf::from(r"C:\Program Files (x86)\GOG Galaxy\Games\Hollow Knight"));
    out
}

/// Install folders of Epic manifests (`*.item`, JSON) whose name mentions the game.
pub fn epic_dirs(manifests: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(manifests) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("item")))
        .filter_map(|e| serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .filter(|m| m["DisplayName"].as_str().is_some_and(|n| n.to_lowercase().contains("hollow knight")))
        .filter_map(|m| m["InstallLocation"].as_str().map(PathBuf::from))
        .collect()
}

fn program_data() -> PathBuf {
    std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
}

fn drives() -> Vec<PathBuf> {
    (b'C'..=b'Z').map(|d| PathBuf::from(format!("{}:\\", d as char))).filter(|p| p.exists()).collect()
}

/// The game's exe, for a data folder.
pub fn exe_of(data: &Path) -> PathBuf {
    data.parent().map_or_else(|| data.join(EXE), |d| d.join(EXE))
}

#[derive(Clone, Copy)]
enum Hive {
    CurrentUser,
    LocalMachine,
}

#[cfg(windows)]
fn registry_string(hive: Hive, key: &str, value: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::HSTRING;
    let root = match hive {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    };
    let (k, v) = (HSTRING::from(key), HSTRING::from(value));
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its byte length are valid for the call.
    let r = unsafe { RegGetValueW(root, &k, &v, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_end_matches('\0');
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(not(windows))]
fn registry_string(_: Hive, _: &str, _: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("skong-extract-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_game(dir: &Path) {
        let data = dir.join(DATA);
        std::fs::create_dir_all(data.join("Managed")).unwrap();
        for f in ["resources.assets", "globalgamemanagers", "Managed/Assembly-CSharp.dll"] {
            std::fs::write(data.join(f), b"").unwrap();
        }
        std::fs::write(dir.join(EXE), b"").unwrap();
    }

    #[test]
    fn accepts_the_folder_the_exe_or_the_data_folder_and_nothing_else() {
        let d = temp("data-dir");
        let game = d.join("Hollow Knight");
        fake_game(&game);
        let want = game.join(DATA);
        assert_eq!(data_dir(&game), Some(want.clone()));
        assert_eq!(data_dir(&game.join(EXE)), Some(want.clone()));
        assert_eq!(data_dir(&want), Some(want.clone()));
        assert_eq!(exe_of(&want), game.join(EXE));
        // Another game's folder, or a partial copy, isn't it.
        std::fs::remove_file(want.join("Managed/Assembly-CSharp.dll")).unwrap();
        assert_eq!(data_dir(&game), None);
        assert_eq!(data_dir(&d), None);
        assert_eq!(data_dir(&d.join("missing")), None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn finds_the_game_in_a_secondary_steam_library() {
        let d = temp("steam");
        let (steam, lib) = (d.join("Steam"), d.join("SteamLibrary"));
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        let vdf = format!(
            "\"libraryfolders\"\n{{\n\"0\" {{ \"path\" \"{}\" \"apps\" {{ \"228980\" \"1\" }} }}\n\"1\" {{ \"path\" \"{}\" \"apps\" {{ \"367520\" \"1\" }} }}\n}}",
            steam.display().to_string().replace('\\', "\\\\"),
            lib.display().to_string().replace('\\', "\\\\")
        );
        std::fs::write(steam.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        let game = lib.join("steamapps/common/Hollow Knight");
        fake_game(&game);

        let found = steam_games(std::slice::from_ref(&steam));
        assert_eq!(found.first(), Some(&game), "the library listing the game comes first: {found:?}");
        assert_eq!(found.iter().find_map(|p| data_dir(p)), Some(game.join(DATA)));

        // A custom install folder name from the app manifest.
        std::fs::write(lib.join("steamapps/appmanifest_367520.acf"), "\"AppState\" { \"appid\" \"367520\" \"installdir\" \"HK\" }").unwrap();
        assert_eq!(steam_games(std::slice::from_ref(&steam)).first(), Some(&lib.join("steamapps/common/HK")));
        // No vdf: the Steam folder itself is still checked.
        assert_eq!(steam_games(&[d.join("Other")]), vec![d.join("Other/steamapps/common/Hollow Knight")]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reads_epic_manifests() {
        let d = temp("epic");
        std::fs::write(d.join("a.item"), r#"{"DisplayName":"Hollow Knight","InstallLocation":"D:\\Epic\\HollowKnight"}"#).unwrap();
        std::fs::write(d.join("b.item"), r#"{"DisplayName":"Other","InstallLocation":"D:\\Epic\\Other"}"#).unwrap();
        std::fs::write(d.join("c.item"), "not json").unwrap();
        assert_eq!(epic_dirs(&d), vec![PathBuf::from(r"D:\Epic\HollowKnight")]);
        assert!(epic_dirs(&d.join("missing")).is_empty());
        std::fs::remove_dir_all(&d).ok();
    }
}
