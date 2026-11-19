//! App side of the secret boss (`skong_core::enemy::grimm`): owns the director, loads his
//! art on a background thread when a show draws near (and drops it a minute after), and
//! renders Grimm, his scarlet flame and his flames (one small click-through window each,
//! pooled), the teleport flashes, and his lines in speech bubbles.
//!
//! Art is always vanilla (`skins/Vanilla/Grimm_Cln.png`, `Flameball_Cln.png`,
//! `Grimm_Flash_Cln.png` from the exporter). Without it the show still runs, drawn with
//! plain procedural shapes (a dark figure with red eyes, embers), so nothing breaks.
//!
//! The logic lives in the core; `app/grimm.rs` is the pet's glue (foes, blows, hits, the
//! reward, achievements, the lantern's placement).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Instant;

use anyhow::Result;
use skong_core::ai::Foe;
use skong_core::ai::rng::Rng;
use skong_core::anim::{ClipId, ClipLibrary};
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::bubble::{BubbleStyle, read_secs};
use skong_core::canvas::{Image, premul};
use skong_core::desktop::WindowId;
use skong_core::enemy::grimm::{GrimmDirector, GrimmEnv, GrimmEvent, GrimmSet, GrimmStep, GrimmWorld, HazardKind, Summon};
use skong_core::geom::{RectI, Vec2};
use skong_core::npc::TalkContext;
use skong_win::sprite_window::SpriteWindow;

use crate::bubble::{BubbleId, Bubbles, faded};
use crate::enemies::flashed;

/// Raster scales of his flames (x the knight's px per unit).
const BAT_SCALE: f32 = 0.5;
const PILLAR_SCALE: f32 = 0.7;
const FIREBALL_SCALE: f32 = 0.6;
const LANTERN_SCALE: f32 = 0.55;
/// Drop the art after this long with no show near (s).
const DROP_AFTER: f32 = 60.0;
/// Load the art this long before a natural lantern is due (s of presence).
const PRELOAD: f32 = 30.0;
/// At most this many flames drawn at once (the rest are still real, just not drawn).
const MAX_FLAMES: usize = 40;

/// His rasterized frames (empty maps: that art is missing; shapes stand in).
#[derive(Default)]
struct Art {
    grimm: HashMap<SpriteKey, Frame>,
    bats: HashMap<SpriteKey, Frame>,
    pillars: HashMap<SpriteKey, Frame>,
    fireballs: HashMap<SpriteKey, Frame>,
    lantern: HashMap<SpriteKey, Frame>,
    flash: HashMap<SpriteKey, Frame>,
    /// Px from his sprite origin down to his feet.
    origin_above_feet: f32,
}

/// What a window shows (redrawn only when it changes).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Look {
    Sprite(SpriteKey, bool, u8, u8),
    /// A procedural shape: (kind, size key, alpha).
    Shape(u8, u32, u8),
}

struct Shown {
    win: SpriteWindow,
    look: Option<Look>,
}

struct FlashFx {
    shown: Shown,
    at: Vec2,
    t: f32,
}

pub struct Troupe {
    set: GrimmSet,
    director: GrimmDirector,
    data_dir: PathBuf,
    vanilla: Option<Skin>,
    scale: f32,
    art: Option<Art>,
    loading: Option<Receiver<Art>>,
    /// Seconds with no show near (the art is dropped after a while).
    idle_for: f32,
    grimm_win: Option<Shown>,
    lantern_win: Option<Shown>,
    flames: HashMap<u32, Shown>,
    flashes: Vec<FlashFx>,
    pool: Vec<SpriteWindow>,
    bubble: Option<BubbleId>,
    rng: Rng,
    pub capturable: bool,
    hidden: bool,
}

