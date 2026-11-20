//! The user's hand on the props and visitors (rules in `skong_core::drag`): press on the
//! training dummy, a visitor, a Geo deposit or a bench (placed or found) and move, and it's
//! picked up; it follows the cursor (at the grab offset) and, let go, falls onto the first
//! good enough ledge below. A plain click does nothing. Only their opaque pixels catch the
//! mouse (layered windows hit-test per pixel); the cursor is only ever read, never moved.
//!
//! Consequences, once it lands: the dummy stays put (pinned, never moved away by itself)
//! and the brain judges it afresh; a visitor goes on with what they were doing, or takes
//! their leave (`Npc::grab`); a find moves to the window it landed on (or lies on the
//! floor), mining progress kept, saved; a bench rides its new window. A knight sitting on
//! a bench that is picked up gets up first; one watching a visitor lifted startles ("!").
use skong_core::acts::Requirement;
use skong_core::ai::Poke;
use skong_core::achievements::AchievementEvent as E;
use skong_core::controller::Side;
use skong_core::discovery::{FindId, World};
use skong_core::drag::{DragKind, DropSite, Fall, Landing, sitting_on};
use skong_core::geom::Vec2;
use skong_core::terrain::{Ledge, LedgeKind};
use skong_win::input;
use skong_win::sprite_window::MouseEvent;

use super::Pet;
use crate::emotes::Emote;
use crate::grab::Grab;

/// What the hand is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Dummy(u64),
    Visitor(u32),
    /// A find: a deposit, or a bench (`true`).
    Find(FindId, bool),
    /// A placed bench (its index).
    Bench(usize),
}

impl Target {
    fn kind(self) -> DragKind {
        match self {
            Target::Dummy(_) => DragKind::Dummy,
            Target::Visitor(_) => DragKind::Visitor,
            Target::Find(_, false) => DragKind::Deposit,
            Target::Find(_, true) => DragKind::FoundBench,
            Target::Bench(_) => DragKind::Bench,
        }
    }

    /// Names this one thing (for "five different things").
    fn key(self) -> String {
        match self {
            Target::Dummy(id) => format!("dummy#{id}"),
            Target::Visitor(id) => format!("visitor#{id}"),
            Target::Find(id, _) => format!("find#{id}"),
            Target::Bench(i) => format!("bench#{i}"),
        }
    }
}

/// The hand: the press, what it holds, what is falling from it.
#[derive(Default)]
pub(super) struct Hand {
    grab: Grab,
    pressed: Option<Target>,
    held: Option<Target>,
    falls: Vec<(Target, Fall)>,
}

impl Pet {
    /// Mouse on the props' and visitors' windows, the carried thing, falling things. Runs
    /// at the start of every tick (the props draw themselves later in the tick).
    pub(super) fn drag_tick(&mut self, dt: f32) {
        let mut events: Vec<(Target, MouseEvent)> = self.pastimes.take_mouse().into_iter().map(|(id, e)| (Target::Dummy(id), e)).collect();
        if let Some(v) = &self.visitors {
            events.extend(v.take_mouse().into_iter().map(|(id, e)| (Target::Visitor(id), e)));
        }
        events.extend(self.discoveries.take_mouse().into_iter().map(|(id, bench, e)| (Target::Find(id, bench), e)));
        events.extend(self.benches.take_mouse().into_iter().map(|(i, e)| (Target::Bench(i), e)));
        for (t, ev) in events {
            self.hand_event(t, ev);
        }
        if let Some(t) = self.hand.held {
            match (self.drag_foot(t), self.hand.grab.held_pos()) {
                // The button came up somewhere we didn't hear of (capture lost): let go.
                (Some(_), Some(p)) if !input::mouse_buttons().0 => {
                    self.hold_target(t, p);
                    self.hand = Hand { falls: std::mem::take(&mut self.hand.falls), ..Hand::default() };
                    self.let_go(t, p, Vec2::ZERO);
                }
                (Some(_), Some(p)) => self.hold_target(t, p),
                // Gone from under the hand (a visitor left, the dummy faded, benches cleared).
                _ => {
                    self.hand = Hand { falls: std::mem::take(&mut self.hand.falls), ..Hand::default() };
                }
            }
        }
        self.step_falls(dt);
    }

