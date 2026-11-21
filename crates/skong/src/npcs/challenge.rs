//! App side of Hornet's challenges (`skong_core::npc::challenge`): deciding race or duel as
//! her talk begins (a race needs a finish on a settled ledge of her monitor that the knight
//! can reach, never on a game's monitor; without one it's a duel), the knight's scene cue,
//! calling a challenge off when the user needs the screen (or the Shade waits, or enemies
//! came), and the windows of her props (the duel's needle and thread, the race's barb).
use anyhow::Result;
use skong_core::ai::SceneCue;
use skong_core::assets::SpriteKey;
use skong_core::canvas::{Image, premul};
use skong_core::geom::Vec2;
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::npc::race::pick_finish;
use skong_core::npc::{ChallengeKind, ChallengePlan, DrawnProp, NpcPhase, NpcProp};
use skong_core::portal::{Footing, has_headroom};
use skong_win::sprite_window::SpriteWindow;

use super::{ARRIVAL_HEADROOM, Visitors, World, footprint};
use crate::bubble::faded;

/// One extra sprite's window (the needle, its thread, the barb).
pub(super) struct PropWin {
    pub win: SpriteWindow,
    /// What it shows now, at what opacity.
    drawn: Option<(Look, u8)>,
    /// The last plainly drawn prop (art missing), kept while it looks the same.
    plain: Option<(Look, Image)>,
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

    /// Where her barb will land: a settled, reachable ledge on her monitor, with room
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
                // The route must end at the barb, not merely near it.
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

    /// Her props (the duel's needle and thread, her sphere, the race's barb): one small
    /// click-through window each, while they're out. A prop whose art is missing is drawn
    /// plainly when it can be (the barb), else not shown.
    pub(super) fn draw_props(&mut self) -> Result<()> {
        for l in &mut self.live {
            let props = l.npc.props(&self.set, self.ppu);
            let frames = self.frames.get(l.npc.def);
            while l.props.len() < props.len() {
                let win = SpriteWindow::new()?;
                win.set_click_through(true);
                win.set_capturable(self.capturable);
                l.props.push(PropWin { win, drawn: None, plain: None });
            }
            for (k, pw) in l.props.iter_mut().enumerate() {
                let p = props.get(k);
                let shown = match p.map(|p| (p, frames.and_then(|f| f.get(&p.key)))) {
                    Some((p, Some(f))) => Some((p, Look::Art(p.key, p.flip), &f.image, f.anchor_x(p.flip), f.anchor[1])),
                    Some((p @ NpcProp { drawn: Some(DrawnProp::Barb { radius, burst }), .. }, None)) => {
                        let look = Look::Barb(radius.round() as u32, (burst.clamp(0.0, 1.0) * 8.0) as u8);
                        if pw.plain.as_ref().is_none_or(|(l, _)| *l != look) {
                            pw.plain = Some((look, plain_barb(radius.round(), (burst.clamp(0.0, 1.0) * 8.0).floor() / 8.0)));
                        }
                        pw.plain.as_ref().map(|(_, img)| (p, look, img, img.width as f32 / 2.0, img.height as f32 / 2.0))
                    }
                    _ => None,
                };
                let Some((p, look, image, ax, ay)) = shown else {
                    if pw.drawn.take().is_some() {
                        pw.win.set_visible(false);
                    }
                    continue;
                };
                let x = (p.at.x - ax).round() as i32;
                let y = (p.at.y - ay).round() as i32;
                let alpha = (p.alpha.clamp(0.0, 1.0) * l.npc.alpha.clamp(0.0, 1.0) * 255.0) as u8;
                if pw.drawn.is_none() {
                    pw.win.set_visible(true);
                }
                if pw.drawn != Some((look, alpha)) {
                    if alpha == 255 {
                        pw.win.present_flipped(image, x, y, p.flip)?;
                    } else {
                        pw.win.present_flipped(&faded(image, alpha), x, y, p.flip)?;
                    }
                    pw.drawn = Some((look, alpha));
                } else {
                    pw.win.move_to(x, y)?;
                }
            }
        }
        Ok(())
    }
}

/// What a prop window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// A sprite (mirrored or not).
    Art(SpriteKey, bool),
    /// A plainly drawn barb: tip radius (px), burst step (0..=8).
    Barb(u32, u8),
}

/// A spiked silk ball (Hornet's barb, when its art is missing): pale spikes with a dark
/// rim around a red knot of silk, `radius` px to the tips; as `burst` goes 0..1 the spikes
/// fly apart, the knot gone, and fade.
fn plain_barb(radius: f32, burst: f32) -> Image {
    const SPIKES: f32 = 11.0;
    let spread = 0.6 * radius * burst;
    let size = (2.0 * (radius + spread) + 4.0).ceil() as u32;
    let mut img = Image::new(size, size);
    let c = size as f32 / 2.0;
    let core = 0.38 * radius;
    let knot = 0.3 * radius;
    let a = (255.0 * (1.0 - burst)).round() as u8;
    let (pale, rim, red) = (premul(236, 226, 230, a), premul(30, 24, 32, a), premul(150, 44, 60, a));
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            // Bursting: everything beyond the knot slides outwards.
            let r = (dx * dx + dy * dy).sqrt() - spread;
            if burst > 0.0 && r < core {
                continue;
            }
            let turn = dy.atan2(dx) / std::f32::consts::TAU * SPIKES + 0.25;
            // 0 on a spike's axis, 1 halfway to the next.
            let w = (turn - turn.round()).abs() * 2.0;
            let tip = core + (radius - core) * (1.0 - w).max(0.0).powf(1.6);
            let px = if burst == 0.0 && r <= knot - 1.5 {
                red
            } else if burst == 0.0 && r <= knot {
                rim
            } else if r <= tip - 2.4 - 1.2 * w {
                pale
            } else if r <= tip {
                rim
            } else {
                continue;
            };
            img.px[(y * size + x) as usize] = px;
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::plain_barb;

    #[test]
    fn a_plain_barb_is_drawn_and_bursts_into_fading_shards() {
        let opaque = |i: &skong_core::canvas::Image| i.px.iter().filter(|p| **p >> 24 == 255).count();
        let middle = |i: &skong_core::canvas::Image| i.px[(i.height / 2 * i.width + i.width / 2) as usize];
        let whole = plain_barb(50.0, 0.0);
        assert_eq!(middle(&whole) >> 24, 255, "a solid knot in the middle");
        assert!(opaque(&whole) > 2000, "{} px", opaque(&whole));
        let bursting = plain_barb(50.0, 0.5);
        assert!(bursting.width > whole.width, "the shards fly outwards");
        assert_eq!(middle(&bursting), 0, "the knot is gone");
        assert!(bursting.px.iter().any(|p| *p != 0) && opaque(&bursting) == 0, "fading shards");
    }
}
