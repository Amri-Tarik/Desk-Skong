//! Pet glue for enemies and sparring (see `crate::enemies`, `skong_core::enemy` and the
//! brain's `Fight` behaviour): foes and bumps go to the brain before it thinks, its blows
//! come back as strikes (a hit marker on the foe; a kill bursts Geo).
use skong_core::ai::{BehaviourKind, FoeKind, FoeStrike};
use skong_win::{input, monitor};

use super::{Pet, vanilla_of};
use crate::enemies::{Enemies, KnightState};
use crate::npcs::World;

impl Pet {
    pub(super) fn load_enemies(&self, capturable: bool) -> Option<Enemies> {
        match Enemies::load(&self.data.args_data, vanilla_of(&self.data.skins), self.settings.scale, super::seed()) {
            Ok(mut e) => {
                e.capturable = capturable;
                Some(e)
            }
            Err(e) => {
                log::warn!("enemies disabled: {e:#}");
                None
            }
        }
    }

    /// Before the brain thinks: enemies and a sparring visitor, and the bumps they gave.
    pub(super) fn see_foes(&mut self) {
        let mut foes = self.enemies.as_ref().map(|e| e.foes()).unwrap_or_default();
        let mut hits = std::mem::take(&mut self.enemy_hits);
        if let Some(v) = self.visitors.as_mut() {
            foes.extend(v.foes());
            hits += v.take_knight_hits();
        }
        self.brain.see_foes(foes, hits);
        let cue = self.enemies.as_ref().and_then(|e| e.gauntlet_cue());
        self.brain.see_gauntlet(cue);
    }

    /// The brain's blows that landed: hit markers, knockback, kills (Geo bursts).
    pub(super) fn apply_strikes(&mut self, strikes: &[FoeStrike]) {
        let scale = self.settings.scale;
        for s in strikes {
            let right = s.at.x >= s.from.x;
            match s.kind {
                FoeKind::Enemy => {
                    let Some(e) = self.enemies.as_mut() else { continue };
                    let at = e.centre(s.id).unwrap_or(s.at);
                    self.fx.spawn("cursor_hit", at, right);
                    if let Some((at, geo)) = e.strike(s.id, s.damage, s.from) {
                        self.fx.spawn("cursor_defeat", at, right);
                        self.stakes.burst(at, geo, scale);
                    }
                }
                FoeKind::Sparring => {
                    self.fx.spawn("cursor_hit", s.at, right);
                    if let Some(mut v) = self.visitors.take() {
                        let keys = self.user_input.keys_per_sec();
                        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
                        let settled = |f: &skong_core::portal::Footing| discoveries.settled(terrain, f.owner, f.y, f.x0, f.x1);
                        v.strike_spar(s.id, s.damage, s.from, &self.world(&settled, keys));
                        self.visitors = Some(v);
                    }
                }
            }
        }
    }

    /// The world as enemies (and visitors) see it.
    fn world<'a>(&'a self, settled: &'a dyn Fn(&skong_core::portal::Footing) -> bool, keys_per_sec: f32) -> World<'a> {
        let busy = self.grab.held_pos().is_some()
            || matches!(self.brain.behaviour(), BehaviourKind::Nap | BehaviourKind::Hide | BehaviourKind::KnockedOut | BehaviourKind::Flee | BehaviourKind::Dead);
        World {
            terrain: &self.terrain,
            desk: self.watcher.snapshot(),
            knight: &self.ch,
            graph: self.nav.graph().map(|g| g.as_ref()),
            user_idle_secs: input::idle_secs(),
            keys_per_sec,
            fullscreen: self.brain.senses().fullscreen || self.watcher.snapshot().windows.iter().any(|w| w.fullscreen && w.foreground) || !self.games.game_monitors.is_empty(),
            knight_busy: busy,
            hour: monitor::local_hour(),
            game_monitors: &self.games.game_monitors,
            occupancy: &self.occupancy,
            since_fight: self.brain.since_fight(),
            settled,
        }
    }

    /// Per tick, after the knight moved.
    pub(super) fn tick_enemies(&mut self, dt: f32) {
        let Some(mut e) = self.enemies.take() else { return };
        let keys = self.user_input.keys_per_sec();
        let visiting = self.visitors.as_ref().is_some_and(|v| v.present());
        let knight = (self.brain.behaviour() != BehaviourKind::Dead).then(|| self.ch.body());
        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
        let settled = |f: &skong_core::portal::Footing| discoveries.settled(terrain, f.owner, f.y, f.x0, f.x1);
        let behaviour = self.brain.behaviour();
        let ks = KnightState {
            fighting: matches!(behaviour, BehaviourKind::Fight | BehaviourKind::Train),
            down: behaviour == BehaviourKind::Dead,
            hurt: self.brain.mind().emotions.hurt,
            backed_off: self.brain.fight_backed_off(),
        };
        let news = e.tick(dt, &self.world(&settled, keys), knight, visiting, ks);
        self.enemy_hits += news.hits;
        self.enemies = Some(e);
        // Gauntlet: arrivals dropping in through a subtle dust portal, the reward burst.
        for at in &news.puffs {
            self.fx.spawn("arrive_puff", *at, false);
            self.fx.spawn("arrive_puff_side", *at, false);
            self.fx.spawn("arrive_puff_side", *at, true);
        }
        if let Some(geo) = news.reward {
            let at = self.ch.body().center();
            self.stakes.burst(at, geo, self.settings.scale);
        }
        for ev in &news.gauntlet {
            // Stable names (`gauntlet_started`, `gauntlet_wave_cleared`, `gauntlet_completed`,
            // `gauntlet_aborted`, ...): achievements and the logs key on them.
            log::info!("event: {} {ev:?}", ev.name());
            if matches!(ev, skong_core::enemy::GauntletEvent::Completed { .. }) {
                self.achievements.gauntlets_cleared += 1;
            }
        }
    }

    /// The tray / panel: start a gauntlet.
    pub(super) fn start_gauntlet(&mut self) {
        match self.enemies.as_mut().map(|e| e.start_gauntlet()) {
            Some(true) => log::info!("gauntlet: requested (starts at the next good moment)"),
            Some(false) => log::warn!("gauntlet: unavailable (see the enemies.json `gauntlet` section)"),
            None => log::warn!("enemies: not loaded"),
        }
    }

    /// The bored knight (or the menu) asks for an enemy.
    pub(super) fn invite_enemy(&mut self) {
        match self.enemies.as_mut() {
            Some(e) => e.invite(),
            None => log::debug!("an enemy was wanted, but enemies are unavailable"),
        }
    }

    pub(super) fn hide_enemies(&mut self) {
        if let Some(e) = self.enemies.as_mut() {
            e.hide_all();
        }
    }

    pub(super) fn raise_enemies(&self) {
        if let Some(e) = &self.enemies {
            e.raise_all();
        }
    }

    pub(super) fn rescale_enemies(&mut self) {
        let (skins, scale) = (&self.data.skins, self.settings.scale);
        if let Some(e) = self.enemies.as_mut()
            && let Err(err) = e.rescale(vanilla_of(skins), scale)
        {
            log::warn!("enemies: {err:#}");
        }
    }
}