    fn hand_event(&mut self, t: Target, ev: MouseEvent) {
        if let MouseEvent::Down { .. } = ev {
            if self.hand.held.is_some() || self.hand.falls.iter().any(|(f, _)| *f == t) {
                return; // one thing at a time; a falling one lands first
            }
            let Some(foot) = self.drag_foot(t) else { return };
            self.hand.pressed = Some(t);
            self.hand.grab.on_event(ev, foot);
            return;
        }
        if self.hand.pressed != Some(t) {
            return;
        }
        let foot = self.drag_foot(t).unwrap_or_default();
        match self.hand.grab.on_event(ev, foot) {
            Some(Poke::Grab) => {
                if self.pick_up(t) {
                    self.hand.held = Some(t);
                } else {
                    self.hand = Hand { falls: std::mem::take(&mut self.hand.falls), ..Hand::default() };
                }
            }
            Some(Poke::Release { vel }) => {
                self.hand.pressed = None;
                if self.hand.held.take() == Some(t) {
                    let feet = self.drag_foot(t).unwrap_or(foot);
                    self.let_go(t, feet, vel);
                }
            }
            // A plain click: nothing (they aren't the knight).
            Some(Poke::Click { .. }) => self.hand.pressed = None,
            None => {}
        }
    }

    /// Where `t` stands (its foot), while it exists.
    fn drag_foot(&self, t: Target) -> Option<Vec2> {
        match t {
            Target::Dummy(id) => self.pastimes.dummy_at(id),
            Target::Visitor(id) => self.visitors.as_ref().and_then(|v| v.feet(id)),
            Target::Find(id, _) => self.discoveries.foot(id),
            Target::Bench(i) => self.benches.foot(i),
        }
    }

    fn drag_half(&self, t: Target) -> f32 {
        match t {
            Target::Dummy(_) => self.pastimes.dummy_half_width(),
            Target::Visitor(_) => self.visitors.as_ref().map_or(30.0, |v| v.half_width()),
            Target::Find(id, _) => self.discoveries.half_width(id, &self.benches),
            Target::Bench(i) => self.benches.half_width(i),
        }
    }

    /// Lift `t`: the consequences start now. `false`: it can't be picked up after all.
    fn pick_up(&mut self, t: Target) -> bool {
        let Some(foot) = self.drag_foot(t) else { return false };
        let ok = match t {
            Target::Dummy(id) => self.pastimes.pick_up(id),
            Target::Visitor(id) => {
                let mut ok = false;
                self.with_visitors(|v, w, _| ok = v.grab(id, w));
                if ok {
                    // Lifted before his eyes: "!", and he turns to look (when free to).
                    self.emotes.show(Emote::Bang);
                    if self.act.is_none() && self.ch.is_grounded() && (foot.x - self.ch.pos.x).abs() > 4.0 {
                        self.ch.facing = if foot.x > self.ch.pos.x { Side::Right } else { Side::Left };
                    }
                }
                ok
            }
            Target::Find(id, bench) => {
                if bench {
                    self.stand_up_from(foot, self.drag_half(t));
                }
                self.discoveries.pick_up(id)
            }
            Target::Bench(i) => {
                self.stand_up_from(foot, self.drag_half(t));
                self.benches.pick_up(i)
            }
        };
        if ok {
            log::info!("drag: the user picked up the {} at ({:.0},{:.0})", t.kind().label(), foot.x, foot.y);
        }
        ok
    }

    /// The bench at `foot` is being picked up: if he sits on it, he gets up first.
    fn stand_up_from(&mut self, foot: Vec2, half: f32) {
        let d = &self.data;
        let seated = self.seat_y.is_some() && self.act.as_ref().is_some_and(|p| d.acts.get(p.act()).requires == Requirement::Seat);
        if !sitting_on(seated.then_some(self.ch.pos), foot, half) {
            return;
        }
        log::info!("drag: his bench was picked up; he gets up");
        if let Some(p) = self.act.as_mut()
            && !p.wrap_up(&d.acts, &d.lib, &mut self.anim, true)
        {
            self.act = None;
        }
        self.seat_y = None;
    }

    fn hold_target(&mut self, t: Target, foot: Vec2) {
        match t {
            Target::Dummy(_) => self.pastimes.hold(foot),
            Target::Visitor(id) => {
                if let Some(v) = self.visitors.as_mut() {
                    v.hold(id, foot);
                }
            }
            Target::Find(..) => self.discoveries.hold(foot),
            Target::Bench(i) => self.benches.hold(i, foot),
        }
    }

