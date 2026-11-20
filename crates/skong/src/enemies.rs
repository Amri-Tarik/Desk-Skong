//! App side of enemies: schedules their arrivals (rare, organic; or when the bored knight
//! asks), picks where they show up, renders each in its own sprite window (vanilla art,
//! a white flash when struck), applies the knight's blows, and reports bumps and kills.
//! Also runs gauntlets (waves of enemies and a reward; `skong_core::enemy::gauntlet`).
//!
//! The logic lives in `skong_core::enemy`; this module is rendering and plumbing.
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use skong_core::ai::Foe;
use skong_core::ai::rng::Rng;
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::canvas::Image;
use skong_core::desktop::WindowId;
use skong_core::enemy::{Enemy, EnemyEnv, EnemyPlacement, EnemySet, EntryKind, GauntletCue, GauntletDirector, GauntletEvent, GauntletWorld, placed_arrival, plan_arrival};
use skong_core::geom::{RectF, Vec2};
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::npc::{VisitConditions, VisitScheduler};
use skong_core::portal::Footing;
use skong_win::sprite_window::SpriteWindow;

use crate::bubble::faded;
use crate::npcs::World;

/// A picked spot: a feet point on a ledge of a window (`None`: the floor).
type Spot = (Vec2, Option<WindowId>);

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
    /// Gauntlet events (started, wave cleared, completed, aborted...; see
    /// [`GauntletEvent::name`]).
    pub gauntlet: Vec<GauntletEvent>,
    /// A completed gauntlet's reward: burst this much Geo by the knight.
    pub reward: Option<u32>,
    /// Gauntlet arrivals dropping in: a subtle dust portal at these feet points.
    pub puffs: Vec<Vec2>,
}

/// The knight, as the gauntlet cares.
#[derive(Clone, Copy, Debug, Default)]
pub struct KnightState {
    /// Fighting (enemies, a spar, the cursor) or in a lesson.
    pub fighting: bool,
    pub down: bool,
    /// How hurt he is (0..1).
    pub hurt: f32,
    /// He backed off from a fight (knocked about) and leaves enemies alone for now.
    pub backed_off: bool,
    /// His Shade waits to be reclaimed (no gauntlet starts meanwhile).
    pub shade: bool,
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
    /// Gauntlets (waves of enemies, a reward).
    gauntlet: Option<GauntletDirector>,
    /// Enemies beaten since the pet last looked: (id, definition name).
    pub killed: Vec<(u32, String)>,
    /// Test panel: one of this kind asked for, maybe at a picked spot.
    wanted: Option<(usize, Option<Spot>)>,
    /// What came of the test panel's requests (ok, line).
    pub notes: Vec<(bool, String)>,
}

impl Enemies {
    /// Every kind of enemy that may wander in (definition names).
    pub fn kinds(&self) -> Vec<String> {
        self.set.defs.iter().map(|d| d.name.clone()).collect()
    }

