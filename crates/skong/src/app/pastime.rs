//! Pastime props glue for [`Pet`] (`crate::pastimes`): the training dummy and lumaflies turn
//! up on their schedules (never while the user types hard, is away, a fullscreen app is up,
//! or he's dead; never on a game monitor), the brain learns where they are, and its blows and
//! swipes reach them. Kept out of `app.rs`.
use skong_core::ai::{BehaviourKind, KnownDummy, PastimeEvent, Reach, dummy_stand};
use skong_core::geom::Vec2;
use skong_core::nav::NavGoal;
use skong_core::pastime::Swipe;
use skong_core::terrain::LedgeKind;

use super::Pet;
use crate::pastimes::Where;

/// A dummy the brain can't get to this long (s) is moved (or sent away).
const DUMMY_UNREACHABLE_SECS: f32 = 20.0;

impl Pet {
    pub(super) fn pastimes_tick(&mut self, dt: f32) {
        let s = self.brain.senses();
        let allowed = !s.user_busy && !s.user_away && !s.fullscreen && self.brain.behaviour() != BehaviourKind::Dead;
        let desk = self.watcher.snapshot();
        let k = self.ch.pos;
        // The knight's monitor (never a game's: he isn't on one).
        let area = desk
            .monitor_at(k.x as i32, k.y as i32 - 1)
            .filter(|m| !self.games.game_monitors.contains(&m.rect))
            .map(|m| m.work.to_f());
        let bars: Vec<(f32, f32, f32)> = match area {
            Some(a) => self.terrain.ledges.iter().filter(|l| l.kind != LedgeKind::Ground && l.owner.is_some() && l.y >= a.top && l.y <= a.bottom).map(|l| (l.x0, l.x1, l.y)).collect(),
            None => Vec::new(),
        };
        if self.pastimes.due(dt, allowed, area, &bars, k) {
            self.spawn_dummy(false);
        }
        if allowed && area.is_some() {
            self.move_unreachable_dummy();
        }
        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
        let settled = |l: &skong_core::terrain::Ledge| discoveries.ledge_settled(terrain, l);
        let (created, news) = self.pastimes.update(dt, self.watcher.snapshot(), &self.terrain, &mut self.occupancy, &settled);
        for (at, hit) in news.hits {
            // The game's hit marker, along the blow.
            self.fx.spawn("cursor_hit", at, !hit.from_right);
        }
        if created {
            self.win.raise(); // the knight stays in front
        }
    }

    /// Put the dummy where the brain's own test says he'd go and practise, on his monitor
    /// (`debug`: nearer, any settled surface). Logs why not, if not.
    pub(super) fn spawn_dummy(&mut self, debug: bool) -> bool {
        let graph = self.nav.graph().map(|g| g.as_ref());
        let ch = &self.ch;
        let practisable = |d: &KnownDummy| {
            graph.is_some_and(|g| dummy_stand(d, ch, &mut |at, st| g.plan(ch, NavGoal::Point(at), st).map(|r| Reach { eta: r.eta, end: Some(r.end) })).is_some())
        };
        let (ages, now) = self.discoveries.ages();
        let w = Where { desk: self.watcher.snapshot(), terrain: &self.terrain, ages, now, occupancy: &self.occupancy, knight: ch.pos, practisable: &practisable };
        match self.pastimes.spawn_dummy(&w, debug) {
            Ok(_) => true,
            Err(miss) => {
                let why = if graph.is_none() { "no route planner yet".to_string() } else { miss.to_string() };
                if debug {
                    log::warn!("training dummy: no spot he'd practise at near ({:.0},{:.0}): {why}", ch.pos.x, ch.pos.y);
                } else {
                    log::debug!("training dummy: not now ({why})");
                }
                false
            }
        }
    }

    /// Would he walk over and practise on a dummy standing at `foot` on a ledge `span` (the
    /// brain's own test against the current route planner)? For the test panel's placement.
    pub(super) fn dummy_practisable_at(&self, foot: Vec2, span: (f32, f32)) -> bool {
        let ch = &self.ch;
        let d = self.pastimes.known_at(foot, span);
        self.nav.graph().is_some_and(|g| dummy_stand(&d, ch, &mut |at, st| g.plan(ch, NavGoal::Point(at), st).map(|r| Reach { eta: r.eta, end: Some(r.end) })).is_some())
    }

    /// A dummy the brain wants but can't get to for a while: moved somewhere he can, or (no
    /// such place) sent away.
    fn move_unreachable_dummy(&mut self) {
        let Some((id, secs)) = self.brain.dummy_unreachable() else { return };
        if secs < DUMMY_UNREACHABLE_SECS || self.pastimes.dummy_id() != Some(id) {
            return;
        }
        if !self.pastimes.relocatable(id) {
            // The user put it there on purpose (the Test tab, or dragged it): it stays (the
            // brain logs why he ignores it).
            return;
        }
        log::info!("training dummy #{id}: the knight can't get to it ({secs:.0} s); moving it");
        if !self.spawn_dummy(false) {
            log::info!("training dummy #{id}: nowhere better for it; it fades away");
            self.pastimes.retire_dummy();
        }
    }

    /// A lumafly now (debug), on the knight's monitor.
    pub(super) fn send_lumafly(&mut self) {
        let desk = self.watcher.snapshot();
        let k = self.ch.pos;
        let Some(area) = desk.monitor_at(k.x as i32, k.y as i32 - 1).or(desk.primary()).map(|m| m.work.to_f()) else { return };
        let bars: Vec<(f32, f32, f32)> = self.terrain.ledges.iter().filter(|l| l.kind != LedgeKind::Ground && l.owner.is_some()).map(|l| (l.x0, l.x1, l.y)).collect();
        self.pastimes.spawn_lumafly(area, &bars, k);
    }

    /// Before the brain ticks: where the dummy and the lumaflies are.
    pub(super) fn pastime_percepts(&mut self) {
        let (dummy, flies) = self.pastimes.percepts();
        self.brain.set_pastimes(dummy, &flies);
    }

    /// After the brain ticks: its blows on the dummy and swipes at lumaflies.
    pub(super) fn pastime_intent(&mut self) {
        for ev in self.brain.take_pastime_events() {
            match ev {
                PastimeEvent::DummyHit { hit, delay } => self.pastimes.strike(hit, delay),
                PastimeEvent::Swipe { fly, from } => {
                    if self.pastimes.swipe(fly, from) == Swipe::Caught {
                        log::info!("caught a lumafly (it slips away)");
                    }
                }
            }
        }
    }
}