impl Troupe {
    pub fn load(data_dir: &Path, vanilla: Option<&Skin>, scale: f32, seed: u32) -> Result<Troupe> {
        let meta = SpriteBank::load_metadata(data_dir)?;
        let set = GrimmSet::load(&data_dir.join("../grimm.json"), &data_dir.join("animations"), &meta)?;
        let director = GrimmDirector::new(&set.cfg, seed);
        log::info!(
            "grimm: ready (a lantern once {} achievements are found, then ~{:.0} min of presence){}",
            set.cfg.lantern.min_achievements,
            director.lantern_due_in() / 60.0,
            if vanilla.is_some_and(|s| s.atlas_for("Grimm Cln").is_some()) { "" } else { "; no Grimm art exported: shapes stand in" }
        );
        Ok(Troupe {
            set,
            director,
            data_dir: data_dir.to_path_buf(),
            vanilla: vanilla.cloned(),
            scale,
            art: None,
            loading: None,
            idle_for: 0.0,
            grimm_win: None,
            lantern_win: None,
            flames: HashMap::new(),
            flashes: Vec::new(),
            pool: Vec::new(),
            bubble: None,
            rng: Rng::new(seed ^ 0x6a17),
            capturable: false,
            hidden: false,
        })
    }

    fn ppu(&self) -> f32 {
        64.0 * self.scale
    }

    /// New render scale: the art is rasterized again when next needed.
    pub fn rescale(&mut self, scale: f32) {
        self.scale = scale;
        self.art = None;
        self.loading = None;
        self.forget_looks();
    }

    fn forget_looks(&mut self) {
        for s in self.grimm_win.iter_mut().chain(self.lantern_win.iter_mut()).chain(self.flames.values_mut()) {
            s.look = None;
        }
    }

    /// The debug command (panel only).
    pub fn request(&mut self, lantern: bool) {
        self.director.request(if lantern { Summon::Lantern } else { Summon::Direct });
    }

    /// The flame or the show is on (or about to be): nothing else should start.
    pub fn active(&self) -> bool {
        self.director.active()
    }

    pub fn foes(&self) -> Vec<Foe> {
        self.director.foes(&self.set, self.ppu())
    }

    /// The knight's blow on Grimm: `Some(beaten)` when it landed.
    pub fn strike(&mut self, damage: u32) -> Option<bool> {
        self.director.strike(damage, &self.set)
    }

    /// The knight struck the lantern.
    pub fn strike_lantern(&mut self, knight_x: f32) -> Option<GrimmEvent> {
        let ppu = self.ppu();
        self.director.strike_lantern(&self.set, knight_x, ppu)
    }

    /// Grimm's body centre (hit markers).
    pub fn centre(&self) -> Option<Vec2> {
        self.director.grimm().map(|g| g.body(&self.set, self.ppu()).center())
    }

    /// Where the flame or Grimm is (game-monitor checks).
    pub fn spot(&self) -> Option<Vec2> {
        self.director.grimm().map(|g| g.pos).or(self.director.lantern().map(|l| l.at))
    }

    /// A game took the stage: everything vanishes at once.
    pub fn vanish(&mut self, bubbles: Option<&mut Bubbles>) -> Vec<GrimmEvent> {
        let ev = self.director.vanish();
        self.clear_windows(bubbles);
        ev
    }

    pub fn lantern_placed(&mut self, at: Option<(Vec2, Option<WindowId>)>, env: &GrimmEnv) -> Option<GrimmEvent> {
        self.director.lantern_placed(at, env)
    }

    /// Hide everything (games on every monitor); the next tick shows it again.
    pub fn hide_all(&mut self) {
        if !self.hidden {
            self.hidden = true;
            for s in self.grimm_win.iter().chain(self.lantern_win.iter()).chain(self.flames.values()) {
                s.win.set_visible(false);
            }
        }
    }

    pub fn raise_all(&self) {
        for s in self.grimm_win.iter().chain(self.lantern_win.iter()).chain(self.flames.values()).chain(self.flashes.iter().map(|f| &f.shown)) {
            s.win.raise();
        }
    }

