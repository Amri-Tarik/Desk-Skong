//! Pastime props glue for [`Pet`] (`crate::pastimes`): the training dummy and lumaflies turn
//! up on their schedules (never while the user types hard, is away, a fullscreen app is up,
//! or he's dead; never on a game monitor), the brain learns where they are, and its blows and
//! swipes reach them. Kept out of `app.rs`.
use skong_core::ai::{BehaviourKind, PastimeEvent};
use skong_core::geom::Vec2;
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::pastime::Swipe;
use skong_core::terrain::LedgeKind;

use super::Pet;
use crate::pastimes::Where;

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

    /// Put the dummy somewhere he can get to on his monitor (`debug`: any surface).
    pub(super) fn spawn_dummy(&mut self, debug: bool) -> bool {
        let graph = self.nav.graph().map(|g| g.as_ref());
        let ch = &self.ch;
        let reachable = |p: Vec2| {
            graph.is_none_or(|g| {
                [TravelStyle::Normal, TravelStyle::Hurry]
                    .into_iter()
                    .any(|st| g.plan(ch, NavGoal::Point(p), st).is_some_and(|r| (r.end.x - p.x).abs() <= 40.0 && (r.end.y - p.y).abs() <= 4.0))
            })
        };
        let (ages, now) = self.discoveries.ages();
        let w = Where { desk: self.watcher.snapshot(), terrain: &self.terrain, ages, now, occupancy: &self.occupancy, knight: ch.pos, reachable: &reachable };
        self.pastimes.spawn_dummy(&w, debug).is_some()
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
