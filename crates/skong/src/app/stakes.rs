//! Battle stakes glue for [`Pet`] (death, the Shade, respawn, Geo, spells): kept out of
//! `app.rs`, which only calls [`Pet::stakes_sense`] / [`Pet::stakes_events`] around the
//! brain tick, [`Pet::stakes_tick`] before drawing, and [`Pet::apply_stakes`] for commands.
//!
//! * The brain decides (dies, asks for a respawn, hunts the Shade, casts); this side
//!   simulates and renders the Shade and the Geo, keeps the wallet, spawns spell effects
//!   on the act steps that release them, and persists everything in the settings file.
use std::path::Path;

use anyhow::Result;
use skong_core::acts::ActId;
use skong_core::ai::{BehaviourKind, ShadeSense, StakeEvent, StakesSense};
use skong_core::controller::{Character, Side};
use skong_core::geom::Vec2;
use std::collections::HashMap;

use skong_core::desktop::WindowId;
use skong_core::geom::{RectF, RectI};
use skong_core::stakes::{DeathKind, GeoDrop, GeoField, GeoSources, Shade, StakesSave};
use skong_ipc::{SPELLS, StakesCommand, StakesInfo};
use skong_win::monitor;

use super::{Pet, seed};

mod view;
use view::{Lib, View};

/// Death acts: the knight stays visible while one plays, then is hidden until the respawn.
const DEATH_ACTS: [&str; 3] = ["death", "spike_death", "acid_death"];
/// Looping effects tied to one act step.
const LOOPING_FX: [&str; 2] = ["cyclone", "na_charge"];
/// How long the mask piece lies on the ground (s).
const HEAD_SECS: f32 = 6.0;

pub struct Stakes {
    pub save: StakesSave,
    shade: Option<Shade>,
    geo: GeoField,
    /// Shattered windows and Geo showers.
    sources: GeoSources,
    /// Windows of the last desktop snapshot (to tell which ones closed).
    windows: HashMap<WindowId, (RectI, bool)>,
    /// Monitors with a fullscreen game (nothing of ours goes there).
    games: Vec<RectI>,
    desk_generation: u64,
    /// Terrain generation the Shade was last settled for (0 = settle now).
    settled_for: u64,
    /// Monitor work areas: the Shade and the coins stay inside them.
    areas: Vec<RectF>,
    /// `None` when the Shade / Geo art metadata is missing (stakes still work, unseen).
    view: Option<View>,
    /// Died and not yet respawned.
    dead: bool,
    /// The knight window is hidden (dead, death act over).
    hidden: bool,
    /// Where the mask piece drops once the death act is over (and whether it is cracked).
    head: Option<(Vec2, bool)>,
    /// Shade slashes that connected since the brain last looked.
    shade_hits: u32,
    /// Running act + step last seen (spell effects fire on step changes).
    last_step: Option<(ActId, usize)>,
    dirty: bool,
    since_save: f32,
    since_raise: f32,
}

impl Stakes {
    pub fn new(data: &Path, save: StakesSave, capturable: bool, cfg: &skong_core::ai::StakesConfig) -> Self {
        let view = View::load(data, capturable).map_err(|e| log::warn!("stakes art unavailable: {e:#}")).ok();
        let shade = save.shade.map(|s| Shade::new(s.pos(), s.geo, false, seed()));
        if let Some(s) = &save.shade {
            log::info!("stakes: {} geo in the wallet; a shade waits at ({:.0},{:.0}) with {} geo", save.geo, s.x, s.y, s.geo);
        }
        Self {
            save,
            shade,
            geo: GeoField::new(seed()),
            sources: GeoSources::new(cfg.sources, seed()),
            windows: HashMap::new(),
            games: Vec::new(),
            desk_generation: 0,
            settled_for: 0,
            areas: Vec::new(),
            view,
            dead: false,
            hidden: false,
            head: None,
            shade_hits: 0,
            last_step: None,
            dirty: false,
            since_save: 0.0,
            since_raise: 0.0,
        }
    }

