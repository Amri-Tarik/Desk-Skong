//! Runtime for hero effects: each live effect is a small sprite window (pooled) playing
//! its clip; attached effects follow the knight, detached ones stay where they spawned.
//!
//! Procedural particles (`skong_core::particles`: dust bits, crystal shards, Geo sparkles)
//! ride along: each live burst is one small click-through window sized to its particles'
//! bounding box, redrawn at most [`PARTICLE_FPS`] times a second, gone when they are.
use std::collections::HashMap;

use anyhow::Result;
use skong_core::anim::{AnimEvent, Animator};
use skong_core::assets::{Frame, FrameStore, SpriteKey};
use skong_core::canvas::Image;
use skong_core::effects::EffectSet;
use skong_core::geom::Vec2;
use skong_core::particles::{Burst, Spec};
use skong_win::sprite_window::SpriteWindow;

/// Redraw rate of particle windows (the tick runs at 120 Hz).
const PARTICLE_FPS: f32 = 60.0;
/// At most this many particle bursts at once (new one-shots are skipped beyond it).
const MAX_BURSTS: usize = 8;
/// Unpacked effect frames not drawn for this long are dropped (they stay packed).
const EVICT_PERIOD: f32 = 60.0;

struct LiveBurst {
    /// Streams (fed every tick while their condition holds) and `particles_once` bursts
    /// carry a tag; plain one-shots don't.
    tag: Option<&'static str>,
    burst: Burst,
    win: SpriteWindow,
    since_draw: f32,
    shown: bool,
}