    /// The art: loaded in the background when a show draws near, dropped after.
    fn art_housekeeping(&mut self, dt: f32, achievements: usize) {
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(art) => {
                    self.art = Some(art);
                    self.loading = None;
                    self.forget_looks();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.art = Some(Art::default());
                }
            }
        }
        let soon = achievements >= self.set.cfg.lantern.min_achievements && self.set.cfg.lantern.enabled && self.director.lantern_due_in() < PRELOAD;
        if self.director.active() || soon {
            self.idle_for = 0.0;
            if self.art.is_none() && self.loading.is_none() {
                self.start_loading();
            }
        } else {
            self.idle_for += dt;
            if self.idle_for > DROP_AFTER && self.art.take().is_some() {
                log::debug!("grimm: art dropped until the next show");
            }
        }
    }

    fn start_loading(&mut self) {
        let Some(skin) = self.vanilla.clone() else {
            self.art = Some(Art::default());
            return;
        };
        let ppu = self.ppu();
        let s = &self.set;
        let frames = |lib: &ClipLibrary, clip: Option<ClipId>| -> Vec<SpriteKey> {
            match clip {
                Some(c) => lib.get(c).frames.iter().filter_map(|f| f.sprite).collect(),
                None => lib.sprite_keys(),
            }
        };
        let sets: Vec<(Vec<SpriteKey>, f32)> = vec![
            (frames(&s.lib, None), ppu * s.cfg.art_scale),
            (frames(&s.lib, Some(s.clips.firebat)), ppu * BAT_SCALE),
            (frames(&s.lib, Some(s.clips.pillar)), ppu * PILLAR_SCALE),
            (frames(&s.fx, Some(s.fireball)), ppu * FIREBALL_SCALE),
            (frames(&s.fx, Some(s.fireball)), ppu * LANTERN_SCALE),
            (s.flash.as_ref().map(|(l, c)| frames(l, Some(*c))).unwrap_or_default(), ppu * s.cfg.art_scale),
        ];
        let idle = s.lib.get(s.clips.idle).frames.iter().find_map(|f| f.sprite);
        let dir = self.data_dir.clone();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("grimm-art".into()).spawn(move || {
            let t = Instant::now();
            let load = || -> Result<Art> {
                let bank = SpriteBank::load_metadata(&dir)?;
                let refs: Vec<(&[SpriteKey], f32)> = sets.iter().map(|(k, s)| (k.as_slice(), *s)).collect();
                let mut v = bank.rasterize_sets(&skin, None, &refs)?.into_iter();
                let mut next = || v.next().unwrap_or_default();
                let grimm = next();
                let origin_above_feet = idle.and_then(|k| grimm.get(&k)).map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
                Ok(Art { grimm, bats: next(), pillars: next(), fireballs: next(), lantern: next(), flash: next(), origin_above_feet })
            };
            let art = load().unwrap_or_else(|e| {
                log::warn!("grimm art: {e:#}; shapes stand in");
                Art::default()
            });
            log::info!("grimm: art ready in {:?} ({} frames{})", t.elapsed(), art.grimm.len(), if art.grimm.is_empty() { ", none: shapes stand in" } else { "" });
            let _ = tx.send(art);
        });
        match spawned {
            Ok(_) => self.loading = Some(rx),
            Err(e) => {
                log::warn!("grimm art thread: {e}");
                self.art = Some(Art::default());
            }
        }
    }

    /// Advance the director and draw. `w.ready` should say whether placements can be
    /// route-checked; this adds whether the art is there. The app handles the returned
    /// step's lantern placement, hits, reward and events.
    pub fn tick(&mut self, dt: f32, w: &GrimmWorld, env: &GrimmEnv, mut bubbles: Option<&mut Bubbles>, talk: &TalkContext, area: RectI) -> GrimmStep {
        if self.hidden {
            self.hidden = false;
            for s in self.grimm_win.iter().chain(self.lantern_win.iter()).chain(self.flames.values()) {
                s.win.set_visible(true);
            }
        }
        self.art_housekeeping(dt, w.achievements);
        let world = GrimmWorld { ready: w.ready && self.art.is_some(), ..*w };
        let step = self.director.tick(dt, &world, env, &self.set);
        for e in &step.events {
            log::info!("event: {} {e:?}", e.name());
        }
        if let Some(kind) = step.say
            && let Some(b) = bubbles.as_deref_mut()
        {
            let line = self.set.cfg.lines.pick(kind, talk, &mut self.rng);
            if let Some(old) = self.bubble.take() {
                b.close(old);
            }
            if let (Some(text), Some(head)) = (line, self.head()) {
                let secs = read_secs(&text) + 1.2;
                self.bubble = b.say(head, &text, BubbleStyle::Speech, secs, area);
            }
        }
        if let Some(b) = bubbles.as_deref_mut()
            && let Some(id) = self.bubble
        {
            match self.head() {
                Some(head) if b.is_open(id) => b.follow(id, head),
                _ => {
                    b.close(id);
                    self.bubble = None;
                }
            }
        }
        for at in &step.flashes {
            self.flash_at(*at);
        }
        if let Err(e) = self.draw(dt) {
            log::warn!("grimm: {e:#}");
        }
        if !self.director.active() && (self.grimm_win.is_some() || self.lantern_win.is_some() || !self.flames.is_empty()) {
            self.clear_windows(bubbles);
        }
        step
    }

    /// Just above his head (the bubble's anchor).
    fn head(&self) -> Option<Vec2> {
        let g = self.director.grimm()?;
        let u = self.ppu();
        Some(Vec2::new(g.pos.x, g.pos.y - self.set.cfg.size[1] * u - 0.4 * u))
    }

    fn take_window(&mut self) -> Option<Shown> {
        let win = match self.pool.pop() {
            Some(w) => {
                w.set_visible(true);
                w
            }
            None => match SpriteWindow::new() {
                Ok(w) => {
                    w.set_click_through(true);
                    w
                }
                Err(e) => {
                    log::warn!("grimm window: {e:#}");
                    return None;
                }
            },
        };
        win.set_capturable(self.capturable);
        Some(Shown { win, look: None })
    }

    fn release(&mut self, s: Shown) {
        s.win.set_visible(false);
        self.pool.push(s.win);
    }

    fn clear_windows(&mut self, bubbles: Option<&mut Bubbles>) {
        let mut all: Vec<Shown> = self.grimm_win.take().into_iter().chain(self.lantern_win.take()).collect();
        all.extend(self.flames.drain().map(|(_, s)| s));
        all.extend(self.flashes.drain(..).map(|f| f.shown));
        for s in all {
            self.release(s);
        }
        if let (Some(b), Some(id)) = (bubbles, self.bubble.take()) {
            b.close(id);
        }
    }

    fn flash_at(&mut self, at: Vec2) {
        if self.art.as_ref().is_some_and(|a| !a.flash.is_empty())
            && self.flashes.len() < 4
            && let Some(shown) = self.take_window()
        {
            self.flashes.push(FlashFx { shown, at, t: 0.0 });
        }
    }

    fn draw(&mut self, dt: f32) -> Result<()> {
        let ppu = self.ppu();
        let Some(art) = self.art.take() else { return Ok(()) };
        let res = self.draw_with(&art, dt, ppu);
        self.art = Some(art);
        res
    }

    fn draw_with(&mut self, art: &Art, dt: f32, ppu: f32) -> Result<()> {
        // Grimm.
        let seen = self.director.grimm().filter(|g| g.alpha > 0.01).map(|g| (g.pos, g.alpha, g.flash, g.facing_right, g.sprite(&self.set), g.body(&self.set, ppu)));
        match seen {
            Some((pos, alpha, flash, flip, key, body)) => {
                if self.grimm_win.is_none() {
                    self.grimm_win = self.take_window();
                }
                let alpha = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
                let flash = (flash.clamp(0.0, 1.0) * 4.0).round() as u8;
                if let Some(s) = self.grimm_win.as_mut() {
                    match key.and_then(|k| art.grimm.get(&k).map(|f| (k, f))) {
                        Some((k, frame)) => {
                            let origin = Vec2::new(pos.x, pos.y - art.origin_above_feet);
                            let (x, y) = ((origin.x - frame.anchor_x(flip)).round() as i32, (origin.y - frame.anchor[1]).round() as i32);
                            let look = Look::Sprite(k, flip, alpha, flash);
                            if s.look != Some(look) {
                                let mut img = std::borrow::Cow::Borrowed(&frame.image);
                                if flash > 0 {
                                    img = std::borrow::Cow::Owned(flashed(&img, flash as f32 / 4.0));
                                }
                                if alpha < 255 {
                                    img = std::borrow::Cow::Owned(faded(&img, alpha));
                                }
                                s.win.present_flipped(&img, x, y, flip)?;
                                s.look = Some(look);
                            } else {
                                s.win.move_to(x, y)?;
                            }
                        }
                        None => {
                            // No art: a dark figure with red eyes.
                            let (w, h) = (body.width().round() as u32, body.height().round() as u32);
                            let look = Look::Shape(0, (w << 16) | h, alpha);
                            let (x, y) = (body.left.round() as i32, body.top.round() as i32);
                            if s.look != Some(look) {
                                s.win.present(&faded(&figure(w, h), alpha), x, y)?;
                                s.look = Some(look);
                            } else {
                                s.win.move_to(x, y)?;
                            }
                        }
                    }
                }
            }
            None => {
                if let Some(s) = self.grimm_win.take() {
                    self.release(s);
                }
            }
        }

        // The lantern's scarlet flame: dim, flickering.
        match self.director.lantern().copied() {
            Some(l) => {
                if self.lantern_win.is_none() {
                    self.lantern_win = self.take_window();
                }
                let flicker = 0.62 + 0.12 * (l.age * 5.3).sin() + 0.06 * (l.age * 11.7).sin();
                let alpha = (l.alpha.clamp(0.0, 1.0) * flicker * 255.0) as u8;
                let clip = self.set.fx.get(self.set.fireball);
                let key = frame_of(clip.frames.iter().filter_map(|f| f.sprite).collect::<Vec<_>>().as_slice(), l.age * clip.fps * 0.7, true);
                if let Some(s) = self.lantern_win.as_mut() {
                    let (img, look) = match key.and_then(|k| art.lantern.get(&k).map(|f| (k, f))) {
                        Some((k, f)) => (f.image.clone(), Look::Sprite(k, false, alpha / 8 * 8, 0)),
                        None => {
                            let d = (0.7 * ppu) as u32;
                            (ember(d, d), Look::Shape(1, d, alpha / 8 * 8))
                        }
                    };
                    let (x, y) = ((l.at.x - img.width as f32 * 0.5).round() as i32, (l.at.y - img.height as f32 + 0.1 * ppu).round() as i32);
                    if s.look != Some(look) {
                        s.win.present(&faded(&img, alpha), x, y)?;
                        s.look = Some(look);
                    } else {
                        s.win.move_to(x, y)?;
                    }
                }
            }
            None => {
                if let Some(s) = self.lantern_win.take() {
                    self.release(s);
                }
            }
        }

        // His flames.
        let hazards: Vec<_> = self.director.grimm().map(|g| g.hazards.iter().take(MAX_FLAMES).copied().collect()).unwrap_or_default();
        let gone: Vec<u32> = self.flames.keys().copied().filter(|id| !hazards.iter().any(|h| h.id == *id)).collect();
        for id in gone {
            if let Some(s) = self.flames.remove(&id) {
                self.release(s);
            }
        }
        for h in &hazards {
            if !self.flames.contains_key(&h.id)
                && let Some(s) = self.take_window()
            {
                self.flames.insert(h.id, s);
            }
            let Some(s) = self.flames.get_mut(&h.id) else { continue };
            let c = &self.set.clips;
            let sprite = |lib: &ClipLibrary, clip: ClipId, t: f32, looped: bool| {
                let cl = lib.get(clip);
                frame_of(cl.frames.iter().filter_map(|f| f.sprite).collect::<Vec<_>>().as_slice(), t * cl.fps, looped)
            };
            let flip = h.vel.x > 0.0; // the art flies left
            let (img, look, bottom) = match h.kind {
                HazardKind::Firebat => match sprite(&self.set.lib, c.firebat, h.age, true).and_then(|k| art.bats.get(&k).map(|f| (k, f))) {
                    Some((k, f)) => (f.image.clone(), Look::Sprite(k, flip, 255, 0), false),
                    None => (ember((2.0 * h.half.x) as u32, (2.0 * h.half.y) as u32), Look::Shape(2, 0, 255), false),
                },
                HazardKind::Fireball => match sprite(&self.set.fx, self.set.fireball, h.age, true).and_then(|k| art.fireballs.get(&k).map(|f| (k, f))) {
                    Some((k, f)) => (f.image.clone(), Look::Sprite(k, flip, 255, 0), false),
                    None => (ember((2.0 * h.half.x) as u32, (2.0 * h.half.y) as u32), Look::Shape(3, 0, 255), false),
                },
                HazardKind::Pillar if h.armed() => match sprite(&self.set.lib, c.pillar, h.lit, false).and_then(|k| art.pillars.get(&k).map(|f| (k, f))) {
                    Some((k, f)) => (f.image.clone(), Look::Sprite(k, false, 255, 0), true),
                    None => {
                        let (w, hh) = ((2.0 * h.half.x) as u32, (2.0 * h.half.y) as u32);
                        (column(w, hh), Look::Shape(4, (w << 16) | hh, 255), true)
                    }
                },
                HazardKind::Pillar => {
                    // The warning: an ember glow rising from the ground where it will erupt.
                    let grow = (1.0 - h.warn / 0.65).clamp(0.0, 1.0);
                    let (w, hh) = ((2.0 * h.half.x) as u32, (ppu * (0.25 + 0.6 * grow)) as u32);
                    (glow(w, hh), Look::Shape(5, (w << 16) | hh, 255), true)
                }
                HazardKind::Spikes => {
                    // Peeking (dark, short) while they warn, then up (tall, bright tips).
                    let up = h.armed();
                    let (w, hh) = ((2.0 * h.half.x) as u32, if up { (2.0 * h.half.y) as u32 } else { (0.35 * 2.0 * h.half.y) as u32 });
                    (spikes(w, hh, ppu, up), Look::Shape(if up { 6 } else { 7 }, (w << 16) | hh, 255), true)
                }
            };
            let x = (h.pos.x - img.width as f32 * 0.5).round() as i32;
            let y = if bottom { (h.pos.y + h.half.y - img.height as f32).round() as i32 } else { (h.pos.y - img.height as f32 * 0.5).round() as i32 };
            if s.look != Some(look) {
                s.win.present_flipped(&img, x, y, flip && matches!(look, Look::Sprite(..)))?;
                s.look = Some(look);
            } else {
                s.win.move_to(x, y)?;
            }
        }

        // Teleport flashes.
        let mut done = Vec::new();
        if let Some((lib, clip)) = &self.set.flash {
            let cl = lib.get(*clip);
            let keys: Vec<SpriteKey> = cl.frames.iter().filter_map(|f| f.sprite).collect();
            for (i, f) in self.flashes.iter_mut().enumerate() {
                f.t += dt;
                let n = (f.t * cl.fps) as usize;
                match keys.get(n).and_then(|k| art.flash.get(k).map(|fr| (*k, fr))) {
                    Some((k, fr)) => {
                        let (x, y) = ((f.at.x - fr.anchor_x(false)).round() as i32, (f.at.y - fr.anchor[1]).round() as i32);
                        let look = Look::Sprite(k, false, 255, 0);
                        if f.shown.look != Some(look) {
                            f.shown.win.present(&fr.image, x, y)?;
                            f.shown.look = Some(look);
                        }
                    }
                    None => done.push(i),
                }
            }
        } else {
            done.extend(0..self.flashes.len());
        }
        for i in done.into_iter().rev() {
            let f = self.flashes.remove(i);
            self.release(f.shown);
        }
        Ok(())
    }
}

