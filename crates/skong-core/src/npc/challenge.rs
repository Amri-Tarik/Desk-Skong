//! Hornet's challenges: after the conversation she may challenge the knight — half the
//! time to a [`race`](super::race) to where her barb lands, half the time to a short
//! [`duel`](super::duel) she wins. The app decides before the talk (a race needs a finish
//! on a settled, reachable ledge: [`ChallengePlan`]); the visitor actor runs it.
use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::ai::rng::Rng;
use crate::ai::{Foe, SceneCue};
use crate::anim::{Animator, ClipId, ClipLibrary, WrapMode};
use crate::assets::SpriteKey;
use crate::desktop::WindowId;
use crate::geom::Vec2;

use super::actor::{NpcEnv, NpcEvent, NpcProp};
use super::duel::{Duel, DuelClips, DuelDef};
use super::race::{Race, RaceClips, RaceDef};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeDef {
    /// Chance (0..1) the conversation ends in a challenge.
    pub chance: f32,
    /// Share of challenges that are races (the rest are duels).
    pub race_share: f32,
    pub race: RaceDef,
    pub duel: DuelDef,
}

/// Which challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChallengeKind {
    Race,
    Duel,
}

/// What the app settled before the talk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChallengePlan {
    /// A race to `finish` (feet point on a settled ledge the knight can reach); the knight's
    /// planned time there, when known, paces her.
    Race { finish: Vec2, owner: Option<WindowId>, knight_eta: Option<f32> },
    Duel,
}

/// The challenges' clips, resolved.
#[derive(Clone, Debug)]
pub struct ChallengeClips {
    pub race: RaceClips,
    pub duel: DuelClips,
}

impl ChallengeDef {
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.chance) || !(0.0..=1.0).contains(&self.race_share) {
            bail!("challenge: chance and race_share must be within 0..1");
        }
        self.race.validate().context("challenge")?;
        self.duel.validate().context("challenge")
    }

    pub fn resolve(&self, lib: &ClipLibrary, who: &str) -> Result<ChallengeClips> {
        Ok(ChallengeClips { race: self.race.resolve(lib, who)?, duel: self.duel.resolve(lib, who)? })
    }

    /// Race or duel (`race_share` of the time a race).
    pub fn pick(&self, rng: &mut Rng) -> ChallengeKind {
        if rng.chance(self.race_share) { ChallengeKind::Race } else { ChallengeKind::Duel }
    }
}

/// The visitor's body, lent to a challenge for one tick.
pub(super) struct Body<'a> {
    pub pos: &'a mut Vec2,
    pub looking_right: &'a mut bool,
    pub alpha: &'a mut f32,
    pub flash: &'a mut f32,
    pub anim: &'a mut Animator,
    pub lib: &'a ClipLibrary,
    pub rng: &'a mut Rng,
}

impl Body<'_> {
    pub fn play(&mut self, clip: ClipId, restart: bool) {
        self.anim.play(self.lib, clip, restart);
    }

    pub fn face(&mut self, x: f32) {
        if (x - self.pos.x).abs() > 2.0 {
            *self.looking_right = x > self.pos.x;
        }
    }
}

/// What the actor does after a challenge tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Next {
    Stay,
    /// She landed somewhere new (find her footing again).
    Landed,
    /// Over: linger this long, then leave as usual.
    Linger(f32),
    /// Over: she is gone (leapt away).
    Gone,
}

#[derive(Clone, Debug)]
pub(super) enum Challenge {
    Race(Box<Race>),
    Duel(Box<Duel>),
}

impl Challenge {
    pub fn is_race(&self) -> bool {
        matches!(self, Challenge::Race(_))
    }

    pub fn airborne(&self) -> bool {
        match self {
            Challenge::Race(r) => r.airborne(),
            Challenge::Duel(_) => false,
        }
    }

    /// Her parting line, said once the outcome is in (not when it's called off).
    pub fn set_farewell(&mut self, text: Option<String>) {
        match self {
            Challenge::Race(r) => r.farewell = text,
            Challenge::Duel(d) => d.farewell = text,
        }
    }

    pub fn tick(&mut self, dt: f32, def: &ChallengeDef, clips: &ChallengeClips, body: &mut Body, env: &NpcEnv, events: &mut Vec<NpcEvent>) -> Next {
        match self {
            Challenge::Race(r) => r.tick(dt, &def.race, &clips.race, body, env, events),
            Challenge::Duel(d) => d.tick(dt, &def.duel, &clips.duel, body, env, events),
        }
    }

    /// Called off (the user needs the screen, the Shade, enemies, she was sent away).
    pub fn abandon(&mut self, def: &ChallengeDef, body: &mut Body, events: &mut Vec<NpcEvent>) {
        match self {
            Challenge::Race(r) => r.abandon(&def.race, body, events),
            Challenge::Duel(d) => d.abandon(&def.duel, body, events),
        }
    }

    /// The knight's blow (the duel only). Returns whether it landed.
    pub fn strike(&mut self, damage: u32, from_x: f32, body: &mut Body, def: &ChallengeDef, env: &NpcEnv) -> bool {
        match self {
            Challenge::Race(_) => false,
            Challenge::Duel(d) => {
                let span = super::duel::span_of(env, *body.pos, &def.duel, env.ppu);
                d.strike(damage, from_x, body, span, env.ppu)
            }
        }
    }

    pub fn cue(&self, serial: u32) -> Option<SceneCue> {
        match self {
            Challenge::Race(r) => Some(r.cue(serial)),
            Challenge::Duel(_) => None,
        }
    }

    pub fn foe(&self, id: u32, pos: Vec2, def: &ChallengeDef, ppu: f32) -> Option<Foe> {
        match self {
            Challenge::Race(_) => None,
            Challenge::Duel(d) => d.foe(id, pos, &def.duel, ppu),
        }
    }

    pub fn props(&self, pos: Vec2, looking_right: bool, lib: &ClipLibrary, def: &ChallengeDef, clips: &ChallengeClips, ppu: f32) -> Vec<NpcProp> {
        match self {
            Challenge::Race(r) => r.props(lib, &clips.race, ppu),
            Challenge::Duel(d) => d.props(pos, looking_right, lib, &def.duel, &clips.duel, ppu),
        }
    }
}

/// The sprite of `clip` `t` seconds in (looped, or held on its last frame).
pub(super) fn frame_at(lib: &ClipLibrary, clip: ClipId, t: f32, looped: bool) -> Option<SpriteKey> {
    let c = lib.get(clip);
    let n = c.frames.len();
    if n == 0 {
        return None;
    }
    let i = (t.max(0.0) * c.fps) as usize;
    let i = if looped || matches!(c.wrap, WrapMode::Loop) { i % n } else { i.min(n - 1) };
    c.frames[i].sprite
}

/// One line of `pool` (validated non-empty).
pub(super) fn pick_line(rng: &mut Rng, pool: &[String]) -> String {
    pool.get(rng.next_u32() as usize % pool.len().max(1)).cloned().unwrap_or_default()
}
