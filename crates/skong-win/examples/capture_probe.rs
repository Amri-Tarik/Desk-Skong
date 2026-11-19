//! Does Desktop Duplication see windows marked `WDA_EXCLUDEFROMCAPTURE`?
//!
//! Shows two of our sprite windows for about a second: a *capturable* white panel with
//! a black line, and an *excluded* red square covering the right half of it (the same
//! window type the knight uses). Then grabs the panel area through desktop duplication
//! and reports what is visible under the red square; repeats with the square made
//! capturable as a control.
//!
//! ```text
//! cargo run -p skong-win --example capture_probe --release
//! ```
use std::time::{Duration, Instant};

use anyhow::Result;
use skong_core::canvas::Image;
use skong_core::geom::RectI;
use skong_win::capture::grab_region;
use skong_win::desktop::enable_per_monitor_dpi;
use skong_win::monitor::primary_work_area;
use skong_win::sprite_window::SpriteWindow;
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};

const W: u32 = 400;
const H: u32 = 200;
const LINE_Y: u32 = 100;

fn main() -> Result<()> {
    enable_per_monitor_dpi();
    let work = primary_work_area();
    let (x, y) = (work.left + 200, work.top + 200);

    // Panel: opaque white, black 2 px line across (premultiplied ARGB).
    let mut panel = Image::new(W, H);
    for (i, p) in panel.px.iter_mut().enumerate() {
        let row = i as u32 / W;
        *p = if (LINE_Y..LINE_Y + 2).contains(&row) { 0xff00_0000 } else { 0xffff_ffff };
    }
    let mut square = Image::new(W / 2, H);
    square.px.fill(0xffff_0000);

    let mut a = SpriteWindow::new()?;
    a.set_capturable(true);
    a.present(&panel, x, y)?;
    let mut b = SpriteWindow::new()?; // excluded from capture by default, like the knight
    b.present(&square, x + W as i32 / 2, y)?;
    b.raise();

    let region = RectI::new(x, y, x + W as i32, y + H as i32);
    for (label, capturable) in [("excluded square", false), ("capturable square (control)", true)] {
        b.set_capturable(capturable);
        pump(Duration::from_millis(400));
        let px = grab_region(region)?;
        let at = |px_x: u32, px_y: u32| px[(px_y * W + px_x) as usize] & 0x00ff_ffff;
        let (left_line, under_line, under_bg) = (at(100, LINE_Y), at(300, LINE_Y), at(300, 40));
        let verdict = match (under_line, under_bg) {
            (0x00_0000, 0xff_ffff) => "duplication shows the panel BEHIND the square (window invisible to capture)",
            (0xff_0000, 0xff_0000) => "duplication shows the square (window visible to capture)",
            _ => "unexpected pixels (something else is on top?)",
        };
        println!("{label}: left-half line {left_line:06x}, under square: line row {under_line:06x}, background {under_bg:06x} -> {verdict}");
    }
    Ok(())
}

/// Let DWM compose (and our windows process their messages) for `d`.
fn pump(d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until {
        let mut msg = MSG::default();
        // SAFETY: standard message pump on this thread's queue.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
