//! Placeable props (benches): the knight's resting spots. A bench sits on a ledge, rides
//! along with the window it stands on, and drops to the next ledge if that window goes.
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

use crate::assets::read_json;
use crate::desktop::WindowId;
use crate::geom::Vec2;
use crate::terrain::Terrain;

#[derive(Clone, Debug, Deserialize)]
pub struct BenchStyle {
    pub name: String,
    /// Sprite file stem in `skins/Vanilla/props`.
    pub sprite: String,
    /// Seat height as a fraction of the sprite height, from the bottom.
    pub seat: f32,
    /// Extra offset (game units, y up) applied to the sitting knight.
    #[serde(default)]
    pub sit_offset: [f32; 2],
}

#[derive(Debug, Deserialize)]
struct PropsFile {
    benches: Vec<BenchStyle>,
}

pub fn load_bench_styles(path: &Path) -> Result<Vec<BenchStyle>> {
    Ok(read_json::<PropsFile>(path)?.benches)
}

/// A placed bench. `pos` is its bottom-center, resting on a ledge.
#[derive(Clone, Debug, PartialEq)]
pub struct Bench {
    pub style: usize,
    pub pos: Vec2,
    /// Window whose top ledge supports it (None = ground / content ledge without owner).
    pub owner: Option<WindowId>,
}

impl Bench {
    /// Place at `at`, dropped onto the first ledge at or below it. None if nothing is below.
    pub fn place(style: usize, at: Vec2, terrain: &Terrain) -> Option<Bench> {
        let (y, owner) = ledge_below(terrain, at)?;
        Some(Bench { style, pos: Vec2::new(at.x, y), owner })
    }

    /// Keep the bench on its window when the desktop changes; drop it if its support is gone.
    pub fn follow(&mut self, old: &Terrain, new: &Terrain) {
        if let Some(id) = self.owner
            && let (Some(a), Some(b)) = (old.window_rect(id), new.window_rect(id))
        {
            self.pos.x += b.left - a.left;
            self.pos.y += b.top - a.top;
        }
        let supported = new.ledges.iter().any(|l| (l.y - self.pos.y).abs() < 1.0 && l.x0 <= self.pos.x && self.pos.x <= l.x1);
        if !supported {
            match ledge_below(new, self.pos) {
                Some((y, owner)) => {
                    self.pos.y = y;
                    self.owner = owner;
                }
                None => {
                    // Nothing below (dragged off-screen): put it on the bottom of the play area.
                    self.pos.y = new.bounds.bottom;
                    self.owner = None;
                }
            }
        }
    }
}

/// Highest ledge at or below `p` spanning `p.x`.
pub fn ledge_below(terrain: &Terrain, p: Vec2) -> Option<(f32, Option<WindowId>)> {
    terrain
        .ledges
        .iter()
        .filter(|l| l.x0 <= p.x && p.x <= l.x1 && l.y >= p.y - 0.5)
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .map(|l| (l.y, l.owner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectF;
    use crate::terrain::{Ledge, LedgeKind, WindowBox};

    fn terrain(win_top: f32) -> Terrain {
        Terrain {
            windows: vec![WindowBox { id: 7, rect: RectF::new(100.0, win_top, 500.0, win_top + 300.0) }],
            ledges: vec![
                Ledge { x0: 100.0, x1: 500.0, y: win_top, kind: LedgeKind::OneWay, owner: Some(7) },
                Ledge { x0: 0.0, x1: 1000.0, y: 1000.0, kind: LedgeKind::Ground, owner: None },
            ],
            bounds: RectF::new(0.0, 0.0, 1000.0, 1000.0),
            ..Default::default()
        }
    }

    #[test]
    fn places_on_window_and_rides_it() {
        let t0 = terrain(400.0);
        let mut b = Bench::place(0, Vec2::new(200.0, 50.0), &t0).expect("placed");
        assert_eq!((b.pos.y, b.owner), (400.0, Some(7)));
        let t1 = terrain(300.0);
        b.follow(&t0, &t1);
        assert_eq!(b.pos, Vec2::new(200.0, 300.0));
    }

    #[test]
    fn drops_to_ground_when_window_closes() {
        let t0 = terrain(400.0);
        let mut b = Bench::place(0, Vec2::new(200.0, 50.0), &t0).expect("placed");
        let mut t1 = terrain(400.0);
        t1.windows.clear();
        t1.ledges.remove(0);
        b.follow(&t0, &t1);
        assert_eq!((b.pos.y, b.owner), (1000.0, None));
    }
}
