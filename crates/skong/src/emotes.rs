//! The knight is silent: instead of speech he shows small emote icons above his head —
//! "zzz" while asleep, a Geo coin when he spots loot, "!" when startled or excited, "?"
//! when something catches his curiosity. One small capture-excluded window follows him.
//!
//! Thoughts (Geo: "I want that"; "?": "what's that?") sit in a small thought cloud so they
//! read as his thinking, not as an object floating over him. Reactions stay bare: "!" is a
//! startle (a jolt, not a thought) and the rising "zzz" is the usual sleep sign.
use std::path::Path;

use anyhow::Result;
use skong_core::anim::ClipLibrary;
use skong_core::assets::{Skin, SpriteBank};
use skong_core::bubble::thought_cloud;
use skong_core::canvas::Image;
use skong_core::geom::Vec2;
use skong_win::sprite_window::SpriteWindow;
use skong_win::text::TextPainter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emote {
    Zzz,
    Geo,
    Bang,
    Question,
}

impl Emote {
    fn duration(self) -> f32 {
        match self {
            Emote::Zzz => 2.4,
            Emote::Geo => 1.6,
            Emote::Bang => 1.2,
            Emote::Question => 1.8,
        }
    }

    /// Shown inside a thought cloud (see the module docs).
    fn in_cloud(self) -> bool {
        matches!(self, Emote::Geo | Emote::Question)
    }

    /// Reactions (startle, sleep) always show; curiosity/greed respect a cooldown.
    fn urgent(self) -> bool {
        matches!(self, Emote::Bang | Emote::Zzz)
    }
}

/// Minimum seconds between non-urgent emotes, so he doesn't spam icons.
const COOLDOWN: f32 = 8.0;

pub struct Emotes {
    win: SpriteWindow,
    painter: Option<TextPainter>,
    coin: Option<Image>,
    ppu: f32,
    current: Option<(Emote, f32)>,
    clock: f32,
    last_shown: f32,
    /// (emote, phase) currently uploaded to the window.
    drawn: Option<(Emote, u32)>,
    /// Size of the uploaded image.
    size: (u32, u32),
    visible: bool,
}

impl Emotes {
    pub fn new(data: &Path, vanilla: Option<&Skin>, px_per_unit: f32, capturable: bool) -> Result<Emotes> {
        let win = SpriteWindow::new()?;
        win.set_capturable(capturable);
        win.set_click_through(true);
        win.set_visible(false);
        let mut e = Emotes { win, painter: None, coin: None, ppu: px_per_unit, current: None, clock: 0.0, last_shown: -COOLDOWN, drawn: None, size: (0, 0), visible: false };
        e.rescale(data, vanilla, px_per_unit);
        Ok(e)
    }

    /// (Re)build the glyph font and the coin icon for a render scale.
    pub fn rescale(&mut self, data: &Path, vanilla: Option<&Skin>, px_per_unit: f32) {
        self.ppu = px_per_unit;
        let px = (0.7 * px_per_unit).round().max(12.0) as i32;
        self.painter = TextPainter::new(&["Trajan Pro", "Perpetua Titling MT", "Georgia"], px, false).map_err(|e| log::warn!("emote font: {e:#}")).ok();
        self.coin = vanilla.and_then(|v| coin_icon(data, v, px_per_unit * 0.85).map_err(|e| log::warn!("emote coin: {e:#}")).ok());
        self.drawn = None;
    }

    /// Show `e` (ignored while a non-urgent one is on cooldown, or the same one is up).
    pub fn show(&mut self, e: Emote) {
        if self.current.is_some_and(|(c, _)| c == e) {
            return;
        }
        if !e.urgent() && self.clock - self.last_shown < COOLDOWN {
            return;
        }
        self.current = Some((e, 0.0));
        self.last_shown = self.clock;
    }

    /// Keep "zzz" going while he sleeps (re-shown as each cycle ends).
    pub fn sleeping(&mut self, asleep: bool) {
        if asleep && self.current.is_none() {
            self.show(Emote::Zzz);
        }
    }

    /// Seconds since start (for debug cycling).
    pub fn clock(&self) -> f32 {
        self.clock
    }

    pub fn hide(&mut self) {
        self.current = None;
        if self.visible {
            self.win.set_visible(false);
            self.visible = false;
        }
    }

    pub fn raise(&self) {
        if self.visible {
            self.win.raise();
        }
    }

