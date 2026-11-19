//! Battle stakes (death, the Shade, Geo, spells): debug commands and panel status.
use serde::{Deserialize, Serialize};

/// Spell / nail-art acts the panel can trigger, indexed by [`StakesCommand::Cast`].
pub const SPELLS: &[(&str, &str)] = &[
    ("fireball", "Vengeful Spirit"),
    ("scream", "Howling Wraiths"),
    ("great_slash", "Great Slash"),
    ("cyclone_slash", "Cyclone Slash"),
    ("dash_slash", "Dash Slash"),
    ("thorns", "Thorns"),
    ("focus_heal", "Focus"),
];

/// Debug / manual triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StakesCommand {
    /// Kill the knight (he leaves a Shade with his Geo, then respawns).
    Die,
    /// Burst some Geo out of the cursor position.
    DropGeo,
    /// A Geo shower now: coins tumble out of a subtle portal over a settled, reachable
    /// ledge (tray "Geo shower", panel "Geo shower").
    SpillGeo,
    /// Send the knight after his Shade now.
    HuntShade,
    /// Play a spell act (index into [`SPELLS`]).
    Cast(u8),
}

/// Wallet, Shade and soul, for the panel's Status.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StakesInfo {
    pub geo: u32,
    /// Loose coins lying around (value).
    pub loose_geo: u32,
    /// Where the Shade floats and what it carries.
    pub shade: Option<(f32, f32, u32)>,
    /// Soul meter 0..1.
    pub soul: f32,
    pub deaths: u32,
    pub shades_defeated: u32,
}
