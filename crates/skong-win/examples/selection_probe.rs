//! Passive check of the text-selection watcher: listens for a few seconds and prints where
//! highlighted text is (rectangles only; the text is never read). Opens no window and sends
//! no input: select some text in Notepad / Word / a browser while it runs.
//!
//! ```text
//! cargo run -p skong-win --example selection_probe -- [seconds, default 15]
//! ```
use std::time::{Duration, Instant};

use anyhow::Result;
use skong_win::desktop::enable_per_monitor_dpi;
use skong_win::selection::SelectionWatcher;

fn main() -> Result<()> {
    enable_per_monitor_dpi();
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(15);
    let started = Instant::now();
    let mut w = SelectionWatcher::start()?;
    println!("watching text selections for {secs} s (start-up {:?})", started.elapsed());
    let end = Instant::now() + Duration::from_secs(secs);
    let mut changes = 0;
    while Instant::now() < end {
        if w.poll() {
            changes += 1;
            match w.rect() {
                Some(r) => println!("{:>6.2}s  selection at ({}, {}) {}x{}", started.elapsed().as_secs_f32(), r.left, r.top, r.width(), r.height()),
                None => println!("{:>6.2}s  no selection", started.elapsed().as_secs_f32()),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let t = Instant::now();
    drop(w);
    println!("{changes} change(s); stopped in {:?}", t.elapsed());
    Ok(())
}
