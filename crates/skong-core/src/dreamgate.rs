//! The **Dream Gate**: fallback travel to a prize no route reaches.
//!
//! The brain (`ai::dream_gate`) decides: a prize he really wants (Geo, a deposit, a shiny,
//! a visitor, the Shade) that the planner cannot get him to, standing on a settled ledge
//! with room for him, and the gate is off cooldown. He plants a gate there
//! ([`GateEvent::Set`]: a quiet glow on the target ledge), charges and vanishes
//! (`dreamgate_warp`), and reappears at the gate (`dreamgate_in`).
//!
//! The app owns the timing of the jump itself through [`GateRuntime`]: the moment the warp
//! act ends (after its `DG Warp` step), the knight is moved to the gate and the arrival act
//! starts in the same tick, so no idle frame shows in between. If the gate has stopped
//! being a good spot by then (its window moved away, a game took the monitor), he simply
//! reappears where he stood.
use crate::geom::Vec2;
use crate::terrain::{Ledge, Terrain};

/// The knight's act that charges the warp and vanishes (`DG Warp Charge`, `DG Warp`).
pub const WARP_ACT: &str = "dreamgate_warp";
/// The knight's act that reappears (`DG Warp In`).
pub const ARRIVE_ACT: &str = "dreamgate_in";
/// The gate taking shape where it is planted (Spells `DG Set Charge`, once).
pub const SET_FX: &str = "dg_gate_set";
/// The planted gate, glowing quietly until he warps to it (Spells `DG Set Ready`, looping).
pub const GATE_FX: &str = "dg_gate";

/// Seconds between planting and the looping glow taking over (`DG Set Charge` length).
const SET_SECS: f32 = 0.25;
/// A gate nobody warps to fades after this long (s): the brain gave up without saying so.
const MAX_SECS: f32 = 20.0;
/// Room above the gate, in knight heights.
const HEADROOM: f32 = 1.25;

/// Brain → app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GateEvent {
    /// Plant the gate at `at` (feet point on a ledge).
    Set { at: Vec2 },
    /// Never mind: the gate fades (the prize went away, something urgent came up).
    Close,
}

/// What the app should do (see [`GateRuntime`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GateAction {
    /// Start effect `name` at the feet point.
    Spawn(&'static str, Vec2),
    /// End every instance of effect `name`.
    Stop(&'static str),
    /// Put the knight's feet here and start [`ARRIVE_ACT`].
    WarpTo(Vec2),
    /// The gate is gone: start [`ARRIVE_ACT`] where he stands.
    Reappear,
}

/// Where to plant a gate for a prize at feet level `at`: on the ledge under it, settled,
/// wide enough for the knight (`body` = width, height in px) and with room above him (no
/// wall through him, no other ledge crammed over his head, inside the desktop). The point
/// is nudged along the ledge to keep him fully on it.
pub fn gate_spot(terrain: &Terrain, settled: impl Fn(&Ledge) -> bool, at: Vec2, body: Vec2) -> Option<Vec2> {
    let ledge = terrain
        .ledges
        .iter()
        .filter(|l| (l.y - at.y).abs() <= 3.0 && l.x0 - 1.0 <= at.x && at.x <= l.x1 + 1.0)
        .max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)))?;
    let half = body.x * 0.5 + 2.0;
    if ledge.x1 - ledge.x0 < 2.0 * half || !settled(ledge) {
        return None;
    }
    let x = at.x.clamp(ledge.x0 + half, ledge.x1 - half);
    let (y, top) = (ledge.y, ledge.y - body.y * HEADROOM);
    if top < terrain.bounds.top {
        return None;
    }
    let (x0, x1) = (x - half, x + half);
    let wall = terrain.walls.iter().any(|w| w.x > x0 && w.x < x1 && w.y0 < y - 1.0 && w.y1 > top);
    let cramped = terrain.ledges.iter().any(|l| l.y > top && l.y < y - 2.0 && l.x0 < x1 && l.x1 > x0);
    (!wall && !cramped).then_some(Vec2::new(x, y))
}

#[derive(Clone, Copy, Debug)]
struct Gate {
    at: Vec2,
    age: f32,
    glowing: bool,
}

/// The app's side of a gate: its glow and the jump at the end of the warp act.
#[derive(Clone, Debug, Default)]
pub struct GateRuntime {
    gate: Option<Gate>,
    /// The warp act reached its vanishing step.
    vanishing: bool,
}

impl GateRuntime {
    /// Where the planted gate is, if there is one.
    pub fn at(&self) -> Option<Vec2> {
        self.gate.map(|g| g.at)
    }

