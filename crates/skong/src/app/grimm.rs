//! Pet glue for the secret boss (see `crate::grimm` and `skong_core::enemy::grimm`): what
//! the troupe needs to know about the world each tick, the lantern's placement (settled,
//! reachable ledges near the knight, never on a game's monitor), his blows on Grimm and the
//! lantern, Grimm's hits on him (they go to the brain with the enemies' bumps), the reward
//! burst, the "!" when the show begins, and the events achievements care about.
use skong_core::ai::BehaviourKind;
use skong_core::enemy::grimm::{GrimmEnv, GrimmEvent, GrimmWorld, plan_lantern};
use skong_core::geom::{RectF, RectI, Vec2};
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::npc::TalkContext;
use skong_core::portal::Footing;
use skong_win::{input, monitor};

use super::{Pet, vanilla_of};
use crate::emotes::Emote;
use crate::grimm::Troupe;

impl Pet {
    pub(super) fn load_troupe(&self, capturable: bool) -> Option<Troupe> {
        match Troupe::load(&self.data.args_data, vanilla_of(&self.data.skins), self.settings.scale, super::seed()) {
            Ok(mut t) => {
                t.capturable = capturable;
                Some(t)
            }
            Err(e) => {
                log::warn!("the troupe is not coming: {e:#}");
                None
            }
        }
    }

    /// The panel's hidden Debug section: Grimm (or his lantern) at the next good moment.
    pub(super) fn summon_grimm(&mut self, lantern: bool) {
        match self.troupe.as_mut() {
            Some(t) => {
                t.request(lantern);
                log::info!("grimm: {} requested (at the next good moment)", if lantern { "the lantern" } else { "the troupe master" });
            }
            None => log::warn!("grimm: not loaded (see data/grimm.json)"),
        }
    }

    /// The flame or the show is on (no visitors, no other enemies meanwhile).
    pub(super) fn troupe_active(&self) -> bool {
        self.troupe.as_ref().is_some_and(|t| t.active())
    }

    /// The knight's monitor work area: the stage.
    fn stage(&self) -> RectI {
        let d = self.watcher.snapshot();
        d.monitor_at(self.ch.pos.x as i32, (self.ch.pos.y - 1.0) as i32).or(d.primary()).map_or(RectI::new(0, 0, 1920, 1040), |m| m.work)
    }

    /// Per tick, after the enemies.
    pub(super) fn tick_troupe(&mut self, dt: f32) {
        let Some(mut t) = self.troupe.take() else { return };
        let games = self.games.game_monitors.clone();
        let on_game = |p: Vec2| games.iter().any(|r| r.contains(p.x as i32, (p.y - 1.0) as i32));
        // A game took the stage: the troupe vanishes at once (no line, no penalty).
        if t.active() && (t.spot().is_some_and(on_game) || on_game(self.ch.pos)) {
            let bubbles = self.visitors.as_mut().map(|v| &mut v.bubbles);
            for e in t.vanish(bubbles) {
                log::info!("event: {} {e:?}", e.name());
            }
        }
        let behaviour = self.brain.behaviour();
        let down = behaviour == BehaviourKind::Dead;
        let busy = self.grab.held_pos().is_some() || matches!(behaviour, BehaviourKind::Nap | BehaviourKind::Hide | BehaviourKind::KnockedOut | BehaviourKind::Flee);
        let desk = self.watcher.snapshot();
        let fullscreen = self.brain.senses().fullscreen || desk.windows.iter().any(|w| w.fullscreen && w.foreground) || !games.is_empty();
        let (gauntlet, others) = self.enemies.as_ref().map_or((false, 0), |e| (e.gauntlet_busy(), e.foes().len()));
        let w = GrimmWorld {
            achievements: self.achievements.unlocked(),
            keys_per_sec: self.user_input.keys_per_sec(),
            user_idle_secs: input::idle_secs(),
            fullscreen,
            visitor_present: self.visitors.as_ref().is_some_and(|v| v.present()),
            gauntlet,
            others_alive: others,
            knight_busy: busy,
            knight_down: down,
            shade: self.stakes.has_shade(),
            knight_at: self.ch.pos,
            ready: self.nav.graph().is_some(),
        };
        let stage = self.stage();
        let ppu = 64.0 * self.settings.scale;
        let talk = TalkContext::from_desktop(desk, Some(monitor::local_hour()));
        let knight = (!down).then(|| self.ch.body());
        let step = {
            let env = GrimmEnv { terrain: &self.terrain, knight, ppu, area: stage.to_f() };
            let bubbles = self.visitors.as_mut().map(|v| &mut v.bubbles);
            t.tick(dt, &w, &env, bubbles, &talk, stage)
        };
        let mut events = step.events.clone();
        if step.place_lantern {
            let at = self.lantern_spot(ppu);
            if at.is_none() {
                log::info!("grimm: no good spot for the lantern right now");
            }
            let env = GrimmEnv { terrain: &self.terrain, knight, ppu, area: stage.to_f() };
            if let Some(e) = t.lantern_placed(at, &env) {
                log::info!("event: {} {e:?}", e.name());
                events.push(e);
            }
        }
        // Held or steered by the user: his blows don't count (no backlog for afterwards).
        if self.hittable() {
            self.enemy_hits += step.hits;
        }
        if let Some(by) = step.hit_by.filter(|_| self.hittable()) {
            log::info!("grimm: {by:?} caught the knight");
        }
        if let Some((at, geo)) = step.reward {
            self.stakes.burst(at, geo, self.settings.scale);
        }
        if events.iter().any(|e| matches!(e, GrimmEvent::Summoned { .. })) {
            // The show begins: a brief "!" before he answers the bow.
            self.emotes.show(Emote::Bang);
        }
        self.achievements.troupe.extend(events);
        self.troupe = Some(t);
    }

