//! Tunables. Everything has a sensible default and is serde-deserializable with
//! `#[serde(default)]`, so a partial TOML/JSON file only needs the fields it changes.
use serde::Deserialize;

use super::rng::Rng;

/// Who this knight is. Each trait is 0..1; 0.5 is "average".
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Personality {
    /// How fast boredom builds and how strongly novelty attracts.
    pub curiosity: f32,
    /// How fast the urge to make mischief builds (still gated by `bother_level`).
    pub playfulness: f32,
    /// How much it wants to be near the cursor / the user.
    pub sociability: f32,
    /// Stamina and pep: slower energy drain, readier to run.
    pub vigour: f32,
    /// Seeks high ledges, starts fights, shrugs off scares.
    pub boldness: f32,
    /// How easily it is startled.
    pub skittishness: f32,
}

impl Default for Personality {
    fn default() -> Self {
        Self { curiosity: 0.6, playfulness: 0.5, sociability: 0.5, vigour: 0.5, boldness: 0.5, skittishness: 0.4 }
    }
}

impl Personality {
    /// The default personality with every trait jittered by up to ±0.25, so different
    /// seeds give noticeably different (but never extreme) knights.
    pub fn from_seed(seed: u32) -> Self {
        let mut rng = Rng::new(seed ^ 0x5EED_CAFE);
        let mut jitter = |v: f32| (v + rng.range(-0.25, 0.25)).clamp(0.05, 0.95);
        let d = Self::default();
        Self {
            curiosity: jitter(d.curiosity),
            playfulness: jitter(d.playfulness),
            sociability: jitter(d.sociability),
            vigour: jitter(d.vigour),
            boldness: jitter(d.boldness),
            skittishness: jitter(d.skittishness),
        }
    }
}

/// Slow need dynamics, in "fraction of the 0..1 range per minute".
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct NeedRates {
    /// Energy lost per minute while idling (exertion multiplies it up to 3x).
    pub energy_drain: f32,
    /// Energy regained per minute while asleep.
    pub energy_restore: f32,
    /// Boredom gained per minute when nothing new happens (scaled by curiosity).
    pub boredom_rise: f32,
    /// Loneliness gained per minute away from the cursor (scaled by sociability).
    pub loneliness_rise: f32,
    /// Loneliness relieved per minute while near the cursor of a present user.
    pub loneliness_relief: f32,
    /// Mischief gained per minute (scaled by playfulness and `bother_level`).
    pub mischief_rise: f32,
}

impl Default for NeedRates {
    fn default() -> Self {
        Self {
            energy_drain: 1.0 / 45.0,
            energy_restore: 1.0 / 6.0,
            boredom_rise: 0.2,
            loneliness_rise: 0.08,
            loneliness_relief: 0.5,
            mischief_rise: 0.1,
        }
    }
}

/// Cursor battle tuning.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct BattleConfig {
    /// Cursor within this distance of the knight counts as "near" (px).
    pub cursor_near: f32,
    /// Cursor must rest near the knight this long to count as an invitation (s).
    pub park_secs: f32,
    /// Cursor slower than this counts as resting (px/s).
    pub park_speed: f32,
    /// Stop approaching and issue the challenge within this distance (px).
    pub challenge_range: f32,
    /// Slash reach from the body center (px).
    pub attack_range: f32,
    /// Seconds between slashes.
    pub attack_cooldown: f32,
    /// A cursor moving faster than this when slashed dodges the hit (px/s).
    pub dodge_speed: f32,
    /// Knight hits needed to win.
    pub win_hits: u32,
    /// Cursor hits (clicks on the knight) that make the knight lose.
    pub lose_hits: u32,
    /// Two cursor hits within this window stun the knight (s).
    pub stun_window: f32,
    /// Give up after this long (s).
    pub max_duration: f32,
    /// Give up when the cursor stays farther than this for `give_up_secs` (px).
    pub give_up_distance: f32,
    pub give_up_secs: f32,
    /// Minimum seconds between battles at `bother_level` 1 (divided by the level).
    pub cooldown: f32,
}

impl Default for BattleConfig {
    fn default() -> Self {
        Self {
            cursor_near: 320.0,
            park_secs: 1.2,
            park_speed: 40.0,
            challenge_range: 260.0,
            attack_range: 95.0,
            attack_cooldown: 0.45,
            dodge_speed: 1400.0,
            win_hits: 5,
            lose_hits: 3,
            stun_window: 1.5,
            max_duration: 45.0,
            give_up_distance: 900.0,
            give_up_secs: 3.0,
            cooldown: 90.0,
        }
    }
}

/// All brain tunables.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct BrainConfig {
    /// 0 = never bothers the user (no battles, no title-bar squatting), 1 = gremlin.
    pub bother_level: f32,
    /// Fixed personality; `None` derives one from the seed.
    pub personality: Option<Personality>,
    /// Seconds between utility re-evaluations.
    pub think_interval: f32,
    /// Seconds a freshly chosen behaviour is protected from non-urgent challengers.
    pub min_commit: f32,
    /// A challenger must beat the current behaviour's utility by this margin.
    pub hysteresis: f32,
    /// Utilities at or above this are urgent: they bypass commitment.
    pub urgent: f32,
    pub needs: NeedRates,
    pub battle: BattleConfig,
    /// User idle this long = away (pet naps, stops bothering).
    pub user_away_secs: f32,
    /// Keyboard rate at or above this means the user is busy typing (keys/s).
    pub typing_busy_rate: f32,
    /// Event novelty half-life (s).
    pub novelty_half_life: f32,
    /// How long a visited place stays "known" (s).
    pub visit_memory_secs: f32,
    /// Utility lost per second of travel.
    pub travel_cost_per_sec: f32,
    /// Only this many top points of interest are route-planned per evaluation.
    pub max_planned_pois: usize,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            bother_level: 0.5,
            personality: None,
            think_interval: 0.25,
            min_commit: 2.0,
            hysteresis: 0.12,
            urgent: 1.0,
            needs: NeedRates::default(),
            battle: BattleConfig::default(),
            user_away_secs: 300.0,
            typing_busy_rate: 1.5,
            novelty_half_life: 90.0,
            visit_memory_secs: 600.0,
            travel_cost_per_sec: 0.03,
            max_planned_pois: 4,
        }
    }
}
