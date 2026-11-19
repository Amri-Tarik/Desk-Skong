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

/// Cursor-companion tuning (the default behaviour when nothing else is going on).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompanionConfig {
    /// Comfortable distance from the cursor's projection at `bother_level` 1 (px).
    pub comfort: f32,
    /// ... and at `bother_level` 0: keeps a polite distance.
    pub polite_comfort: f32,
    /// Stand this far beside the projection rather than right under the cursor (bother 1 / 0).
    pub beside: f32,
    pub polite_beside: f32,
    /// The cursor must stay out of comfort range this long before he follows (s).
    pub drift_secs: f32,
    /// Hurry (instead of stroll) when the goal is farther than this (px).
    pub hurry_beyond: f32,
    /// Seconds between contextual acts while keeping company.
    pub gap: [f32; 2],
    /// Cursor resting this long → settle down and sit nearby (s).
    pub settle_secs: f32,
    /// Look for a seat within this distance of the cursor projection when settling (px).
    pub seat_range: f32,
    /// Minimum seconds between two `stare_at_user`.
    pub stare_cooldown: f32,
    /// Chance that a gap ends in a plain fidget instead of a contextual act.
    pub fidget_chance: f32,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            comfort: 240.0,
            polite_comfort: 450.0,
            beside: 70.0,
            polite_beside: 220.0,
            drift_secs: 0.6,
            hurry_beyond: 900.0,
            gap: [1.0, 3.0],
            settle_secs: 5.0,
            seat_range: 500.0,
            stare_cooldown: 25.0,
            fidget_chance: 0.5,
        }
    }
}

/// Activity-watching tuning.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct WatchConfig {
    /// Watching time for a barely interesting / very interesting spot (s); satiation
    /// fills over this budget.
    pub min_secs: f32,
    pub max_secs: f32,
    /// Spot quiet this long ends the watch (s).
    pub quiet_secs: f32,
    /// Sit down (when perched / at a bench) once a long-lived spot was watched this long (s).
    pub sit_after: f32,
    /// Seconds between watching acts.
    pub gap: [f32; 2],
    /// Spots weaker than this are ignored.
    pub min_intensity: f32,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self { min_secs: 4.0, max_secs: 15.0, quiet_secs: 1.5, sit_after: 5.0, gap: [0.3, 1.0], min_intensity: 0.2 }
    }
}

/// Priority-tier tuning (see [`super::Tier`]).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct TierConfig {
    /// Utilities at or below this don't count as eligible for their tier.
    pub eligible: f32,
    /// Energy below this is a critical need: Rest / Nap rise to the Needs tier.
    pub critical_energy: f32,
    /// Hurt above this is a critical need: Recover rises to the Needs tier.
    pub critical_hurt: f32,
    /// While a need is critical, loot only stays an engagement within this distance (px):
    /// a coin next to him beats sitting, a far one can wait for the nap.
    pub tired_loot_reach: f32,
    /// A behaviour only competes in the Engagement / Needs tier when its utility reaches this;
    /// below it, it competes as Ambient. Keeps a merely *possible* cursor battle (low
    /// baseline) from outranking everything, while an invited one still preempts.
    pub engagement_min: f32,
    pub needs_min: f32,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self { eligible: 0.05, critical_energy: 0.2, critical_hurt: 0.6, tired_loot_reach: 300.0, engagement_min: 0.35, needs_min: 0.3 }
    }
}

/// All brain tunables.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct BrainConfig {
    /// 0 = never bothers the user (no battles, no title-bar squatting), 1 = gremlin.
    pub bother_level: f32,
    /// Retreat and keep quiet whenever *any* fullscreen window is in front. Off by default:
    /// the app keeps the knight away from games itself; other fullscreen apps are fine.
    pub hide_for_fullscreen: bool,
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
    /// Priority tiers.
    pub tiers: TierConfig,
    pub needs: NeedRates,
    pub battle: BattleConfig,
    /// Death, the Shade, Geo and spells.
    pub stakes: super::stakes::StakesConfig,
    pub companion: CompanionConfig,
    pub watch: WatchConfig,
    /// Seconds of plain standing between purposeful actions when idle.
    pub idle_gap: [f32; 2],
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
            hide_for_fullscreen: false,
            personality: None,
            think_interval: 0.25,
            min_commit: 2.0,
            hysteresis: 0.12,
            urgent: 1.0,
            tiers: TierConfig::default(),
            needs: NeedRates::default(),
            battle: BattleConfig::default(),
            stakes: super::stakes::StakesConfig::default(),
            companion: CompanionConfig::default(),
            watch: WatchConfig::default(),
            idle_gap: [1.0, 3.0],
            user_away_secs: 300.0,
            typing_busy_rate: 1.5,
            novelty_half_life: 90.0,
            visit_memory_secs: 600.0,
            travel_cost_per_sec: 0.03,
            max_planned_pois: 4,
        }
    }
}