    /// A settled, reachable spot for the lantern near the knight (never on a game's monitor).
    fn lantern_spot(&self, ppu: f32) -> Option<(Vec2, Option<skong_core::desktop::WindowId>)> {
        let graph = self.nav.graph()?;
        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
        let settled = |f: &Footing| discoveries.settled(terrain, f.owner, f.y, f.x0, f.x1);
        let footings: Vec<Footing> = graph.surfaces().iter().map(|s| Footing { x0: s.x0, x1: s.x1, y: s.y, owner: s.owner }).filter(|f| settled(f)).collect();
        let games = &self.games.game_monitors;
        let monitors: Vec<_> = self.watcher.snapshot().monitors.iter().filter(|m| !games.contains(&m.rect)).cloned().collect();
        let knight = &self.ch;
        let body = |p: Vec2| RectF::new(p.x - 0.4 * ppu, p.y - 1.2 * ppu, p.x + 0.4 * ppu, p.y);
        let mut reachable = |p: Vec2| self.occupancy.is_free(body(p), None) && graph.plan(knight, NavGoal::Point(p), TravelStyle::Hurry).is_some();
        let mut rng = skong_core::ai::rng::Rng::new(super::seed());
        plan_lantern(&footings, knight.pos, &monitors, ppu, &mut rng, &mut reachable)
    }

    /// The knight's blow on Grimm (hit marker; beaten: the defeat burst marker too).
    pub(super) fn strike_grimm(&mut self, damage: u32, at: Vec2, right: bool) {
        let Some(t) = self.troupe.as_mut() else { return };
        let at = t.centre().unwrap_or(at);
        if let Some(beaten) = t.strike(damage) {
            self.fx.spawn("cursor_hit", at, right);
            if beaten {
                self.fx.spawn("cursor_defeat", at, right);
            }
        }
    }

    /// The knight struck the scarlet flame: the show begins.
    pub(super) fn strike_lantern(&mut self, at: Vec2, right: bool) {
        let x = self.ch.pos.x;
        let Some(ev) = self.troupe.as_mut().and_then(|t| t.strike_lantern(x)) else { return };
        log::info!("event: {} {ev:?}", ev.name());
        self.fx.spawn("cursor_hit", at, right);
        self.emotes.show(Emote::Bang);
        self.achievements.troupe.push(ev);
    }

    pub(super) fn hide_troupe(&mut self) {
        if let Some(t) = self.troupe.as_mut() {
            t.hide_all();
        }
    }

    pub(super) fn raise_troupe(&self) {
        if let Some(t) = &self.troupe {
            t.raise_all();
        }
    }

    pub(super) fn rescale_troupe(&mut self) {
        let scale = self.settings.scale;
        if let Some(t) = self.troupe.as_mut() {
            t.rescale(scale);
        }
    }
}
