//! The knight is silent: instead of speech he shows small emote icons above his head —
//! "zzz" while asleep, a Geo coin when he spots loot, "!" when startled or excited, "?"
//! when something catches his curiosity, and the brain's other expressions
//! (`skong_core::ai::Expression`): "…" when bored, an anger mark before a fight, a bare "?"
//! when he couldn't get somewhere, a single "z" when drowsy, a heart after a nice chat, a
//! sweat drop when badly hurt. One small capture-excluded window follows him.
//!
//! Thoughts (Geo: "I want that"; "?": "what's that?"; "…": "nothing to do") sit in a small
//! thought cloud so they read as his thinking, not as an object floating over him.
//! Reactions stay bare: "!" is a startle (a jolt, not a thought) and the rising "zzz" is the
//! usual sleep sign. The marks without a glyph (anger, heart, sweat, sparkle, the dots) are
//! drawn in code: pale fills with a soft dark rim, like the glyphs, readable on any
//! background. Every bubble fades in and out briefly.
use std::path::Path;

use anyhow::Result;
use skong_core::ai::Expression;
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
    /// A small, quiet sparkle: something was achieved (see `app/achievements.rs`), or a big
    /// Geo haul.
    Spark,
    /// "…" in a thought cloud: bored.
    Dots,
    /// An anger mark: enemies or a cursor to fight.
    Anger,
    /// A bare "?": couldn't get to what he wanted.
    Lost,
    /// A single "z": drowsy.
    Drowsy,
    /// A small heart: a nice chat with a visitor.
    Heart,
    /// A sweat drop: badly hurt.
    Sweat,
}

impl Emote {
    /// Every emote (debug cycling, `--emote-test`).
    pub const ALL: [Emote; 11] = [
        Emote::Zzz,
        Emote::Geo,
        Emote::Bang,
        Emote::Question,
        Emote::Spark,
        Emote::Dots,
        Emote::Anger,
        Emote::Lost,
        Emote::Drowsy,
        Emote::Heart,
        Emote::Sweat,
    ];

    fn duration(self) -> f32 {
        match self {
            Emote::Zzz => 2.4,
            Emote::Geo => 1.6,
            Emote::Bang => 1.2,
            Emote::Question => 1.8,
            Emote::Spark => 1.4,
            Emote::Dots => 2.4,
            Emote::Anger => 1.4,
            Emote::Lost => 1.6,
            Emote::Drowsy => 2.0,
            Emote::Heart => 1.8,
            Emote::Sweat => 1.6,
        }
    }

    /// Shown inside a thought cloud (see the module docs).
    fn in_cloud(self) -> bool {
        matches!(self, Emote::Geo | Emote::Question | Emote::Dots)
    }

    /// Curiosity/greed respect a cooldown here; everything else always shows (the brain's
    /// expressions are already rate-limited by `skong_core::ai::expression`).
    fn urgent(self) -> bool {
        !matches!(self, Emote::Geo | Emote::Question)
    }
}

impl From<Expression> for Emote {
    fn from(e: Expression) -> Self {
        match e {
            Expression::Alert => Emote::Bang,
            Expression::Angry => Emote::Anger,
            Expression::Sweat => Emote::Sweat,
            Expression::Lost => Emote::Lost,
            Expression::Fond => Emote::Heart,
            Expression::Greedy => Emote::Geo,
            Expression::Curious => Emote::Question,
            Expression::Drowsy => Emote::Drowsy,
            Expression::Bored => Emote::Dots,
        }
    }
}

/// Minimum seconds between non-urgent emotes, so he doesn't spam icons.
const COOLDOWN: f32 = 8.0;
/// Fade in / out (s); the alpha moves in [`FADE_STEPS`] steps (each one re-uploads the
/// small image).
const FADE_IN: f32 = 0.12;
const FADE_OUT: f32 = 0.3;
const FADE_STEPS: f32 = 4.0;

/// Outline colour of glyphs and drawn marks.
const RIM: [u32; 3] = [20, 20, 30];
/// Thought-cloud ink.
const INK: [u32; 3] = [40, 40, 56];

