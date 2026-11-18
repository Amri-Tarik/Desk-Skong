//! Platform-independent core of Desk-Skong.
//!
//! Coordinate convention everywhere in this crate: **virtual-desktop pixels,
//! x to the right, y DOWN** (same as Win32), so no flipping at the platform edge.
//! Game data (tk2d) is y-up world units; conversion happens once in `assets`.

pub mod acts;
pub mod anim;
pub mod assets;
pub mod canvas;
pub mod controller;
pub mod desktop;
pub mod geom;
pub mod terrain;
