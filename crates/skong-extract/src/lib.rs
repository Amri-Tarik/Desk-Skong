//! First-run art setup: reads the sprite atlases Desk-Skong draws from the player's own
//! Hollow Knight install and writes them as PNGs into the local Vanilla skin folder.
//! Nothing from the game ships with Desk-Skong; this runs once, on the player's PC, and
//! only ever reads the game's files.
//!
//! - [`serialized`]: Unity serialized files (object tables, no type trees).
//! - [`texture`] / [`decode`] / [`bc7`]: `Texture2D` headers, `.resS` streams, pixel formats.
//! - [`sprite`]: Unity sprites (the bench and rock props).
//! - [`catalog`]: finding each tk2d collection's atlas and each prop in the game files.
//! - [`extract`]: the plan (from `data/hk`), parallel decode, the skin folder swap.
//! - [`install`] / [`vdf`]: finding the game (Steam libraries, GOG, Epic).
pub mod bc7;
pub mod catalog;
pub mod decode;
pub mod extract;
pub mod install;
pub mod reader;
pub mod serialized;
pub mod sprite;
pub mod texture;
pub mod vdf;

pub use extract::{MARKER, Plan, Progress, Report, extract, plan, recorded_game};
