//! Carried items: the map and the lantern. Taking one out only changes his look: the
//! locomotion swaps its idle / walk clips for the carried variants (`Map Idle` / `Map Walk`,
//! `Lantern Idle` / `Lantern Run`, see [`crate::locomotion::Locomotion::set_carry`]) while
//! the brain keeps doing its usual thing (following the cursor, strolling, idling). There is
//! no act of its own, so no back-and-forth patrol on a small ledge.
//!
//! * The **lantern** comes out where it's dark around him (`Senses::dark`: a dark-themed
//!   window, a dim screen, night) and stays out while it is. There's no clip for taking it
//!   out or putting it away: it simply appears, as in the game. Acts play over it.
//! * The **map** comes out now and then (`take_out_map`: more often when curious, or on
//!   arriving at a new window with nowhere to sit), for a random 15–40 s, and goes back with
//!   `put_map_away`. While it's out he reads it instead of glancing about (look up / down,
//!   stares are skipped).
//!
//! Anything that needs his hands or attention puts it away first: any behaviour other than
//! idling, keeping company or exploring (fights, the Shade, Geo, visitors, pastimes, rest...),
//! a seat or any act that isn't a glance, and — for the map — leaving the ground or moving
//! faster than a stroll (the map clips only walk).
use crate::nav::TravelStyle;

use super::behaviours::{BehaviourKind, Tier};
use super::percept::Percepts;
use super::{Brain, Intent, Motion};

/// What the knight has in his hands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carry {
    Map,
    Lantern,
}

impl Carry {
    pub fn label(self) -> &'static str {
        match self {
            Carry::Map => "map out",
            Carry::Lantern => "lantern out",
        }
    }
}

/// The act that takes the map out (`Map Open`); the map is carried once it plays.
pub(crate) const TAKE_OUT_MAP: &str = "take_out_map";
/// The act that puts it away (`Map Away`).
pub(crate) const PUT_MAP_AWAY: &str = "put_map_away";
/// How long the map stays out (s).
const MAP_SECS: [f32; 2] = [15.0, 40.0];
/// Before the map comes out again (s).
const MAP_GAP: [f32; 2] = [90.0, 150.0];
/// The first look at the map, at the earliest (s after start).
const MAP_FIRST: f64 = 30.0;
/// He considers taking the map out this often (s), with a curiosity-driven chance.
const MAP_ROLL: f64 = 6.0;
/// Dark this long before the lantern comes out (s).
const LANTERN_DELAY: f32 = 1.5;
/// Light this long before it goes (s).
const LANTERN_LINGER: f32 = 3.0;
/// Out again at the earliest this long after something else needed his hands (s).
const LANTERN_GAP: f64 = 8.0;
/// Longest wait for the put-away clip to start before simply tucking the map away (s).
const STOW_PATIENCE: f64 = 0.6;
/// Acts skipped while reading the map (he's busy with it); any other act puts it away.
const GLANCES: [&str; 4] = ["look_up", "look_down", "stare_at_user", "gaze_into_screen"];

#[derive(Clone, Debug)]
pub(crate) struct Carrying {
    pub item: Option<Carry>,
    /// The map goes back at this time.
    until: f64,
    map_ready_at: f64,
    next_roll: f64,
    lantern_ready_at: f64,
    dark_for: f32,
    light_for: f32,
    /// Putting the map away since this time (waiting for its clip to start).
    stowing: Option<f64>,
}

impl Default for Carrying {
    fn default() -> Self {
        Self { item: None, until: 0.0, map_ready_at: MAP_FIRST, next_roll: 0.0, lantern_ready_at: 0.0, dark_for: 0.0, light_for: 0.0, stowing: None }
    }
}

/// Behaviours he can do with something in his hands.
fn hands_free(kind: BehaviourKind) -> bool {
    matches!(kind, BehaviourKind::Idle | BehaviourKind::Companion | BehaviourKind::Explore)
}

