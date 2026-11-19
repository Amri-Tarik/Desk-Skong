//! Keep the knight out of games. A game running borderless/exclusive fullscreen on a
//! monitor removes that monitor from his world: he warps to a free monitor and stays
//! there (the seam becomes a wall); if every monitor has a game, he disappears entirely
//! (no drawing, no simulation, no screen capture) until one is free again.
//! Other fullscreen apps (F11 browsers, videos, presentations) are left alone.
use std::time::{Duration, Instant};

use skong_core::desktop::DesktopSnapshot;
use skong_core::geom::{RectI, Vec2};
use skong_core::terrain::{Facing, Terrain, Wall, WallKind};
use skong_win::games::{GameDetector, exclusive_fullscreen};

/// Exclusive fullscreen isn't visible in the window list; poll it at this rate.
const EXCLUSIVE_POLL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct GameGuard {
    det: Option<GameDetector>,
    seen_generation: u64,
    last_poll: Option<Instant>,
    exclusive: bool,
    /// Full rects of monitors a game currently occupies.
    pub game_monitors: Vec<RectI>,
}

impl GameGuard {
    pub fn new() -> Self {
        Self { det: Some(GameDetector::new()), ..Self::default() }
    }

    /// Re-evaluate when the desktop changed (or periodically for exclusive fullscreen).
    /// Returns true if the set of game monitors changed.
    pub fn update(&mut self, desk: &DesktopSnapshot) -> bool {
        let poll_due = self.last_poll.is_none_or(|t| t.elapsed() >= EXCLUSIVE_POLL);
        if desk.generation == self.seen_generation && !poll_due {
            return false;
        }
        self.seen_generation = desk.generation;
        if poll_due {
            self.last_poll = Some(Instant::now());
            self.exclusive = exclusive_fullscreen();
        }
        let mut games: Vec<RectI> = Vec::new();
        let mut found: Vec<String> = Vec::new();
        if self.exclusive {
            // The display belongs to the game; treat every monitor as taken.
            games.extend(desk.monitors.iter().map(|m| m.rect));
        } else if let Some(det) = self.det.as_mut() {
            for w in desk.windows.iter().filter(|w| w.fullscreen) {
                if det.is_game(w.pid)
                    && let Some(m) = desk.monitors.iter().find(|m| m.rect.contains((w.rect.left + w.rect.right) / 2, (w.rect.top + w.rect.bottom) / 2))
                    && !games.contains(&m.rect)
                {
                    found.push(format!("game detected on monitor {:?}: {} ({})", m.rect, w.title, w.process));
                    games.push(m.rect);
                }
            }
        }
        let changed = games != self.game_monitors;
        // Only when the picture changes (this re-runs every second while a game is up).
        if changed {
            for line in &found {
                log::info!("{line}");
            }
        }
        self.game_monitors = games;
        changed
    }

    pub fn is_game_monitor(&self, r: &RectI) -> bool {
        self.game_monitors.contains(r)
    }

    /// Work area of a monitor without a game, preferring the one nearest `near`.
    pub fn free_work_area(&self, desk: &DesktopSnapshot, near: Vec2) -> Option<RectI> {
        desk.monitors
            .iter()
            .filter(|m| !self.is_game_monitor(&m.rect))
            .min_by(|a, b| dist(&a.work, near).total_cmp(&dist(&b.work, near)))
            .map(|m| m.work)
    }

    /// Monitor (full rect) the point is on.
    pub fn monitor_of(desk: &DesktopSnapshot, p: Vec2) -> Option<RectI> {
        desk.monitors.iter().find(|m| m.rect.contains(p.x as i32, p.y.max(0.0) as i32 - 1)).map(|m| m.rect)
    }

    /// Remove game monitors from the knight's world and wall off their seams.
    pub fn filter_terrain(&self, t: &mut Terrain, desk: &DesktopSnapshot) {
        if self.game_monitors.is_empty() {
            return;
        }
        let inside = |x: f32, y: f32| self.game_monitors.iter().any(|g| g.to_f().contains(Vec2::new(x, y)));
        t.ledges.retain(|l| !inside((l.x0 + l.x1) * 0.5, l.y - 1.0));
        t.walls.retain(|w| !inside(w.x, (w.y0 + w.y1) * 0.5));
        for g in &self.game_monitors {
            for m in desk.monitors.iter().filter(|m| !self.is_game_monitor(&m.rect)) {
                let (y0, y1) = (m.rect.top.max(g.top) as f32, m.rect.bottom.min(g.bottom) as f32);
                if y1 <= y0 {
                    continue;
                }
                // Free monitor on the left of the game: block moving right into it, and v.v.
                if m.rect.right == g.left {
                    t.walls.push(Wall { x: g.left as f32, y0: m.rect.top as f32, y1: m.rect.bottom as f32, face: Facing::Left, owner: None, kind: WallKind::Monitor });
                }
                if m.rect.left == g.right {
                    t.walls.push(Wall { x: g.right as f32, y0: m.rect.top as f32, y1: m.rect.bottom as f32, face: Facing::Right, owner: None, kind: WallKind::Monitor });
                }
            }
        }
    }
}

fn dist(r: &RectI, p: Vec2) -> f32 {
    let c = Vec2::new((r.left + r.right) as f32 * 0.5, (r.top + r.bottom) as f32 * 0.5);
    c.distance(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skong_core::desktop::Monitor;
    use skong_core::terrain::TerrainOptions;

    fn desk() -> DesktopSnapshot {
        let mon = |r: RectI, primary| Monitor { rect: r, work: r, primary, scale_milli: 1000 };
        DesktopSnapshot { generation: 1, monitors: vec![mon(RectI::new(0, 0, 2560, 1440), true), mon(RectI::new(-1920, 0, 0, 1080), false)], windows: vec![] }
    }

    #[test]
    fn game_monitor_is_cut_out_and_walled_off() {
        let d = desk();
        let mut t = Terrain::from_desktop(&d, &TerrainOptions::default());
        let g = GameGuard { game_monitors: vec![RectI::new(0, 0, 2560, 1440)], ..GameGuard::default() };
        g.filter_terrain(&mut t, &d);
        assert!(t.ledges.iter().all(|l| l.x1 <= 0.0), "only the left monitor's ground remains: {:?}", t.ledges);
        assert!(t.walls.iter().any(|w| w.x == 0.0 && w.face == Facing::Left), "seam walled off");
        assert_eq!(g.free_work_area(&d, Vec2::new(1000.0, 1000.0)), Some(RectI::new(-1920, 0, 0, 1080)));
    }
}
