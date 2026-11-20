//! User settings persisted to `%APPDATA%\Desk-Skong\settings.json`.
//!
//! Updates keep them: a missing field takes its default, a field this version doesn't know
//! (written by a newer one) is kept and written back ([`Settings::extra`]), saves are atomic
//! and a malformed file is kept as `settings.json.bak` before starting fresh
//! ([`skong_core::persist`]).
use std::path::{Path, PathBuf};

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
    /// "Lantern on dark backgrounds" (on by default): he holds up his lantern while
    /// standing over dark-mode windows or a dark wallpaper. Off: it never comes out.
    pub lantern: bool,
    /// Launch the control panel together with the pet.
    pub open_panel: bool,
    /// Wallet and the Shade waiting where the knight last died.
    pub stakes: skong_core::stakes::StakesSave,
    /// "Write a log file (for bug reports)" (off by default): the log also goes to
    /// `%APPDATA%\Desk-Skong\logs\skong-<date>.log` ([`crate::logring`]).
    pub log_file: bool,
    /// The Hollow Knight data folder (`…\hollow_knight_Data`) the art was last read from,
    /// tried first by "Re-extract game art". Taken from the extracted art's record on start.
    pub game_dir: Option<String>,
    /// Seconds the pet has been running, over every session (a statistic, not a score:
    /// it keeps counting in the sandbox). Shown on the panel's Home tab.
    pub played_secs: f64,
    /// Settings format (see [`Settings::migrate`]); files from before it read as 0.
    #[serde(default)]
    pub version: u32,
    /// Fields this version doesn't know (a newer one wrote them): kept as they are, so
    /// going back to an older release and forward again loses nothing.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The current settings format.
const VERSION: u32 = 1;

impl Default for Settings {
    fn default() -> Self {
        Self { skin: None, scale: 0.6, bother: 0.5, share_visible: false, text_curiosity: true, lantern: true, open_panel: true, stakes: Default::default(), log_file: false, game_dir: None, played_secs: 0.0, version: VERSION, extra: Default::default() }
    }
}

pub const SCALES: &[(&str, f32)] = &[("Tiny", 0.4), ("Small", 0.5), ("Normal", 0.6), ("Large", 0.8), ("Huge", 1.0)];
pub const BOTHER: &[(&str, f32)] = &[("Off", 0.0), ("Low", 0.25), ("Medium", 0.5), ("High", 1.0)];

/// The pet's own folder, `%APPDATA%\Desk-Skong` (settings, world, achievements, logs).
pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Desk-Skong"))
}

fn path() -> Option<PathBuf> {
    app_dir().map(|d| d.join("settings.json"))
}

/// Fields of earlier versions that are gone for good (dropped rather than kept as
/// [`Settings::extra`]).
const OBSOLETE: &[&str] = &["click_through"];

impl Settings {
    pub fn load() -> Settings {
        path().map_or_else(Settings::default, |p| Self::load_from(&p))
    }

    /// Whether the saved settings ask for a log file: read early, before [`Settings::load`]
    /// (which handles a malformed file; this only says no).
    pub fn log_file_wanted() -> bool {
        path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| Self::parse(&s).ok()).is_some_and(|s| s.log_file)
    }

    /// Missing → defaults; malformed → kept as `<file>.bak`, defaults.
    fn load_from(path: &Path) -> Settings {
        skong_core::persist::load_or_fresh(path, Self::parse, Settings::default)
    }

    fn parse(json: &str) -> serde_json::Result<Settings> {
        let mut s: Settings = serde_json::from_str(json)?;
        s.migrate();
        Ok(s)
    }

    /// Bring an older file up to date. 0 → 1: highlighted-text curiosity became on by
    /// default (files saved before then hold the old default, off, not a choice). A file
    /// from a newer version keeps its (higher) version number.
    fn migrate(&mut self) {
        if self.version < 1 {
            self.text_curiosity = true;
        }
        for k in OBSOLETE {
            self.extra.remove(*k);
        }
        self.version = self.version.max(VERSION);
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Err(e) = self.save_to(&p) {
            log::warn!("could not save settings to {}: {e}", p.display());
        }
    }

    /// Atomically: a crash mid-save keeps the previous file whole.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        skong_core::persist::write_atomic(path, json.as_bytes())
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

    #[test]
    fn the_lantern_is_on_by_default_and_the_choice_sticks() {
        assert!(Settings::default().lantern);
        // Files from before the setting: on.
        assert!(Settings::parse(r#"{"scale":0.6,"version":1}"#).expect("old file").lantern);
        let off = Settings { lantern: false, ..Settings::default() };
        let again = Settings::parse(&serde_json::to_string(&off).unwrap()).expect("round trip");
        assert!(!again.lantern);
        assert_eq!(again, off);
    }

    #[test]
    fn older_files_load_and_newer_fields_survive_a_round_trip() {
        // A 0.2-era file (before the log-file switch): everything kept, the new field off.
        let old = r#"{"skin":"Vanilla","scale":0.8,"bother":0.25,"share_visible":true,"text_curiosity":false,
            "lantern":false,"open_panel":false,"stakes":{"geo":1234,"shade":null,"deaths":3,"shades_defeated":2},"version":1}"#;
        let s = Settings::parse(old).expect("old file");
        assert_eq!((s.skin.as_deref(), s.scale, s.bother), (Some("Vanilla"), 0.8, 0.25));
        assert!(s.share_visible && !s.text_curiosity && !s.lantern && !s.open_panel && !s.log_file);
        assert_eq!((s.stakes.geo, s.stakes.deaths, s.stakes.shades_defeated), (1234, 3, 2));
        assert!(s.extra.is_empty());
        // A file from a newer release: its unknown fields and version are written back as
        // they were, known ones still read.
        let newer = r#"{"scale":0.5,"log_file":true,"version":7,"future_switch":true,"future_map":{"a":[1,2]}}"#;
        let s = Settings::parse(newer).expect("newer file");
        assert!(s.log_file && s.scale == 0.5 && s.version == 7);
        let back: serde_json::Value = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back["future_switch"], true);
        assert_eq!(back["future_map"]["a"][1], 2);
        assert_eq!(back["version"], 7);
        // Gone-for-good fields are dropped, not carried along.
        assert!(!Settings::parse(r#"{"click_through":true}"#).unwrap().extra.contains_key("click_through"));
        // Defaults: the log file is opt-in.
        assert!(!Settings::default().log_file);
    }

    #[test]
    fn settings_save_atomically_and_a_malformed_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("skong-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default(), "missing: defaults");
        let mine = Settings { scale: 1.0, log_file: true, ..Settings::default() };
        mine.save_to(&path).expect("save");
        mine.save_to(&path).expect("save again");
        assert_eq!(Settings::load_from(&path), mine);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file left");
        std::fs::write(&path, r#"{"scale": 1.0, "stakes": {"geo": 99"#).unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default(), "malformed: defaults");
        assert!(std::fs::read_to_string(dir.join("settings.json.bak")).unwrap().contains("\"geo\": 99"), "the wallet is kept in the backup");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
