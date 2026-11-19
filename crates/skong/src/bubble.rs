//! Speech and thought bubbles: a small click-through, capture-excluded window per bubble
//! that follows its speaker. Speech uses Hollow Knight's dialogue look (dark box with the
//! fleur ornaments from `Dialogue Cln`); thoughts use the dream dialogue cloud.
//!
//! Usable by any feature: [`Bubbles::say`] returns an id to move ([`Bubbles::follow`]) or
//! close ([`Bubbles::close`]) the bubble; it also expires on its own.
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::bubble::{BubbleLayout, BubbleMetrics, BubbleStyle};
use skong_core::canvas::{Image, over, premul};
use skong_core::geom::{RectI, Vec2};
use skong_win::sprite_window::SpriteWindow;
use skong_win::text::TextPainter;

/// Fonts tried in order (HK uses Perpetua; Palatino / Georgia ship with Windows).
const SERIF: &[&str] = &["Perpetua", "Palatino Linotype", "Book Antiqua", "Georgia"];
const FADE: f32 = 0.18;

pub type BubbleId = u32;

/// Ornaments rasterized from the game's dialogue collection.
#[derive(Default)]
struct Art {
    fleur_top: Option<Image>,
    fleur_bottom: Option<Image>,
    dream_box: Option<Image>,
}

impl Art {
    fn load(bank: &SpriteBank, vanilla: Option<&Skin>) -> Art {
        let Some(col) = bank.collection_index("Dialogue Cln") else { return Art::default() };
        let key = |name: &str| (0..64u32).map(|i| SpriteKey { collection: col, sprite: i }).find(|k| bank.sprite_name(*k) == Some(name));
        let names = ["dialogue_fleur_top0008", "dialogue_fleur_bottom0008", "Dream_Dialogue_Box0005"];
        let keys: Vec<SpriteKey> = names.iter().filter_map(|n| key(n)).collect();
        let Some(skin) = vanilla else { return Art::default() };
        let frames: HashMap<SpriteKey, Frame> = match bank.rasterize_sets(skin, None, &[(&keys, 40.0)]) {
            Ok(mut v) => v.pop().unwrap_or_default(),
            Err(e) => {
                log::warn!("bubble art: {e:#}");
                return Art::default();
            }
        };
        let img = |n: &str| key(n).and_then(|k| frames.get(&k)).map(|f| f.image.clone()).filter(|i| i.width > 1);
        Art { fleur_top: img(names[0]), fleur_bottom: img(names[1]), dream_box: img(names[2]) }
    }
}

struct Live {
    id: BubbleId,
    win: SpriteWindow,
    img: Image,
    layout: BubbleLayout,
    anchor: Vec2,
    area: RectI,
    age: f32,
    secs: f32,
    /// Closing: fades out from now.
    closing: Option<f32>,
    shown_alpha: u8,
    shown_at: Option<(i32, i32)>,
}

pub struct Bubbles {
    speech: Option<TextPainter>,
    thought: Option<TextPainter>,
    art: Art,
    live: Vec<Live>,
    pool: Vec<SpriteWindow>,
    next: BubbleId,
    pub capturable: bool,
}

impl Bubbles {
    pub fn new(data_dir: &Path, vanilla: Option<&Skin>, scale: f32) -> Bubbles {
        let art = SpriteBank::load_metadata(data_dir).map(|b| Art::load(&b, vanilla)).unwrap_or_default();
        let mut b = Bubbles { speech: None, thought: None, art, live: Vec::new(), pool: Vec::new(), next: 1, capturable: false };
        b.rescale(scale);
        b
    }

    /// Fonts follow the pet's size (but stay readable).
    pub fn rescale(&mut self, scale: f32) {
        let px = (19.0 * scale.clamp(0.85, 1.6)).round() as i32;
        self.speech = TextPainter::new(SERIF, px, false).map_err(|e| log::warn!("bubble font: {e:#}")).ok();
        self.thought = TextPainter::new(SERIF, px, true).ok();
        if let Some(p) = &self.speech {
            log::info!("bubbles: font '{}' {px}px", p.face);
        }
    }

    /// Show `text` above `anchor` (the speaker's head top) for `secs`, kept inside `area`.
    pub fn say(&mut self, anchor: Vec2, text: &str, style: BubbleStyle, secs: f32, area: RectI) -> Option<BubbleId> {
        let img_layout = self.compose(text, style);
        let (img, layout) = match img_layout {
            Ok(v) => v,
            Err(e) => {
                log::warn!("bubble: {e:#}");
                return None;
            }
        };
        let win = match self.pool.pop() {
            Some(w) => w,
            None => match SpriteWindow::new() {
                Ok(w) => {
                    w.set_click_through(true);
                    w
                }
                Err(e) => {
                    log::warn!("bubble window: {e:#}");
                    return None;
                }
            },
        };
        win.set_capturable(self.capturable);
        let id = self.next;
        self.next += 1;
        log::info!("bubble #{id} {style:?}: \"{text}\"");
        self.live.push(Live { id, win, img, layout, anchor, area, age: 0.0, secs, closing: None, shown_alpha: 0, shown_at: None });
        Some(id)
    }

