//! Perception: what the app tells the brain each tick ([`Percepts`]) and what the brain
//! derives from successive ticks ([`Senses`]: desktop diffs, cursor velocity, user
//! presence, ...). Platform-neutral: the app fills [`Percepts`] from whatever OS layer.
use std::collections::BTreeMap;

use crate::acts::ActId;
use crate::controller::{Character, MoveEvent, MoveInput};
use crate::desktop::{DesktopSnapshot, WindowId};
use crate::geom::{RectI, Vec2};
use crate::terrain::Terrain;

use super::activity::Activity;
use super::config::BrainConfig;

/// Everything the brain perceives this tick. Built by the app; borrowed, cheap.
#[derive(Clone, Copy, Debug)]
pub struct Percepts<'a> {
    /// Seconds since the previous tick (> 0).
    pub dt: f32,
    pub knight: &'a Character,
    pub terrain: &'a Terrain,
    /// Latest desktop snapshot. Diffs are computed by the brain whenever
    /// `generation` changes, so pass the same snapshot every tick until it changes.
    pub desktop: &'a DesktopSnapshot,
    /// `None` when the cursor position is unknown (e.g. secure desktop).
    pub cursor: Option<Cursor>,
    pub user: UserActivity,
    /// A fullscreen app (game, video, presentation) is in front. The brain also
    /// derives this from the snapshot; set it when the platform knows better.
    pub fullscreen_front: bool,
    /// User interactions with the knight since the previous tick.
    pub pokes: &'a [Poke],
    /// The act currently playing, if any.
    pub act: Option<RunningAct>,
    /// Placed benches (feet position of the sitting spot). Bench acts (sitting clips)
    /// are only picked while the knight stands at one.
    pub benches: &'a [Vec2],
    /// The knight can perch (sit with dangling legs) on the ledge he stands on: a window or
    /// content edge with room below. Seated acts then work without a bench.
    pub perch: bool,
    /// Controller events since the previous brain tick (incl. `on_terrain_rebuilt`).
    pub move_events: &'a [MoveEvent],
    /// Where things are changing on screen (vision). Pass an empty one when unknown.
    pub activity: &'a Activity,
    /// The input actually applied on the previous physics step (after acts zeroed it).
    /// Lets the brain notice "pressing but not moving" (walking into a wall).
    pub last_input: MoveInput,
    /// Local time of day in hours (0..24), if known. Night = lantern time.
    pub local_hour: Option<f32>,
    /// The knight's surroundings are dark (dark-themed window behind him, dimmed screen).
    pub dark: bool,
    /// Where the user's highlighted text is on screen (opt-in; the app sends `None` while
    /// the feature is off). Only the rectangle: the brain never sees the text.
    pub selection: Option<RectI>,
}

/// Cursor state in virtual-desktop pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cursor {
    pub pos: Vec2,
    pub left: bool,
    pub right: bool,
}

/// Global user input activity (not limited to the knight).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UserActivity {
    /// Seconds since the last keyboard or mouse input anywhere.
    pub idle_secs: f32,
    /// Recent key-press rate (keys/s, e.g. averaged over the last ~3 s).
    pub keys_per_sec: f32,
}

/// Direct interaction with the knight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Poke {
    /// Clicked on the knight (during a battle: the cursor lands a hit).
    Click { pos: Vec2 },
    /// Picked up / being dragged; physics is the app's business until `Release`.
    Grab,
    /// Let go (thrown) with this velocity (px/s).
    Release { vel: Vec2 },
}

/// The act the app is currently playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningAct {
    pub id: ActId,
    /// The current step may be cut short.
    pub interruptible: bool,
}

/// A window that disappeared since the previous snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClosedWindow {
    pub id: WindowId,
    pub rect: RectI,
}

/// Desktop changes observed this tick (empty unless the generation changed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesktopChanges {
    pub opened: Vec<WindowId>,
    pub closed: Vec<ClosedWindow>,
    /// Windows whose rect moved, with the top-left delta.
    pub moved: Vec<(WindowId, Vec2)>,
    /// New foreground window, if focus changed.
    pub focused: Option<WindowId>,
}

