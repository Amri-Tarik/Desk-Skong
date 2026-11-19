//! Rendering for the stakes: the Shade (+ its slash arc), Geo coins, fireballs, one-shot
//! flashes and the dropped mask piece, each a small layered window (pooled).
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use skong_core::anim::{AnimEvent, Animator, ClipId, ClipLibrary};
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::geom::{RectF, Vec2};
use skong_core::stakes::{GeoField, Shade, ShadeTick};
use skong_core::terrain::Terrain;
use skong_win::sprite_window::SpriteWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lib {
    Shade,
    Geo,
    Spells,
}

/// One animated sprite in its own window.
struct Sprite {
    win: SpriteWindow,
    anim: Animator,
    drawn: Option<(SpriteKey, bool)>,
    shown: bool,
}

impl Sprite {
    fn draw(&mut self, lib: &ClipLibrary, frames: &HashMap<SpriteKey, Frame>, at: Vec2, flip: bool, feet: bool, blocked: &[RectF]) -> Result<()> {
        let Some(key) = self.anim.current(lib) else { return Ok(()) };
        let Some(frame) = frames.get(&key) else { return Ok(()) };
        draw_frame(&mut self.win, &mut self.drawn, &mut self.shown, key, frame, at, flip, feet, blocked)
    }
}

/// Present `frame` with its origin at `at` (or its bottom edge on `at.y` when `feet`).
/// Hidden while `at` is on a `blocked` monitor (a fullscreen game).
#[allow(clippy::too_many_arguments)]
fn draw_frame(win: &mut SpriteWindow, drawn: &mut Option<(SpriteKey, bool)>, shown: &mut bool, key: SpriteKey, frame: &Frame, at: Vec2, flip: bool, feet: bool, blocked: &[RectF]) -> Result<()> {
    if blocked.iter().any(|r| r.contains(at)) {
        if *shown {
            win.set_visible(false);
            *shown = false;
        }
        return Ok(());
    }
    let below = frame.image.height as f32 - frame.anchor[1];
    let origin = if feet { Vec2::new(at.x, at.y - below) } else { at };
    let x = (origin.x - frame.anchor_x(flip)).round() as i32;
    let y = (origin.y - frame.anchor[1]).round() as i32;
    if *drawn != Some((key, flip)) {
        win.present_flipped(&frame.image, x, y, flip)?;
        *drawn = Some((key, flip));
    } else {
        win.move_to(x, y)?;
    }
    if !*shown {
        win.set_visible(true);
        *shown = true;
    }
    Ok(())
}

/// A fireball in flight (or bursting at its end).
struct Shot {
    sprite: Sprite,
    pos: Vec2,
    vel: Vec2,
    travelled: f32,
    ending: bool,
}

/// A one-shot animation at a fixed spot (fireball end, the Shade's death glow).
struct Flash {
    sprite: Sprite,
    lib: Lib,
    at: Vec2,
    flip: bool,
}

/// The mask piece left on the ground where the knight died.
struct Head {
    win: SpriteWindow,
    drawn: Option<(SpriteKey, bool)>,
    shown: bool,
    key: SpriteKey,
    at: Vec2,
    left: f32,
}

pub struct View {
    bank: SpriteBank,
    shade_lib: ClipLibrary,
    geo_lib: ClipLibrary,
    spells_lib: ClipLibrary,
    frames: HashMap<SpriteKey, Frame>,
    /// (skin, scale) the frames were rasterized for.
    look: Option<(usize, f32)>,
    shade: Option<Sprite>,
    shade_fx: Option<Sprite>,
    coins: HashMap<u32, Sprite>,
    shots: Vec<Shot>,
    flashes: Vec<Flash>,
    head: Option<Head>,
    pool: Vec<SpriteWindow>,
    events: Vec<AnimEvent>,
    capturable: bool,
    /// Monitors with a fullscreen game: nothing of ours shows there.
    blocked: Vec<RectF>,
}

/// Fireball speed and range at scale 1 (px/s, px).
const SHOT_SPEED: f32 = 1100.0;
const SHOT_RANGE: f32 = 1100.0;

impl View {
    pub fn load(data: &Path, capturable: bool) -> Result<View> {
        let bank = SpriteBank::load_metadata(data)?;
        let anim = |f: &str| ClipLibrary::load(&data.join("animations").join(f), &bank);
        Ok(View {
            shade_lib: anim("Hollow_Shade.json")?,
            geo_lib: anim("Geo.json")?,
            spells_lib: anim("Spells_Anim.json")?,
            bank,
            frames: HashMap::new(),
            look: None,
            shade: None,
            shade_fx: None,
            coins: HashMap::new(),
            shots: Vec::new(),
            flashes: Vec::new(),
            head: None,
            pool: Vec::new(),
            events: Vec::new(),
            capturable,
            blocked: Vec::new(),
        })
    }

    fn lib(&self, l: Lib) -> &ClipLibrary {
        match l {
            Lib::Shade => &self.shade_lib,
            Lib::Geo => &self.geo_lib,
            Lib::Spells => &self.spells_lib,
        }
    }

