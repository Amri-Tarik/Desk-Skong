//! Platform-neutral description of what is on screen. Produced by `skong-win`,
//! consumed by `terrain`, `nav` and the AI.
use crate::geom::RectI;

/// Opaque native window handle (HWND as integer on Windows).
pub type WindowId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// Full monitor rect in virtual-desktop pixels.
    pub rect: RectI,
    /// Work area (excludes taskbar / app bars).
    pub work: RectI,
    pub primary: bool,
    /// Effective DPI scale (1.0 = 96 DPI).
    pub scale_milli: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: WindowId,
    /// Visible frame bounds (DWM extended frame bounds, no invisible resize borders).
    pub rect: RectI,
    pub title: String,
    pub class: String,
    pub process: String,
    pub pid: u32,
    pub maximized: bool,
    /// Covers a whole monitor with no frame (games, video players, F11 browsers).
    pub fullscreen: bool,
    /// Window is the current foreground window.
    pub foreground: bool,
    pub topmost: bool,
}

/// Snapshot of the desktop. `windows` is ordered **front to back** (index 0 = topmost).
/// Only visible, non-cloaked, non-minimized, non-tool, non-overlay windows are listed;
/// the taskbar is represented by monitor work areas, not as a window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopSnapshot {
    /// Increments every time the platform layer observes a change.
    pub generation: u64,
    pub monitors: Vec<Monitor>,
    pub windows: Vec<WindowInfo>,
}

impl DesktopSnapshot {
    pub fn window(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.iter().find(|w| w.id == id)
    }
    pub fn monitor_at(&self, x: i32, y: i32) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.rect.contains(x, y))
    }
    pub fn primary(&self) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.primary).or(self.monitors.first())
    }
}
