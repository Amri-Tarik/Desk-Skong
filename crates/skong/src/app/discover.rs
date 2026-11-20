//! Discoveries glue for [`Pet`]: per-tick prop update, what the brain perceives, brain
//! discovery events, and the debug commands / tray submenu. Kept out of `app.rs`.
//! Shinies on the ground (`crate::shinies`) live here too: they share the placement rules.
use anyhow::Result;
use skong_core::ai::{BehaviourKind, DiscoveryEvent};
use skong_core::geom::Vec2;
use skong_core::nav::{NavGoal, TravelStyle};
use skong_ipc::Command;
use skong_win::monitor;
use skong_win::runloop::Flow;
use skong_win::tray::MenuItem;

use super::Pet;
use crate::shinies::Where;

const ID_DISCOVER0: u32 = 5000;
const ID_FORGET: u32 = 5003;
const KINDS: [&str; 5] = ["Reveal a bench here", "Reveal a geo deposit here", "Drop a shiny nearby", "Put up a training dummy", "Let a lumafly drift by"];
/// `Command::Discover` kind that drops a shiny instead of revealing a find.
const SHINY: u8 = 2;
/// ... that puts up the training dummy / sends a lumafly (pastimes).
const DUMMY: u8 = 3;
const LUMAFLY: u8 = 4;
/// The pick-up act and the step at which the shiny is in his hand.
const PICK_ACT: &str = "pick_up_trinket";
const PICK_CLIP: &str = "Collect Normal 2";

/// Tray submenu: reveal a find on the knight's window (or the one under the cursor).
pub(super) fn menu() -> MenuItem {
    let mut items: Vec<MenuItem> = KINDS.iter().enumerate().map(|(i, k)| MenuItem::action(ID_DISCOVER0 + i as u32, *k)).collect();
    items.push(MenuItem::Separator);
    items.push(MenuItem::action(ID_FORGET, "Forget all discoveries"));
    MenuItem::sub("Discoveries (debug)", items)
}

pub(super) fn menu_command(id: u32) -> Option<Command> {
    match id {
        ID_FORGET => Some(Command::ForgetDiscoveries),
        i if (ID_DISCOVER0..ID_DISCOVER0 + KINDS.len() as u32).contains(&i) => Some(Command::Discover((i - ID_DISCOVER0) as u8)),
        _ => None,
    }
}

impl Pet {
    pub(super) fn discoveries_tick(&mut self, dt: f32) {
        self.benches.claim(&mut self.occupancy);
        self.discoveries.update(dt, (self.watcher.snapshot(), &self.terrain), &self.benches, &mut self.occupancy, &mut self.fx, (&self.ch, &self.win));
        self.shinies_tick(dt);
        self.pastimes_tick(dt);
    }

    /// Shinies: turn up now and then (never while the user types hard, is away, or a
    /// fullscreen app is up), vanish into his hand at "Collect Normal 2".
    fn shinies_tick(&mut self, dt: f32) {
        let s = self.brain.senses();
        let allowed = !s.user_busy && !s.user_away && !s.fullscreen && self.brain.behaviour() != BehaviourKind::Dead;
        if self.shinies.due(dt, allowed) {
            self.spawn_shiny(false);
        }
        let in_hand = self.act.as_ref().is_some_and(|p| self.data.acts.get(p.act()).name == PICK_ACT) && self.data.lib.get(self.anim.clip()).name == PICK_CLIP;
        if in_hand
            && let Some((at, geo)) = self.shinies.picked()
            && geo > 0
        {
            // A little Geo pops out of it.
            let up = Vec2::new(0.0, 20.0 * self.settings.scale);
            self.stakes.burst(at - up, geo, self.settings.scale);
        }
        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
        let settled = |l: &skong_core::terrain::Ledge| discoveries.ledge_settled(terrain, l);
        if self.shinies.update(self.watcher.snapshot(), &self.terrain, &mut self.occupancy, &settled) {
            self.win.raise(); // the knight stays in front of the glint
        }
    }