/// The frame of a clip's sprites at frame position `n` (looped or held on the last).
fn frame_of(keys: &[SpriteKey], n: f32, looped: bool) -> Option<SpriteKey> {
    if keys.is_empty() {
        return None;
    }
    let i = n.max(0.0) as usize;
    Some(keys[if looped { i % keys.len() } else { i.min(keys.len() - 1) }])
}

/// Fallback Grimm: a dark, tall figure with two red eyes.
fn figure(w: u32, h: u32) -> Image {
    let mut img = Image::new(w.max(2), h.max(2));
    let (cw, ch) = (img.width as f32, img.height as f32);
    for y in 0..img.height {
        for x in 0..img.width {
            let (nx, ny) = ((x as f32 + 0.5) / cw * 2.0 - 1.0, (y as f32 + 0.5) / ch);
            // A cloak widening downwards, a round head on top.
            let head = (nx * nx) / 0.16 + ((ny - 0.12) * (ny - 0.12)) / 0.012 <= 1.0;
            let cloak = ny > 0.18 && nx.abs() <= 0.35 + 0.6 * (ny - 0.18);
            if head || cloak {
                let eye = (ny - 0.12).abs() < 0.025 && (nx.abs() - 0.15).abs() < 0.07;
                img.px[(y * img.width + x) as usize] = if eye { premul(230, 40, 50, 255) } else { premul(28, 18, 24, 245) };
            }
        }
    }
    img
}

