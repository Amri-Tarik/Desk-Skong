//! User settings persisted to `%APPDATA%\Desk-Skong\settings.json`.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Skin folder name; `None` = vanilla / first found.
    pub skin: Option<String>,
    /// Render scale (1.0 = in-game size at 1080p).
    pub scale: f32,
    /// 0 = never bothers you, 1 = maximum mischief (cursor battles…).
    pub bother: f32,
    pub click_through: bool,
    /// Launch the control panel together with the pet.
    pub open_panel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { skin: None, scale: 0.6, bother: 0.5, click_through: false, open_panel: true }
    }
}

pub const SCALES: &[(&str, f32)] = &[("Tiny", 0.4), ("Small", 0.5), ("Normal", 0.6), ("Large", 0.8), ("Huge", 1.0)];
pub const BOTHER: &[(&str, f32)] = &[("Off", 0.0), ("Low", 0.25), ("Medium", 0.5), ("High", 1.0)];

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Desk-Skong").join("settings.json"))
}

impl Settings {
    pub fn load() -> Settings {
        let Some(p) = path() else { return Settings::default() };
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("ignoring malformed {}: {e}", p.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        let result = p
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&p, serde_json::to_string_pretty(self).unwrap_or_default()));
        if let Err(e) = result {
            log::warn!("could not save settings to {}: {e}", p.display());
        }
    }
}