    /// Mid-warp: the knight is fading out and will reappear when the act ends.
    pub fn vanishing(&self) -> bool {
        self.vanishing
    }

    fn close(&mut self, out: &mut Vec<GateAction>) {
        if self.gate.take().is_some() {
            out.push(GateAction::Stop(SET_FX));
            out.push(GateAction::Stop(GATE_FX));
        }
    }

    /// A brain event.
    pub fn event(&mut self, ev: GateEvent, out: &mut Vec<GateAction>) {
        self.close(out);
        if let GateEvent::Set { at } = ev {
            self.gate = Some(Gate { at, age: 0.0, glowing: false });
            out.push(GateAction::Spawn(SET_FX, at));
        }
    }

    /// Once per tick, after the act player: `act` is the running act's name and step,
    /// `valid` whether a point is still a good gate (on a settled ledge, not on a game
    /// monitor).
    pub fn tick(&mut self, dt: f32, act: Option<(&str, usize)>, valid: impl Fn(Vec2) -> bool, out: &mut Vec<GateAction>) {
        if let Some(g) = &mut self.gate {
            g.age += dt;
            if !g.glowing && g.age >= SET_SECS {
                g.glowing = true;
                out.push(GateAction::Spawn(GATE_FX, g.at));
            }
            if g.age > MAX_SECS && !self.vanishing {
                self.close(out);
            }
        }
        match act {
            Some((WARP_ACT, step)) => {
                // Step 0 charges (`DG Warp Charge`), step 1 vanishes (`DG Warp`).
                self.vanishing |= step >= 1;
                return;
            }
            _ if !self.vanishing => return,
            _ => {}
        }
        // The warp act just ended (or was cut short while he was fading out).
        self.vanishing = false;
        let to = self.gate.map(|g| g.at).filter(|&at| valid(at));
        self.close(out);
        out.push(match to {
            Some(at) => GateAction::WarpTo(at),
            None => GateAction::Reappear,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
    use crate::geom::RectI;
    use crate::terrain::{LedgeKind, TerrainOptions, Wall, WallKind};

    const BODY: Vec2 = Vec2::new(40.0, 80.0);

    fn terrain() -> Terrain {
        let w = WindowInfo {
            id: 1,
            rect: RectI::new(200, 400, 900, 900),
            title: "w".into(),
            class: "Test".into(),
            process: "editor.exe".into(),
            pid: 1,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        };
        let desk = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows: vec![w],
        };
        Terrain::from_desktop(&desk, &TerrainOptions::default())
    }

    #[test]
    fn a_gate_stands_on_a_settled_ledge_with_room_above() {
        let t = terrain();
        let all = |_: &Ledge| true;
        assert_eq!(gate_spot(&t, all, Vec2::new(500.0, 400.0), BODY), Some(Vec2::new(500.0, 400.0)), "the title bar");
        assert_eq!(gate_spot(&t, all, Vec2::new(1500.0, 1040.0), BODY), Some(Vec2::new(1500.0, 1040.0)), "the floor");
        // Near the end of a ledge: nudged on so he stands on it entirely.
        assert_eq!(gate_spot(&t, all, Vec2::new(201.0, 400.0), BODY), Some(Vec2::new(222.0, 400.0)));
        assert_eq!(gate_spot(&t, all, Vec2::new(500.0, 600.0), BODY), None, "mid-air");
        // A ledge that is still settling (a window being dragged, content scrolling past).
        let floor_only = |l: &Ledge| l.owner.is_none();
        assert_eq!(gate_spot(&t, floor_only, Vec2::new(500.0, 400.0), BODY), None);
        assert!(gate_spot(&t, floor_only, Vec2::new(1500.0, 1040.0), BODY).is_some());
    }

    #[test]
    fn no_gate_without_headroom() {
        let mut t = terrain();
        let all = |_: &Ledge| true;
        // A shelf right above his head.
        t.ledges.push(Ledge { x0: 450.0, x1: 600.0, y: 360.0, kind: LedgeKind::Content, owner: Some(1) });
        assert_eq!(gate_spot(&t, all, Vec2::new(500.0, 400.0), BODY), None);
        assert!(gate_spot(&t, all, Vec2::new(700.0, 400.0), BODY).is_some(), "clear of the shelf");
        // A wall through him.
        t.walls.push(Wall { x: 705.0, y0: 300.0, y1: 400.0, face: crate::terrain::Facing::Left, owner: Some(1), kind: WallKind::Content });
        assert_eq!(gate_spot(&t, all, Vec2::new(700.0, 400.0), BODY), None);
        // Too close to the top of the desktop.
        let mut high = terrain();
        high.ledges.push(Ledge { x0: 1000.0, x1: 1200.0, y: 40.0, kind: LedgeKind::OneWay, owner: None });
        assert_eq!(gate_spot(&high, all, Vec2::new(1100.0, 40.0), BODY), None);
    }

    fn drive(rt: &mut GateRuntime, dt: f32, act: Option<(&str, usize)>, valid: bool) -> Vec<GateAction> {
        let mut out = Vec::new();
        rt.tick(dt, act, |_| valid, &mut out);
        out
    }

    #[test]
    fn the_warp_sequence_plants_glows_and_jumps_when_the_warp_act_ends() {
        let mut rt = GateRuntime::default();
        let at = Vec2::new(500.0, 400.0);
        let mut out = Vec::new();
        rt.event(GateEvent::Set { at }, &mut out);
        assert_eq!(out, vec![GateAction::Spawn(SET_FX, at)], "the gate takes shape");
        // The plant act plays; the gate settles into its quiet glow.
        assert!(drive(&mut rt, 0.1, Some(("dreamgate_set", 0)), true).is_empty());
        assert_eq!(drive(&mut rt, 0.2, Some(("dreamgate_set", 1)), true), vec![GateAction::Spawn(GATE_FX, at)]);
        assert!(drive(&mut rt, 0.1, None, true).is_empty(), "between acts nothing happens");
        // Charging, then fading out: still here.
        assert!(drive(&mut rt, 0.5, Some((WARP_ACT, 0)), true).is_empty());
        assert!(!rt.vanishing());
        assert!(drive(&mut rt, 0.1, Some((WARP_ACT, 1)), true).is_empty());
        assert!(rt.vanishing());
        // The act ends: the gate is used up and he is moved there in the same tick.
        assert_eq!(drive(&mut rt, 0.01, None, true), vec![GateAction::Stop(SET_FX), GateAction::Stop(GATE_FX), GateAction::WarpTo(at)]);
        assert_eq!(rt.at(), None);
        assert!(drive(&mut rt, 0.1, Some((ARRIVE_ACT, 0)), true).is_empty());
    }

    #[test]
    fn a_gate_gone_bad_means_reappearing_in_place() {
        let mut rt = GateRuntime::default();
        let mut out = Vec::new();
        rt.event(GateEvent::Set { at: Vec2::new(500.0, 400.0) }, &mut out);
        drive(&mut rt, 0.5, Some((WARP_ACT, 1)), true);
        // The window under the gate was dragged away meanwhile.
        let out = drive(&mut rt, 0.01, None, false);
        assert_eq!(out.last(), Some(&GateAction::Reappear));
        assert_eq!(rt.at(), None);
    }

    #[test]
    fn a_charge_cut_short_or_a_close_never_jumps() {
        let mut rt = GateRuntime::default();
        let at = Vec2::new(500.0, 400.0);
        let mut out = Vec::new();
        rt.event(GateEvent::Set { at }, &mut out);
        // Grabbed while charging: the act is gone before the vanishing step.
        drive(&mut rt, 0.3, Some((WARP_ACT, 0)), true);
        assert!(!drive(&mut rt, 0.01, None, true).iter().any(|a| matches!(a, GateAction::WarpTo(_) | GateAction::Reappear)));
        assert_eq!(rt.at(), Some(at), "the gate waits for the brain");
        out.clear();
        rt.event(GateEvent::Close, &mut out);
        assert_eq!(out, vec![GateAction::Stop(SET_FX), GateAction::Stop(GATE_FX)]);
        assert_eq!(rt.at(), None);
        // Closing again is harmless.
        out.clear();
        rt.event(GateEvent::Close, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_forgotten_gate_fades_on_its_own() {
        let mut rt = GateRuntime::default();
        let mut out = Vec::new();
        rt.event(GateEvent::Set { at: Vec2::new(500.0, 400.0) }, &mut out);
        for _ in 0..(MAX_SECS as usize + 1) {
            drive(&mut rt, 1.0, None, true);
        }
        assert_eq!(rt.at(), None);
    }

    #[test]
    fn a_new_gate_replaces_the_old_one() {
        let mut rt = GateRuntime::default();
        let mut out = Vec::new();
        rt.event(GateEvent::Set { at: Vec2::new(500.0, 400.0) }, &mut out);
        out.clear();
        let b = Vec2::new(1500.0, 1040.0);
        rt.event(GateEvent::Set { at: b }, &mut out);
        assert_eq!(out, vec![GateAction::Stop(SET_FX), GateAction::Stop(GATE_FX), GateAction::Spawn(SET_FX, b)]);
        assert_eq!(rt.at(), Some(b));
    }
}