    /// Geo coins bursting out at `at` (e.g. a broken geo deposit).
    pub fn burst(&mut self, at: Vec2, amount: u32, scale: f32) {
        self.geo.scale = scale;
        self.geo.burst(at, amount);
    }

    /// Fullscreen games on these monitors: hide our sprites there, keep Geo off them.
    pub fn hide_on(&mut self, monitors: &[RectI]) {
        self.games = monitors.to_vec();
        if let Some(v) = self.view.as_mut() {
            v.hide_on(&monitors.iter().map(|m| m.to_f()).collect::<Vec<_>>());
        }
    }

    pub fn info(&self, soul: f32) -> StakesInfo {
        StakesInfo {
            geo: self.save.geo,
            loose_geo: self.geo.total(),
            shade: self.shade.as_ref().filter(|s| s.alive()).map(|s| (s.home.x, s.home.y, s.geo)),
            soul,
            deaths: self.save.deaths,
            shades_defeated: self.save.shades_defeated,
        }
    }
}

impl Pet {
    /// Before the brain thinks: the Shade, resting coins, and the Shade's hits.
    pub(super) fn stakes_sense(&mut self) {
        let st = &mut self.stakes;
        let shade = st.shade.as_ref().filter(|s| s.alive()).map(|s| ShadeSense { pos: s.pos, vulnerable: s.vulnerable(), fighting: s.fighting(), geo: s.geo });
        let coins = st.geo.resting().map(|c| c.pos).collect();
        let shade_hits = std::mem::take(&mut st.shade_hits);
        self.brain.sense_stakes(StakesSense { shade, coins, shade_hits, scale: self.settings.scale });
    }

    /// React to the brain's stakes events (after its acts / attacks were applied).
    pub(super) fn stakes_events(&mut self, events: &[StakeEvent]) {
        for e in events {
            match *e {
                StakeEvent::Died { at, kind, cracked } => self.die(at, kind, cracked),
                StakeEvent::Respawn { at } => {
                    self.ch = Character::new(self.ch.params, self.ch.abilities, at - Vec2::new(0.0, 1.0));
                    self.act = None;
                    self.seat_y = None;
                    self.stakes.dead = false;
                    log::info!("the knight respawns at ({:.0},{:.0})", at.x, at.y);
                }
                StakeEvent::Cast { spell, side } => {
                    if self.ch.is_grounded() {
                        self.ch.facing = side;
                    }
                    log::info!("spell: {}", spell.label());
                }
                StakeEvent::StrikeShade { damage, from } => {
                    if let Some(sh) = self.stakes.shade.as_mut()
                        && sh.strike(damage, from)
                    {
                        log::info!("the shade is beaten");
                    }
                }
                StakeEvent::AbandonCoin { at } => self.stakes.geo.abandon(at),
                StakeEvent::Victory { at } => {
                    let [lo, hi] = self.brain.config().stakes.victory_geo;
                    let amount = lo + seed() % (hi.saturating_sub(lo) + 1);
                    self.stakes.geo.scale = self.settings.scale;
                    self.stakes.geo.burst(at, amount);
                    log::info!("the cursor drops {amount} geo");
                }
            }
        }
    }

    fn die(&mut self, at: Vec2, kind: DeathKind, cracked: bool) {
        let st = &mut self.stakes;
        let carried = st.save.geo;
        let shade_at = Vec2::new(at.x, at.y - self.ch.size.y * 0.6);
        let lost = st.save.die(shade_at, kind.leaves_shade());
        if kind.leaves_shade() {
            let mut shade = Shade::new(shade_at, carried, true, seed());
            shade.scale = self.settings.scale;
            st.shade = Some(shade);
            st.settled_for = 0;
        }
        st.dead = true;
        st.head = Some((at, cracked));
        st.dirty = true;
        log::info!("the knight died ({kind:?}); his shade carries {carried} geo{}", if lost > 0 { format!(", an older shade's {lost} geo is lost") } else { String::new() });
    }

