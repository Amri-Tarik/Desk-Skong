//! The achievement banner on screen: Hollow Knight's "area title" reveal, near the top
//! centre of the knight's monitor — "Achievement" small, the title large in Cinzel, the
//! ornamental divider, the description — over a soft dark mist so it reads on any
//! wallpaper. Fades in 0.6 s, holds 4 s, fades out 1 s; several unlocks play in turn.
//!
//! Queue, timing, placement and the "never over a fullscreen game or app" rule are pure
//! (`skong_core::banner`); this file owns the window and draws the text with GDI, using
//! the bundled fonts registered privately for this process. The window is layered,
//! click-through, never activated, topmost, and follows "Visible in screen shares" like
//! every sprite window. Rendered once per banner; the fades only change the window's
//! opacity.
use skong_core::banner::{self, BannerText, Banners};
use skong_core::canvas::Image;
use skong_core::desktop::DesktopSnapshot;
use skong_core::fonts;
use skong_core::geom::{RectI, Vec2};
use skong_win::sprite_window::SpriteWindow;
use skong_win::text::{TextPainter, register_font};

/// Colours: pale silver caption, bone-white title, the divider, the description, the mist.
const CAPTION: [u8; 3] = [196, 204, 216];
const TITLE: [u8; 3] = [242, 236, 222];
const DIVIDER: [u8; 3] = [212, 216, 224];
const BODY: [u8; 3] = [224, 224, 228];
const MIST: [u8; 3] = [6, 8, 12];
const SHADOW: [u8; 3] = [0, 0, 0];

/// Pixel sizes at 100 % scaling.
const CAPTION_PX: f32 = 19.0;
const TITLE_PX: f32 = 54.0;
const BODY_PX: f32 = 26.0;
const MAX_TEXT_W: f32 = 860.0;

struct Painters {
    scale_milli: u32,
    caption: TextPainter,
    title: TextPainter,
    body: TextPainter,
}

pub struct Banner {
    banners: Banners,
    win: Option<SpriteWindow>,
    capturable: bool,
    painters: Option<Painters>,
    /// The banner currently drawn into the window.
    drawn: Option<u64>,
    visible: bool,
}

impl Banner {
    pub fn new(capturable: bool) -> Self {
        for f in [fonts::HEADING, fonts::BODY] {
            if let Err(e) = register_font(f) {
                log::warn!("banner font: {e:#}");
            }
        }
        Self { banners: Banners::default(), win: None, capturable, painters: None, drawn: None, visible: false }
    }

    pub fn push(&mut self, text: BannerText) {
        self.banners.push(text);
    }

    /// Re-assert topmost (housekeeping).
    pub fn raise(&self) {
        if self.visible
            && let Some(w) = &self.win
        {
            w.raise();
        }
    }

    /// One tick: start, fade, end banners; hide while `paused` (they wait).
    pub fn tick(&mut self, dt: f32, desk: &DesktopSnapshot, game_monitors: &[RectI], knight: Vec2, paused: bool) {
        if paused {
            self.banners.interrupt();
            self.hide();
            return;
        }
        if !self.banners.busy() {
            self.hide();
            return;
        }
        let blocked = banner::blocked_monitors(desk, game_monitors);
        let shown = self.banners.tick(dt, |r| blocked.contains(r), || banner::pick_monitor(desk, knight, &blocked)).map(|s| (s.seq, s.alpha, s.monitor.clone(), (self.drawn != Some(s.seq)).then(|| s.text.clone())));
        let Some((seq, alpha, monitor, fresh)) = shown else {
            self.hide();
            return;
        };
        if let Some(text) = fresh {
            self.drawn = Some(seq);
            let img = self.render(&text, monitor.scale_milli);
            let (x, y) = banner::origin(monitor.work, img.width, img.height);
            log::info!("banner: \"{}\" on monitor {:?}", text.title, monitor.rect);
            let Some(win) = self.window() else { return };
            let shown = win.set_opacity(0).and_then(|_| win.present(&img, x, y));
            if let Err(e) = shown {
                log::warn!("banner: {e:#}");
            }
            win.set_visible(true);
            self.visible = true;
        }
        if let Some(w) = self.win.as_mut()
            && let Err(e) = w.set_opacity((alpha * 255.0).round() as u8)
        {
            log::warn!("banner: {e:#}");
        }
    }

    fn hide(&mut self) {
        if self.visible
            && let Some(w) = &self.win
        {
            w.set_visible(false);
        }
        self.visible = false;
        self.drawn = None;
    }

    fn window(&mut self) -> Option<&mut SpriteWindow> {
        if self.win.is_none() {
            match SpriteWindow::new() {
                Ok(w) => {
                    w.set_click_through(true);
                    w.set_capturable(self.capturable);
                    w.set_visible(false);
                    self.win = Some(w);
                }
                Err(e) => log::warn!("banner window: {e:#}"),
            }
        }
        self.win.as_mut()
    }

