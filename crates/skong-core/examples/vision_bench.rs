//! Timing of content-surface detection on synthetic screens.
//!
//! ```text
//! cargo run -p skong-core --example vision_bench --release
//! ```
use std::time::{Duration, Instant};

use skong_core::geom::RectI;
use skong_core::vision::{PixelView, SurfaceMap, VisionParams};

fn main() {
    for (w, h) in [(1920usize, 1080usize), (2560, 1440), (3840, 2160)] {
        let px = busy_screen(w, h);
        let view = PixelView::new(&px, w, h, w).expect("view");
        let mut m = SurfaceMap::new(w, h, VisionParams::default());
        let full = [RectI::new(0, 0, w as i32, h as i32)];
        let dirty_probe = [RectI::new(0, 0, w as i32, 8)];
        m.rescan(&view, &full);
        let full_t = median(25, || {
            m.rescan(&view, &full);
        });
        let n = m.raw_surfaces().len();
        let scan_t = median(15, || {
            m.scan(&view, &full);
        });
        let finish_t = median(15, || {
            m.scan(&view, &dirty_probe);
            m.finish();
        });
        let dirty = [RectI::new(400, 300, 800, 500)];
        let small_t = median(200, || {
            m.rescan(&view, &dirty);
        });
        let mut t = 0.0;
        let upd_t = median(200, || {
            t += 0.1;
            m.update(t);
        });
        println!(
            "{w}x{h}: full rescan {:.2} ms (scan {:.2} + finish(full width) {:.2}) ({n} surfaces), 400x200 dirty rescan {:.3} ms, hysteresis update {:.3} ms",
            ms(full_t),
            ms(scan_t),
            ms(finish_t),
            ms(small_t),
            ms(upd_t)
        );
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn median(n: usize, mut f: impl FnMut()) -> Duration {
    let mut v: Vec<Duration> = (0..n)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed()
        })
        .collect();
    v.sort();
    v[n / 2]
}

/// White page with many lines, boxes and text-like blobs.
fn busy_screen(w: usize, h: usize) -> Vec<u32> {
    let mut px = vec![0xffff_ffffu32; w * h];
    let mut seed = 0x1234_5678u32;
    let mut rnd = |m: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize % m.max(1)
    };
    let fill = |px: &mut [u32], x0: usize, y0: usize, x1: usize, y1: usize, c: u32| {
        for y in y0..y1.min(h) {
            px[y * w + x0.min(w)..y * w + x1.min(w)].fill(c);
        }
    };
    for _ in 0..300 {
        let (x, y, len, th) = (rnd(w), rnd(h), 40 + rnd(800), 1 + rnd(6));
        let c = 0xff00_0000 | rnd(0xff_ffff) as u32;
        fill(&mut px, x, y, x + len, y + th, c);
    }
    for _ in 0..20_000 {
        let (x, y) = (rnd(w), rnd(h));
        fill(&mut px, x, y, x + 2 + rnd(7), y + 4 + rnd(8), 0xff20_2020);
    }
    px
}