    /// Simulate and draw the Shade, coins, fireballs; hide the knight while dead; save.
    pub(super) fn stakes_tick(&mut self, dt: f32) -> Result<()> {
        let scale = self.settings.scale;
        self.stakes_act_fx();
        if let Some(v) = self.stakes.view.as_mut() {
            v.ensure_look(self.skin, &self.data.skins[self.skin], super::vanilla_of(&self.data.skins), scale)?;
        }

        // The Shade.
        let hunted = self.brain.behaviour() == BehaviourKind::ShadeHunt;
        let knight = (!self.stakes.dead).then(|| self.ch.body().center());
        let origin = self.knight_origin();
        let st = &mut self.stakes;
        st.areas.clear();
        let games = &st.games;
        st.areas.extend(self.watcher.snapshot().monitors.iter().filter(|m| !games.contains(&m.rect)).map(|m| m.work.to_f()));
        let mut drawn = None;
        if let Some(sh) = st.shade.as_mut() {
            sh.scale = scale;
            if st.settled_for != self.terrain.generation {
                st.settled_for = self.terrain.generation;
                if sh.settle(&self.terrain, &st.areas) {
                    log::info!("the shade moves somewhere reachable: ({:.0},{:.0})", sh.home.x, sh.home.y);
                    st.save.shade_moved(sh.home);
                    st.dirty = true;
                }
            }
            let tick = sh.step(dt, knight, hunted, &st.areas);
            st.shade_hits += u32::from(tick.hit_knight);
            if tick.burst {
                let got = st.save.reclaim();
                log::info!("shade burst: {got} geo reclaimed ({} in the wallet)", st.save.geo);
                if let Some(v) = st.view.as_mut() {
                    v.flash(Lib::Shade, "Death Glow", sh.pos, false);
                }
                st.shade = None;
                st.dirty = true;
            } else {
                drawn = Some(tick);
            }
        }
        if let Some(v) = st.view.as_mut() {
            v.shade(st.shade.as_ref().zip(drawn), dt)?;
        }

        // Geo: new drops (shattered windows, showers), physics, pickups.
        let st = &mut self.stakes;
        st.geo.scale = scale;
        st.geo.lifetime = self.brain.config().stakes.coin_lifetime;
        let desk = self.watcher.snapshot();
        let loose = st.geo.total();
        if desk.generation != st.desk_generation {
            let first = st.desk_generation == 0;
            st.desk_generation = desk.generation;
            let now: HashMap<WindowId, (RectI, bool)> = desk.windows.iter().map(|w| (w.id, (w.rect, w.fullscreen))).collect();
            for (id, &(rect, fullscreen)) in &st.windows {
                // Games (fullscreen) closing don't shatter, nor anything on a game monitor.
                let on_game = st.games.iter().any(|g| !g.intersect(&rect).is_empty());
                if first || fullscreen || on_game || now.contains_key(id) || !skong_win::desktop::window_closed(*id) {
                    continue;
                }
                if let Some(d) = st.sources.on_closed(rect, loose) {
                    log::info!("a closed window shatters into {} geo", d.amount);
                    st.geo.spill(&d);
                }
            }
            st.windows = now;
        }
        let s = self.brain.senses();
        // Showers fall on one (game-free) monitor.
        let sky = if st.areas.is_empty() { self.terrain.bounds } else { st.areas[seed() as usize % st.areas.len()] };
        if let Some(d) = st.sources.tick(dt, !s.user_away, s.fullscreen || s.user_busy, loose, sky) {
            log::info!("a geo shower: {} geo", d.amount);
            st.geo.spill(&d);
        }
        st.geo.step(dt, &self.terrain, &st.areas);
        st.geo.evict_from(&self.occupancy);
        if !st.dead {
            let got: u32 = st.geo.collect(self.ch.body()).iter().map(|p| p.value).sum();
            if got > 0 {
                st.save.earn(got);
                st.dirty = true;
                self.fx.spawn("geo_get", origin, self.ch.facing == Side::Right);
                log::debug!("picked up {got} geo ({} total)", st.save.geo);
            }
        }
        if let Some(v) = st.view.as_mut() {
            v.coins(&st.geo, dt)?;
            v.update(dt, &self.terrain, scale, &self.look.bank)?;
        }

        // Dead: hidden once the death act is over, with the mask piece left behind.
        let dying = self.act.as_ref().is_some_and(|p| DEATH_ACTS.contains(&self.data.acts.get(p.act()).name.as_str()));
        let hide = st.dead && !dying;
        if hide && let Some((at, cracked)) = st.head.take() {
            let clip = if cracked { "Death Head Cracked" } else { "Death Head Normal" };
            let key = self.data.lib.id(clip).and_then(|id| self.data.lib.get(id).frames.first().and_then(|f| f.sprite));
            if let (Some(v), Some(key)) = (st.view.as_mut(), key) {
                v.drop_head(key, at, HEAD_SECS);
            }
        }
        if hide != st.hidden {
            st.hidden = hide;
            self.win.set_visible(!hide && !self.paused);
        }

        // Persist and show the wallet.
        st.since_save += dt;
        if st.dirty && st.since_save > 1.0 {
            st.dirty = false;
            st.since_save = 0.0;
            self.settings.stakes = st.save.clone();
            self.settings.save();
        }
        st.since_raise += dt;
        if st.since_raise > 2.0 {
            st.since_raise = 0.0;
            if let Some(v) = &st.view {
                v.raise_all();
            }
        }
        let tip = match &st.save.shade {
            Some(s) => format!("Desk-Skong — {} geo (shade holds {})", st.save.geo, s.geo),
            None => format!("Desk-Skong — {} geo", st.save.geo),
        };
        self.tray.set_tip(&tip);
        Ok(())
    }