impl Brain {
    /// What he carries this tick (after the behaviour ran): take the map / lantern out, keep
    /// it, or put it away when something needs his hands. Sets `out.carry`.
    pub(crate) fn carry_tick(&mut self, p: &Percepts, out: &mut Intent) {
        let now = self.now;
        let dt = p.dt.max(0.0);
        let kind = self.active.kind();
        let Brain { carry: c, senses: s, rep, rng, mind, .. } = self;
        if s.dark {
            c.dark_for += dt;
            c.light_for = 0.0;
        } else {
            c.light_for += dt;
            c.dark_for = 0.0;
        }
        let name_of = |id| rep.catalog.get(id).map(|a| a.name.as_str());
        let playing = p.act.and_then(|a| name_of(a.id));
        let grounded = p.knight.is_grounded();
        let friendly = hands_free(kind);
        let urgent = kind.tier() == Tier::Urgent || s.held;

        // Taking the map out (asked for by him below, or by a behaviour: exploring).
        if playing == Some(TAKE_OUT_MAP) && c.item != Some(Carry::Map) {
            c.item = Some(Carry::Map);
            c.until = now + rng.range(MAP_SECS[0], MAP_SECS[1]) as f64;
            c.stowing = None;
        }

        match c.item {
            Some(Carry::Map) => {
                let other_act = playing.is_some_and(|n| n != TAKE_OUT_MAP && n != PUT_MAP_AWAY);
                if playing == Some(PUT_MAP_AWAY) || urgent || !grounded || other_act {
                    // Put away (its clip under way), or no time / no way for the clip.
                    c.item = None;
                    c.stowing = None;
                    c.map_ready_at = now + rng.range(MAP_GAP[0], MAP_GAP[1]) as f64;
                } else {
                    let wants = out.act.and_then(name_of);
                    let glance = wants.is_some_and(|n| GLANCES.contains(&n));
                    if glance {
                        out.act = None; // reading his map instead
                    }
                    let busy = wants.is_some_and(|n| !glance && n != TAKE_OUT_MAP);
                    let hurry = match out.motion {
                        Motion::Navigate { style, .. } => style != TravelStyle::Stroll,
                        Motion::Direct(_) => true,
                        Motion::Hold => false,
                    };
                    if c.stowing.is_none() && (!friendly || busy || hurry || now >= c.until) {
                        c.stowing = Some(now);
                    }
                    if let Some(since) = c.stowing {
                        if now - since > STOW_PATIENCE {
                            c.item = None;
                            c.stowing = None;
                            c.map_ready_at = now + rng.range(MAP_GAP[0], MAP_GAP[1]) as f64;
                        } else if playing.is_none() {
                            // Fold it first, then go on (the behaviour waits for the clip).
                            out.act = rep.catalog.id(PUT_MAP_AWAY);
                            out.motion = Motion::Hold;
                            out.cancel_act = false;
                            out.exit_act = None;
                        }
                    }
                }
            }
            Some(Carry::Lantern) => {
                if c.light_for >= LANTERN_LINGER || !friendly || urgent {
                    c.item = None;
                    if !friendly || urgent {
                        c.lantern_ready_at = now + LANTERN_GAP;
                    }
                }
            }
            None => {
                let calm = friendly && !urgent && grounded && playing.is_none() && out.act.is_none();
                if calm && s.dark && c.dark_for >= LANTERN_DELAY && now >= c.lantern_ready_at {
                    c.item = Some(Carry::Lantern);
                } else if calm && !s.dark && matches!(out.motion, Motion::Hold) && now >= c.map_ready_at && now >= c.next_roll {
                    c.next_roll = now + MAP_ROLL;
                    let curious = mind.personality.curiosity;
                    if rng.chance(0.2 + 0.2 * curious) {
                        out.act = rep.catalog.id(TAKE_OUT_MAP);
                    }
                }
            }
        }
        out.carry = c.item;
    }
}
