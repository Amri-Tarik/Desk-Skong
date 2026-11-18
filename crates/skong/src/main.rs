//! Desk-Skong: a Hollow Knight desktop pet that lives on your windows.
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

mod app;
mod benches;
mod grab;
mod browser;
mod keyboard;
mod settings;
mod navbuild;

pub struct Args {
    pub data: PathBuf,
    pub skins: PathBuf,
    pub skin: Option<String>,
    pub scale: Option<f32>,
    pub clip: String,
    pub browse: bool,
    pub capturable: bool,
    /// Live content terrain (screen capture). Off with `--no-vision`.
    pub vision: bool,
    pub sit_test: bool,
}

const USAGE: &str = "usage: skong [--browse] [--capturable] [--no-vision] [--data DIR] [--skins DIR] [--skin NAME] [--scale F] [--clip NAME]";

fn parse_args() -> Result<Args> {
    let mut a = Args {
        data: "data/hk".into(),
        skins: "skins".into(),
        skin: None,
        scale: None,
        clip: "Idle".into(),
        browse: false,
        capturable: false,
        vision: true,
        sit_test: false,
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
            "--no-vision" => a.vision = false,
            "--sit-test" => a.sit_test = true,
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
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
