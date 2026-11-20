//! The achievement banner ("area title"): a Hollow Knight style reveal near the top of the
//! knight's monitor — a small caption, the title in engraved capitals, an ornamental
//! divider, the description — fading in, holding, fading out. Several unlocks queue up and
//! play one after another.
//!
//! Everything here is pure: the queue and its timing ([`Banners`]), which monitor may show
//! one ([`blocked_monitors`], [`pick_monitor`]: never over a fullscreen game or app), where
//! it sits ([`origin`]), line wrapping, and the drawn parts (the dark mist behind the text,
//! the divider). The window and the text rasterizing live in the pet (`skong/src/banner.rs`).
use std::collections::VecDeque;

use crate::canvas::{Image, over, premul};
use crate::desktop::{DesktopSnapshot, Monitor};
use crate::geom::{RectI, Vec2};

/// Fade in, hold, fade out (s).
pub const FADE_IN: f32 = 0.6;
pub const HOLD: f32 = 4.0;
pub const FADE_OUT: f32 = 1.0;
pub const TOTAL: f32 = FADE_IN + HOLD + FADE_OUT;
/// Pause between two queued banners (s).
pub const GAP: f32 = 0.5;
/// Unlocks waiting beyond this many are dropped (the panel lists them all anyway).
pub const MAX_QUEUED: usize = 12;
/// The banner's top edge, as a fraction of the work area's height below its top.
pub const TOP_FRACTION: f32 = 0.07;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerText {
    /// The small line above the title ("Achievement").
    pub caption: String,
    pub title: String,
    pub description: String,
}

impl BannerText {
    pub fn achievement(title: &str, description: &str) -> Self {
        Self { caption: "Achievement".into(), title: title.into(), description: description.into() }
    }
}

struct Current {
    text: BannerText,
    seq: u64,
    t: f32,
    monitor: Monitor,
}

/// What to show this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown<'a> {
    pub text: &'a BannerText,
    /// Changes with every banner (re-render when it does).
    pub seq: u64,
    /// 0..1, eased.
    pub alpha: f32,
    pub monitor: &'a Monitor,
}

/// The queue of banners and the one playing.
#[derive(Default)]
pub struct Banners {
    queue: VecDeque<BannerText>,
    current: Option<Current>,
    /// Seconds before the next one may start.
    gap: f32,
    seq: u64,
}

/// Opacity `t` seconds into a banner: eased in over [`FADE_IN`], held, eased out over
/// [`FADE_OUT`]; 0 outside.
pub fn alpha_at(t: f32) -> f32 {
    let smooth = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    if !(0.0..TOTAL).contains(&t) {
        0.0
    } else if t < FADE_IN {
        smooth(t / FADE_IN)
    } else if t < FADE_IN + HOLD {
        1.0
    } else {
        1.0 - smooth((t - FADE_IN - HOLD) / FADE_OUT)
    }
}

impl Banners {
    pub fn push(&mut self, text: BannerText) {
        if self.queue.len() < MAX_QUEUED {
            self.queue.push_back(text);
        }
    }

    /// Something is showing or waiting.
    pub fn busy(&self) -> bool {
        self.current.is_some() || !self.queue.is_empty()
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Advance by `dt`. `blocked` says whether a monitor (full rect) may not show a banner
    /// right now (a fullscreen game or app on it); `pick` chooses the monitor for the next
    /// one (`None`: nowhere free, keep waiting). A banner whose monitor gets blocked
    /// vanishes at once and, unless it was already fading out, plays again later.
    pub fn tick(&mut self, dt: f32, blocked: impl Fn(&RectI) -> bool, pick: impl FnOnce() -> Option<Monitor>) -> Option<Shown<'_>> {
        self.gap = (self.gap - dt).max(0.0);
        if let Some(c) = self.current.as_mut() {
            if blocked(&c.monitor.rect) {
                let c = self.current.take()?;
                if c.t < FADE_IN + HOLD {
                    self.queue.push_front(c.text);
                }
                self.gap = GAP;
                return None;
            }
            c.t += dt;
            if c.t >= TOTAL {
                self.current = None;
                self.gap = GAP;
                return None;
            }
        } else if self.gap <= 0.0 && !self.queue.is_empty() {
            let monitor = pick()?;
            let text = self.queue.pop_front()?;
            self.seq += 1;
            self.current = Some(Current { text, seq: self.seq, t: 0.0, monitor });
        }
        let c = self.current.as_ref()?;
        Some(Shown { text: &c.text, seq: c.seq, alpha: alpha_at(c.t), monitor: &c.monitor })
    }