    /// Put a shiny somewhere he can get to on his monitor (`debug`: any surface, even a
    /// fresh one, and beyond the cap). Returns whether one turned up.
    fn spawn_shiny(&mut self, debug: bool) -> bool {
        let graph = self.nav.graph().map(|g| g.as_ref());
        let ch = &self.ch;
        // A route that really ends on the spot (not merely near it).
        let reachable = |p: Vec2| {
            graph.is_none_or(|g| {
                [TravelStyle::Normal, TravelStyle::Hurry]
                    .into_iter()
                    .any(|st| g.plan(ch, NavGoal::Point(p), st).is_some_and(|r| (r.end.x - p.x).abs() <= 12.0 && (r.end.y - p.y).abs() <= 4.0))
            })
        };
        let (ages, now) = self.discoveries.ages();
        let w = Where { desk: self.watcher.snapshot(), terrain: &self.terrain, ages, now, occupancy: &self.occupancy, knight: ch.pos, reachable: &reachable };
        self.shinies.spawn(&w, debug).is_some()
    }

    /// Before the brain ticks: found benches join `benches`, the brain learns the places
    /// and the shinies, and the seat here (found bench / spring water line), if any.
    pub(super) fn discovery_percepts(&mut self, benches: &mut Vec<Vec2>) -> Option<f32> {
        benches.extend(self.discoveries.bench_spots());
        self.brain.set_places(&self.discoveries.places());
        self.brain.set_shinies(&self.shinies.percepts());
        self.pastime_percepts();
        self.discoveries.seat_here(&self.ch, &self.benches)
    }

    pub(super) fn on_discovery(&mut self, ev: DiscoveryEvent) {
        match ev {
            DiscoveryEvent::Explored(id) => self.discoveries.explored(id, (self.watcher.snapshot(), &self.terrain), &self.benches, &self.occupancy),
            DiscoveryEvent::Used(id) => self.discoveries.used(id),
            DiscoveryEvent::Struck(id) => {
                if let Some(d) = self.discoveries.struck(id, &self.ch, &mut self.fx) {
                    // Out of the rock, a little above its foot, like a geo rock in the game.
                    let up = Vec2::new(0.0, 30.0 * self.settings.scale);
                    self.stakes.burst(d.at - up, d.geo, self.settings.scale);
                }
            }
            DiscoveryEvent::PickUp(id) => self.shinies.picking(id),
        }
    }

    /// The bored knight asked for a shiny (a world event).
    pub(super) fn shiny_requested(&mut self) {
        if !self.spawn_shiny(false) {
            log::debug!("bored knight wanted a shiny, but there was no room for one");
        }
    }

    pub(super) fn apply_discovery(&mut self, c: Command) -> Result<Flow> {
        match c {
            Command::Discover(SHINY) => {
                if self.spawn_shiny(true) {
                    self.note(true, "a shiny turned up near him");
                } else {
                    self.note(false, "shiny: no free, reachable spot on the knight's monitor");
                }
            }
            Command::Discover(DUMMY) => {
                if self.spawn_dummy(true) {
                    self.note(true, "a training dummy stands near him");
                } else {
                    self.note(false, "training dummy: no free, reachable spot on the knight's monitor");
                }
            }
            Command::Discover(LUMAFLY) => {
                self.send_lumafly();
                self.note(true, "a lumafly drifts in over his monitor's windows");
            }
            Command::Discover(kind) => {
                let desk = self.watcher.snapshot();
                let under_cursor = || {
                    let (x, y) = monitor::cursor_pos()?;
                    desk.windows.iter().find(|w| w.rect.contains(x, y)).map(|w| w.id)
                };
                match self.ch.support.and_then(|s| s.owner).or_else(under_cursor) {
                    Some(id) => self.discoveries.force(kind, id, (desk, &self.terrain), &self.benches, &self.occupancy),
                    None => self.note(false, "discover: stand the knight on a window (or point at one) first"),
                }
            }
            Command::ForgetDiscoveries => {
                self.discoveries.forget();
                self.shinies.clear();
                self.pastimes.clear();
            }
            other => return self.apply(other),
        }
        Ok(Flow::Continue)
    }
}