    /// Move a bubble with its speaker.
    pub fn follow(&mut self, id: BubbleId, anchor: Vec2) {
        if let Some(l) = self.live.iter_mut().find(|l| l.id == id) {
            l.anchor = anchor;
        }
    }

    /// Fade a bubble out now.
    pub fn close(&mut self, id: BubbleId) {
        if let Some(l) = self.live.iter_mut().find(|l| l.id == id)
            && l.closing.is_none()
        {
            l.closing = Some(l.age);
        }
    }

    /// Show / hide every bubble window (games).
    pub fn set_visible(&self, on: bool) {
        for l in self.live.iter().filter(|l| l.shown_at.is_some()) {
            l.win.set_visible(on);
        }
    }

    /// Remove bubbles over these screen areas at once (game monitors).
    pub fn drop_on(&mut self, areas: &[RectI]) {
        if areas.is_empty() {
            return;
        }
        let (gone, keep): (Vec<Live>, Vec<Live>) = std::mem::take(&mut self.live).into_iter().partition(|l| areas.iter().any(|r| r.contains(l.anchor.x as i32, l.anchor.y as i32)));
        self.live = keep;
        for l in gone {
            l.win.set_visible(false);
            self.pool.push(l.win);
        }
    }

    pub fn raise_all(&self) {
        for l in &self.live {
            l.win.raise();
        }
    }

    pub fn update(&mut self, dt: f32) {
        let mut done = Vec::new();
        for (i, l) in self.live.iter_mut().enumerate() {
            l.age += dt;
            if l.closing.is_none() && l.age >= l.secs {
                l.closing = Some(l.age);
            }
            let fade_in = (l.age / FADE).min(1.0);
            let fade_out = l.closing.map_or(1.0, |t| 1.0 - (l.age - t) / FADE);
            if fade_out <= 0.0 {
                done.push(i);
                continue;
            }
            let alpha = ((fade_in.min(fade_out)).clamp(0.0, 1.0) * 255.0) as u8;
            let (x, y) = l.layout.place(l.anchor, l.area);
            let res = if alpha != l.shown_alpha {
                l.shown_alpha = alpha;
                let img = if alpha == 255 { l.img.clone() } else { faded(&l.img, alpha) };
                let r = l.win.present(&img, x, y);
                if l.shown_at.is_none() {
                    l.win.set_visible(true);
                    l.win.raise();
                }
                r
            } else if l.shown_at != Some((x, y)) {
                l.win.move_to(x, y)
            } else {
                Ok(())
            };
            l.shown_at = Some((x, y));
            if let Err(e) = res {
                log::warn!("bubble: {e:#}");
            }
        }
        for i in done.into_iter().rev() {
            let l = self.live.swap_remove(i);
            l.win.set_visible(false);
            self.pool.push(l.win);
        }
    }

    /// Lay out and paint a bubble image.
    fn compose(&mut self, text: &str, style: BubbleStyle) -> Result<(Image, BubbleLayout)> {
        let painter = match style {
            BubbleStyle::Speech => self.speech.as_mut(),
            BubbleStyle::Thought => self.thought.as_mut().or(self.speech.as_mut()),
        }
        .ok_or_else(|| anyhow::anyhow!("no font"))?;
        let lh = painter.line_height();
        let metrics = BubbleMetrics::for_line_height(lh, style);
        let layout = BubbleLayout::new(text, metrics, &|s| painter.measure(s));
        let (w, h) = (layout.size.x as u32, layout.size.y as u32);
        let mut img = Image::new(w.max(1), h.max(1));
        let (bx, by) = (layout.box_at.x as i32, layout.box_at.y as i32);
        let (bw, bh) = (layout.box_size.x as u32, layout.box_size.y as u32);
        match style {
            BubbleStyle::Speech => {
                paint_box(&mut img, bx, by, bw, bh, (lh * 0.5) as i32);
                if let Some(f) = &self.art.fleur_top {
                    let s = fit(f, bw as f32 * 0.92, metrics.ornament_top * 1.6);
                    img.blit_over(&s, (w as i32 - s.width as i32) / 2, (by - s.height as i32 / 2).max(0), false);
                }
                if let Some(f) = &self.art.fleur_bottom {
                    let s = fit(f, bw as f32 * 0.6, metrics.ornament_bottom * 1.6);
                    img.blit_over(&s, (w as i32 - s.width as i32) / 2, (by + bh as i32 - s.height as i32 / 2).min(h as i32 - s.height as i32), false);
                }
            }
            BubbleStyle::Thought => match &self.art.dream_box {
                Some(d) => {
                    let mut cloud = stretched(d, bw, bh);
                    feather(&mut cloud, (lh * 0.7) as u32);
                    img.blit_over(&cloud, bx, by, false)
                }
                None => paint_box(&mut img, bx, by, bw, bh, (lh * 0.8) as i32),
            },
        }
        let color = match style {
            BubbleStyle::Speech => [242, 240, 236],
            BubbleStyle::Thought => [226, 218, 255],
        };
        for (i, line) in layout.lines.iter().enumerate() {
            let at = layout.line_origin(i);
            // Soft shadow first, then the text.
            painter.draw(&mut img, at.x as i32 + 1, at.y as i32 + 1, line, [0, 0, 0], 0.8)?;
            painter.draw(&mut img, at.x as i32, at.y as i32, line, color, 1.0)?;
        }
        Ok((img, layout))
    }
}