    /// Hide the current banner and hold the queue (pause): it plays again from the start.
    pub fn interrupt(&mut self) {
        if let Some(c) = self.current.take() {
            self.queue.push_front(c.text);
        }
    }
}

/// Monitors (full rects) that may not show a banner: those a game occupies (the game
/// guard's list, exclusive fullscreen included) and those whose frontmost window is
/// fullscreen (a video, a presentation, an F11 browser).
pub fn blocked_monitors(desk: &DesktopSnapshot, game_monitors: &[RectI]) -> Vec<RectI> {
    desk.monitors
        .iter()
        .filter(|m| {
            game_monitors.contains(&m.rect) || {
                // `windows` is front to back: the first one centred on this monitor is on top.
                let front = desk.windows.iter().find(|w| m.rect.contains((w.rect.left + w.rect.right) / 2, (w.rect.top + w.rect.bottom) / 2));
                front.is_some_and(|w| w.fullscreen)
            }
        })
        .map(|m| m.rect)
        .collect()
}

/// The knight's monitor when it's free, else the free one nearest him; `None` when every
/// monitor is blocked.
pub fn pick_monitor(desk: &DesktopSnapshot, knight: Vec2, blocked: &[RectI]) -> Option<Monitor> {
    let centre = |r: &RectI| Vec2::new((r.left + r.right) as f32 * 0.5, (r.top + r.bottom) as f32 * 0.5);
    let free = || desk.monitors.iter().filter(|m| !blocked.contains(&m.rect));
    let (x, y) = (knight.x as i32, knight.y.max(0.0) as i32 - 1);
    free().find(|m| m.rect.contains(x, y)).or_else(|| free().min_by(|a, b| centre(&a.rect).distance(knight).total_cmp(&centre(&b.rect).distance(knight)))).cloned()
}

/// Top-left of a `w`×`h` banner on a monitor with work area `work`: centred, near the top.
pub fn origin(work: RectI, w: u32, h: u32) -> (i32, i32) {
    let x = work.left + (work.width() - w as i32) / 2;
    let y = work.top + (work.height() as f32 * TOP_FRACTION).round() as i32;
    (x, y.min(work.bottom - h as i32).max(work.top))
}

