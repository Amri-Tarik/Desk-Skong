//! What survives a restart: the wallet and the Shade (where it floats, what it carries).
//! Stored by the app inside its settings file; every field defaults so old files load.
use serde::{Deserialize, Serialize};

use crate::geom::Vec2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShadeSave {
    /// Home point (screen px, the Shade's body centre).
    pub x: f32,
    pub y: f32,
    /// Geo the knight lost when he died.
    pub geo: u32,
}

impl ShadeSave {
    pub fn pos(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StakesSave {
    /// Geo in the knight's wallet.
    pub geo: u32,
    /// The Shade waiting where he last died, if not reclaimed yet.
    pub shade: Option<ShadeSave>,
    /// Lifetime totals (flavour for the panel).
    pub deaths: u32,
    pub shades_defeated: u32,
}

impl StakesSave {
    /// The knight died at `shade_at` (where the Shade will float). His Geo goes to a new
    /// Shade; an older, unreclaimed Shade vanishes with its Geo (as in the game).
    /// Returns the Geo lost for good. `leaves_shade = false` (hazards) keeps everything.
    pub fn die(&mut self, shade_at: Vec2, leaves_shade: bool) -> u32 {
        self.deaths += 1;
        if !leaves_shade {
            return 0;
        }
        let lost = self.shade.take().map_or(0, |s| s.geo);
        self.shade = Some(ShadeSave { x: shade_at.x, y: shade_at.y, geo: self.geo });
        self.geo = 0;
        lost
    }

    /// The Shade was beaten: its Geo comes back. Returns the amount recovered.
    pub fn reclaim(&mut self) -> u32 {
        let Some(s) = self.shade.take() else { return 0 };
        self.shades_defeated += 1;
        self.geo = self.geo.saturating_add(s.geo);
        s.geo
    }

    pub fn earn(&mut self, geo: u32) {
        self.geo = self.geo.saturating_add(geo);
    }

    /// Keep the saved Shade where the live one drifted to.
    pub fn shade_moved(&mut self, to: Vec2) {
        if let Some(s) = &mut self.shade {
            s.x = to.x;
            s.y = to.y;
        }
    }
}
