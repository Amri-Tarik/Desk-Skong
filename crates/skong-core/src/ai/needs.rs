//! Internal state: slow **needs** (hours/minutes) and fast **emotions** (seconds), plus
//! the mood-derived choices that colour every behaviour (travel style, act tags).
use crate::nav::TravelStyle;

use super::config::{BrainConfig, Personality};

/// Slow drives, each 0..1 where high means "wants".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Needs {
    /// 1 = fully rested, 0 = exhausted.
    pub energy: f32,
    /// Wants something new to look at.
    pub boredom: f32,
    /// Wants to be near the user (cursor).
    pub loneliness: f32,
    /// Wants to bother the user. Never builds when `bother_level` is 0.
    pub mischief: f32,
}

impl Default for Needs {
    fn default() -> Self {
        Self { energy: 0.85, boredom: 0.3, loneliness: 0.3, mischief: 0.1 }
    }
}

/// Fast emotions, each 0..1, decaying exponentially toward 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Emotions {
    pub startle: f32,
    pub annoyance: f32,
    pub excitement: f32,
    /// Bruised pride after losing a fight / being thrown around.
    pub hurt: f32,
}

/// Half-lives (s) of [`Emotions`].
const HALF_LIFE_STARTLE: f32 = 4.0;
const HALF_LIFE_ANNOYANCE: f32 = 30.0;
const HALF_LIFE_EXCITEMENT: f32 = 20.0;
const HALF_LIFE_HURT: f32 = 60.0;

/// What the knight is doing this tick, as far as need dynamics care.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Activity {
    pub asleep: bool,
    /// Sitting (bench or perch): recovers energy slowly.
    pub seated: bool,
    /// 0 = idle, 1 = sprinting / fighting.
    pub exertion: f32,
    /// Near a present user's cursor.
    pub with_user: bool,
}

/// Needs + emotions + personality.
#[derive(Clone, Debug, PartialEq)]
pub struct Mind {
    pub personality: Personality,
    pub needs: Needs,
    pub emotions: Emotions,
}

/// Mood tags used by `data/acts.json`.
pub const MOOD_TAGS: [&str; 8] = ["calm", "curious", "playful", "bold", "tired", "scared", "hurt", "sad"];

impl Mind {
    pub fn new(personality: Personality) -> Self {
        Self { personality, needs: Needs::default(), emotions: Emotions::default() }
    }

    pub(crate) fn update(&mut self, dt: f32, act: Activity, user_away: bool, cfg: &BrainConfig) {
        let per_min = dt / 60.0;
        let p = self.personality;
        let r = &cfg.needs;
        let n = &mut self.needs;

        if act.asleep {
            n.energy += r.energy_restore * per_min;
        } else if act.seated {
            n.energy += 0.35 * r.energy_restore * per_min;
        } else {
            let stamina = 1.3 - 0.6 * p.vigour;
            n.energy -= r.energy_drain * (1.0 + 2.0 * act.exertion) * stamina * per_min;
        }
        if !act.asleep {
            n.boredom += r.boredom_rise * (0.4 + p.curiosity) * per_min;
        }
        if act.with_user {
            n.loneliness -= r.loneliness_relief * per_min;
        } else {
            let pace = if user_away { 1.5 } else { 1.0 };
            n.loneliness += r.loneliness_rise * (0.4 + p.sociability) * pace * per_min;
        }
        let bother = cfg.bother_level.clamp(0.0, 1.0);
        if bother <= 0.0 {
            n.mischief = 0.0;
        } else if !act.asleep && !user_away {
            n.mischief += r.mischief_rise * (0.3 + p.playfulness) * bother * per_min;
        }
        for v in [&mut n.energy, &mut n.boredom, &mut n.loneliness, &mut n.mischief] {
            *v = v.clamp(0.0, 1.0);
        }

        let e = &mut self.emotions;
        for (v, hl) in [
            (&mut e.startle, HALF_LIFE_STARTLE),
            (&mut e.annoyance, HALF_LIFE_ANNOYANCE),
            (&mut e.excitement, HALF_LIFE_EXCITEMENT),
            (&mut e.hurt, HALF_LIFE_HURT),
        ] {
            *v = (*v * 0.5f32.powf(dt / hl)).clamp(0.0, 1.0);
        }
    }

    /// Scare the knight: scaled by skittishness, softened by boldness.
    pub fn startle(&mut self, amount: f32) {
        let p = self.personality;
        let gain = (0.6 + p.skittishness - 0.4 * p.boldness).clamp(0.3, 1.5);
        self.emotions.startle = (self.emotions.startle + amount * gain).clamp(0.0, 1.0);
    }