pub struct Emotes {
    win: SpriteWindow,
    painter: Option<TextPainter>,
    coin: Option<Image>,
    ppu: f32,
    current: Option<(Emote, f32)>,
    clock: f64,
    last_shown: f64,
    /// (emote, phase) of the rendered picture in `base`.
    based: Option<(Emote, u32)>,
    base: Option<Image>,
    /// (emote, phase, fade level) currently uploaded to the window.
    drawn: Option<(Emote, u32, u32)>,
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
        let mut e = Emotes {
            win,
            painter: None,
            coin: None,
            ppu: px_per_unit,
            current: None,
            clock: 0.0,
            last_shown: -(COOLDOWN as f64),
            based: None,
            base: None,
            drawn: None,
            size: (0, 0),
            visible: false,
        };
        e.rescale(data, vanilla, px_per_unit);
        Ok(e)
    }

    /// (Re)build the glyph font and the coin icon for a render scale.
    pub fn rescale(&mut self, data: &Path, vanilla: Option<&Skin>, px_per_unit: f32) {
        self.ppu = px_per_unit;
        let px = (0.7 * px_per_unit).round().max(12.0) as i32;
        self.painter = TextPainter::new(&["Trajan Pro", "Perpetua Titling MT", "Georgia"], px, false).map_err(|e| log::warn!("emote font: {e:#}")).ok();
        self.coin = vanilla.and_then(|v| coin_icon(data, v, px_per_unit * 0.85).map_err(|e| log::warn!("emote coin: {e:#}")).ok());
        self.based = None;
        self.drawn = None;
    }

    /// Show `e` (ignored while a non-urgent one is on cooldown, or the same one is up).
    pub fn show(&mut self, e: Emote) {
        if self.current.is_some_and(|(c, _)| c == e) {
            return;
        }
        if !e.urgent() && ((self.clock - self.last_shown) as f32) < COOLDOWN {
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
    pub fn clock(&self) -> f64 {
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
        self.clock += dt as f64;
        let Some((e, t)) = self.current.as_mut() else { return Ok(()) };
        *t += dt;
        let (e, t) = (*e, *t);
        let d = e.duration();
        if t >= d {
            self.hide();
            return Ok(());
        }
        // Phase drives the picture (zzz grows); bob drives the position.
        let phase = match e {
            Emote::Zzz => ((t / d) * 3.0).floor().min(2.0) as u32,
            _ => 0,
        };
        let rise = match e {
            // Drifting up as sleep signs do.
            Emote::Zzz | Emote::Drowsy => t / d * 0.4,
            // A sweat drop slides down a little.
            Emote::Sweat => 0.2 - t / d * 0.15,
            _ => (t * 6.0).min(1.0) * 0.15 + (t * 5.0).sin() * 0.03,
        } * self.ppu;
        let level = fade_level(t, d);
        if self.based != Some((e, phase)) {
            let Some(img) = self.render(e, phase) else {
                self.hide();
                return Ok(());
            };
            self.base = Some(img);
            self.based = Some((e, phase));
        }
        let Some(base) = self.base.as_ref() else { return Ok(()) };
        let (w, h) = (base.width, base.height);
        let x = (head.x - w as f32 * 0.5).round() as i32;
        let y = (head.y - rise - h as f32).round() as i32;
        if self.drawn != Some((e, phase, level)) {
            let img = faded(base, level as f32 / FADE_STEPS);
            self.win.present(&img, x, y)?;
            self.drawn = Some((e, phase, level));
            self.size = (w, h);
        } else {
            self.win.move_to(x, y)?;
        }
        if !self.visible {
            self.win.set_visible(true);
            self.visible = true;
        }
        Ok(())
    }

    fn render(&mut self, e: Emote, phase: u32) -> Option<Image> {
        // Mark sizes follow the render scale.
        let unit = |f: f32| (f * self.ppu).round().max(6.0);
        if e.in_cloud() {
            let icon = match e {
                Emote::Geo => self.coin.clone()?,
                Emote::Dots => dots((0.07 * self.ppu).max(2.0)),
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
            Emote::Question | Emote::Lost => self.glyphs("?"),
            Emote::Zzz => self.glyphs(["z", "z Z", "z Z z"][phase as usize]),
            Emote::Drowsy => self.glyphs("z"),
            Emote::Spark => Some(sparkle((0.3 * self.ppu).round().max(7.0) as u32)),
            Emote::Anger => Some(anger(unit(0.26))),
            Emote::Heart => Some(heart(unit(0.24))),
            Emote::Sweat => Some(sweat(unit(0.2))),
            Emote::Dots => Some(dots((0.07 * self.ppu).max(2.0))),
        }
    }

    /// Dark text, tightly sized (drawn on a pale thought cloud).
    fn ink(&mut self, text: &str) -> Option<Image> {
        let p = self.painter.as_mut()?;
        let w = p.measure(text).ceil() as u32 + 2;
        let h = p.line_height().ceil() as u32;
        let mut img = Image::new(w, h);
        p.draw(&mut img, 1, 0, text, INK.map(|c| c as u8), 1.0).ok()?;
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
            p.draw(&mut img, pad + dx, pad + dy, text, RIM.map(|c| c as u8), 0.85).ok()?;
        }
        p.draw(&mut img, pad, pad, text, [245, 245, 250], 1.0).ok()?;
        Some(img)
    }
}

/// Fade step (0..=FADE_STEPS) at `t` of an emote lasting `d` s.
fn fade_level(t: f32, d: f32) -> u32 {
    let a = (t / FADE_IN).min((d - t) / FADE_OUT).clamp(0.0, 1.0);
    (a * FADE_STEPS).ceil() as u32
}

/// `img` with its opacity scaled by `a` (premultiplied: every channel scales).
fn faded(img: &Image, a: f32) -> Image {
    if a >= 1.0 {
        return img.clone();
    }
    let k = (a.clamp(0.0, 1.0) * 256.0) as u32;
    let mut out = img.clone();
    for p in &mut out.px {
        let c = *p;
        let ch = |shift: u32| ((((c >> shift) & 255) * k) >> 8) << shift;
        *p = ch(24) | ch(16) | ch(8) | ch(0);
    }
    out
}

/// A `w`×`h` mark: `inside(x, y)` (px from the image centre, y down) filled with `rgb`,
/// anti-aliased (3×3 samples), with an optional 2 px soft dark rim like the glyphs'.
fn shape(w: u32, h: u32, rgb: [u32; 3], rim: bool, inside: impl Fn(f32, f32) -> bool) -> Image {
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let mut cov = vec![0.0f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut n = 0;
            for sy in 0..3 {
                for sx in 0..3 {
                    let px = x as f32 + (sx as f32 + 0.5) / 3.0 - cx;
                    let py = y as f32 + (sy as f32 + 0.5) / 3.0 - cy;
                    n += inside(px, py) as u32;
                }
            }
            cov[(y * w + x) as usize] = n as f32 / 9.0;
        }
    }
    let mut img = Image::new(w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = (y * w as i32 + x) as usize;
            let f = cov[i];
            let mut r = 0.0f32;
            if rim && f < 1.0 {
                for dy in -2..=2i32 {
                    for dx in -2..=2i32 {
                        let (nx, ny) = (x + dx, y + dy);
                        if dx * dx + dy * dy <= 5 && nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                            r = r.max(cov[(ny * w as i32 + nx) as usize]);
                        }
                    }
                }
            }
            let ra = 0.85 * r * (1.0 - f);
            let a = f + ra;
            if a <= 0.0 {
                continue;
            }
            let ch = |k: usize| (rgb[k] as f32 * f + RIM[k] as f32 * ra).round().min(255.0) as u32;
            img.px[i] = (((a * 255.0).round().min(255.0) as u32) << 24) | (ch(0) << 16) | (ch(1) << 8) | ch(2);
        }
    }
    img
}

