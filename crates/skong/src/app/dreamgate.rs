//! Dream Gate glue for [`Pet`] (see [`skong_core::dreamgate`]): the brain plants a gate at a
//! prize no route reaches ([`Pet::on_gate_event`]); the gate glows quietly on the target
//! ledge, and the moment the warp act ends ([`Pet::dream_gate_tick`], before drawing) the
//! knight is moved there and reappears, with no idle frame in between. A gate whose ledge
//! went away (or whose monitor a game took over) lets him reappear where he stood.
use skong_core::ai::Navigator;
use skong_core::controller::Character;
use skong_core::dreamgate::{ARRIVE_ACT, GateAction, GateEvent};
use skong_core::geom::Vec2;

use super::Pet;
use crate::gameguard::GameGuard;

impl Pet {
    /// A gate event from the brain.
    pub(super) fn on_gate_event(&mut self, ev: GateEvent) {
        let mut out = Vec::new();
        self.gate.event(ev, &mut out);
        self.apply_gate(out);
    }

    /// Glow timing and the jump at the end of the warp act (after the act player, before
    /// drawing).
    pub(super) fn dream_gate_tick(&mut self, dt: f32) {
        let mut out = Vec::new();
        let act = self.act.as_ref().map(|p| (self.data.acts.get(p.act()).name.as_str(), p.step_index()));
        let (terrain, games, desk) = (&self.terrain, &self.games, self.watcher.snapshot());
        let valid = |at: Vec2| {
            let on_ledge = terrain.ledges.iter().any(|l| (l.y - at.y).abs() <= 3.0 && l.x0 <= at.x && at.x <= l.x1);
            let on_game = GameGuard::monitor_of(desk, at).is_some_and(|m| games.is_game_monitor(&m));
            on_ledge && !on_game
        };
        self.gate.tick(dt, act, valid, &mut out);
        self.apply_gate(out);
    }

    fn apply_gate(&mut self, out: Vec<GateAction>) {
        for a in out {
            match a {
                GateAction::Spawn(name, at) => self.fx.spawn(name, at, false),
                GateAction::Stop(name) => self.fx.stop(name),
                GateAction::WarpTo(at) => {
                    let facing = self.ch.facing;
                    self.ch = Character::new(self.ch.params, self.ch.abilities, at - Vec2::new(0.0, 1.0));
                    self.ch.facing = facing;
                    self.seat_y = None;
                    self.forced_nav = false;
                    self.nav.stop();
                    log::info!("dream gate: the knight warps to ({:.0},{:.0})", at.x, at.y);
                    self.dream_gate_arrive();
                }
                GateAction::Reappear => {
                    log::info!("dream gate: the gate is gone; the knight reappears where he stood");
                    self.dream_gate_arrive();
                }
            }
        }
    }

    fn dream_gate_arrive(&mut self) {
        match self.data.acts.id(ARRIVE_ACT) {
            Some(id) => self.start_act(id),
            None => self.act = None,
        }
    }
}