    pub fn excite(&mut self, amount: f32) {
        self.emotions.excitement = (self.emotions.excitement + amount).clamp(0.0, 1.0);
    }

    pub fn annoy(&mut self, amount: f32) {
        self.emotions.annoyance = (self.emotions.annoyance + amount).clamp(0.0, 1.0);
    }

    pub fn hurt(&mut self, amount: f32) {
        self.emotions.hurt = (self.emotions.hurt + amount).clamp(0.0, 1.0);
    }

    /// Satisfy (negative) or feed (positive) a need.
    pub fn adjust(&mut self, need: impl FnOnce(&mut Needs) -> &mut f32, delta: f32) {
        let v = need(&mut self.needs);
        *v = (*v + delta).clamp(0.0, 1.0);
    }

    /// 0 = serene, 1 = wired.
    pub fn arousal(&self) -> f32 {
        let e = self.emotions;
        (e.excitement + 0.7 * e.startle + 0.3 * e.annoyance + 0.15 * (self.personality.vigour - 0.5)).clamp(0.0, 1.0)
    }

    pub fn tiredness(&self) -> f32 {
        1.0 - self.needs.energy
    }

    /// Mood → how to travel: calm knights stroll, excited ones hurry, exhausted ones
    /// never run.
    pub fn travel_style(&self) -> TravelStyle {
        if self.needs.energy < 0.2 {
            return TravelStyle::Stroll;
        }
        let a = self.arousal() - 0.25 * (self.tiredness() - 0.3).max(0.0);
        if a >= 0.55 {
            TravelStyle::Hurry
        } else if a >= 0.25 {
            TravelStyle::Normal
        } else {
            TravelStyle::Stroll
        }
    }

    /// How much each mood tag fits right now (for weighting acts).
    pub fn tag_weight(&self, tag: &str, user_away: bool) -> f32 {
        let n = self.needs;
        let e = self.emotions;
        let p = self.personality;
        match tag {
            "calm" => 0.25 + 0.6 * (1.0 - self.arousal()),
            "curious" => 0.2 + n.boredom * (0.5 + p.curiosity),
            "playful" => 0.1 + n.mischief + 0.5 * e.excitement,
            "bold" => 0.5 * p.boldness + 0.4 * e.excitement,
            "tired" => 1.5 * self.tiredness().powi(2),
            "scared" => 2.0 * e.startle,
            "hurt" => 2.0 * e.hurt,
            "sad" => {
                if user_away {
                    0.8 * n.loneliness
                } else {
                    0.2 * n.loneliness * n.loneliness
                }
            }
            _ => 0.0,
        }
    }

    /// The dominant mood tag (for debug overlays).
    pub fn dominant_mood(&self, user_away: bool) -> &'static str {
        MOOD_TAGS
            .iter()
            .copied()
            .map(|t| (t, self.tag_weight(t, user_away)))
            .fold(("calm", f32::MIN), |best, cur| if cur.1 > best.1 { cur } else { best })
            .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_drift_and_emotions_decay() {
        let cfg = BrainConfig::default();
        let mut m = Mind::new(Personality::default());
        m.emotions.startle = 1.0;
        let e0 = m.needs.energy;
        let b0 = m.needs.boredom;
        for _ in 0..600 {
            m.update(0.1, Activity::default(), false, &cfg);
        }
        assert!(m.needs.energy < e0);
        assert!(m.needs.boredom > b0);
        assert!(m.emotions.startle < 0.01);
    }

    #[test]
    fn sleep_restores_energy() {
        let cfg = BrainConfig::default();
        let mut m = Mind::new(Personality::default());
        m.needs.energy = 0.1;
        for _ in 0..1200 {
            m.update(0.1, Activity { asleep: true, ..Default::default() }, true, &cfg);
        }
        assert!(m.needs.energy > 0.3);
    }

    #[test]
    fn no_mischief_without_bother() {
        let cfg = BrainConfig { bother_level: 0.0, ..Default::default() };
        let mut m = Mind::new(Personality { playfulness: 1.0, ..Default::default() });
        m.needs.mischief = 0.8;
        m.update(0.1, Activity::default(), false, &cfg);
        assert_eq!(m.needs.mischief, 0.0);
    }

    #[test]
    fn style_follows_arousal_and_energy() {
        let mut m = Mind::new(Personality::default());
        assert_eq!(m.travel_style(), TravelStyle::Stroll);
        m.emotions.excitement = 0.9;
        assert_eq!(m.travel_style(), TravelStyle::Hurry);
        m.needs.energy = 0.1;
        assert_eq!(m.travel_style(), TravelStyle::Stroll);
    }
}