    pub fn load(data_dir: &Path, vanilla: Option<&Skin>, scale: f32, seed: u32) -> Result<Enemies> {
        let meta = SpriteBank::load_metadata(data_dir)?;
        let set = EnemySet::load(&data_dir.join("../enemies.json"), &data_dir.join("animations"), &meta)?;
        let mut rng = Rng::new(seed);
        let sched = VisitScheduler::new(set.schedule, &mut rng);
        let gauntlet = match GauntletDirector::new(set.gauntlet.clone(), &set) {
            Ok(g) => Some(g),
            Err(e) => {
                log::warn!("gauntlets disabled: {e:#}");
                None
            }
        };
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
            gauntlet,
            killed: Vec::new(),
            wanted: None,
            notes: Vec::new(),
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

    /// Start a gauntlet (the tray / panel): at the next good moment (no other fight, no
    /// visitor, no fullscreen app, nobody typing hard).
    pub fn start_gauntlet(&mut self) -> bool {
        match self.gauntlet.as_mut() {
            Some(g) => {
                g.request();
                true
            }
            None => false,
        }
    }

    /// The gauntlet under way, for the brain.
    pub fn gauntlet_cue(&self) -> Option<GauntletCue> {
        self.gauntlet.as_ref().and_then(|g| g.cue())
    }

    /// A gauntlet is under way (no visitors meanwhile).
    pub fn gauntlet_active(&self) -> bool {
        self.gauntlet.as_ref().is_some_and(|g| g.active())
    }

    /// A gauntlet is under way or waiting to start.
    pub fn gauntlet_busy(&self) -> bool {
        self.gauntlet.as_ref().is_some_and(|g| g.active() || g.pending().is_some())
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
        self.killed.push((id, d.name.clone()));
        if let Some(g) = self.gauntlet.as_mut() {
            g.on_kill(id, &mut self.rng);
        }
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
    /// visiting (no enemies then). `ks`: what the gauntlet needs to know about him.
    pub fn tick(&mut self, dt: f32, w: &World, knight: Option<RectF>, visitor_present: bool, ks: KnightState) -> EnemyNews {
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

        // 0. A gauntlet (under way or waiting to start) holds back every other arrival.
        let gauntlet = self.tick_gauntlet(dt, w, visitor_present, ks, &mut news);
        self.admit_wanted(w, visitor_present, gauntlet);
        if gauntlet && !self.forced {
            // The bored knight's wish for an enemy is more than granted.
            self.invited = false;
        }

        // 1. Arrivals: the schedule, invitations, the rest of a group. Never during a visit,
        // a fullscreen app, fast typing, the user away or the knight asleep / held.
        let alive = self.live.iter().filter(|l| l.enemy.alive()).count();
        let max = self.set.rules.max_alive;
        let base = VisitConditions {
            user_idle_secs: w.user_idle_secs,
            keys_per_sec: w.keys_per_sec,
            fullscreen: w.fullscreen,
            visitor_present: visitor_present || gauntlet,
            knight_busy: w.knight_busy,
        };
        let due = self.sched.tick(dt, &VisitConditions { visitor_present: base.visitor_present || alive >= max || self.pending > 0, ..base }, &mut self.rng);
        let free = self.sched.blocked(&base).is_none();
        let forced_ok = self.forced && !w.fullscreen && !visitor_present && !gauntlet;
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

    /// The gauntlet's tick: start / waves / reward / abort. Returns whether one is under way
    /// or waiting to start.
    fn tick_gauntlet(&mut self, dt: f32, w: &World, visitor_present: bool, ks: KnightState, news: &mut EnemyNews) -> bool {
        let Some(mut g) = self.gauntlet.take() else { return false };
        let alive: Vec<u32> = self.live.iter().filter(|l| l.enemy.alive() && !l.enemy.dismissed()).map(|l| l.enemy.id).collect();
        let gw = GauntletWorld {
            keys_per_sec: w.keys_per_sec,
            user_idle_secs: w.user_idle_secs,
            away_secs: self.set.schedule.away_secs,
            fullscreen: w.fullscreen,
            visitor_present,
            others_alive: alive.iter().filter(|&&id| !g.is_member(id)).count(),
            knight_busy: w.knight_busy,
            knight_fighting: ks.fighting,
            knight_down: ks.down,
            knight_hurt: ks.backed_off || ks.hurt > g.cfg.hurt,
            shade: ks.shade,
            knight_at: w.knight.pos,
            ready: w.graph.is_some(),
        };
        let step = g.tick(dt, &gw, &|id| alive.contains(&id), &mut self.rng);
        for l in self.live.iter_mut().filter(|l| step.dismiss.contains(&l.enemy.id)) {
            l.enemy.dismiss();
        }
        if let Some(def) = step.spawn {
            let placed = self.spawn_def(w, def);
            if let Some((id, at, dropped)) = placed {
                if let Some(l) = self.live.iter_mut().find(|l| l.enemy.id == id) {
                    l.enemy.hold(g.hold_secs());
                }
                if dropped {
                    news.puffs.push(at);
                }
            }
            g.spawned(placed.map(|p| p.0));
        }
        news.reward = step.reward;
        news.gauntlet.extend(step.events);
        let busy = g.active() || g.pending().is_some();
        self.gauntlet = Some(g);
        busy
    }

    /// Pick an enemy and where it shows up, and let it in.
    fn spawn(&mut self, w: &World) {
        let items: Vec<(usize, f32)> = self.set.defs.iter().enumerate().map(|(i, d)| (i, d.weight)).collect();
        if let Some(def) = self.rng.pick_weighted(&items) {
            self.spawn_def(w, def);
        }
    }

    /// Let an enemy of `def` in at a good spot. Returns its id, where it lands and whether it
    /// drops in from above (`None`: no good spot right now).
    fn spawn_def(&mut self, w: &World, def: usize) -> Option<(u32, Vec2, bool)> {
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
            return None;
        };
        self.admit(w, def, at)
    }

    /// Test panel: an enemy of kind `def` now (`at`: dropping in right there, a spot the
    /// user picked; else wherever arrivals go). Only a fullscreen app, a visitor or the
    /// troupe, or a gauntlet holds it back (the panel hears why).
    pub fn send_kind(&mut self, def: usize, at: Option<Spot>) -> Result<(), String> {
        let d = self.set.defs.get(def).ok_or_else(|| format!("no enemy kind #{def}"))?;
        log::info!("enemies: a {} asked for (test panel)", d.display);
        self.wanted = Some((def, at));
        Ok(())
    }

    /// Display names, indexed like [`Self::send_kind`].
    pub fn displays(&self) -> Vec<String> {
        self.set.defs.iter().map(|d| d.display.clone()).collect()
    }

    /// The test panel's enemy, at the first moment the nav graph is there.
    fn admit_wanted(&mut self, w: &World, visitor_present: bool, gauntlet: bool) {
        let Some((def, at)) = self.wanted.take() else { return };
        let name = self.set.defs[def].display.clone();
        let why = if w.fullscreen {
            Some("a fullscreen app is up")
        } else if visitor_present {
            Some("a visitor or the troupe is here")
        } else if gauntlet {
            Some("a gauntlet is under way")
        } else if w.graph.is_none() {
            self.wanted = Some((def, at)); // wait for the nav graph
            return;
        } else {
            None
        };
        if let Some(why) = why {
            self.notes.push((false, format!("no {name} now: {why}")));
            return;
        }
        let placed = match at {
            Some((p, owner)) => {
                let a = placed_arrival(&self.set.defs[def], p, owner, self.ppu);
                self.admit(w, def, a)
            }
            None => self.spawn_def(w, def),
        };
        match placed {
            Some((_, p, _)) => self.notes.push((true, format!("a {name} comes in at ({:.0},{:.0})", p.x, p.y))),
            None => self.notes.push((false, format!("no good spot for a {name} right now (settled, reachable, on his monitor)"))),
        }
    }

    /// Let an enemy of `def` in at `at`.
    fn admit(&mut self, w: &World, def: usize, at: EnemyPlacement) -> Option<(u32, Vec2, bool)> {
        let ppu = self.ppu;
        let d = &self.set.defs[def];
        let win = match SpriteWindow::new() {
            Ok(win) => win,
            Err(e) => {
                log::warn!("enemy window: {e:#}");
                return None;
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
        Some((id, at.at, at.entry == EntryKind::Drop))
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
