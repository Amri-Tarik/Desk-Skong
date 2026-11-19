//! App side of enemies: schedules their arrivals (rare, organic; or when the bored knight
//! asks), picks where they show up, renders each in its own sprite window (vanilla art,
//! a white flash when struck), applies the knight's blows, and reports bumps and kills.
//!
//! The logic lives in `skong_core::enemy`; this module is rendering and plumbing.
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use skong_core::ai::Foe;
use skong_core::ai::rng::Rng;
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::canvas::Image;
use skong_core::enemy::{Enemy, EnemyEnv, EnemySet, EntryKind, plan_arrival};
use skong_core::geom::{RectF, Vec2};
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::npc::{VisitConditions, VisitScheduler};
use skong_core::portal::Footing;
use skong_win::sprite_window::SpriteWindow;

use crate::bubble::faded;
use crate::npcs::World;

struct LiveEnemy {
    enemy: Enemy,
    win: SpriteWindow,
    /// Last drawn (frame, mirrored, alpha, flash level).
    drawn: Option<(SpriteKey, bool, u8, u8)>,
}

/// What happened this tick, for the pet.
#[derive(Default)]
pub struct EnemyNews {
    /// Bumps the knight took.
    pub hits: u32,
}

pub struct Enemies {
    set: EnemySet,
    meta: SpriteBank,
    frames: Vec<HashMap<SpriteKey, Frame>>,
    /// Per def: sprite origin above the feet (px at the current scale).
    origin_above_feet: Vec<f32>,
    live: Vec<LiveEnemy>,
    sched: VisitScheduler,
    rng: Rng,
    next_id: u32,
    ppu: f32,
    pub capturable: bool,
    /// More of the current group still to come, and the countdown to the next one.
    pending: usize,
    pending_in: f32,
    /// One asked for (the bored knight, the menu).
    invited: bool,
    /// The invitation came from the user (tray / panel): ignore presence and sleep.
    forced: bool,
    hidden: bool,
}

impl Enemies {
    pub fn load(data_dir: &Path, vanilla: Option<&Skin>, scale: f32, seed: u32) -> Result<Enemies> {
        let meta = SpriteBank::load_metadata(data_dir)?;
        let set = EnemySet::load(&data_dir.join("../enemies.json"), &data_dir.join("animations"), &meta)?;
        let mut rng = Rng::new(seed);
        let sched = VisitScheduler::new(set.schedule, &mut rng);
        log::info!(
            "enemies: {} ({}), first in ~{:.0} min of presence",
            set.defs.len(),
            set.defs.iter().map(|d| d.display.as_str()).collect::<Vec<_>>().join(", "),
            sched.due_in() / 60.0
        );
        let mut e = Enemies {
            set,
            meta,
            frames: Vec::new(),
            origin_above_feet: Vec::new(),
            live: Vec::new(),
            sched,
            rng,
            next_id: 1,
            ppu: 64.0 * scale,
            capturable: false,
            pending: 0,
            pending_in: 0.0,
            invited: false,
            forced: false,
            hidden: false,
        };
        e.rescale(vanilla, scale)?;
        Ok(e)
    }

    /// Rasterize every enemy's clips at the pet's scale (vanilla art only).
    pub fn rescale(&mut self, vanilla: Option<&Skin>, scale: f32) -> Result<()> {
        self.ppu = 64.0 * scale;
        let Some(skin) = vanilla else {
            log::warn!("enemies: no vanilla skin export; enemies stay invisible");
            return Ok(());
        };
        let keys: Vec<Vec<SpriteKey>> = (0..self.set.defs.len()).map(|i| self.set.lib(i).sprite_keys()).collect();
        let sets: Vec<(&[SpriteKey], f32)> = keys.iter().map(|k| (k.as_slice(), self.ppu)).collect();
        self.frames = self.meta.rasterize_sets(skin, None, &sets)?;
        self.origin_above_feet = (0..self.set.defs.len())
            .map(|i| {
                let idle = self.set.lib(i).get(self.set.clips[i].idle);
                let f = idle.frames.iter().find_map(|f| f.sprite).and_then(|k| self.frames[i].get(&k));
                let below = f.map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
                self.set.defs[i].origin_above_feet.map_or(below, |u| u * self.ppu)
            })
            .collect();
        for l in &mut self.live {
            l.drawn = None;
        }
        Ok(())
    }

    /// Let one wander in as soon as there is a good spot (the bored knight, the menu).
    pub fn invite(&mut self) {
        self.invited = true;
    }

    /// Send one now (the tray / panel): it comes even while the user is away or the knight
    /// sleeps (it will wake him); only a fullscreen app or a visitor holds it back.
    pub fn send(&mut self) {
        self.invited = true;
        self.forced = true;
    }

    /// For the brain.
    pub fn foes(&self) -> Vec<Foe> {
        self.live.iter().filter_map(|l| l.enemy.foe(&self.set, self.ppu)).collect()
    }

