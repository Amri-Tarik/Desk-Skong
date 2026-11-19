//! Discoveries glue for [`Pet`]: per-tick prop update, what the brain perceives, brain
//! discovery events, and the debug commands / tray submenu. Kept out of `app.rs`.
use anyhow::Result;
use skong_core::ai::DiscoveryEvent;
use skong_core::geom::Vec2;
use skong_ipc::Command;
use skong_win::monitor;
use skong_win::runloop::Flow;
use skong_win::tray::MenuItem;

use super::Pet;

const ID_DISCOVER0: u32 = 5000;
const ID_FORGET: u32 = 5003;
const KINDS: [&str; 3] = ["Reveal a bench here", "Reveal a hot spring here", "Reveal a geo deposit here"];

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
        self.discoveries.update(dt, (self.watcher.snapshot(), &self.terrain), &self.benches, &mut self.fx, &self.ch, &self.win);
    }

    /// Before the brain ticks: found benches join `benches`, the brain learns the places,
    /// and the seat here (found bench / spring water line), if any.
    pub(super) fn discovery_percepts(&mut self, benches: &mut Vec<Vec2>) -> Option<f32> {
        benches.extend(self.discoveries.bench_spots());
        self.brain.set_places(&self.discoveries.places());
        self.discoveries.seat_here(&self.ch, &self.benches)
    }

    pub(super) fn on_discovery(&mut self, ev: DiscoveryEvent) {
        match ev {
            DiscoveryEvent::Explored(id) => self.discoveries.explored(id, self.watcher.snapshot(), &self.terrain, &self.benches),
            DiscoveryEvent::Used(id) => self.discoveries.used(id),
            DiscoveryEvent::Struck(id) => {
                if let Some(d) = self.discoveries.struck(id, &self.ch, &mut self.fx) {
                    // Out of the rock, a little above its foot, like a geo rock in the game.
                    let up = Vec2::new(0.0, 30.0 * self.settings.scale);
                    self.stakes.burst(d.at - up, d.geo, self.settings.scale);
                }
            }
        }
    }

    pub(super) fn apply_discovery(&mut self, c: Command) -> Result<Flow> {
        match c {
            Command::Discover(kind) => {
                let desk = self.watcher.snapshot();
                let under_cursor = || {
                    let (x, y) = monitor::cursor_pos()?;
                    desk.windows.iter().find(|w| w.rect.contains(x, y)).map(|w| w.id)
                };
                match self.ch.support.and_then(|s| s.owner).or_else(under_cursor) {
                    Some(id) => self.discoveries.force(kind, id, desk, &self.terrain, &self.benches),
                    None => log::warn!("discover: stand the knight on a window (or point at one) first"),
                }
            }
            Command::ForgetDiscoveries => self.discoveries.forget(),
            other => return self.apply(other),
        }
        Ok(Flow::Continue)
    }
}
