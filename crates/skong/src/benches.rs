//! Benches (placed props, each in its own small window) and seat finding: the knight can
//! sit at a bench or perch on the edge of any window-top ledge with room for his legs.
use std::path::Path;

use anyhow::{Context, Result};
use skong_core::assets::{Atlas, read_json};
use skong_core::canvas::Image;
use skong_core::controller::Character;
use skong_core::geom::Vec2;
use skong_core::props::{Bench, BenchStyle, load_bench_styles};
use skong_core::terrain::{LedgeKind, Terrain};
use skong_win::sprite_window::SpriteWindow;

/// Sitting clips (Sit, Sit Idle, Sitting Asleep, Sit Map …) put the seat line this far below
/// the sprite origin, in game units (y down). Tuned visually on the Dirtmouth bench at
/// 64 px/unit: the cloak drapes over the seat front, the seat sits at +46 px (standing
/// feet are at +88 px) and the sprite ends ~28 px lower.
pub const SEAT_BELOW_ORIGIN: f32 = 46.0 / 64.0;
/// How far the dangling legs/cloak reach below the seat line (game units).
pub const LEG_LENGTH: f32 = 28.0 / 64.0;

#[derive(serde::Deserialize)]
struct SpriteMeta {
    #[serde(default = "ppu")]
    pixels_per_unit: f32,
}
fn ppu() -> f32 {
    64.0
}

pub struct Benches {
    pub styles: Vec<BenchStyle>,
    /// Source art per style (None if not exported) + its pixels-per-unit.
    src: Vec<Option<(Image, f32)>>,
    /// Art scaled for the current render scale.
    scaled: Vec<Option<Image>>,
    scale: f32,
    pub placed: Vec<(Bench, SpriteWindow)>,
    /// Debug: let screenshots see bench windows.
    pub capturable: bool,
}

impl Benches {
    /// Styles from `props.json`, art from `<vanilla>/props/<sprite>.png`.
    pub fn load(props_json: &Path, vanilla: Option<&Path>, scale: f32) -> Result<Benches> {
        let styles = load_bench_styles(props_json)?;
        let src = styles
            .iter()
            .map(|s| {
                let dir = vanilla?.join("props");
                let atlas = Atlas::load_png(&dir.join(format!("{}.png", s.sprite))).ok()?;
                let ppu = read_json::<SpriteMeta>(&dir.join(format!("{}.json", s.sprite))).map_or(64.0, |m| m.pixels_per_unit);
                Some((Image::from_rgba(atlas.width, atlas.height, &atlas.rgba), ppu))
            })
            .collect::<Vec<_>>();
        let available = src.iter().filter(|s| s.is_some()).count();
        log::info!("benches: {available}/{} styles available", styles.len());
        let mut b = Benches { styles, scaled: vec![None; src.len()], src, scale: 0.0, placed: Vec::new(), capturable: false };
        b.rescale(scale);
        Ok(b)
    }

    /// Art of a style at the current scale (None if not exported).
    pub fn art(&self, style: usize) -> Option<&Image> {
        self.scaled.get(style)?.as_ref()
    }

    pub fn available(&self, style: usize) -> bool {
        self.src.get(style).is_some_and(|s| s.is_some())
    }

    pub fn rescale(&mut self, scale: f32) {
        self.scale = scale;
        self.scaled = self.src.iter().map(|s| s.as_ref().map(|(img, ppu)| img.scaled(64.0 * scale / ppu))).collect();
        for (b, w) in &mut self.placed {
            if let Some(img) = &self.scaled[b.style] {
                let _ = present(w, img, b);
            }
        }
    }

    pub fn place(&mut self, style: usize, at: Vec2, terrain: &Terrain) -> Result<()> {
        let img = self.scaled.get(style).and_then(|i| i.as_ref()).context("bench art not exported (run the exporter with --export-sprites)")?;
        let bench = Bench::place(style, at, terrain).context("no surface below the cursor")?;
        let mut w = SpriteWindow::new()?;
        w.set_capturable(self.capturable);
        present(&mut w, img, &bench)?;
        self.placed.push((bench, w));
        Ok(())
    }

    pub fn clear(&mut self) {
        self.placed.clear();
    }

    /// Keep benches on their windows; move their windows when they moved.
    pub fn follow(&mut self, old: &Terrain, new: &Terrain) {
        for (b, w) in &mut self.placed {
            let before = b.pos;
            b.follow(old, new);
            if b.pos != before
                && let Some(img) = &self.scaled[b.style]
            {
                let _ = w.move_to((b.pos.x - img.width as f32 * 0.5).round() as i32, (b.pos.y - img.height as f32).round() as i32);
            }
        }
    }

    /// Show only benches that are not on one of `hidden` monitors (games).
    pub fn hide_on(&self, hidden: &[skong_core::geom::RectI]) {
        for (b, w) in &self.placed {
            let covered = hidden.iter().any(|r| r.contains(b.pos.x as i32, b.pos.y as i32 - 1));
            w.set_visible(!covered);
        }
    }

    pub fn raise_all(&self) {
        for (_, w) in &self.placed {
            w.raise();
        }
    }

    /// Seat height (screen y) of a placed bench.
    pub fn seat_y(&self, i: usize) -> f32 {
        let (b, _) = &self.placed[i];
        let h = self.scaled[b.style].as_ref().map_or(0.0, |img| img.height as f32);
        b.pos.y - h * self.styles[b.style].seat
    }

    /// Bench standing on the knight's current surface, if any (closest first).
    pub fn on_my_surface(&self, ch: &Character) -> Option<usize> {
        let s = ch.support.as_ref()?;
        self.placed
            .iter()
            .enumerate()
            .filter(|(_, (b, _))| (b.pos.y - s.y).abs() < 1.0 && b.pos.x >= s.x0 && b.pos.x <= s.x1)
            .min_by(|a, b| (a.1.0.pos.x - ch.pos.x).abs().total_cmp(&(b.1.0.pos.x - ch.pos.x).abs()))
            .map(|(i, _)| i)
    }

    /// Where the knight would sit right now: at a bench he stands at, else perched on his
    /// ledge if it's a window/content edge with nothing within leg length below.
    pub fn seat_here(&self, ch: &Character, terrain: &Terrain) -> Option<f32> {
        if let Some(i) = self.on_my_surface(ch)
            && (self.placed[i].0.pos.x - ch.pos.x).abs() < 6.0
        {
            return Some(self.seat_y(i));
        }
        let s = ch.support.as_ref().filter(|_| ch.is_grounded())?;
        if s.kind == LedgeKind::Ground {
            return None;
        }
        let legs = LEG_LENGTH * ch.params.px_per_unit;
        let blocked = terrain.ledges.iter().any(|l| l.x0 <= ch.pos.x && ch.pos.x <= l.x1 && l.y > s.y + 0.5 && l.y < s.y + legs);
        (!blocked).then_some(s.y)
    }
}

fn present(w: &mut SpriteWindow, img: &Image, b: &Bench) -> Result<()> {
    w.present(img, (b.pos.x - img.width as f32 * 0.5).round() as i32, (b.pos.y - img.height as f32).round() as i32)
}