/// A soft scarlet ember (fallback flames, the fallback lantern).
fn ember(w: u32, h: u32) -> Image {
    let mut img = Image::new(w.max(2), h.max(2));
    let (cw, ch) = (img.width as f32, img.height as f32);
    for y in 0..img.height {
        for x in 0..img.width {
            let (nx, ny) = ((x as f32 + 0.5) / cw * 2.0 - 1.0, (y as f32 + 0.5) / ch * 2.0 - 1.0);
            let d = (nx * nx + ny * ny).sqrt();
            if d < 1.0 {
                let a = ((1.0 - d) * 1.6).min(1.0);
                img.px[(y * img.width + x) as usize] = premul(255, (90.0 + 120.0 * (1.0 - d)) as u8, 60, (a * 235.0) as u8);
            }
        }
    }
    img
}

/// Fallback pillar: a translucent red column, brighter in the middle.
fn column(w: u32, h: u32) -> Image {
    let mut img = Image::new(w.max(2), h.max(2));
    let cw = img.width as f32;
    for y in 0..img.height {
        for x in 0..img.width {
            let nx = ((x as f32 + 0.5) / cw * 2.0 - 1.0).abs();
            let a = ((1.0 - nx) * 1.4).min(1.0);
            img.px[(y * img.width + x) as usize] = premul(255, (70.0 + 140.0 * (1.0 - nx)) as u8, 80, (a * 220.0) as u8);
        }
    }
    img
}