impl DesktopChanges {
    pub fn is_empty(&self) -> bool {
        self.opened.is_empty() && self.closed.is_empty() && self.moved.is_empty() && self.focused.is_none()
    }
}

/// Diffs successive snapshots. The first snapshot is a baseline (nothing "opened").
#[derive(Clone, Debug, Default)]
pub(crate) struct DesktopTracker {
    generation: Option<u64>,
    prev: BTreeMap<WindowId, RectI>,
    foreground: Option<WindowId>,
}

impl DesktopTracker {
    pub fn update(&mut self, desk: &DesktopSnapshot) -> DesktopChanges {
        let mut ch = DesktopChanges::default();
        if self.generation == Some(desk.generation) {
            return ch;
        }
        let baseline = self.generation.is_none();
        let now: BTreeMap<WindowId, RectI> = desk.windows.iter().map(|w| (w.id, w.rect)).collect();
        let fg = desk.windows.iter().find(|w| w.foreground).map(|w| w.id);
        if !baseline {
            for (&id, rect) in &now {
                match self.prev.get(&id) {
                    None => ch.opened.push(id),
                    Some(old) if old != rect => {
                        let d = Vec2::new((rect.left - old.left) as f32, (rect.top - old.top) as f32);
                        if d != Vec2::ZERO {
                            ch.moved.push((id, d));
                        }
                    }
                    Some(_) => {}
                }
            }
            for (&id, &rect) in &self.prev {
                if !now.contains_key(&id) {
                    ch.closed.push(ClosedWindow { id, rect });
                }
            }
            if fg != self.foreground {
                ch.focused = fg;
            }
        }
        self.generation = Some(desk.generation);
        self.prev = now;
        self.foreground = fg;
        ch
    }
}

/// Derived cursor kinematics relative to the knight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CursorSense {
    pub pos: Vec2,
    /// Smoothed velocity (px/s).
    pub vel: Vec2,
    /// Offset from the knight's body center to the cursor.
    pub offset: Vec2,
    /// Seconds the cursor has been (nearly) still.
    pub still_for: f32,
    /// Seconds the cursor has been near the knight.
    pub near_for: f32,
    /// Seconds the cursor has been far (beyond `give_up_distance`).
    pub far_for: f32,
}

impl CursorSense {
    pub fn speed(&self) -> f32 {
        self.vel.length()
    }
    pub fn dist(&self) -> f32 {
        self.offset.length()
    }
}

/// Horizontal distance from a bench that counts as "at" it (px).
pub const BENCH_REACH: f32 = 6.0;

/// Pressing a direction while grounded but not moving for this long = stuck (s).
pub const STUCK_SECS: f32 = 0.25;

/// What the brain knows beyond the raw percepts.
#[derive(Clone, Debug, Default)]
pub struct Senses {
    desktop: DesktopTracker,
    /// Changes seen this tick.
    pub changes: DesktopChanges,
    pub cursor: Option<CursorSense>,
    /// Brain clock time of the last `PlatformMoved` event.
    pub carried_at: Option<f32>,
    /// Window the knight stood on last tick.
    pub support_owner: Option<WindowId>,
    /// The window the knight stood on just closed under it (this tick).
    pub support_closed: Option<ClosedWindow>,
    /// Being held by the user.
    pub held: bool,
    pub fullscreen: bool,
    /// User is typing fast.
    pub user_busy: bool,
    /// User idle longer than `user_away_secs`.
    pub user_away: bool,
    /// User became active after being away (this tick).
    pub user_returned: bool,
    /// The bench the knight is standing at, if any.
    pub at_bench: Option<Vec2>,
    /// Can perch on the current ledge (from [`Percepts::perch`]).
    pub perch: bool,
    /// Something woke the knight this tick (poke, user returned). Set by reflexes.
    pub woken: bool,
    /// Seconds spent pressing a direction while grounded without moving.
    pub push_for: f32,
    /// Became stuck this tick (`push_for` crossed [`STUCK_SECS`]).
    pub stuck: bool,
    /// Started sliding down a wall this tick.
    pub wall_slide: bool,
    /// Night (local time) or dark surroundings.
    pub dark: bool,
    /// Brain clock time the user last came back after being away.
    pub returned_at: Option<f32>,
    /// NPC visitors on screen (from [`super::Brain::see_visitors`]).
    pub visitors: Vec<super::Visitor>,
}

