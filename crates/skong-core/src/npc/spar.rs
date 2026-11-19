//! Sparring: a visitor who fancies himself a warrior (Zote) may challenge the knight after
//! their talk. He fights with his own fight clips (from a second library merged into his,
//! see `extra_libraries`): stance, wind-up, a charge along his ledge, a stagger when struck.
//! He takes a few blows, goes down, gets up, says something to save face, and leaves. The
//! knight fights him like an enemy (the brain's `Fight` behaviour), but nobody dies.
use anyhow::{Result, bail};
use serde::Deserialize;

use crate::anim::{ClipId, ClipLibrary};

/// Clip names for the spar (from the visitor's merged library).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SparClipNames {
    /// Standing guard between charges.
    pub stance: String,
    /// Wind-up before a charge (played once).
    pub antic: String,
    /// The charge (loops while he moves).
    pub attack: String,
    /// After a charge (default: `stance`).
    #[serde(default)]
    pub recover: Option<String>,
    /// Struck (default: `stance`).
    #[serde(default)]
    pub hit: Option<String>,
    /// Knocked down (played once; its last frame is held while he lies there).
    pub fall: String,
    pub get_up: String,
    /// Said with the challenge (default: `stance`).
    #[serde(default)]
    pub taunt: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SparDef {
    /// Chance (0..1) he challenges the knight after their conversation.
    pub chance: f32,
    /// The challenge (one picked).
    pub challenge: Vec<String>,
    pub clips: SparClipNames,
    /// Blows he takes before going down (spells count more).
    pub hits: u32,
    /// Charge speed (game units/s) and how long a charge lasts (s).
    pub charge_speed: f32,
    pub charge_secs: f32,
    /// Seconds on guard between charges (random in range).
    #[serde(default = "pause")]
    pub pause: [f32; 2],
    /// Said once he is back on his feet after losing (one picked).
    pub lose: Vec<String>,
    /// Said when the spar ends without a winner (it dragged on, the knight wandered off).
    pub abandon: Vec<String>,
    /// The spar ends after this long (s).
    #[serde(default = "max_secs")]
    pub max_secs: f32,
    /// Body [w, h] in game units (blows and the knight's reach).
    pub size: [f32; 2],
}

fn pause() -> [f32; 2] {
    [0.8, 1.8]
}
fn max_secs() -> f32 {
    45.0
}

/// [`SparClipNames`] resolved.
#[derive(Clone, Debug)]
pub struct SparClips {
    pub stance: ClipId,
    pub antic: ClipId,
    pub attack: ClipId,
    pub recover: ClipId,
    pub hit: ClipId,
    pub fall: ClipId,
    pub get_up: ClipId,
    pub taunt: ClipId,
}

impl SparClips {
    pub fn contains(&self, c: ClipId) -> bool {
        [self.stance, self.antic, self.attack, self.recover, self.hit, self.fall, self.get_up, self.taunt].contains(&c)
    }
}

impl SparDef {
    pub fn validate(&self) -> Result<()> {
        let pos = |x: f32| x.is_finite() && x > 0.0;
        if !(0.0..=1.0).contains(&self.chance) {
            bail!("spar: chance must be within 0..1");
        }
        if self.hits == 0 || !pos(self.charge_speed) || !pos(self.charge_secs) || !pos(self.max_secs) || !pos(self.size[0]) || !pos(self.size[1]) {
            bail!("spar: hits, speeds, times and size must be positive");
        }
        if self.pause[0] < 0.0 || self.pause[1] < self.pause[0] {
            bail!("spar: bad pause range");
        }
        if self.challenge.is_empty() || self.lose.is_empty() || self.abandon.is_empty() {
            bail!("spar: needs challenge, lose and abandon lines");
        }
        if self.challenge.iter().chain(&self.lose).chain(&self.abandon).any(|l| l.contains('{')) {
            bail!("spar: lines are plain text (no placeholders)");
        }
        Ok(())
    }

    pub fn resolve(&self, lib: &ClipLibrary, who: &str) -> Result<SparClips> {
        let get = |n: &str| match lib.id(n) {
            Some(id) => Ok(id),
            None => bail!("npc '{who}': spar clip '{n}' not in its libraries"),
        };
        let c = &self.clips;
        let stance = get(&c.stance)?;
        let opt = |n: &Option<String>| n.as_deref().map(get).transpose().map(|o| o.unwrap_or(stance));
        Ok(SparClips {
            stance,
            antic: get(&c.antic)?,
            attack: get(&c.attack)?,
            recover: opt(&c.recover)?,
            hit: opt(&c.hit)?,
            fall: get(&c.fall)?,
            get_up: get(&c.get_up)?,
            taunt: opt(&c.taunt)?,
        })
    }
}
