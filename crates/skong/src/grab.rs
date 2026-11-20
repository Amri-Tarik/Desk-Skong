//! Turning mouse events on the knight's window into pokes: a click, or press-drag-release
//! to pick him up and throw him with the cursor's recent velocity.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use skong_core::ai::Poke;
use skong_core::drag::is_drag;
use skong_core::geom::Vec2;
use skong_win::sprite_window::MouseEvent;

/// Velocity is measured over this much recent cursor history.
const THROW_WINDOW: Duration = Duration::from_millis(90);
const MAX_THROW: f32 = 4000.0;

#[derive(Default)]
pub struct Grab {
    /// Cursor position at button-down, and the knight's feet offset from it.
    press: Option<(Vec2, Vec2)>,
    held: bool,
    cursor: Vec2,
    history: VecDeque<(Instant, Vec2)>,
}

impl Grab {
    /// Feed one event; `feet` is the knight's current feet position.
    pub fn on_event(&mut self, ev: MouseEvent, feet: Vec2) -> Option<Poke> {
        let now = Instant::now();
        match ev {
            MouseEvent::Down { x, y } => {
                let p = Vec2::new(x as f32, y as f32);
                self.press = Some((p, feet - p));
                self.cursor = p;
                self.history.clear();
                self.history.push_back((now, p));
                None
            }
            MouseEvent::Move { x, y } => {
                let p = Vec2::new(x as f32, y as f32);
                self.cursor = p;
                self.history.push_back((now, p));
                while self.history.front().is_some_and(|(t, _)| now.duration_since(*t) > THROW_WINDOW) {
                    self.history.pop_front();
                }
                match self.press {
                    // Past the drag threshold (`skong_core::drag::DRAG_START`): picked up.
                    Some((start, _)) if !self.held && is_drag(start, p) => {
                        self.held = true;
                        Some(Poke::Grab)
                    }
                    _ => None,
                }
            }
            MouseEvent::Up { x, y } => {
                let p = Vec2::new(x as f32, y as f32);
                let was_held = std::mem::take(&mut self.held);
                let press = self.press.take();
                if was_held {
                    Some(Poke::Release { vel: self.velocity(now, p) })
                } else {
                    press.map(|_| Poke::Click { pos: p })
                }
            }
            MouseEvent::RightClick { .. } => None,
        }
    }

    /// Where the knight's feet should be while held.
    pub fn held_pos(&self) -> Option<Vec2> {
        let (_, offset) = self.press?;
        self.held.then_some(self.cursor + offset)
    }

    fn velocity(&self, now: Instant, p: Vec2) -> Vec2 {
        let Some(&(t0, p0)) = self.history.front() else { return Vec2::ZERO };
        let dt = now.duration_since(t0).as_secs_f32();
        if dt < 0.005 {
            return Vec2::ZERO;
        }
        let v = (p - p0) / dt;
        if v.length() > MAX_THROW { v.normalize() * MAX_THROW } else { v }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_press_is_a_click() {
        let mut g = Grab::default();
        let feet = Vec2::new(100.0, 200.0);
        assert_eq!(g.on_event(MouseEvent::Down { x: 100, y: 150 }, feet), None);
        assert_eq!(g.on_event(MouseEvent::Move { x: 103, y: 151 }, feet), None);
        assert_eq!(g.on_event(MouseEvent::Up { x: 103, y: 151 }, feet), Some(Poke::Click { pos: Vec2::new(103.0, 151.0) }));
        assert_eq!(g.held_pos(), None);
    }

    #[test]
    fn drag_holds_at_grab_offset_then_throws() {
        let mut g = Grab::default();
        let feet = Vec2::new(100.0, 200.0);
        g.on_event(MouseEvent::Down { x: 100, y: 150 }, feet);
        assert_eq!(g.on_event(MouseEvent::Move { x: 120, y: 150 }, feet), Some(Poke::Grab));
        // Feet stay 50 px below the grab point.
        assert_eq!(g.held_pos(), Some(Vec2::new(120.0, 200.0)));
        std::thread::sleep(Duration::from_millis(20));
        g.on_event(MouseEvent::Move { x: 160, y: 140 }, feet);
        match g.on_event(MouseEvent::Up { x: 160, y: 140 }, feet) {
            Some(Poke::Release { vel }) => assert!(vel.x > 100.0 && vel.y < 0.0, "{vel}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(g.held_pos(), None);
    }
}
