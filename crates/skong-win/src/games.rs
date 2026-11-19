//! Is a fullscreen window a game? Games get the knight out of the way; other fullscreen
//! apps (F11 browsers, presentations) don't.
//!
//! Signals, strongest first:
//! * exclusive fullscreen (`SHQueryUserNotificationState` = D3D fullscreen);
//! * the executable is in Windows' own game list (Game Bar / Game DVR: the
//!   `HKCU\System\GameConfigStore\Children\*\MatchedExeFullPath` values);
//! * the executable lives under a game store / launcher library folder.
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW};
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::core::{PCWSTR, PWSTR, w};

use crate::desktop::query_exe_path;

/// Path fragments of game libraries (lowercase, backslash-separated).
const GAME_DIRS: &[&str] = &[
    r"\steamapps\common\",
    r"\epic games\",
    r"\gog games\",
    r"\gog galaxy\games\",
    r"\xboxgames\",
    r"\riot games\",
    r"\battle.net\",
    r"\ubisoft game launcher\games\",
    r"\ea games\",
    r"\origin games\",
    r"\itch\apps\",
    r"\amazon games\library\",
];

/// Heuristic on the executable path alone.
pub fn is_game_path(path: &str) -> bool {
    let p = path.to_lowercase().replace('/', "\\");
    GAME_DIRS.iter().any(|d| p.contains(d))
}

/// A game holds the display exclusively (the desktop isn't visible at all).
pub fn exclusive_fullscreen() -> bool {
    // SAFETY: plain query.
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN)
}

pub struct GameDetector {
    /// Lowercased executable paths Windows knows as games.
    known: HashSet<String>,
    loaded: Instant,
    by_pid: HashMap<u32, bool>,
}

impl Default for GameDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl GameDetector {
    pub fn new() -> Self {
        let known = load_game_config_store();
        log::info!("game detection: {} games known to Windows", known.len());
        Self { known, loaded: Instant::now(), by_pid: HashMap::new() }
    }

    /// Is process `pid` a game? Cached per pid; the Windows list is reloaded hourly.
    pub fn is_game(&mut self, pid: u32) -> bool {
        if self.loaded.elapsed() > Duration::from_secs(3600) {
            self.known = load_game_config_store();
            self.loaded = Instant::now();
            self.by_pid.clear();
        }
        if let Some(&g) = self.by_pid.get(&pid) {
            return g;
        }
        let g = query_exe_path(pid).is_some_and(|p| self.known.contains(&p.to_lowercase()) || is_game_path(&p));
        if self.by_pid.len() > 512 {
            self.by_pid.clear();
        }
        self.by_pid.insert(pid, g);
        g
    }
}

/// `MatchedExeFullPath` of every entry under `HKCU\System\GameConfigStore\Children`.
fn load_game_config_store() -> HashSet<String> {
    let mut out = HashSet::new();
    let mut root = HKEY::default();
    // SAFETY: registry reads into local buffers; the key is closed before returning.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, w!(r"System\GameConfigStore\Children"), Some(0), KEY_READ, &mut root).is_err() {
            return out;
        }
        let mut name = [0u16; 256];
        for i in 0.. {
            let mut len = name.len() as u32;
            if RegEnumKeyExW(root, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            let mut buf = [0u16; 1024];
            let mut bytes = (buf.len() * 2) as u32;
            let ok = RegGetValueW(root, PCWSTR(name.as_ptr()), w!("MatchedExeFullPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut bytes));
            if ok.is_ok() {
                let chars = (bytes as usize / 2).saturating_sub(1);
                out.insert(String::from_utf16_lossy(&buf[..chars.min(buf.len())]).to_lowercase());
            }
        }
        let _ = RegCloseKey(root);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_are_games() {
        assert!(is_game_path(r"E:\SteamLibrary\steamapps\common\Hollow Knight\hollow_knight.exe"));
        assert!(is_game_path(r"C:\Program Files\Epic Games\Fortnite\FortniteClient.exe"));
        assert!(is_game_path(r"C:/XboxGames/Halo/Content/halo.exe"));
        assert!(!is_game_path(r"C:\Program Files\Google\Chrome\Application\chrome.exe"));
        assert!(!is_game_path(r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe"));
    }
}