    /// Let go at `feet` with the hand moving at `vel`: it falls (see `skong_core::drag`).
    fn let_go(&mut self, t: Target, feet: Vec2, vel: Vec2) {
        let ppu = self.ch.params.px_per_unit;
        let half = self.drag_half(t);
        let desk = self.watcher.snapshot();
        let (discoveries, terrain) = (&self.discoveries, &self.terrain);
        let settled = |l: &Ledge| discoveries.ledge_settled(terrain, l);
        let accept = |l: &Ledge| accepts(t, l, desk);
        let site = DropSite { terrain, monitors: &desk.monitors, game_monitors: &self.games.game_monitors, half, settled: &settled, accept: &accept };
        match Fall::start(&site, feet, vel, ppu) {
            Some(f) => self.hand.falls.push((t, f)),
            // Games on every monitor (everything is hidden): it stays where it was let go.
            None => self.set_down(t, Landing { at: feet, owner: None, span: (feet.x, feet.x), kind: LedgeKind::Ground }),
        }
    }

    fn step_falls(&mut self, dt: f32) {
        if self.hand.falls.is_empty() {
            return;
        }
        let ppu = self.ch.params.px_per_unit;
        let mut falls = std::mem::take(&mut self.hand.falls);
        let mut landed = Vec::new();
        {
            let desk = self.watcher.snapshot();
            let (discoveries, terrain) = (&self.discoveries, &self.terrain);
            let settled = |l: &Ledge| discoveries.ledge_settled(terrain, l);
            falls.retain_mut(|(t, f)| {
                let t = *t;
                let accept = |l: &Ledge| accepts(t, l, desk);
                let site = DropSite { terrain, monitors: &desk.monitors, game_monitors: &self.games.game_monitors, half: self.drag_half(t), settled: &settled, accept: &accept };
                match f.step(&site, dt, ppu) {
                    Some(l) => {
                        landed.push((t, l));
                        false
                    }
                    None => true,
                }
            });
        }
        for (t, f) in &falls {
            self.hold_target(*t, f.pos);
        }
        self.hand.falls = falls;
        for (t, l) in landed {
            self.set_down(t, l);
        }
    }

    /// `t` came to rest on `l`.
    fn set_down(&mut self, t: Target, l: Landing) {
        let desk = self.watcher.snapshot();
        let on = match l.owner.and_then(|id| desk.window(id)) {
            Some(w) => format!("{}'s {}", w.process, if l.kind == LedgeKind::Content { "content line" } else { "title bar" }),
            None => "the floor".to_string(),
        };
        log::info!("drag: the {} landed at ({:.0},{:.0}) on {on}", t.kind().label(), l.at.x, l.at.y);
        match t {
            Target::Dummy(id) => {
                if self.pastimes.set_down(id, &self.terrain, l.at, l.owner) {
                    self.brain.dummy_moved(id);
                    // The brain's own test, as for the Test tab's placement.
                    if self.dummy_practisable_at(l.at, l.span) {
                        log::info!("training dummy #{id} stays where it was dropped; he'll go and practise there");
                    } else {
                        log::info!("training dummy #{id} stays where it was dropped, but he can't practise there: no stand spot beside it that his routes (a stroll or a run) land on");
                    }
                }
            }
            Target::Visitor(id) => self.with_visitors(|v, w, _| v.set_down(id, l.at, l.owner, w)),
            Target::Find(id, _) => {
                let desk = self.watcher.snapshot();
                match self.discoveries.set_down(id, l.at, l.owner, (desk, &self.terrain)) {
                    Some(room) if l.owner.is_some() => log::info!("drag: find #{id} now belongs to {room}'s windows"),
                    Some(room) => log::info!("drag: find #{id} lies on the floor (still {room}'s)"),
                    None => log::warn!("drag: find #{id} couldn't move there; it goes back"),
                }
            }
            Target::Bench(i) => self.benches.set_down(i, l.at, l.owner),
        }
        self.achieve(E::Dragged { what: t.key() });
        self.win.raise(); // the knight stays in front
    }
}

/// May `t` land on `l`? Finds only on windows that are rooms (or the floor).
fn accepts(t: Target, l: &Ledge, desk: &skong_core::desktop::DesktopSnapshot) -> bool {
    match t {
        Target::Find(..) => l.owner.is_none_or(|id| World::is_room(desk, id)),
        _ => true,
    }
}
