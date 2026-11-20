//! App side of Hornet's challenges (`skong_core::npc::challenge`): deciding race or duel as
//! her talk begins (a race needs a finish on a settled ledge of her monitor that the knight
//! can reach, never on a game's monitor; without one it's a duel), the knight's scene cue,
//! calling a challenge off when the user needs the screen (or the Shade waits, or enemies
//! came), and the windows of her needle and its thread.
use anyhow::Result;
use skong_core::ai::SceneCue;
use skong_core::assets::SpriteKey;
use skong_core::geom::Vec2;
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::npc::race::pick_finish;
use skong_core::npc::{ChallengeKind, ChallengePlan, NpcPhase};
use skong_core::portal::{Footing, has_headroom};
use skong_win::sprite_window::SpriteWindow;

use super::{ARRIVAL_HEADROOM, Visitors, World, footprint};
use crate::bubble::faded;

/// One extra sprite's window (the needle, its thread).
pub(super) struct PropWin {
    pub win: SpriteWindow,
    drawn: Option<(SpriteKey, bool, u8)>,
}

#[derive(Default)]
pub(super) struct Challenges {
    /// Why challenges are off right now (set by the pet every tick), if they are.
    hold: Option<&'static str>,
    /// Debug: the next Hornet visit challenges, this way.
    pub(super) forced: Option<ChallengeKind>,
}

impl Visitors {
    /// Why no challenge may run now (a fullscreen app, hard typing, the Shade waiting,
    /// enemies about...), or `None`. Set every tick before [`Visitors::tick`].
    pub fn set_challenge_hold(&mut self, why: Option<&'static str>) {
        self.challenges.hold = why;
    }

    /// Debug: Hornet now, and her talk ends in a race (`race`) or a duel.
    pub fn invite_challenge(&mut self, race: bool) {
        self.challenges.forced = Some(if race { ChallengeKind::Race } else { ChallengeKind::Duel });
        self.invite(Some("hornet".into()));
    }

    /// The knight's part in a visitor scene (Hornet's race), if one is on.
    pub fn scene_cue(&self) -> Option<SceneCue> {
        self.live.iter().find_map(|l| l.npc.scene_cue())
    }

    /// As visitor `i`'s talk begins: maybe a challenge after it (race or duel).
    pub(super) fn plan_challenge(&mut self, i: usize, w: &World) {
        let def = self.live[i].npc.def;
        let Some(c) = self.set.defs[def].challenge.as_ref() else { return };
        let forced = self.challenges.forced.take();
        if let Some(why) = self.challenges.hold {
            log::info!("visitor: no challenge now ({why})");
            if forced.is_some() {
                self.notes.push((false, format!("{}: no challenge now ({why}); just a talk", self.set.defs[def].display)));
            }
            return;
        }
        if forced.is_none() && !self.rng.chance(c.chance) {
            return;
        }
        let kind = forced.unwrap_or_else(|| c.pick(&mut self.rng));
        let plan = match kind {
            ChallengeKind::Duel => ChallengePlan::Duel,
            ChallengeKind::Race => match self.race_finish(i, w) {
                Some(plan) => plan,
                None => {
                    log::info!("visitor: no good finish for a race here; a duel instead");
                    if forced.is_some() {
                        self.notes.push((false, "no good finish for a race here (a settled ledge he can reach, with headroom, on her monitor); a duel instead".into()));
                    }
                    ChallengePlan::Duel
                }
            },
        };
        log::info!("visitor: {} will challenge the knight: {plan:?}", self.set.defs[def].display);
        if forced.is_some() {
            let what = if matches!(plan, ChallengePlan::Duel) { "a duel" } else { "a race" };
            self.notes.push((true, format!("{}: {what} after the talk", self.set.defs[def].display)));
        }
        self.live[i].npc.plan_challenge(plan, &self.set);
    }

