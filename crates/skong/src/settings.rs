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
    // (No click-through setting: he can always be picked up and dragged out of the way;
    // only his transparent pixels let clicks through. An old `click_through` is ignored.)
    /// Visible in screen shares and screenshots (otherwise excluded from capture).
    pub share_visible: bool,
    /// Curious about highlighted text (on by default; the tray / panel toggle turns it
    /// off): UI Automation tells the pet *where* a text selection is (a rectangle; the text
    /// itself is never read or stored).
    pub text_curiosity: bool,
    /// Launch the control panel together with the pet.
    pub open_panel: bool,
    /// Wallet and the Shade waiting where the knight last died.
    pub stakes: skong_core::stakes::StakesSave,
    /// Settings format (see [`Settings::migrate`]); files from before it read as 0.
    #[serde(default)]
    pub version: u32,
}

/// The current settings format.
const VERSION: u32 = 1;

impl Default for Settings {
    fn default() -> Self {
        Self { skin: None, scale: 0.6, bother: 0.5, share_visible: false, text_curiosity: true, open_panel: true, stakes: Default::default(), version: VERSION }
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
            Ok(s) => Self::parse(&s).unwrap_or_else(|e| {
                log::warn!("ignoring malformed {}: {e}", p.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    fn parse(json: &str) -> serde_json::Result<Settings> {
        let mut s: Settings = serde_json::from_str(json)?;
        s.migrate();
        Ok(s)
    }

    /// Bring an older file up to date. 0 → 1: highlighted-text curiosity became on by
    /// default (files saved before then hold the old default, off, not a choice).
    fn migrate(&mut self) {
        if self.version < 1 {
            self.text_curiosity = true;
        }
        self.version = VERSION;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_curiosity_is_on_by_default_and_old_files_are_migrated() {
        assert!(Settings::default().text_curiosity);
        // Saved before the change (old default off, an old click-through flag): on, and
        // the flag is gone.
        let old = Settings::parse(r#"{"scale":0.8,"click_through":true,"text_curiosity":false}"#).expect("old file");
        assert!(old.text_curiosity);
        assert_eq!(old.scale, 0.8);
        // A choice made since sticks.
        let off = Settings { text_curiosity: false, ..Settings::default() };
        let again = Settings::parse(&serde_json::to_string(&off).unwrap()).expect("new file");
        assert!(!again.text_curiosity);
    }
}
