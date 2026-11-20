//! Desk-Skong control panel: every tray action, live status and the pet's log stream, plus
//! keyboard driving while focused. Talks to a running pet over `\.\pipe\desk-skong`.
//! Also the first-run art setup window (`--setup`, see `setup.rs`).
//!
//! `skong-panel [--pipe NAME] [--self-test | --self-test-overlay]`
//! `skong-panel --setup --data DIR --out DIR [--game PATH] [--again]`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod link;
mod selftest;
mod setup;
mod ui;

use anyhow::{Result, anyhow, bail};

const USAGE: &str = "usage: skong-panel [--pipe NAME] [--self-test | --self-test-overlay]\n       skong-panel --setup --data DIR --out DIR [--game PATH] [--again]";

fn main() -> Result<()> {
    let mut pipe = skong_ipc::PIPE_NAME.to_string();
    let mut self_test = false;
    let mut overlay = false;
    let (mut setup, mut again) = (false, false);
    let (mut data, mut out, mut game) = (None, None, None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| anyhow!("missing value for {a}"));
        match a.as_str() {
            "--pipe" => pipe = value()?,
            "--self-test" => self_test = true,
            "--self-test-overlay" => (self_test, overlay) = (true, true),
            "--setup" => setup = true,
            "--again" => again = true,
            "--data" => data = Some(value()?.into()),
            "--out" => out = Some(value()?.into()),
            "--game" => game = Some(value()?.into()),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    if setup {
        let (Some(data), Some(out)) = (data, out) else { bail!("--setup needs --data and --out\n{USAGE}") };
        std::process::exit(setup::run(setup::Options { data, out, game, again }));
    }
    if self_test {
        return selftest::run(pipe, overlay);
    }
    ui::run(pipe)
}