/// The anger mark: four short arcs bowing in toward an empty centre (a "vein" mark), each
/// a quarter of a circle centred on its quadrant's outer corner. `r`: half its size (px).
fn anger(r: f32) -> Image {
    let size = (2.0 * r).ceil() as u32 + 7;
    let gap = 0.3 * r;
    let radius = r - gap;
    let half = (0.13 * r).max(1.0);
    shape(size, size, [214, 84, 74], true, move |x, y| {
        let (ax, ay) = (x.abs(), y.abs());
        if ax > r || ay > r {
            return false;
        }
        let d = ((r - ax).powi(2) + (r - ay).powi(2)).sqrt();
        (d - radius).abs() <= half
    })
}

/// A small heart. `r`: half its width (px).
fn heart(r: f32) -> Image {
    let s = r / 1.15;
    let w = (2.0 * r).ceil() as u32 + 7;
    let h = (2.4 * s).ceil() as u32 + 7;
    shape(w, h, [240, 196, 204], true, move |x, y| {
        // (u² + v² − 1)³ − u²v³ ≤ 0, v up; spans v ∈ [−1, 1.24]: centre it.
        let (u, v) = (x / s, -y / s + 0.12);
        (u * u + v * v - 1.0).powi(3) - u * u * v.powi(3) <= 0.0
    })
}

