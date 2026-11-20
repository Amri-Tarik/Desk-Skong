//! Spells and nail arts in fights: which one (if any) fits the moment.
//!
//! * **Soul** fills with every nail hit that lands (a third of a spell per three hits, as
//!   in the game) and pays for spells: Vengeful Spirit, Howling Wraiths, Focus.
//! * **Nail arts** are free but have cooldowns.
//! * Every choice has a reason: a foe level and far away gets a fireball, one right above
//!   gets the wraiths, one close in front a great slash or a cyclone, one at mid range a
//!   dash slash; being hit may answer with thorns; a hurt knight focuses after the fight.
//! * Some moments call for one in particular ([`SpellCues`]): the wraiths at a flyer above,
//!   a great slash into a wind-up, a cyclone between two foes. Those skip the chance roll.
//! * Specials are occasional (`SpellConfig::chance` per opportunity, a minimum gap between
//!   them) so plain slashes stay the bread and butter.
use serde::Deserialize;

use crate::geom::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Spell {
    /// Vengeful Spirit: `Fireball Antic` → `Fireball1 Cast`, a fireball flies forward.
    Fireball,
    /// Howling Wraiths: `Scream Start/Scream/Scream End`, wraiths burst upward.
    Scream,
    /// Great Slash: nail charge → `NA Big Slash`.
    GreatSlash,
    /// Cyclone Slash: `NA Cyclone Start/Cyclone/End`.
    Cyclone,
    /// Dash Slash: `NA Dash Slash` lunging forward.
    DashSlash,
    /// Thorns of Agony: `Thorn Attack` when hit.
    Thorns,
    /// Focus: `Focus` → `Focus Get`, heals after a fight.
    Focus,
}

impl Spell {
    pub const ALL: [Spell; 7] = [Spell::Fireball, Spell::Scream, Spell::GreatSlash, Spell::Cyclone, Spell::DashSlash, Spell::Thorns, Spell::Focus];

    /// Act in `data/acts/stakes.json`.
    pub fn act_name(self) -> &'static str {
        match self {
            Spell::Fireball => "fireball",
            Spell::Scream => "scream",
            Spell::GreatSlash => "great_slash",
            Spell::Cyclone => "cyclone_slash",
            Spell::DashSlash => "dash_slash",
            Spell::Thorns => "thorns",
            Spell::Focus => "focus_heal",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Spell::Fireball => "vengeful spirit",
            Spell::Scream => "howling wraiths",
            Spell::GreatSlash => "great slash",
            Spell::Cyclone => "cyclone slash",
            Spell::DashSlash => "dash slash",
            Spell::Thorns => "thorns of agony",
            Spell::Focus => "focus",
        }
    }

    /// Soul cost (0..1 meter).
    pub fn soul(self) -> f32 {
        match self {
            Spell::Fireball | Spell::Scream | Spell::Focus => SPELL_SOUL,
            _ => 0.0,
        }
    }

    /// Damage dealt when it connects (a nail hit is 1).
    pub fn damage(self) -> u32 {
        match self {
            Spell::Fireball | Spell::GreatSlash | Spell::Cyclone => 2,
            Spell::Scream => 3,
            Spell::DashSlash | Spell::Thorns => 1,
            Spell::Focus => 0,
        }
    }

    /// Does it reach a target at `off` (target − knight body centre, screen px, y down)?
    pub fn reaches(self, off: Vec2, scale: f32) -> bool {
        let (dx, dy) = (off.x.abs() / scale, off.y / scale);
        match self {
            Spell::Fireball => dy.abs() < 90.0 && (160.0..900.0).contains(&dx),
            Spell::Scream => (-420.0..-90.0).contains(&dy) && dx < 170.0,
            Spell::GreatSlash => dy.abs() < 80.0 && dx < 150.0,
            Spell::Cyclone => dy.abs() < 140.0 && dx < 130.0,
            Spell::DashSlash => dy.abs() < 70.0 && (130.0..340.0).contains(&dx),
            Spell::Thorns => off.length() / scale < 170.0,
            Spell::Focus => true,
        }
    }

    fn index(self) -> usize {
        Spell::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// Soul for one spell (a third of the meter).
pub const SPELL_SOUL: f32 = 1.0 / 3.0;
/// Soul gained per landed nail hit.
pub const SOUL_PER_HIT: f32 = 1.0 / 9.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct SpellConfig {
    /// Chance to use a fitting special at each attack opportunity.
    pub chance: f32,
    /// Minimum seconds between specials.
    pub min_gap: f32,
    /// Chance to answer a hit with thorns.
    pub thorns_chance: f32,
    /// Cooldowns per nail art (s).
    pub nail_art_cooldown: f32,
    /// Hurt above this makes him focus after a fight (if he has the soul).
    pub focus_hurt: f32,
    /// Soul to start with.
    pub start_soul: f32,
}

impl Default for SpellConfig {
    fn default() -> Self {
        Self { chance: 0.3, min_gap: 2.5, thorns_chance: 0.35, nail_art_cooldown: 7.0, focus_hurt: 0.15, start_soul: SPELL_SOUL }
    }
}

/// The moment a spell is considered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpellSituation {
    /// Target − knight body centre (screen px, y down). `None` after the fight.
    pub target: Option<Vec2>,
    pub grounded: bool,
    /// The knight was just hit.
    pub just_hit: bool,
    /// The fight is over (focus time).
    pub fight_over: bool,
    /// The knight's hurt (0..1).
    pub hurt: f32,
    /// Uniform random numbers in [0, 1) from the caller's RNG (deterministic brains).
    pub rolls: [f32; 2],
    /// Render scale (px per game unit / 64): reach scales with it.
    pub scale: f32,
    /// What the foe is up to (see [`SpellCues`]).
    pub cues: SpellCues,
}