    /// Advance and draw above `head` (screen point just above the knight's head).
    pub fn update(&mut self, dt: f32, head: Vec2) -> Result<()> {
        self.clock += dt;
        let Some((e, t)) = self.current.as_mut() else { return Ok(()) };
        *t += dt;
        let (e, t) = (*e, *t);
        if t >= e.duration() {
            self.hide();
            return Ok(());
        }
        // Phase drives the picture (zzz grows); bob drives the position.
        let phase = match e {
            Emote::Zzz => ((t / e.duration()) * 3.0).floor().min(2.0) as u32,
            _ => 0,
        };
        let rise = match e {
            Emote::Zzz => t / e.duration() * 0.4,
            _ => (t * 6.0).min(1.0) * 0.15 + (t * 5.0).sin() * 0.03,
        } * self.ppu;
        if self.drawn != Some((e, phase)) {
            let Some(img) = self.render(e, phase) else {
                self.hide();
                return Ok(());
            };
            let x = (head.x - img.width as f32 * 0.5).round() as i32;
            let y = (head.y - rise - img.height as f32).round() as i32;
            self.win.present(&img, x, y)?;
            self.drawn = Some((e, phase));
            self.size = (img.width, img.height);
        } else {
            let x = (head.x - self.size.0 as f32 * 0.5).round() as i32;
            let y = (head.y - rise - self.size.1 as f32).round() as i32;
            self.win.move_to(x, y)?;
        }
        if !self.visible {
            self.win.set_visible(true);
            self.visible = true;
        }
        Ok(())
    }

    fn render(&mut self, e: Emote, phase: u32) -> Option<Image> {
        if e.in_cloud() {
            let icon = match e {
                Emote::Geo => self.coin.clone()?,
                _ => self.ink("?")?,
            };
            let cloud = thought_cloud((icon.width, icon.height), (self.ppu / 38.4 * 1.3).clamp(1.0, 3.0));
            let mut img = cloud.image;
            img.blit_over(&icon, cloud.content_at.0, cloud.content_at.1, false);
            return Some(img);
        }
        match e {
            Emote::Geo => self.coin.clone(),
            Emote::Bang => self.glyphs("!"),
            Emote::Question => self.glyphs("?"),
            Emote::Zzz => self.glyphs(["z", "z Z", "z Z z"][phase as usize]),
        }
    }

    /// Dark text, tightly sized (drawn on a pale thought cloud).
    fn ink(&mut self, text: &str) -> Option<Image> {
        let p = self.painter.as_mut()?;
        let w = p.measure(text).ceil() as u32 + 2;
        let h = p.line_height().ceil() as u32;
        let mut img = Image::new(w, h);
        p.draw(&mut img, 1, 0, text, [40, 40, 56], 1.0).ok()?;
        // Trim to the inked pixels (the line box has room for accents and descenders).
        let inked = |x: u32, y: u32| img.px[(y * w + x) as usize] >> 24 > 0;
        let rows: Vec<u32> = (0..h).filter(|&y| (0..w).any(|x| inked(x, y))).collect();
        let cols: Vec<u32> = (0..w).filter(|&x| (0..h).any(|y| inked(x, y))).collect();
        let (&y0, &y1, &x0, &x1) = (rows.first()?, rows.last()?, cols.first()?, cols.last()?);
        let mut out = Image::new(x1 - x0 + 1, y1 - y0 + 1);
        out.blit_over(&img, -(x0 as i32), -(y0 as i32), false);
        Some(out)
    }

    /// White text with a dark outline (readable on any background).
    fn glyphs(&mut self, text: &str) -> Option<Image> {
        let p = self.painter.as_mut()?;
        let pad = 3;
        let w = p.measure(text).ceil() as u32 + 2 * pad as u32 + 2;
        let h = p.line_height().ceil() as u32 + 2 * pad as u32;
        let mut img = Image::new(w, h);
        for (dx, dy) in [(-2, 0), (2, 0), (0, -2), (0, 2), (-2, -2), (2, -2), (-2, 2), (2, 2), (-1, 0), (1, 0), (0, 1), (0, -1)] {
            p.draw(&mut img, pad + dx, pad + dy, text, [20, 20, 30], 0.85).ok()?;
        }
        p.draw(&mut img, pad, pad, text, [245, 245, 250], 1.0).ok()?;
        Some(img)
    }
}

/// The small Geo coin (first frame of "Small Idle"), slightly enlarged for legibility.
fn coin_icon(data: &Path, vanilla: &Skin, px_per_unit: f32) -> Result<Image> {
    let mut bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Geo.json"), &bank)?;
    let id = lib.id("Small Idle").ok_or_else(|| anyhow::anyhow!("no 'Small Idle' clip"))?;
    let key = lib.get(id).frames.first().and_then(|f| f.sprite).ok_or_else(|| anyhow::anyhow!("empty clip"))?;
    bank.rasterize_skin(vanilla, None, &[key], px_per_unit * 1.3)?;
    bank.frame(key).map(|f| f.image.clone()).ok_or_else(|| anyhow::anyhow!("coin frame missing"))
}