    /// The knight's blow lands on enemy `id`. Returns where it died and how much Geo it
    /// drops when this blow killed it.
    pub fn strike(&mut self, id: u32, damage: u32, from: Vec2) -> Option<(Vec2, u32)> {
        let l = self.live.iter_mut().find(|l| l.enemy.id == id)?;
        if !l.enemy.strike(damage, from, &self.set, self.ppu) {
            return None;
        }
        let d = &self.set.defs[l.enemy.def];
        let amount = d.geo[0] + self.rng.next_u32() % (d.geo[1] - d.geo[0] + 1);
        log::info!("enemy: {} is beaten ({amount} geo)", d.display);
        Some((l.enemy.body(&self.set, self.ppu).center(), amount))
    }

    /// Body centre of enemy `id` (hit markers).
    pub fn centre(&self, id: u32) -> Option<Vec2> {
        self.live.iter().find(|l| l.enemy.id == id).map(|l| l.enemy.body(&self.set, self.ppu).center())
    }

    /// Hide everything (games on every monitor); the next tick shows it again.
    pub fn hide_all(&mut self) {
        if !self.hidden {
            self.hidden = true;
            for l in &self.live {
                l.win.set_visible(false);
            }
        }
    }

    pub fn raise_all(&self) {
        for l in &self.live {
            l.win.raise();
        }
    }

    /// `knight`: his body (None while dead / hidden). `visitor_present`: someone is
    /// visiting (no enemies then).
    pub fn tick(&mut self, dt: f32, w: &World, knight: Option<RectF>, visitor_present: bool) -> EnemyNews {
        let mut news = EnemyNews::default();
        if self.hidden {
            self.hidden = false;
            for l in &self.live {
                l.win.set_visible(true);
            }
        }
        // Games: nothing of ours on their monitors.
        let on_game = |p: Vec2| w.game_monitors.iter().any(|r| r.contains(p.x as i32, (p.y - 1.0) as i32));
        let before = self.live.len();
        self.live.retain(|l| !on_game(l.enemy.pos));
        if self.live.len() != before {
            log::info!("enemy: a game took its monitor; it is gone");
        }

        // 1. Arrivals: the schedule, invitations, the rest of a group. Never during a visit,
        // a fullscreen app, fast typing, the user away or the knight asleep / held.
        let alive = self.live.iter().filter(|l| l.enemy.alive()).count();
        let max = self.set.rules.max_alive;
        let base = VisitConditions {
            user_idle_secs: w.user_idle_secs,
            keys_per_sec: w.keys_per_sec,
            fullscreen: w.fullscreen,
            visitor_present,
            knight_busy: w.knight_busy,
        };
        let due = self.sched.tick(dt, &VisitConditions { visitor_present: visitor_present || alive >= max || self.pending > 0, ..base }, &mut self.rng);
        let free = self.sched.blocked(&base).is_none();
        let forced_ok = self.forced && !w.fullscreen && !visitor_present;
        if w.graph.is_some() && self.invited && (free || forced_ok) {
            // Invited (the bored knight, the menu): one more, even with a full house.
            self.invited = false;
            self.forced = false;
            if alive <= max {
                self.spawn(w);
            }
        } else if due {
            let g = self.set.rules.group;
            let n = g[0] + self.rng.next_u32() as usize % (g[1] - g[0] + 1);
            self.pending = n.min(max.saturating_sub(alive));
            self.pending_in = 0.0;
        }
        if self.pending > 0 {
            self.pending_in -= dt;
            if self.pending_in <= 0.0 && free && alive < max {
                self.pending -= 1;
                self.pending_in = self.set.rules.stagger;
                self.spawn(w);
            }
        }

        // 2. Enemies.
        for l in &mut self.live {
            let area = w.desk.monitor_at(l.enemy.pos.x as i32, (l.enemy.pos.y - 1.0) as i32).or(w.desk.primary()).map_or(w.terrain.bounds, |m| m.work.to_f());
            let env = EnemyEnv { terrain: w.terrain, knight, ppu: self.ppu, area };
            let t = l.enemy.tick(dt, &self.set, &env);
            news.hits += u32::from(t.hit_knight);
        }
        for l in self.live.iter().filter(|l| l.enemy.phase() == skong_core::enemy::EnemyPhase::Gone) {
            log::debug!("enemy: {} gone", self.set.defs[l.enemy.def].display);
        }
        self.live.retain(|l| l.enemy.phase() != skong_core::enemy::EnemyPhase::Gone);
        if let Err(e) = self.draw() {
            log::warn!("enemies: {e:#}");
        }
        news
    }