/// A sweat drop, pointed end up. `r`: radius of its round end (px).
fn sweat(r: f32) -> Image {
    let w = (2.0 * r).ceil() as u32 + 7;
    let h = (3.0 * r).ceil() as u32 + 7;
    let (tip, c) = (-1.5 * r, 0.5 * r);
    shape(w, h, [178, 214, 240], true, move |x, y| {
        if y >= c {
            x * x + (y - c).powi(2) <= r * r
        } else if y >= tip {
            x.abs() <= r * ((y - tip) / (c - tip)).powf(0.8)
        } else {
            false
        }
    })
}

/// "…": three dots of thought-cloud ink, `r` px in radius.
fn dots(r: f32) -> Image {
    let step = 3.2 * r;
    let w = (2.0 * step + 2.0 * r).ceil() as u32 + 2;
    let h = (2.0 * r).ceil() as u32 + 2;
    shape(w, h, INK, false, move |x, y| [-step, 0.0, step].iter().any(|&c| (x - c).powi(2) + y * y <= r * r))
}

/// A four-pointed sparkle `r` px in radius: pale gold with a soft dark rim (readable on any
/// background), concave arms like a glint.
fn sparkle(r: u32) -> Image {
    let size = 2 * r + 3;
    let c = (size / 2) as f32;
    let mut img = Image::new(size, size);
    // Inside the star when |x|^p + |y|^p <= r^p with p < 1 (concave arms).
    let star = |x: f32, y: f32, r: f32| (x.abs() / r).powf(0.5) + (y.abs() / r).powf(0.5) <= 1.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let (a, rgb) = if star(dx, dy, r as f32 - 1.0) {
                (255u32, [255u32, 236, 170])
            } else if star(dx, dy, r as f32 + 0.5) {
                (150, [30, 26, 20])
            } else {
                continue;
            };
            // Premultiplied BGRA.
            img.px[(y * size + x) as usize] = (a << 24) | ((rgb[0] * a / 255) << 16) | ((rgb[1] * a / 255) << 8) | (rgb[2] * a / 255);
        }
    }
    img
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

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(img: &Image, x: u32, y: u32) -> u32 {
        img.px[(y * img.width + x) as usize] >> 24
    }

    #[test]
    fn every_expression_has_an_emote() {
        for e in Expression::ALL {
            assert!(Emote::ALL.contains(&Emote::from(e)), "{e:?}");
        }
    }

    #[test]
    fn fades_in_and_out_in_a_few_steps() {
        let d = 1.6;
        assert_eq!(fade_level(0.0, d), 0);
        assert!(fade_level(0.05, d) < FADE_STEPS as u32);
        assert_eq!(fade_level(0.5, d), FADE_STEPS as u32);
        assert!(fade_level(d - 0.1, d) < FADE_STEPS as u32);
        let levels: std::collections::BTreeSet<u32> = (0..160).map(|i| fade_level(i as f32 * 0.01, d)).collect();
        assert!(levels.len() <= FADE_STEPS as usize + 1, "few re-uploads: {levels:?}");
        let img = anger(12.0);
        let half = faded(&img, 0.5);
        let i = img.px.iter().position(|p| p >> 24 == 255).expect("an opaque pixel");
        assert!((half.px[i] >> 24).abs_diff(127) <= 1);
    }

    #[test]
    fn drawn_marks_scale_and_keep_their_shape() {
        for ppu in [38.4f32, 64.0, 128.0] {
            let unit = |f: f32| (f * ppu).round().max(6.0);
            for (name, img) in [("anger", anger(unit(0.26))), ("heart", heart(unit(0.24))), ("sweat", sweat(unit(0.2))), ("dots", dots((0.07 * ppu).max(2.0)))] {
                let (w, h) = (img.width, img.height);
                assert!(w <= (0.75 * ppu) as u32 + 8 && h <= (0.75 * ppu) as u32 + 8, "{name} at {ppu}: {w}x{h}");
                let inked = img.px.iter().filter(|p| **p >> 24 > 0).count();
                assert!(inked > 10, "{name} at {ppu} is empty");
                // Nothing touches the edge (the rim needs room).
                for x in 0..w {
                    assert_eq!(alpha(&img, x, 0) + alpha(&img, x, h - 1), 0, "{name} clipped");
                }
            }
        }
        // The anger mark's centre is empty; the heart's and the drop's are filled.
        let a = anger(16.0);
        assert_eq!(alpha(&a, a.width / 2, a.height / 2), 0);
        let h = heart(16.0);
        assert_eq!(alpha(&h, h.width / 2, h.height / 2), 255);
        let s = sweat(10.0);
        assert_eq!(alpha(&s, s.width / 2, s.height * 2 / 3), 255);
    }
}