/// A pillar's warning: a glow fading upwards from the ground.
fn glow(w: u32, h: u32) -> Image {
    let mut img = Image::new(w.max(2), h.max(2));
    let (cw, ch) = (img.width as f32, img.height as f32);
    for y in 0..img.height {
        for x in 0..img.width {
            let nx = ((x as f32 + 0.5) / cw * 2.0 - 1.0).abs();
            let up = 1.0 - (y as f32 + 0.5) / ch; // 0 at the ground, 1 at the top
            let a = (1.0 - nx).max(0.0) * (1.0 - up) * 0.9;
            img.px[(y * img.width + x) as usize] = premul(255, 60, 40, (a * 255.0) as u8);
        }
    }
    img
}

/// A row of spikes along the ground: dark blades with red tips (`up`), or just peeking.
fn spikes(w: u32, h: u32, ppu: f32, up: bool) -> Image {
    let mut img = Image::new(w.max(2), h.max(2));
    let pitch = (0.5 * ppu).max(6.0);
    let ch = img.height as f32;
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = (x as f32 % pitch) / pitch * 2.0 - 1.0; // -1..1 across one spike
            let from_top = (y as f32 + 0.5) / ch; // 0 at the tip line, 1 at the ground
            if fx.abs() <= from_top {
                let tip = from_top < 0.3 && up;
                img.px[(y * img.width + x) as usize] = if tip { premul(220, 40, 50, 255) } else { premul(30, 16, 22, if up { 250 } else { 200 }) };
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_frames_loop_or_hold() {
        let k = |i| SpriteKey { collection: 0, sprite: i };
        let keys = [k(1), k(2), k(3)];
        assert_eq!(frame_of(&keys, 4.2, true), Some(k(2)));
        assert_eq!(frame_of(&keys, 9.0, false), Some(k(3)), "held on the last frame");
        assert_eq!(frame_of(&[], 1.0, true), None);
    }

    #[test]
    fn fallback_shapes_are_drawn_and_sized() {
        for img in [figure(40, 90), ember(20, 20), column(30, 200), glow(30, 20), spikes(200, 40, 38.4, true)] {
            assert!(img.width >= 2 && img.height >= 2);
            assert!(img.px.iter().any(|p| p >> 24 > 0), "something visible");
        }
        for img in [figure(40, 90), ember(20, 20), spikes(200, 40, 38.4, true)] {
            assert!(img.px.iter().any(|p| p >> 24 == 0), "a shape, with transparent space around it");
        }
        // Spikes peek darker than they rise.
        let peek = spikes(100, 12, 38.4, false);
        assert!(peek.px.iter().all(|p| p >> 24 <= 200));
    }
}