    /// Spell / nail-art effects on the act steps that release them.
    fn stakes_act_fx(&mut self) {
        let now = self.act.as_ref().map(|p| (p.act(), p.step_index()));
        if now == self.stakes.last_step {
            return;
        }
        self.stakes.last_step = now;
        for fx in LOOPING_FX {
            self.fx.stop(fx);
        }
        let Some((id, step)) = now else { return };
        let origin = self.knight_origin();
        let right = self.ch.facing == Side::Right;
        let ppu = self.ch.params.px_per_unit;
        let spawn: &[&str] = match (self.data.acts.get(id).name.as_str(), step) {
            ("fireball", 1) => {
                let at = origin + Vec2::new(self.ch.facing.sign() * 0.6 * ppu, 0.25 * ppu);
                if let Some(v) = self.stakes.view.as_mut() {
                    v.fireball(at, right, self.settings.scale);
                }
                &[]
            }
            ("scream", 1) => &["scream_base", "scream_heads"],
            ("great_slash", 0) => &["na_charge"],
            ("great_slash", 1) => &["great_slash"],
            ("cyclone_slash", 1) => &["cyclone"],
            ("dash_slash", 0) => &["dash_slash"],
            ("thorns", 0) => &["thorns"],
            ("reclaim_shade", 0) => &["shadow_recharge"],
            _ => &[],
        };
        for fx in spawn {
            self.fx.spawn(fx, origin, right);
        }
    }

    /// Panel / debug triggers.
    pub(super) fn apply_stakes(&mut self, c: StakesCommand) {
        match c {
            StakesCommand::Die => self.brain.kill(DeathKind::Fight),
            StakesCommand::HuntShade => self.brain.hunt_shade(),
            StakesCommand::SpillGeo => {
                let st = &mut self.stakes;
                st.geo.scale = self.settings.scale;
                let sky = if st.areas.is_empty() { self.terrain.bounds } else { st.areas[seed() as usize % st.areas.len()] };
                let d: GeoDrop = st.sources.shower(sky);
                st.geo.spill(&d);
            }
            StakesCommand::DropGeo => {
                let at = monitor::cursor_pos().map_or(self.ch.pos - Vec2::new(0.0, 200.0), |(x, y)| Vec2::new(x as f32, y as f32));
                self.stakes.geo.scale = self.settings.scale;
                self.stakes.geo.burst(at, 30);
            }
            StakesCommand::Cast(i) => {
                let id = SPELLS.get(i as usize).and_then(|(name, _)| self.data.acts.id(name));
                if let Some(id) = id.filter(|_| self.ch.is_grounded()) {
                    self.seat_y = None;
                    self.start_act(id);
                }
            }
        }
    }
}