    /// Where her needle will land: a settled, reachable ledge on her monitor, with room
    /// above; the knight's planned time there paces her.
    fn race_finish(&mut self, i: usize, w: &World) -> Option<ChallengePlan> {
        let ppu = self.ppu;
        let def = self.live[i].npc.def;
        let race = &self.set.defs[def].challenge.as_ref()?.race;
        let start = self.live[i].npc.pos;
        let area = self.work_at(start, w.desk);
        if w.game_monitors.iter().any(|r| r.contains(start.x as i32, (start.y - 1.0) as i32)) {
            return None;
        }
        let graph = w.graph?;
        let ok = |f: &Footing| (w.settled)(f) && has_headroom(f, &w.desk.monitors, ARRIVAL_HEADROOM * ppu);
        let footings: Vec<Footing> = graph.surfaces().iter().map(|s| Footing { x0: s.x0, x1: s.x1, y: s.y, owner: s.owner }).filter(|f| ok(f)).collect();
        let knight = w.knight;
        let mut eta = None;
        let mut reachable = |p: Vec2| {
            if !w.occupancy.is_free(footprint(p, ppu), None) {
                return false;
            }
            match graph.plan(knight, NavGoal::Point(p), TravelStyle::Hurry) {
                // The route must end at the needle, not merely near it.
                Some(r) if r.end.distance(p) <= race.reach * ppu => {
                    eta = Some(r.eta);
                    true
                }
                _ => false,
            }
        };
        let rng = &mut self.rng;
        let mut random = || rng.f32();
        let (finish, owner) = pick_finish(&footings, start, area.to_f(), ppu, race.distance, &mut random, &mut reachable)?;
        Some(ChallengePlan::Race { finish, owner, knight_eta: eta })
    }

    /// Call challenges off while `hold` says so (planned ones are dropped).
    pub(super) fn apply_hold(&mut self, w: &World) {
        let Some(why) = self.challenges.hold else { return };
        for i in 0..self.live.len() {
            let running = matches!(self.live[i].npc.phase(), NpcPhase::Racing | NpcPhase::Sparring | NpcPhase::Talking);
            if self.live[i].npc.challenging() && running {
                let mut events = Vec::new();
                if self.live[i].npc.call_off(&self.set, &mut events) {
                    log::info!("visitor: challenge called off ({why})");
                }
                self.handle_events(i, events, w);
            }
        }
    }

    /// Her needle and thread: one small click-through window each, while they're out.
    pub(super) fn draw_props(&mut self) -> Result<()> {
        for l in &mut self.live {
            let props = l.npc.props(&self.set, self.ppu);
            let frames = self.frames.get(l.npc.def);
            while l.props.len() < props.len() {
                let win = SpriteWindow::new()?;
                win.set_click_through(true);
                win.set_capturable(self.capturable);
                l.props.push(PropWin { win, drawn: None });
            }
            for (k, pw) in l.props.iter_mut().enumerate() {
                let Some((p, frame)) = props.get(k).and_then(|p| frames.and_then(|f| f.get(&p.key)).map(|f| (p, f))) else {
                    if pw.drawn.take().is_some() {
                        pw.win.set_visible(false);
                    }
                    continue;
                };
                let x = (p.at.x - frame.anchor_x(p.flip)).round() as i32;
                let y = (p.at.y - frame.anchor[1]).round() as i32;
                let alpha = (p.alpha.clamp(0.0, 1.0) * l.npc.alpha.clamp(0.0, 1.0) * 255.0) as u8;
                if pw.drawn.is_none() {
                    pw.win.set_visible(true);
                }
                if pw.drawn != Some((p.key, p.flip, alpha)) {
                    if alpha == 255 {
                        pw.win.present_flipped(&frame.image, x, y, p.flip)?;
                    } else {
                        pw.win.present_flipped(&faded(&frame.image, alpha), x, y, p.flip)?;
                    }
                    pw.drawn = Some((p.key, p.flip, alpha));
                } else {
                    pw.win.move_to(x, y)?;
                }
            }
        }
        Ok(())
    }
}
