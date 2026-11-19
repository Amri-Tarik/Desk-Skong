//! Where to stand: the cursor's projection onto terrain, seats (benches and perches on
//! window / content edges) and vantage points for watching a screen region.
use crate::controller::Character;
use crate::geom::{RectI, Vec2};
use crate::terrain::{Ledge, LedgeKind, Terrain};

/// Keep this far from a ledge's ends when choosing a spot on it (px).
const EDGE_MARGIN: f32 = 20.0;
/// How far the sitting knight's legs dangle below the seat (game units; matches the app).
const LEG_LENGTH_UNITS: f32 = 28.0 / 64.0;

fn x_on(l: &Ledge, x: f32) -> f32 {
    let lo = l.x0 + EDGE_MARGIN;
    let hi = (l.x1 - EDGE_MARGIN).max(lo);
    x.clamp(lo, hi)
}

/// Highest ledge at or below `p` that spans `p.x`: where the cursor "is" for a walker.
pub(crate) fn project(terrain: &Terrain, p: Vec2) -> Option<Vec2> {
    terrain
        .ledges
        .iter()
        .filter(|l| l.x0 <= p.x && p.x <= l.x1 && l.y >= p.y - 0.5)
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .map(|l| Vec2::new(x_on(l, p.x), l.y))
}

/// Can the knight perch at `x` on ledge `l`: a window / content edge (not the taskbar
/// floor) with nothing within leg length below. Mirrors the app's seat finding.
pub(crate) fn perchable(terrain: &Terrain, l: &Ledge, x: f32, legs: f32) -> bool {
    let edge = !matches!(l.kind, LedgeKind::Ground);
    edge && !terrain.ledges.iter().any(|o| o.x0 <= x && x <= o.x1 && o.y > l.y + 0.5 && o.y < l.y + legs)
}

/// Seats near `near`, closest first: benches and perch points (one per ledge).
pub(crate) fn seats(terrain: &Terrain, knight: &Character, near: Vec2, benches: &[Vec2], max_dist: f32) -> Vec<Vec2> {
    let legs = knight.params.px(LEG_LENGTH_UNITS);
    let mut out: Vec<Vec2> = benches.to_vec();
    for l in &terrain.ledges {
        if l.x1 - l.x0 < 2.0 * EDGE_MARGIN + 8.0 {
            continue;
        }
        let x = x_on(l, near.x);
        if perchable(terrain, l, x, legs) {
            out.push(Vec2::new(x, l.y));
        }
    }
    out.retain(|s| s.distance(near) <= max_dist);
    out.sort_by(|a, b| a.distance(near).total_cmp(&b.distance(near)));
    out
}

/// How the knight looks at a spot from a vantage point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Standing in front of it (it is behind him): gaze into the screen.
    Front,
    /// Standing on an edge above it: look down.
    FromAbove,
    /// Standing on an edge below it: look up.
    FromBelow,
}

impl View {
    /// The act that fits this view.
    pub fn act(self) -> &'static str {
        match self {
            View::Front => "gaze_into_screen",
            View::FromAbove => "look_down",
            View::FromBelow => "look_up",
        }
    }

    /// How a spot centered at `spot` looks from a knight whose body center is `eye`.
    pub fn of(rect: RectI, spot: Vec2, eye: Vec2) -> View {
        let r = rect.to_f();
        let inside = eye.x >= r.left - 20.0 && eye.x <= r.right + 20.0 && eye.y >= r.top - 20.0 && eye.y <= r.bottom + 20.0;
        let dy = spot.y - eye.y;
        if inside {
            View::Front
        } else if dy < -100.0 {
            View::FromBelow
        } else if dy > 40.0 {
            View::FromAbove
        } else {
            View::Front
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vantage {
    /// Feet position to stand at.
    pub point: Vec2,
    pub view: View,
}

/// Best place to watch `rect` from: the edge just above it (e.g. its window's title bar),
/// a ledge in front of it, or the edge just below; nearer to the knight is better.
pub(crate) fn vantage(terrain: &Terrain, rect: RectI, from: Vec2) -> Option<Vantage> {
    let r = rect.to_f();
    let center = r.center();
    terrain
        .ledges
        .iter()
        .filter(|l| l.x1 - l.x0 >= 2.0 * EDGE_MARGIN + 8.0 && l.x1 >= r.left - 60.0 && l.x0 <= r.right + 60.0)
        .filter_map(|l| {
            let (view, gap, bias) = if l.y <= r.top + 8.0 {
                (View::FromAbove, r.top - l.y, 0.0)
            } else if l.y < r.bottom {
                (View::Front, 0.0, 40.0)
            } else {
                (View::FromBelow, l.y - r.bottom, 120.0)
            };
            // Looking down works from a title bar; looking up works from much farther below.
            let reach = if view == View::FromBelow { 900.0 } else { 350.0 };
            if gap > reach {
                return None;
            }
            let point = Vec2::new(x_on(l, center.x), l.y);
            let cost = gap + bias + 0.3 * point.distance(from) + 0.5 * (point.x - center.x).abs();
            Some((Vantage { point, view }, cost))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{Abilities, CharacterParams};
    use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
    use crate::terrain::TerrainOptions;

    fn terrain() -> Terrain {
        let w = |id, rect| WindowInfo {
            id,
            rect,
            title: String::new(),
            class: String::new(),
            process: String::new(),
            pid: 1,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        };
        let desk = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows: vec![w(1, RectI::new(200, 400, 900, 900))],
        };
        Terrain::from_desktop(&desk, &TerrainOptions::default())
    }

    #[test]
    fn cursor_projects_onto_the_ledge_below() {
        let t = terrain();
        assert_eq!(project(&t, Vec2::new(500.0, 300.0)), Some(Vec2::new(500.0, 400.0)));
        assert_eq!(project(&t, Vec2::new(1500.0, 300.0)).map(|p| p.y), Some(1040.0));
    }

    #[test]
    fn perches_are_window_tops_not_the_floor() {
        let t = terrain();
        let k = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(1000.0, 1040.0));
        let s = seats(&t, &k, Vec2::new(1000.0, 1040.0), &[], 2000.0);
        assert!(!s.is_empty());
        assert!(s.iter().all(|p| p.y == 400.0), "{s:?}");
    }

    #[test]
    fn spot_in_a_window_is_watched_from_its_title_bar() {
        let t = terrain();
        let v = vantage(&t, RectI::new(400, 500, 600, 600), Vec2::new(1000.0, 1040.0)).expect("vantage");
        assert_eq!(v.view, View::FromAbove);
        assert_eq!(v.point, Vec2::new(500.0, 400.0));
    }
}