/// A stream request for this tick: (key, spec, where, dir, px per unit).
type Feed = (&'static str, Spec, Vec2, f32, f32);

struct Live {
    def: usize,
    anim: Animator,
    /// Spawn-time screen position (detached effects) and facing.
    at: Vec2,
    right: bool,
    win: SpriteWindow,
    drawn: Option<(SpriteKey, bool)>,
}

pub struct Fx {
    pub set: EffectSet,
    /// Rasterized frames per effect def (each at its own scale), packed until drawn.
    frames: Vec<FrameStore>,
    /// Seconds to the next eviction of unpacked frames no effect drew meanwhile.
    evict_in: f32,
    live: Vec<Live>,
    pool: Vec<SpriteWindow>,
    events: Vec<AnimEvent>,
    pub capturable: bool,
    bursts: Vec<LiveBurst>,
    /// Particle windows are click-through: kept apart from the effect pool.
    burst_pool: Vec<SpriteWindow>,
    feeds: Vec<Feed>,
    scratch: Image,
    seed: u32,
}

impl Fx {
    pub fn new(set: EffectSet) -> Self {
        let n = set.defs.len();
        Self {
            set,
            frames: (0..n).map(|_| FrameStore::default()).collect(),
            evict_in: EVICT_PERIOD,
            live: Vec::new(),
            pool: Vec::new(),
            events: Vec::new(),
            capturable: false,
            bursts: Vec::new(),
            burst_pool: Vec::new(),
            feeds: Vec::new(),
            scratch: Image::default(),
            seed: 0x5EED_D057,
        }
    }

    /// A one-shot particle burst at screen point `at` (`dir`: +1 right, -1 left).
    pub fn particles(&mut self, spec: Spec, at: Vec2, dir: f32, ppu: f32) {
        self.one_shot(None, spec, at, dir, ppu);
    }

    /// Like [`Self::particles`], but skipped while an earlier burst tagged `tag` is still
    /// alive (rapid repeats, e.g. a shower of Geo picked up, stay one quiet sparkle).
    pub fn particles_once(&mut self, tag: &'static str, spec: Spec, at: Vec2, dir: f32, ppu: f32) {
        if !self.bursts.iter().any(|b| b.tag == Some(tag)) {
            self.one_shot(Some(tag), spec, at, dir, ppu);
        }
    }

    fn one_shot(&mut self, tag: Option<&'static str>, spec: Spec, at: Vec2, dir: f32, ppu: f32) {
        if self.bursts.len() >= MAX_BURSTS {
            return;
        }
        let mut burst = Burst::new(spec, ppu, self.next_seed());
        burst.emit(at, dir);
        self.add_burst(tag, burst);
    }

    /// Keep the particle stream `key` going this tick (it dies out once no longer fed).
    pub fn stream(&mut self, key: &'static str, spec: Spec, at: Vec2, dir: f32, ppu: f32) {
        self.feeds.push((key, spec, at, dir, ppu));
    }

    fn next_seed(&mut self) -> u32 {
        self.seed = self.seed.wrapping_add(0x9E37_79B9);
        self.seed
    }

    fn add_burst(&mut self, tag: Option<&'static str>, burst: Burst) {
        let win = match self.burst_pool.pop() {
            Some(w) => w,
            None => match SpriteWindow::new() {
                Ok(w) => {
                    w.set_capturable(self.capturable);
                    w.set_click_through(true);
                    w.set_visible(false);
                    w
                }
                Err(e) => {
                    log::warn!("particle window: {e:#}");
                    return;
                }
            },
        };
        self.bursts.push(LiveBurst { tag, burst, win, since_draw: f32::INFINITY, shown: false });
    }

    /// Feed streams, step the physics, redraw (throttled), retire empty bursts.
    fn update_particles(&mut self, dt: f32) -> Result<()> {
        for (key, spec, at, dir, ppu) in std::mem::take(&mut self.feeds) {
            if let Some(b) = self.bursts.iter_mut().find(|b| b.tag == Some(key)) {
                b.burst.feed(dt, at, dir);
            } else {
                // A new stream starts with one emission right away.
                self.one_shot(Some(key), spec, at, dir, ppu);
            }
        }
        let mut i = 0;
        while i < self.bursts.len() {
            let b = &mut self.bursts[i];
            b.burst.step(dt);
            if b.burst.is_empty() {
                let b = self.bursts.swap_remove(i);
                b.win.set_visible(false);
                self.burst_pool.push(b.win);
                continue;
            }
            b.since_draw += dt;
            if b.since_draw >= 1.0 / PARTICLE_FPS
                && let Some((x, y)) = b.burst.draw(&mut self.scratch)
            {
                b.since_draw = 0.0;
                b.win.present(&self.scratch, x, y)?;
                if !b.shown {
                    b.win.set_visible(true);
                    b.shown = true;
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// (keys, scale multiplier) per def, for a combined rasterization pass.
    pub fn raster_sets(&self) -> Vec<(Vec<SpriteKey>, f32)> {
        (0..self.set.defs.len()).map(|i| (self.set.sprite_keys(i), self.set.defs[i].raster_scale())).collect()
    }

    /// Install rasterized frames; rotated / squashed effects get their transform baked here.
    pub fn install_frames(&mut self, mut frames: Vec<HashMap<SpriteKey, Frame>>) {
        for (def, map) in self.set.defs.iter().zip(frames.iter_mut()) {
            if def.baked() {
                for f in map.values_mut() {
                    *f = def.bake(f);
                }
            }
        }
        self.frames = frames.into_iter().map(FrameStore::pack).collect();
        for l in &mut self.live {
            l.drawn = None;
        }
    }

    /// Start effect `name` for a knight with sprite origin `origin`.
    pub fn spawn(&mut self, name: &str, origin: Vec2, right: bool) {
        let Some(def) = self.set.id(name) else {
            log::debug!("no effect {name}");
            return;
        };
        log::debug!("fx {name}");
        let win = match self.pool.pop() {
            Some(w) => w,
            None => match SpriteWindow::new() {
                Ok(w) => {
                    w.set_capturable(self.capturable);
                    w
                }
                Err(e) => {
                    log::warn!("effect window: {e:#}");
                    return;
                }
            },
        };
        let (_, clip) = self.set.clips[def];
        let mut anim = Animator::new(clip);
        anim.play(self.set.lib(def), clip, true);
        self.live.push(Live { def, anim, at: origin, right, win, drawn: None });
    }

    /// End every live instance of `name` (looping effects like the superdash trail).
    pub fn stop(&mut self, name: &str) {
        let Some(def) = self.set.id(name) else { return };
        let (gone, keep): (Vec<Live>, Vec<Live>) = std::mem::take(&mut self.live).into_iter().partition(|l| l.def == def);
        self.live = keep;
        self.recycle(gone);
    }

    pub fn is_live(&self, name: &str) -> bool {
        self.set.id(name).is_some_and(|d| self.live.iter().any(|l| l.def == d))
    }

    /// Advance and draw. `origin` / `right` describe the knight now; `ppu` the scale.
    pub fn update(&mut self, dt: f32, origin: Vec2, right: bool, ppu: f32) -> Result<()> {
        self.evict_in -= dt;
        if self.evict_in <= 0.0 {
            self.evict_in = EVICT_PERIOD;
            for f in &mut self.frames {
                f.evict_unused();
            }
        }
        let mut finished = Vec::new();
        for (i, l) in self.live.iter_mut().enumerate() {
            let def = &self.set.defs[l.def];
            let lib = &self.set.libs[self.set.clips[l.def].0];
            self.events.clear();
            l.anim.update(lib, dt, &mut self.events);
            if l.anim.is_finished() {
                finished.push(i);
                continue;
            }
            let (base, facing) = if def.attached { (origin, right) } else { (l.at, l.right) };
            let pos = def.place(base, facing, ppu);
            let flip = def.mirrored(facing);
            let Some(key) = l.anim.current(lib) else { continue };
            let Some(frame) = self.frames[l.def].get(&key) else {
                log::debug!("fx {}: frame {key:?} not rasterized", def.name);
                continue;
            };
            let x = (pos.x - frame.anchor_x(flip)).round() as i32;
            let y = (pos.y - frame.anchor[1]).round() as i32;
            if l.drawn != Some((key, flip)) {
                l.win.present_flipped(&frame.image, x, y, flip)?;
                l.win.set_visible(true);
                l.drawn = Some((key, flip));
            } else {
                l.win.move_to(x, y)?;
            }
        }
        for i in finished.into_iter().rev() {
            let l = self.live.swap_remove(i);
            self.recycle(vec![l]);
        }
        self.update_particles(dt)
    }

    fn recycle(&mut self, gone: Vec<Live>) {
        for l in gone {
            l.win.set_visible(false);
            self.pool.push(l.win);
        }
    }
}