    /// Pick an enemy and where it shows up, and let it in.
    fn spawn(&mut self, w: &World) {
        let items: Vec<(usize, f32)> = self.set.defs.iter().enumerate().map(|(i, d)| (i, d.weight)).collect();
        let Some(def) = self.rng.pick_weighted(&items) else { return };
        let ppu = self.ppu;
        let d = &self.set.defs[def];
        // Settled ledges only (no windows being dragged, no scrolling content).
        let ok = |f: &Footing| (w.settled)(f);
        let footings: Vec<Footing> = match w.graph {
            Some(g) => g.surfaces().iter().map(|s| Footing { x0: s.x0, x1: s.x1, y: s.y, owner: s.owner }).filter(|f| ok(f)).collect(),
            None => w.terrain.ledges.iter().map(|l| Footing { x0: l.x0, x1: l.x1, y: l.y, owner: l.owner }).filter(|f| ok(f)).collect(),
        };
        let monitors: Vec<_> = w.desk.monitors.iter().filter(|m| !w.game_monitors.contains(&m.rect)).cloned().collect();
        let knight = w.knight;
        let body = |p: Vec2| RectF::from_feet(p, Vec2::new(d.size[0], d.size[1]) * ppu);
        let mut reachable = |p: Vec2| {
            let ground = skong_core::props::ledge_below(w.terrain, p).map_or(p, |(y, _)| Vec2::new(p.x, y));
            w.occupancy.is_free(body(ground), None) && w.graph.is_none_or(|g| g.plan(knight, NavGoal::Point(ground), TravelStyle::Hurry).is_some())
        };
        let Some(at) = plan_arrival(d, &footings, knight.pos, &monitors, ppu, &mut self.rng, &mut reachable) else {
            log::info!("enemy: no good spot for a {} right now", d.display);
            return;
        };
        let win = match SpriteWindow::new() {
            Ok(win) => win,
            Err(e) => {
                log::warn!("enemy window: {e:#}");
                return;
            }
        };
        win.set_click_through(true);
        win.set_capturable(self.capturable);
        let how = match at.entry {
            EntryKind::Edge { .. } => "comes in from the edge",
            EntryKind::Drop => "drops in",
        };
        log::info!("enemy: a {} {how} at ({:.0},{:.0})", d.display, at.at.x, at.at.y);
        let id = self.next_id;
        self.next_id += 1;
        let mut enemy = Enemy::new(id, def, &self.set, at, self.rng.next_u32(), ppu);
        let area = w.desk.monitor_at(at.at.x as i32, (at.at.y - 1.0) as i32).or(w.desk.primary()).map_or(w.terrain.bounds, |m| m.work.to_f());
        enemy.attach(&EnemyEnv { terrain: w.terrain, knight: None, ppu, area });
        self.live.push(LiveEnemy { enemy, win, drawn: None });
    }

    fn draw(&mut self) -> Result<()> {
        for l in &mut self.live {
            let def = l.enemy.def;
            let Some(key) = l.enemy.sprite(&self.set) else { continue };
            let Some(frame) = self.frames.get(def).and_then(|f| f.get(&key)) else { continue };
            let flip = l.enemy.facing_right;
            let origin = Vec2::new(l.enemy.pos.x, l.enemy.pos.y - self.origin_above_feet.get(def).copied().unwrap_or(0.0));
            let x = (origin.x - frame.anchor_x(flip)).round() as i32;
            let y = (origin.y - frame.anchor[1]).round() as i32;
            let alpha = (l.enemy.alpha.clamp(0.0, 1.0) * 255.0) as u8;
            let flash = (l.enemy.flash.clamp(0.0, 1.0) * 4.0).round() as u8;
            if l.drawn != Some((key, flip, alpha, flash)) {
                let mut img = std::borrow::Cow::Borrowed(&frame.image);
                if flash > 0 {
                    img = std::borrow::Cow::Owned(flashed(&img, flash as f32 / 4.0));
                }
                if alpha < 255 {
                    img = std::borrow::Cow::Owned(faded(&img, alpha));
                }
                l.win.present_flipped(&img, x, y, flip)?;
                l.drawn = Some((key, flip, alpha, flash));
            } else {
                l.win.move_to(x, y)?;
            }
        }
        Ok(())
    }
}

/// The game's hit flash: blend the sprite toward white by `amount` (0..1), keeping its
/// alpha (premultiplied BGRA).
pub fn flashed(img: &Image, amount: f32) -> Image {
    let k = (amount.clamp(0.0, 1.0) * 255.0) as u32;
    let px = img
        .px
        .iter()
        .map(|&p| {
            let a = p >> 24;
            let ch = |s: u32| {
                let c = (p >> s) & 0xff;
                ((c * (255 - k) + a * k + 127) / 255).min(a) << s
            };
            (a << 24) | ch(16) | ch(8) | ch(0)
        })
        .collect();
    Image { width: img.width, height: img.height, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_whitens_but_keeps_alpha() {
        let img = Image { width: 2, height: 1, px: vec![0x8010_2030, 0] };
        let f = flashed(&img, 1.0);
        assert_eq!(f.px, vec![0x8080_8080, 0], "fully white at the same (premultiplied) alpha; clear stays clear");
        assert_eq!(flashed(&img, 0.0).px, img.px);
    }
}