/// HK-style dialogue backdrop: near-black, slightly translucent, edges feathered.
fn paint_box(img: &mut Image, x: i32, y: i32, w: u32, h: u32, feather: i32) {
    let feather = feather.max(1) as f32;
    for yy in 0..h as i32 {
        for xx in 0..w as i32 {
            let (px, py) = (x + xx, y + yy);
            if px < 0 || py < 0 || px >= img.width as i32 || py >= img.height as i32 {
                continue;
            }
            let edge = (xx.min(w as i32 - 1 - xx) as f32 / feather).min(yy.min(h as i32 - 1 - yy) as f32 / feather).clamp(0.0, 1.0);
            let a = (edge * edge * (3.0 - 2.0 * edge) * 215.0) as u8;
            let i = (py as u32 * img.width + px as u32) as usize;
            img.px[i] = over(premul(6, 6, 10, a), img.px[i]);
        }
    }
}

/// Uniformly scaled to fit within (w, h).
fn fit(img: &Image, w: f32, h: f32) -> Image {
    let f = (w / img.width.max(1) as f32).min(h / img.height.max(1) as f32);
    img.scaled(f.max(0.01))
}

/// Box-filtered resize to exactly (w, h).
fn stretched(src: &Image, w: u32, h: u32) -> Image {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Image::new(w, h);
    let (sx, sy) = (src.width as f32 / w as f32, src.height as f32 / h as f32);
    for y in 0..h {
        let y0 = ((y as f32 * sy) as u32).min(src.height - 1);
        let y1 = (((y + 1) as f32 * sy).ceil() as u32).clamp(y0 + 1, src.height);
        for x in 0..w {
            let x0 = ((x as f32 * sx) as u32).min(src.width - 1);
            let x1 = (((x + 1) as f32 * sx).ceil() as u32).clamp(x0 + 1, src.width);
            let mut acc = [0u32; 4];
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let p = src.px[(yy * src.width + xx) as usize];
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += (p >> (24 - 8 * k as u32)) & 0xff;
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            let c = |k: usize| (acc[k] + n / 2) / n;
            out.px[(y * w + x) as usize] = (c(0) << 24) | (c(1) << 16) | (c(2) << 8) | c(3);
        }
    }
    out
}

/// Premultiplied image at `alpha`/255 opacity.
pub fn faded(img: &Image, alpha: u8) -> Image {
    let a = alpha as u32;
    let px = img
        .px
        .iter()
        .map(|&p| {
            let ch = |s: u32| ((((p >> s) & 0xff) * a + 127) / 255) << s;
            ch(24) | ch(16) | ch(8) | ch(0)
        })
        .collect();
    Image { width: img.width, height: img.height, px }
}

/// Fade the outer `px` pixels of an image to transparent (soft cloud edges).
fn feather(img: &mut Image, px: u32) {
    let px = px.max(1) as f32;
    let (w, h) = (img.width, img.height);
    for y in 0..h {
        for x in 0..w {
            let d = x.min(w - 1 - x).min(y).min(h - 1 - y) as f32;
            if d >= px {
                continue;
            }
            let t = d / px;
            let k = (t * t * (3.0 - 2.0 * t) * 255.0) as u32;
            let i = (y * w + x) as usize;
            let p = img.px[i];
            let ch = |s: u32| ((((p >> s) & 0xff) * k + 127) / 255) << s;
            img.px[i] = ch(24) | ch(16) | ch(8) | ch(0);
        }
    }
}
