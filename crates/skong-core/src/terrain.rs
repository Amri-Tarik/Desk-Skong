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
//!
//! On top of that, **content ledges** ([`LedgeKind::Content`]) and **content walls**
//! ([`WallKind::Content`]) come from what is drawn on screen ([`crate::vision`]): one-way
//! platforms and walls owned by the front-most window showing them, solid even from
//! inside that window. See [`content`] and
//! [`Terrain::from_desktop_and_content`].
use crate::desktop::{DesktopSnapshot, WindowId};
use crate::geom::RectF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgeKind {
    /// Cannot be dropped through (work-area bottom).
    Ground,
    /// One-way: pass up through it, land on it, drop through with "down".
    OneWay,
    /// Detected on-screen content (a drawn line, a table border...). One-way like
    /// [`LedgeKind::OneWay`], but never ignored while the body is inside its owner window.
    Content,
}

impl LedgeKind {
    /// Can be passed upward and dropped through.
    pub fn is_one_way(self) -> bool {
        matches!(self, LedgeKind::OneWay | LedgeKind::Content)
    }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    pub kind: WallKind,
}

/// Where a wall comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WallKind {
    /// Side of a monitor work area (or the terrain bounds).
    Monitor,
    /// Side of a window.
    Window,
    /// A vertical boundary in what is drawn on screen (column border, divider, stroke).
    /// Like content ledges it is solid even from inside its owner window, and a shadow
    /// dash passes through it.
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowBox {
    pub id: WindowId,
    pub rect: RectF,
}

#[derive(Clone, Debug, Default)]
pub struct Terrain {
    /// Desktop generation this terrain was built from.
    pub generation: u64,
    /// Window boxes, front to back. A window's ledges/walls are ignored by a body that
    /// overlaps its interior (e.g. a window opened on top of the knight).
    pub windows: Vec<WindowBox>,
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
    /// Content ledges shorter than this after clipping are dropped (px).
    pub min_content_len: f32,
    /// Content ledges within this many px of a window-top / ground ledge are dropped
    /// where they overlap it (the window frame itself is visible content too).
    pub content_dedupe_px: f32,
    /// Keep content ledges / walls on the desktop wallpaper (owner `None`).
    pub include_wallpaper_content: bool,
    /// Content walls shorter than this after clipping are dropped (px).
    pub min_content_wall_len: f32,
    /// Where a right-facing content wall and a left-facing one to its right leave an
    /// opening narrower than this (px), both are cut there: the knight could neither
    /// fit nor escape. Set it a bit wider than his hitbox.
    pub min_content_gap: f32,
}

impl Default for TerrainOptions {
    fn default() -> Self {
        Self {
            min_ledge_len: 24.0,
            min_wall_len: 24.0,
            min_window_size: 48,
            min_content_len: 32.0,
            content_dedupe_px: 3.0,
            include_wallpaper_content: false,
            min_content_wall_len: 48.0,
            min_content_gap: 40.0,
        }
    }
}

impl Terrain {
    pub fn window_rect(&self, id: WindowId) -> Option<RectF> {
        self.windows.iter().find(|w| w.id == id).map(|w| w.rect)
    }

    /// Build terrain from a desktop snapshot. See module docs for the model.
    pub fn from_desktop(desk: &DesktopSnapshot, opts: &TerrainOptions) -> Self {
        build::build(desk, &[], &[], opts)
    }

    /// [`Self::from_desktop`] plus content ledges and walls (virtual-desktop pixels, owners
    /// as assigned by [`ContentLayer`]). Content is clipped to work areas, hidden behind
    /// windows in front of its owner, dropped inside fullscreen windows (and on the
    /// wallpaper unless enabled), de-duplicated against window edges, and walls leaving
    /// openings narrower than `min_content_gap` are cut.
    pub fn from_desktop_and_content(desk: &DesktopSnapshot, ledges: &[ContentLedge], walls: &[ContentWallSeg], opts: &TerrainOptions) -> Self {
        build::build(desk, ledges, walls, opts)
    }
}

mod build;
pub mod content;

pub use content::{ContentLayer, ContentLedge, ContentWallSeg};
