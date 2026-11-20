//! Desk-Skong: a Hollow Knight desktop pet that lives on your windows.
// Release builds open no console window (players start it from Explorer); its log goes to
// the panel, and to a file if the user turns that on ([`logfile`]). Debug builds keep it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

mod app;
mod logfile;
mod control;
mod logring;
mod overlay;
mod benches;
mod bubble;
mod npcs;
mod enemies;
mod grimm;
mod discoveries;
mod shinies;
mod pastimes;
mod emotes;
mod fx;
mod gameguard;
mod grab;
mod browser;
mod keyboard;
mod movefx;
mod settings;
mod navbuild;
mod placehint;

pub struct Args {
    pub data: PathBuf,
    pub skins: PathBuf,
    pub skin: Option<String>,
    pub scale: Option<f32>,
    pub clip: String,
    pub browse: bool,
    pub capturable: bool,
    /// `--panel` / `--no-panel`: override the "open the control panel on start" setting.
    pub panel: Option<bool>,
    /// Live content terrain (screen capture). Off with `--no-vision`.
    pub vision: bool,
    pub sit_test: bool,
    pub fx_test: bool,
    pub emote_test: bool,
    pub goto_test: bool,
    /// `--visit NAME|any`: invite a visitor right after start (debug).
    pub visit: Option<String>,
    /// `--dream-test`: dream-nail every 15 s (debug).
    pub dream_test: bool,
    /// `--move-test`: scripted dive + turn-in-place loop (screenshots).
    pub move_test: bool,
}

const USAGE: &str = "usage: skong [--browse] [--capturable] [--panel|--no-panel] [--no-vision] [--capture-stats] [--data DIR] [--skins DIR] [--skin NAME] [--scale F] [--clip NAME] [--visit NAME|any]";

fn parse_args() -> Result<Args> {
    let mut a = Args {
        data: "data/hk".into(),
        skins: "skins".into(),
        skin: None,
        scale: None,
        clip: "Idle".into(),
        browse: false,
        capturable: false,
        panel: None,
        vision: true,
        sit_test: false,
        fx_test: false,
        emote_test: false,
        goto_test: false,
        visit: None,
        dream_test: false,
        move_test: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().with_context(|| format!("missing value for {k}"));
        match k.as_str() {
            "--data" => a.data = v()?.into(),
            "--skins" => a.skins = v()?.into(),
            "--skin" => a.skin = Some(v()?),
            "--scale" => a.scale = Some(v()?.parse()?),
            "--clip" => a.clip = v()?,
            "--browse" => a.browse = true,
            "--capturable" => a.capturable = true,
            "--panel" => a.panel = Some(true),
            "--no-panel" => a.panel = Some(false),
            "--no-vision" => a.vision = false,
            "--capture-stats" => skong_win::capture::set_stats_logging(true),
            "--sit-test" => a.sit_test = true,
            "--fx-test" => a.fx_test = true,
            "--emote-test" => a.emote_test = true,
            "--goto-test" => a.goto_test = true,
            "--visit" => a.visit = Some(v()?),
            "--dream-test" => a.dream_test = true,
            "--move-test" => a.move_test = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => bail!("unknown argument {k}\n{USAGE}"),
        }
    }
    Ok(a)
}

fn main() -> Result<()> {
    logring::init();
    logfile::install(settings::app_dir().map(|d| d.join("logs")));
    logfile::set_enabled(settings::Settings::log_file_wanted());
    let result = run();
    if let Err(e) = &result {
        // Without a console nobody would see this: keep it like a crash.
        log::error!("Desk-Skong stopped: {e:#}");
        if let Some(d) = logfile::dir() {
            let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            logfile::write_crash(d, unix, &format!("Desk-Skong {} stopped with an error:\n{e:?}\n", env!("CARGO_PKG_VERSION")));
        }
    }
    result
}

fn run() -> Result<()> {
    skong_win::desktop::enable_per_monitor_dpi();
    let args = parse_args()?;
    // Only one pet at a time: two instances fight over global hotkeys and screen capture.
    let _instance = if args.browse {
        None
    } else {
        match skong_win::monitor::single_instance() {
            Some(g) => Some(g),
            None => {
                log::warn!("Desk-Skong is already running (see the tray); exiting");
                return Ok(());
            }
        }
    };
    if args.browse { browser::run(&args) } else { app::run(&args) }
}
