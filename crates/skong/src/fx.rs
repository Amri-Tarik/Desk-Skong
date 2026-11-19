//! Runtime for hero effects: each live effect is a small sprite window (pooled) playing
//! its clip; attached effects follow the knight, detached ones stay where they spawned.
use std::collections::HashMap;

use anyhow::Result;
use skong_core::anim::{AnimEvent, Animator};
use skong_core::assets::{Frame, SpriteKey};
use skong_core::effects::EffectSet;
use skong_core::geom::Vec2;
use skong_win::sprite_window::SpriteWindow;

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
    /// Rasterized frames per effect def (each at its own scale).
    frames: Vec<HashMap<SpriteKey, Frame>>,
    live: Vec<Live>,
    pool: Vec<SpriteWindow>,
    events: Vec<AnimEvent>,
    pub capturable: bool,
}

impl Fx {
    pub fn new(set: EffectSet) -> Self {
        let n = set.defs.len();
        Self { set, frames: vec![HashMap::new(); n], live: Vec::new(), pool: Vec::new(), events: Vec::new(), capturable: false }
    }

    /// (keys, scale multiplier) per def, for a combined rasterization pass.
    pub fn raster_sets(&self) -> Vec<(Vec<SpriteKey>, f32)> {
        (0..self.set.defs.len()).map(|i| (self.set.sprite_keys(i), self.set.defs[i].raster_scale())).collect()
    }

    pub fn install_frames(&mut self, frames: Vec<HashMap<SpriteKey, Frame>>) {
        self.frames = frames;
        for l in &mut self.live {
            l.drawn = None;
        }
    }

    /// Start effect `name` for a knight with sprite origin `origin`.
    pub fn spawn(&mut self, name: &str, origin: Vec2, right: bool) {
        let Some(def) = self.set.id(name) else { return };
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
            let Some(frame) = self.frames[l.def].get(&key) else { continue };
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
        Ok(())
    }

    fn recycle(&mut self, gone: Vec<Live>) {
        for l in gone {
            l.win.set_visible(false);
            self.pool.push(l.win);
        }
    }
}