    /// Rasterize the Shade, Geo and spell sprites for this skin / scale (once per change).
    pub fn ensure_look(&mut self, skin_idx: usize, skin: &Skin, vanilla: Option<&Skin>, scale: f32) -> Result<()> {
        if self.look == Some((skin_idx, scale)) {
            return Ok(());
        }
        let mut keys = self.shade_lib.sprite_keys();
        keys.extend(self.geo_lib.sprite_keys());
        // Only the fireball from the spells library.
        for name in ["Ball", "Ball End"] {
            if let Some(id) = self.spells_lib.id(name) {
                keys.extend(self.spells_lib.get(id).frames.iter().filter_map(|f| f.sprite));
            }
        }
        keys.sort_by_key(|k| (k.collection, k.sprite));
        keys.dedup();
        let mut sets = self.bank.rasterize_sets(skin, vanilla, &[(&keys, 64.0 * scale)])?;
        self.frames = sets.pop().unwrap_or_default();
        self.look = Some((skin_idx, scale));
        for s in self.shade.iter_mut().chain(self.shade_fx.iter_mut()).chain(self.coins.values_mut()) {
            s.drawn = None;
        }
        log::info!("stakes: {} sprites rasterized (shade, geo, fireball)", self.frames.len());
        Ok(())
    }

    fn sprite(&mut self, lib: Lib, clip: &str) -> Option<Sprite> {
        let id: ClipId = self.lib(lib).id(clip)?;
        let win = match self.pool.pop() {
            Some(w) => w,
            None => match SpriteWindow::new() {
                Ok(w) => {
                    w.set_capturable(self.capturable);
                    w
                }
                Err(e) => {
                    log::warn!("stakes window: {e:#}");
                    return None;
                }
            },
        };
        let mut anim = Animator::new(id);
        anim.play(self.lib(lib), id, true);
        Some(Sprite { win, anim, drawn: None, shown: false })
    }

    fn recycle(&mut self, s: Sprite) {
        s.win.set_visible(false);
        self.pool.push(s.win);
    }

    /// Draw the Shade for this tick (`None` removes it).
    pub fn shade(&mut self, shade: Option<(&Shade, ShadeTick)>, dt: f32) -> Result<()> {
        let Some((sh, tick)) = shade else {
            if let Some(s) = self.shade.take() {
                self.recycle(s);
            }
            if let Some(s) = self.shade_fx.take() {
                self.recycle(s);
            }
            return Ok(());
        };
        if self.shade.is_none() {
            self.shade = self.sprite(Lib::Shade, tick.clip);
        }
        let flip = sh.facing_right; // Shade art faces left
        if let Some(mut s) = self.shade.take() {
            if let Some(id) = self.shade_lib.id(tick.clip) {
                s.anim.play(&self.shade_lib, id, false);
            }
            self.events.clear();
            s.anim.update(&self.shade_lib, dt, &mut self.events);
            s.draw(&self.shade_lib, &self.frames, sh.pos, flip, false, &self.blocked)?;
            self.shade = Some(s);
        }
        match tick.effect {
            Some(fx) => {
                if self.shade_fx.is_none() {
                    self.shade_fx = self.sprite(Lib::Shade, fx);
                }
                if let Some(mut s) = self.shade_fx.take() {
                    self.events.clear();
                    s.anim.update(&self.shade_lib, dt, &mut self.events);
                    s.draw(&self.shade_lib, &self.frames, sh.pos, flip, false, &self.blocked)?;
                    self.shade_fx = Some(s);
                }
            }
            None => {
                if let Some(s) = self.shade_fx.take() {
                    self.recycle(s);
                }
            }
        }
        Ok(())
    }

    /// One coin window per loose coin, playing its resting / airborne spin.
    pub fn coins(&mut self, field: &GeoField, dt: f32) -> Result<()> {
        let coins = field.coins();
        let gone: Vec<u32> = self.coins.keys().copied().filter(|id| !coins.iter().any(|c| c.id == *id)).collect();
        for id in gone {
            if let Some(s) = self.coins.remove(&id) {
                self.recycle(s);
            }
        }
        for c in coins {
            let (rest, air) = c.size.clips();
            let clip = if c.resting { rest } else { air };
            if !self.coins.contains_key(&c.id) {
                let Some(s) = self.sprite(Lib::Geo, clip) else { continue };
                self.coins.insert(c.id, s);
            }
            let Some(id) = self.geo_lib.id(clip) else { continue };
            if let Some(s) = self.coins.get_mut(&c.id) {
                s.anim.play(&self.geo_lib, id, false);
                self.events.clear();
                s.anim.update(&self.geo_lib, dt, &mut self.events);
                // About to vanish (unreachable / left too long): blink.
                let blink_off = field.fading(c) && ((c.age * 6.0) as u32).is_multiple_of(2);
                if blink_off {
                    if s.shown {
                        s.win.set_visible(false);
                        s.shown = false;
                    }
                    continue;
                }
                s.draw(&self.geo_lib, &self.frames, c.pos, false, true, &self.blocked)?;
            }
        }
        Ok(())
    }