    fn painters(&mut self, scale_milli: u32) -> Option<&mut Painters> {
        if self.painters.as_ref().is_none_or(|p| p.scale_milli != scale_milli) {
            let s = scale_milli.max(500) as f32 / 1000.0;
            let px = |v: f32| (v * s).round() as i32;
            let heading = [fonts::HEADING_FACE, "Trajan Pro", "Perpetua Titling MT"];
            let made = (|| -> anyhow::Result<Painters> {
                Ok(Painters {
                    scale_milli,
                    caption: TextPainter::new(&heading, px(CAPTION_PX), false)?,
                    title: TextPainter::new(&heading, px(TITLE_PX), false)?,
                    body: TextPainter::new(&[fonts::BODY_FACE, "Georgia"], px(BODY_PX), false)?,
                })
            })();
            match made {
                Ok(p) => self.painters = Some(p),
                Err(e) => {
                    log::warn!("banner fonts: {e:#}");
                    return None;
                }
            }
        }
        self.painters.as_mut()
    }

    /// The banner image: mist, caption, title, divider, description (centred).
    fn render(&mut self, text: &BannerText, scale_milli: u32) -> Image {
        let s = scale_milli.max(500) as f32 / 1000.0;
        let Some(p) = self.painters(scale_milli) else {
            return Image::new(1, 1);
        };
        let lines = banner::wrap(&text.description, MAX_TEXT_W * s, 2, |t| p.body.measure(t));
        let title_w = p.title.measure(&text.title);
        let half = (title_w * 0.42).clamp(150.0 * s, 340.0 * s);
        let content_w = lines.iter().map(|l| p.body.measure(l)).fold(title_w.max(p.caption.measure(&text.caption)).max(2.0 * half + 20.0 * s), f32::max);
        let (pad_x, pad_y) = (120.0 * s, 36.0 * s);
        let (cap_h, title_h, body_h) = (p.caption.line_height(), p.title.line_height(), p.body.line_height());
        let divider_h = 26.0 * s;
        let h = pad_y + cap_h + 2.0 * s + title_h + 4.0 * s + divider_h + 8.0 * s + body_h * lines.len() as f32 + pad_y + 12.0 * s;
        let w = content_w + 2.0 * pad_x;
        let mut img = Image::new(w.ceil() as u32, h.ceil() as u32);
        banner::draw_mist(&mut img, MIST, 0.66);
        let cx = img.width as f32 * 0.5;
        let mut y = pad_y;
        centred(&mut p.caption, &mut img, cx, y, &text.caption, CAPTION, s);
        y += cap_h + 2.0 * s;
        centred(&mut p.title, &mut img, cx, y, &text.title, TITLE, s);
        y += title_h + 4.0 * s;
        banner::draw_divider(&mut img, cx, y + divider_h * 0.5, half, s, DIVIDER);
        y += divider_h + 8.0 * s;
        for l in &lines {
            centred(&mut p.body, &mut img, cx, y, l, BODY, s);
            y += body_h;
        }
        img
    }
}

/// `text` centred on `cx` with its top at `y`, over a faint dark halo.
fn centred(p: &mut TextPainter, img: &mut Image, cx: f32, y: f32, text: &str, rgb: [u8; 3], s: f32) {
    let x = (cx - p.measure(text) * 0.5).round() as i32;
    let (y, d) = (y.round() as i32, (1.5 * s).round().max(1.0) as i32);
    let mut draw = |dx: i32, dy: i32, rgb: [u8; 3], a: f32| {
        if let Err(e) = p.draw(img, x + dx, y + dy, text, rgb, a) {
            log::warn!("banner text: {e:#}");
        }
    };
    for (dx, dy) in [(-d, 0), (d, 0), (0, -d), (0, d), (-d, -d), (d, d), (-d, d), (d, -d)] {
        draw(dx, dy, SHADOW, 0.22);
    }
    draw(0, 0, rgb, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders in memory only (GDI memory DCs; no window is created).
    #[test]
    fn renders_the_banner_with_the_bundled_fonts() {
        let mut b = Banner::new(false);
        let text = BannerText::achievement("Bounce", "Sprang off an enemy's head with a downward strike, as the old nailsmiths taught.");
        for scale in [1000, 1500] {
            let img = b.render(&text, scale);
            let s = scale as f32 / 1000.0;
            assert!(img.width as f32 > 500.0 * s && img.height as f32 > 180.0 * s, "{}x{}", img.width, img.height);
            assert!(b.win.is_none(), "rendering doesn't create the window");
            let alpha = |x: u32, y: u32| img.px[(y * img.width + x) as usize] >> 24;
            assert_eq!(alpha(0, 0), 0, "the mist fades out at the corners");
            // Bright text pixels in the middle (the title).
            let bright = img.px.iter().filter(|&&p| (p >> 16 & 0xff) > 200 && p >> 24 > 200).count();
            assert!(bright > 300, "{bright}");
            if let Some(dir) = std::env::var_os("SKONG_BANNER_PNG") {
                let path = std::path::Path::new(&dir).join(format!("banner-{scale}.png"));
                let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), img.width, img.height);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                // Over a mid-grey "wallpaper" to see the mist.
                let rgba: Vec<u8> = img.px.iter().flat_map(|&p| {
                    let a = p >> 24;
                    let c = |v: u32| (v + 120 * (255 - a) / 255) as u8;
                    [c(p >> 16 & 0xff), c(p >> 8 & 0xff), c(p & 0xff), 255]
                }).collect();
                enc.write_header().unwrap().write_image_data(&rgba).unwrap();
            }
        }
    }
}
