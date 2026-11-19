//! Desk-Skong control panel: every tray action, live status and the pet's log stream, plus
//! keyboard driving while focused. Talks to a running pet over `\\.\pipe\desk-skong`.
//!
//! `skong-panel [--pipe NAME] [--self-test | --self-test-overlay]`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod link;
mod selftest;
mod ui;

use anyhow::{Result, anyhow, bail};

const USAGE: &str = "usage: skong-panel [--pipe NAME] [--self-test | --self-test-overlay]";

fn main() -> Result<()> {
    let mut pipe = skong_ipc::PIPE_NAME.to_string();
    let mut self_test = false;
    let mut overlay = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pipe" => pipe = it.next().ok_or_else(|| anyhow!("missing value for --pipe"))?,
            "--self-test" => self_test = true,
            "--self-test-overlay" => (self_test, overlay) = (true, true),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    if self_test {
        return selftest::run(pipe, overlay);
    }
    ui::run(pipe)
}