/// Moments that call for one special in particular: it is used whenever it is ready (no
/// chance roll; the gap between specials and its cooldown still hold).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpellCues {
    /// The target flies (a Vengefly above him gets the Howling Wraiths).
    pub flying: bool,
    /// The target winds up an attack right in front of him (a Great Slash cuts it short).
    pub winding_up: bool,
    /// Foes within a Cyclone Slash's reach (two or more: spin).
    pub crowd: u32,
}

impl SpellCues {
    /// The special this moment calls for, if it reaches `off`.
    pub fn called_for(&self, off: Vec2, scale: f32) -> Option<Spell> {
        if self.crowd >= 2 && Spell::Cyclone.reaches(off, scale) {
            Some(Spell::Cyclone)
        } else if self.winding_up && Spell::GreatSlash.reaches(off, scale) {
            Some(Spell::GreatSlash)
        } else if self.flying && Spell::Scream.reaches(off, scale) {
            Some(Spell::Scream)
        } else {
            None
        }
    }
}

/// Soul meter and cooldowns.
#[derive(Clone, Debug, PartialEq)]
pub struct SpellBook {
    pub soul: f32,
    /// Brain-clock time each spell becomes available again.
    ready_at: [f64; 7],
    last_special: f64,
}

impl SpellBook {
    pub fn new(cfg: &SpellConfig) -> Self {
        Self { soul: cfg.start_soul.clamp(0.0, 1.0), ready_at: [0.0; 7], last_special: f64::NEG_INFINITY }
    }

    /// A nail hit landed.
    pub fn gain(&mut self, hits: u32) {
        self.soul = (self.soul + SOUL_PER_HIT * hits as f32).min(1.0);
    }

    fn ready(&self, spell: Spell, now: f64) -> bool {
        now >= self.ready_at[spell.index()] && self.soul + 1e-4 >= spell.soul()
    }

    /// The spell that fits `s` right now, if any (and pay for it).
    pub fn choose(&mut self, s: &SpellSituation, now: f64, cfg: &SpellConfig) -> Option<Spell> {
        let pick = self.candidate(s, now, cfg)?;
        self.soul = (self.soul - pick.soul()).max(0.0);
        self.last_special = now;
        let cd = match pick {
            Spell::GreatSlash | Spell::Cyclone | Spell::DashSlash => cfg.nail_art_cooldown,
            Spell::Thorns => 4.0,
            _ => cfg.min_gap,
        };
        self.ready_at[pick.index()] = now + cd as f64;
        Some(pick)
    }

    fn candidate(&self, s: &SpellSituation, now: f64, cfg: &SpellConfig) -> Option<Spell> {
        if s.fight_over {
            return (s.hurt > cfg.focus_hurt && s.grounded && self.ready(Spell::Focus, now)).then_some(Spell::Focus);
        }
        if s.just_hit {
            // Thorns are a reaction, not a choice: no gap rule.
            let near = s.target.is_none_or(|t| Spell::Thorns.reaches(t, s.scale));
            return (near && self.ready(Spell::Thorns, now) && s.rolls[0] < cfg.thorns_chance).then_some(Spell::Thorns);
        }
        let off = s.target?;
        if !s.grounded || ((now - self.last_special) as f32) < cfg.min_gap {
            return None;
        }
        // The moment calls for it: no roll.
        if let Some(sp) = s.cues.called_for(off, s.scale).filter(|sp| self.ready(*sp, now)) {
            return Some(sp);
        }
        if s.rolls[0] >= cfg.chance {
            return None;
        }
        // Most specific first: the situation decides, the roll only breaks ties.
        let order: &[Spell] = if s.rolls[1] < 0.5 {
            &[Spell::Scream, Spell::DashSlash, Spell::Fireball, Spell::GreatSlash, Spell::Cyclone]
        } else {
            &[Spell::Scream, Spell::DashSlash, Spell::Fireball, Spell::Cyclone, Spell::GreatSlash]
        };
        order.iter().copied().find(|sp| sp.reaches(off, s.scale) && self.ready(*sp, now))
    }
}