    /// Launch a fireball from `at` going right or left.
    pub fn fireball(&mut self, at: Vec2, right: bool, scale: f32) {
        if let Some(sprite) = self.sprite(Lib::Spells, "Ball") {
            let vel = Vec2::new(if right { SHOT_SPEED } else { -SHOT_SPEED } * scale, 0.0);
            self.shots.push(Shot { sprite, pos: at, vel, travelled: 0.0, ending: false });
        }
    }

    /// A one-shot clip at a fixed spot.
    pub fn flash(&mut self, lib: Lib, clip: &str, at: Vec2, flip: bool) {
        if let Some(sprite) = self.sprite(lib, clip) {
            self.flashes.push(Flash { sprite, lib, at, flip });
        }
    }

    /// Leave the mask piece (`key` from the knight's own bank) on the ground at `at`.
    pub fn drop_head(&mut self, key: SpriteKey, at: Vec2, secs: f32) {
        if let Some(h) = self.head.take() {
            h.win.set_visible(false);
            self.pool.push(h.win);
        }
        let Some(s) = self.sprite(Lib::Shade, "Idle") else { return };
        self.head = Some(Head { win: s.win, drawn: None, shown: false, key, at, left: secs });
    }

    /// Fireballs, flashes and the mask piece (`knight` = the knight's rasterized frames).
    pub fn update(&mut self, dt: f32, terrain: &Terrain, scale: f32, knight: &SpriteBank) -> Result<()> {
        let mut done = Vec::new();
        for (i, shot) in self.shots.iter_mut().enumerate() {
            self.events.clear();
            shot.sprite.anim.update(&self.spells_lib, dt, &mut self.events);
            if shot.ending {
                if shot.sprite.anim.is_finished() {
                    done.push(i);
                    continue;
                }
            } else {
                let from = shot.pos;
                let to = from + shot.vel * dt;
                shot.travelled += (to - from).length();
                let b = terrain.bounds;
                let wall = terrain.walls.iter().any(|w| w.y0 <= to.y && to.y <= w.y1 && (w.x - from.x) * (w.x - to.x) < 0.0);
                shot.pos = to;
                if wall || shot.travelled > SHOT_RANGE * scale || to.x < b.left || to.x > b.right {
                    shot.ending = true;
                    if let Some(id) = self.spells_lib.id("Ball End") {
                        shot.sprite.anim.play(&self.spells_lib, id, true);
                    }
                }
            }
            shot.sprite.draw(&self.spells_lib, &self.frames, shot.pos, shot.vel.x > 0.0, false, &self.blocked)?;
        }
        for i in done.into_iter().rev() {
            let s = self.shots.swap_remove(i).sprite;
            self.recycle(s);
        }

        let mut done = Vec::new();
        for (i, f) in self.flashes.iter_mut().enumerate() {
            let lib = match f.lib {
                Lib::Shade => &self.shade_lib,
                Lib::Geo => &self.geo_lib,
                Lib::Spells => &self.spells_lib,
            };
            self.events.clear();
            f.sprite.anim.update(lib, dt, &mut self.events);
            if f.sprite.anim.is_finished() {
                done.push(i);
                continue;
            }
            f.sprite.draw(lib, &self.frames, f.at, f.flip, false, &self.blocked)?;
        }
        for i in done.into_iter().rev() {
            let s = self.flashes.swap_remove(i).sprite;
            self.recycle(s);
        }

        if let Some(h) = &mut self.head {
            h.left -= dt;
            if h.left <= 0.0 {
                if let Some(h) = self.head.take() {
                    h.win.set_visible(false);
                    self.pool.push(h.win);
                }
            } else if let Some(frame) = knight.frame(h.key) {
                draw_frame(&mut h.win, &mut h.drawn, &mut h.shown, h.key, frame, h.at, false, true, &self.blocked)?;
            }
        }
        Ok(())
    }

    /// Keep our windows above other topmost windows (housekeeping).
    pub fn raise_all(&self) {
        for s in self.shade.iter().chain(self.coins.values()) {
            s.win.raise();
        }
    }
}

impl View {
    /// Fullscreen games on these monitors: hide everything now; drawing keeps whatever is
    /// on them hidden until the game goes away.
    pub fn hide_on(&mut self, monitors: &[RectF]) {
        self.blocked = monitors.to_vec();
        let all = self.shade.iter_mut().chain(self.shade_fx.iter_mut()).chain(self.coins.values_mut()).chain(self.shots.iter_mut().map(|s| &mut s.sprite)).chain(self.flashes.iter_mut().map(|f| &mut f.sprite));
        for s in all {
            if s.shown {
                s.win.set_visible(false);
                s.shown = false;
            }
        }
        if let Some(h) = &mut self.head {
            h.win.set_visible(false);
            h.shown = false;
        }
    }
}
