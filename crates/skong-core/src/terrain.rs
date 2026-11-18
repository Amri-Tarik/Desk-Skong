//! Collision geometry derived from the desktop.
//!
//! Model: every visible window is a **platform box** in front of which the knight lives.
//!
//! * its top edge is a one-way [`Ledge`] (solid when falling onto it from above),
//! * its left/right edges are [`Wall`]s that only block motion coming from outside,
//! * its bottom edge is passable.
//!
//! Parts of edges hidden behind higher windows are removed. Monitor work-area bottoms
//! (taskbar tops) are solid ground; monitor outer edges are inward-facing walls.
use crate::desktop::{DesktopSnapshot, WindowId};
use crate::geom::RectF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgeKind {
    /// Cannot be dropped through (work-area bottom).
    Ground,
    /// One-way: pass up through it, land on it, drop through with "down".
    OneWay,
}

/// Horizontal surface at height `y`, spanning `x0..x1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ledge {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub kind: LedgeKind,
    pub owner: Option<WindowId>,
}

/// Which way the wall's solid face points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    /// Face points to -x: blocks things moving right (+x) that start left of it.
    Left,
    /// Face points to +x: blocks things moving left (-x) that start right of it.
    Right,
}

/// Vertical wall at `x`, spanning `y0..y1` (y0 < y1, y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wall {
    pub x: f32,
    pub y0: f32,
    pub y1: f32,
    pub face: Facing,
    pub owner: Option<WindowId>,
}

#[derive(Clone, Debug, Default)]
pub struct Terrain {
    /// Desktop generation this terrain was built from.
    pub generation: u64,
    pub ledges: Vec<Ledge>,
    pub walls: Vec<Wall>,
    /// Union of monitor rects: the playable area.
    pub bounds: RectF,
}

#[derive(Clone, Debug)]
pub struct TerrainOptions {
    /// Ignore window edge fragments shorter than this (px).
    pub min_ledge_len: f32,
    pub min_wall_len: f32,
    /// Windows narrower/shorter than this are ignored entirely.
    pub min_window_size: i32,
}

impl Default for TerrainOptions {
    fn default() -> Self {
        Self { min_ledge_len: 24.0, min_wall_len: 24.0, min_window_size: 48 }
    }
}

impl Terrain {
    /// Build terrain from a desktop snapshot. See module docs for the model.
    pub fn from_desktop(desk: &DesktopSnapshot, opts: &TerrainOptions) -> Self {
        build::build(desk, opts)
    }
}

mod build;