impl Senses {
    pub(crate) fn update(&mut self, p: &Percepts, now: f32, cfg: &BrainConfig) {
        self.changes = self.desktop.update(p.desktop);
        let center = p.knight.body().center();

        self.cursor = p.cursor.map(|c| {
            let prev = self.cursor.unwrap_or(CursorSense { pos: c.pos, ..Default::default() });
            let inst = (c.pos - prev.pos) / p.dt.max(1e-4);
            let vel = prev.vel.lerp(inst, 0.35);
            let offset = c.pos - center;
            let b = &cfg.battle;
            let tick = |on: bool, t: f32| if on { t + p.dt } else { 0.0 };
            CursorSense {
                pos: c.pos,
                vel,
                offset,
                still_for: tick(vel.length() < b.park_speed, prev.still_for),
                near_for: tick(offset.length() < b.cursor_near, prev.near_for),
                far_for: tick(offset.length() > b.give_up_distance, prev.far_for),
            }
        });

        if p.move_events.iter().any(|e| matches!(e, MoveEvent::PlatformMoved { .. })) {
            self.carried_at = Some(now);
        }
        self.support_closed = self.support_owner.and_then(|owner| self.changes.closed.iter().find(|c| c.id == owner).copied());
        self.support_owner = p.knight.support.and_then(|s| s.owner);

        for poke in p.pokes {
            match poke {
                Poke::Grab => self.held = true,
                Poke::Release { .. } => self.held = false,
                Poke::Click { .. } => {}
            }
        }

        let derived_fullscreen = cfg.hide_for_fullscreen
            && (p.desktop.windows.first().is_some_and(|w| w.fullscreen) || p.desktop.windows.iter().any(|w| w.fullscreen && w.foreground));
        self.fullscreen = p.fullscreen_front || derived_fullscreen;
        self.user_busy = p.user.keys_per_sec >= cfg.typing_busy_rate;
        let away = p.user.idle_secs >= cfg.user_away_secs;
        self.user_returned = self.user_away && !away;
        self.user_away = away;
        if self.user_returned {
            self.returned_at = Some(now);
        }
        let k = p.knight;
        self.at_bench = p
            .benches
            .iter()
            .copied()
            .find(|b| k.is_grounded() && (b.x - k.pos.x).abs() <= BENCH_REACH && (b.y - k.pos.y).abs() <= 8.0);
        self.perch = p.perch && k.is_grounded();
        self.woken = false;
        // Charging a crystal dash holds a direction without moving for 0.8 s: not stuck (the
        // brain used to call that a wall and cancel the trip mid-charge).
        let charging = p.last_input.superdash_held || matches!(k.state, crate::controller::MoveState::SuperdashCharge { .. });
        let pressing = k.is_grounded() && p.act.is_none() && !charging && p.last_input.x.abs() > 0.1 && k.vel.x.abs() < 8.0;
        let before = self.push_for;
        self.push_for = if pressing { self.push_for + p.dt } else { 0.0 };
        self.stuck = before < STUCK_SECS && self.push_for >= STUCK_SECS;
        self.wall_slide = p.move_events.iter().any(|e| matches!(e, MoveEvent::WallSlideStart { .. }));
        let night = p.local_hour.is_some_and(|h| !(6.5..20.5).contains(&h));
        self.dark = p.dark || night;
    }

    /// A seat is available right here: at a bench, or perched on a ledge edge.
    pub fn can_sit(&self) -> bool {
        self.at_bench.is_some() || self.perch
    }

    /// Seconds since the window under the knight last moved (he fell off / it slid away).
    pub fn since_carried(&self, now: f32) -> f32 {
        self.carried_at.map_or(f32::INFINITY, |t| now - t)
    }
}
