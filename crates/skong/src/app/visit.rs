//! Pet glue for NPC visitors, speech bubbles and dream-nail thoughts (see `crate::npcs`).
use anyhow::Result;
use skong_core::ai::{BehaviourKind, Visitor};
use skong_core::controller::Side;
use skong_ipc::Command;
use skong_win::runloop::Flow;
use skong_win::{input, monitor};

use super::{Pet, vanilla_of};
use crate::npcs::{Visitors, World};

impl Pet {
    pub(super) fn load_visitors(&self, capturable: bool) -> Option<Visitors> {
        match Visitors::load(&self.data.args_data, vanilla_of(&self.data.skins), self.settings.scale, super::seed()) {
            Ok(mut v) => {
                v.capturable = capturable;
                v.bubbles.capturable = capturable;
                Some(v)
            }
            Err(e) => {
                log::warn!("visitors disabled: {e:#}");
                None
            }
        }
    }

    pub(super) fn visitor_percepts(&self) -> Vec<Visitor> {
        self.visitors.as_ref().map(|v| v.percepts()).unwrap_or_default()
    }

    /// Build the visitors' view of the world and run `f` on them.
    fn with_visitors(&mut self, f: impl FnOnce(&mut Visitors, &World, &mut crate::fx::Fx)) {
        let Some(v) = self.visitors.as_mut() else { return };
        let busy = self.grab.held_pos().is_some()
            || matches!(self.brain.behaviour(), BehaviourKind::Nap | BehaviourKind::Hide | BehaviourKind::KnockedOut | BehaviourKind::Flee | BehaviourKind::Fight | BehaviourKind::Gauntlet)
            // No visits in the middle of a gauntlet.
            || self.enemies.as_ref().is_some_and(|e| e.gauntlet_active());
        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
        let settled = |f: &skong_core::portal::Footing| discoveries.settled(terrain, f.owner, f.y, f.x0, f.x1);
        let world = World {
            terrain: &self.terrain,
            desk: self.watcher.snapshot(),
            knight: &self.ch,
            graph: self.nav.graph().map(|g| g.as_ref()),
            user_idle_secs: input::idle_secs(),
            keys_per_sec: self.user_input.keys_per_sec(),
            // Any fullscreen app in front (video, slides, games) postpones visits.
            fullscreen: self.brain.senses().fullscreen || self.watcher.snapshot().windows.iter().any(|w| w.fullscreen && w.foreground) || !self.games.game_monitors.is_empty(),
            knight_busy: busy,
            hour: monitor::local_hour(),
            game_monitors: &self.games.game_monitors,
            occupancy: &self.occupancy,
            since_fight: self.brain.since_fight(),
            settled: &settled,
        };
        f(v, &world, &mut self.fx);
    }

    /// The brain's side of a lesson (drill done / stop) for the teaching visitor.
    pub(super) fn lesson_signal(&mut self, r: skong_core::ai::LessonReport) {
        self.with_visitors(|v, w, _| v.lesson(r, w));
    }

    /// The brain stands beside a visitor and wants to talk.
    pub(super) fn greet_visitor(&mut self, id: u32) {
        self.with_visitors(|v, w, _| v.greet(id, w));
    }

    /// Per tick, after the knight moved and animated.
    pub(super) fn tick_visitors(&mut self, dt: f32) {
        let clip = self.data.lib.get(self.anim.clip()).name.clone();
        let origin = self.knight_origin();
        let right = self.ch.facing == Side::Right;
        self.with_visitors(|v, w, fx| {
            v.knight_clip(&clip, origin, right, fx);
            v.tick(dt, w, fx);
        });
        if let Some(v) = &self.visitors {
            v.claim(&mut self.occupancy);
        }
        if self.visitors.as_mut().is_some_and(|v| v.take_dream_request()) && self.act.is_none() {
            let _ = self.apply_visit(Command::DreamNail);
        }
    }

    /// Games cover every monitor: hide visitors and bubbles until the next tick.
    pub(super) fn hide_visitors(&mut self) {
        if let Some(v) = self.visitors.as_mut() {
            v.hide_all();
        }
    }

    /// The brain's dream nail is aimed at a visitor.
    pub(super) fn aim_dream_at_visitor(&mut self, id: u32) {
        if let Some(v) = self.visitors.as_mut() {
            v.aim_dream(id);
        }
    }

    pub(super) fn raise_visitors(&self) {
        if let Some(v) = &self.visitors {
            v.raise_all();
        }
    }

    pub(super) fn rescale_visitors(&mut self) {
        let (skins, scale) = (&self.data.skins, self.settings.scale);
        if let Some(v) = self.visitors.as_mut()
            && let Err(e) = v.rescale(vanilla_of(skins), scale)
        {
            log::warn!("visitors: {e:#}");
        }
    }

    pub(super) fn apply_visit(&mut self, c: Command) -> Result<Flow> {
        match c {
            Command::InviteVisitor => match self.visitors.as_mut() {
                Some(v) => v.invite(None),
                None => log::warn!("visitors are not available"),
            },
            Command::DreamNail => {
                let talking = self.visitors.as_ref().is_some_and(|v| v.conversing());
                if !talking
                    && self.ch.is_grounded()
                    && let Some(id) = self.data.acts.id("dream_nail")
                {
                    self.seat_y = None;
                    self.start_act(id);
                }
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }
}