/// Split `text` into at most `max_lines` lines no wider than `width` (by `measure`); words
/// that don't fit on the last line end it with an ellipsis.
pub fn wrap(text: &str, width: f32, max_lines: usize, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if line.is_empty() || measure(&candidate) <= width {
            line = candidate;
            continue;
        }
        if lines.len() + 1 == max_lines {
            // Last line allowed: cut here.
            line.push('…');
            lines.push(line);
            return lines;
        }
        lines.push(std::mem::replace(&mut line, word.to_string()));
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Composite `rgb` at coverage `a` (0..1) onto pixel (x, y).
fn plot(img: &mut Image, x: i32, y: i32, rgb: [u8; 3], a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x >= img.width as i32 || y >= img.height as i32 {
        return;
    }
    let i = (y as u32 * img.width + x as u32) as usize;
    img.px[i] = over(premul(rgb[0], rgb[1], rgb[2], (a.min(1.0) * 255.0).round() as u8), img.px[i]);
}

/// A soft dark ellipse filling `img`, so pale text reads on any wallpaper: `strength` at
/// the centre, fading to nothing at the edges.
pub fn draw_mist(img: &mut Image, rgb: [u8; 3], strength: f32) {
    let (w, h) = (img.width as f32, img.height as f32);
    for y in 0..img.height {
        for x in 0..img.width {
            let dx = (x as f32 + 0.5 - w * 0.5) / (w * 0.5);
            let dy = (y as f32 + 0.5 - h * 0.5) / (h * 0.5);
            let r = (dx * dx + dy * dy).sqrt();
            let f = 1.0 - ((r - 0.5) / 0.5).clamp(0.0, 1.0);
            plot(img, x as i32, y as i32, rgb, strength * f * f * (3.0 - 2.0 * f));
        }
    }
}

/// A rhombus centred at (cx, cy) with half-diagonals (rx, ry): filled, or an outline
/// `stroke` px wide. Anti-aliased.
fn rhombus(img: &mut Image, (cx, cy): (f32, f32), (rx, ry): (f32, f32), stroke: Option<f32>, rgb: [u8; 3], alpha: f32) {
    let k = rx.min(ry);
    for y in (cy - ry - 2.0).floor() as i32..=(cy + ry + 2.0).ceil() as i32 {
        for x in (cx - rx - 2.0).floor() as i32..=(cx + rx + 2.0).ceil() as i32 {
            let d = ((x as f32 + 0.5 - cx).abs() / rx + (y as f32 + 0.5 - cy).abs() / ry - 1.0) * k;
            let cov = match stroke {
                Some(s) => (s * 0.5 + 0.5 - d.abs()).clamp(0.0, 1.0),
                None => (0.5 - d).clamp(0.0, 1.0),
            };
            plot(img, x, y, rgb, cov * alpha);
        }
    }
}

/// The divider under the title (an original motif): a hairline tapering away from an
/// outlined diamond with a small solid one inside, a small diamond riding the line on
/// each side, a dot past each end. Symmetric about `cx`; `half` is the line's half length.
pub fn draw_divider(img: &mut Image, cx: f32, cy: f32, half: f32, scale: f32, rgb: [u8; 3]) {
    let s = scale.max(0.5);
    let gap = 12.0 * s;
    let thick = 1.3 * s;
    for x in (cx - half).floor() as i32..=(cx + half).ceil() as i32 {
        let dx = (x as f32 + 0.5 - cx).abs();
        if dx < gap || dx > half {
            continue;
        }
        let taper = 1.0 - ((dx - gap) / (half - gap)).powi(2);
        for y in (cy - thick - 1.0).floor() as i32..=(cy + thick + 1.0).ceil() as i32 {
            let cov = (thick * 0.5 + 0.5 - (y as f32 + 0.5 - cy).abs()).clamp(0.0, 1.0);
            plot(img, x, y, rgb, cov * taper * 0.9);
        }
    }
    rhombus(img, (cx, cy), (7.5 * s, 11.0 * s), Some(1.3 * s), rgb, 1.0);
    rhombus(img, (cx, cy), (2.6 * s, 4.2 * s), None, rgb, 1.0);
    for side in [-1.0, 1.0] {
        rhombus(img, (cx + side * 24.0 * s, cy), (3.2 * s, 4.6 * s), None, rgb, 0.95);
        let (dx, r) = (half + 6.0 * s, 1.7 * s);
        for y in (cy - r - 1.0) as i32..=(cy + r + 1.0) as i32 {
            for x in (cx + side * dx - r - 1.0) as i32..=(cx + side * dx + r + 1.0) as i32 {
                let d = ((x as f32 + 0.5 - cx - side * dx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                plot(img, x, y, rgb, (r + 0.5 - d).clamp(0.0, 1.0) * 0.8);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::WindowInfo;

    fn text(t: &str) -> BannerText {
        BannerText::achievement(t, "did a thing")
    }

    fn mon(r: RectI, primary: bool) -> Monitor {
        Monitor { rect: r, work: RectI::new(r.left, r.top, r.right, r.bottom - 40), primary, scale_milli: 1000 }
    }

    fn desk() -> DesktopSnapshot {
        DesktopSnapshot { generation: 1, monitors: vec![mon(RectI::new(0, 0, 2560, 1440), true), mon(RectI::new(-1920, 0, 0, 1080), false)], windows: vec![] }
    }

    fn window(r: RectI, fullscreen: bool) -> WindowInfo {
        WindowInfo { id: 1, rect: r, title: String::new(), class: String::new(), process: String::new(), pid: 1, maximized: false, fullscreen, foreground: true, topmost: false }
    }

    #[test]
    fn fades_in_holds_and_fades_out() {
        assert_eq!(alpha_at(-0.1), 0.0);
        assert_eq!(alpha_at(0.0), 0.0);
        assert!((alpha_at(FADE_IN * 0.5) - 0.5).abs() < 1e-4);
        assert_eq!(alpha_at(FADE_IN), 1.0);
        assert_eq!(alpha_at(FADE_IN + HOLD - 0.01), 1.0);
        assert!((alpha_at(FADE_IN + HOLD + FADE_OUT * 0.5) - 0.5).abs() < 1e-4);
        assert_eq!(alpha_at(TOTAL), 0.0);
        // Monotonic up, then down.
        let a: Vec<f32> = (0..=56).map(|i| alpha_at(i as f32 * 0.1)).collect();
        let peak = a.iter().position(|&v| v == 1.0).unwrap();
        assert!(a[..=peak].windows(2).all(|w| w[0] <= w[1]) && a[peak..].windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn queued_banners_play_one_after_another() {
        let d = desk();
        let mut b = Banners::default();
        b.push(text("One"));
        b.push(text("Two"));
        let pick = || Some(d.monitors[0].clone());
        let mut seen: Vec<(String, u64)> = Vec::new();
        let mut shown_secs = 0.0;
        for _ in 0..(2.0 * (TOTAL + GAP) / 0.05) as usize + 20 {
            if let Some(s) = b.tick(0.05, |_| false, pick) {
                shown_secs += 0.05;
                if seen.last().is_none_or(|(_, q)| *q != s.seq) {
                    seen.push((s.text.title.clone(), s.seq));
                }
            }
        }
        assert_eq!(seen.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["One", "Two"]);
        assert!((shown_secs - 2.0 * TOTAL).abs() < 0.2, "{shown_secs}");
        assert!(!b.busy());
    }

    #[test]
    fn waits_while_every_monitor_is_blocked_and_replays_an_interrupted_one() {
        let d = desk();
        let mut b = Banners::default();
        b.push(text("Waiting"));
        for _ in 0..100 {
            assert!(b.tick(0.1, |_| true, || None).is_none(), "nowhere to show it");
        }
        assert_eq!(b.queued(), 1);
        // Free again: it starts.
        let s = b.tick(0.1, |_| false, || Some(d.monitors[0].clone())).unwrap();
        assert_eq!((s.text.title.as_str(), s.alpha), ("Waiting", 0.0));
        for _ in 0..20 {
            b.tick(0.1, |_| false, || unreachable!());
        }
        // A game goes fullscreen on its monitor mid-hold: gone at once, back in the queue.
        assert!(b.tick(0.1, |r| *r == d.monitors[0].rect, || unreachable!()).is_none());
        assert_eq!(b.queued(), 1);
        // After the gap it may start on the other monitor, from the beginning.
        assert!(b.tick(0.1, |_| false, || Some(d.monitors[1].clone())).is_none(), "gap first");
        let s = (0..10).find_map(|_| b.tick(0.1, |r| *r == d.monitors[0].rect, || Some(d.monitors[1].clone())).map(|s| (s.alpha, s.monitor.rect))).unwrap();
        assert_eq!(s, (0.0, d.monitors[1].rect));
        // Interrupted while fading out: not replayed.
        let mut b = Banners::default();
        b.push(text("Late"));
        b.tick(0.0, |_| false, || Some(d.monitors[0].clone()));
        b.tick(FADE_IN + HOLD + 0.2, |_| false, || None);
        b.tick(0.0, |_| true, || None);
        assert!(!b.busy());
    }

    #[test]
    fn interrupt_holds_the_banner_for_later() {
        let d = desk();
        let mut b = Banners::default();
        b.push(text("Paused"));
        assert!(b.tick(0.0, |_| false, || Some(d.monitors[0].clone())).is_some());
        b.interrupt();
        assert_eq!(b.queued(), 1);
        let s = b.tick(0.0, |_| false, || Some(d.monitors[0].clone())).unwrap();
        assert_eq!(s.alpha, 0.0);
    }

    #[test]
    fn the_queue_is_bounded() {
        let mut b = Banners::default();
        for i in 0..MAX_QUEUED + 5 {
            b.push(text(&i.to_string()));
        }
        assert_eq!(b.queued(), MAX_QUEUED);
    }

    #[test]
    fn never_over_a_game_or_fullscreen_app() {
        let mut d = desk();
        let (main, side) = (d.monitors[0].clone(), d.monitors[1].clone());
        let knight_main = Vec2::new(1000.0, 1400.0);
        // Nothing fullscreen: the knight's own monitor.
        assert!(blocked_monitors(&d, &[]).is_empty());
        assert_eq!(pick_monitor(&d, knight_main, &[]), Some(main.clone()));
        assert_eq!(pick_monitor(&d, Vec2::new(-500.0, 1000.0), &[]), Some(side.clone()));
        // A game on his monitor (the guard's list): the other one.
        let blocked = blocked_monitors(&d, &[main.rect]);
        assert_eq!(blocked, vec![main.rect]);
        assert_eq!(pick_monitor(&d, knight_main, &blocked), Some(side.clone()));
        // A fullscreen video in front on the side monitor, a normal window on the main one.
        d.windows = vec![window(side.rect, true), window(RectI::new(100, 100, 900, 700), false)];
        assert_eq!(blocked_monitors(&d, &[]), vec![side.rect]);
        // A fullscreen window *behind* a normal one doesn't count.
        d.windows = vec![window(RectI::new(100, 100, 900, 700), false), window(main.rect, true)];
        assert!(blocked_monitors(&d, &[]).is_empty());
        // Everything blocked: wait.
        assert_eq!(pick_monitor(&d, knight_main, &[main.rect, side.rect]), None);
        // Off every monitor (thrown?): the nearest free one.
        assert_eq!(pick_monitor(&d, Vec2::new(5000.0, 500.0), &[]), Some(main));
    }

    #[test]
    fn sits_centred_near_the_top_of_the_work_area() {
        let work = RectI::new(0, 0, 2560, 1400);
        let (x, y) = origin(work, 800, 200);
        assert_eq!(x, 880);
        assert_eq!(y, (1400.0 * TOP_FRACTION).round() as i32);
        let side = RectI::new(-1920, 0, 0, 1040);
        let (x, y) = origin(side, 600, 180);
        assert_eq!(x + 300, -960);
        assert!(y >= 0 && y + 180 <= 1040);
        // Taller than the work area allows: pinned to the top.
        assert_eq!(origin(RectI::new(0, 0, 800, 300), 400, 400).1, 0);
    }

    #[test]
    fn wraps_long_descriptions_into_two_lines_at_most() {
        let measure = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(wrap("short one", 200.0, 2, measure), ["short one"]);
        assert_eq!(wrap("aaaa bbbb cccc", 95.0, 2, measure), ["aaaa bbbb", "cccc"]);
        let cut = wrap("one two three four five six seven", 90.0, 2, measure);
        assert_eq!(cut.len(), 2);
        assert!(cut[1].ends_with('…'), "{cut:?}");
        assert!(wrap("", 100.0, 2, measure).is_empty());
        // A single word wider than the line still shows.
        assert_eq!(wrap("Unbreakable", 20.0, 2, measure), ["Unbreakable"]);
    }

    #[test]
    fn the_divider_is_symmetric_and_the_mist_fades_out() {
        let mut img = Image::new(301, 41);
        draw_divider(&mut img, 150.5, 20.5, 120.0, 1.0, [230, 230, 230]);
        let alpha = |x: u32, y: u32| img.px[(y * img.width + x) as usize] >> 24;
        assert!(alpha(150, 20) > 200, "centre diamond");
        assert!(alpha(150 - 60, 20) > 0 && alpha(150 + 60, 20) > 0, "the line");
        assert_eq!(alpha(5, 20), 0, "nothing past the end dots");
        for y in 0..41 {
            for dx in 0..150 {
                assert!(alpha(150 - dx, y).abs_diff(alpha(150 + dx, y)) <= 2, "symmetric at dx {dx} y {y}");
            }
        }
        let mut mist = Image::new(200, 80);
        draw_mist(&mut mist, [0, 0, 0], 0.6);
        let a = |x: u32, y: u32| mist.px[(y * 200 + x) as usize] >> 24;
        assert!((150..=155).contains(&a(100, 40)), "{}", a(100, 40));
        assert_eq!(a(0, 0), 0);
        assert_eq!(a(0, 40), 0);
    }
}
