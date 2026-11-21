//! When visitors come: rarely and organically. The countdown only runs while the user is
//! at the computer; a due visit waits for a good moment (no fullscreen app, no fast
//! typing, no visitor around, the knight not asleep or held). A blocked visit stays due
//! and comes as soon as the moment is good (within `retry`), so the longest wait between
//! visits is the top of the `gap` range in presence time plus the time it was blocked.
use serde::Deserialize;

use crate::ai::rng::Rng;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct VisitConfig {
    pub enabled: bool,
    /// Seconds of presence before the first visit of a session (random in range).
    pub first: [f32; 2],
    /// Seconds of presence between visits (random in range).
    pub gap: [f32; 2],
    /// Typing at least this fast (keys/s) postpones a visit.
    pub busy_keys_per_sec: f32,
    /// Idle longer than this (s) = user away: the clock stops.
    pub away_secs: f32,
    /// A postponed visit re-checks after this long (s).
    pub retry: f32,
}

impl Default for VisitConfig {
    fn default() -> Self {
        Self { enabled: true, first: [420.0, 1200.0], gap: [600.0, 2400.0], busy_keys_per_sec: 2.5, away_secs: 90.0, retry: 20.0 }
    }
}

/// The world right now, as far as visits care.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisitConditions {
    pub user_idle_secs: f32,
    pub keys_per_sec: f32,
    pub fullscreen: bool,
    /// A visitor is already around.
    pub visitor_present: bool,
    /// The knight can't receive anyone (asleep, held, hiding...).
    pub knight_busy: bool,
}

#[derive(Clone, Debug)]
pub struct VisitScheduler {
    pub cfg: VisitConfig,
    /// Presence-seconds until the next visit is due.
    next_in: f32,
    retry_in: f32,
}

impl VisitScheduler {
    pub fn new(cfg: VisitConfig, rng: &mut Rng) -> Self {
        let next_in = rng.range(cfg.first[0], cfg.first[1].max(cfg.first[0]));
        Self { cfg, next_in, retry_in: 0.0 }
    }

    /// Seconds of presence until the next visit is due (0 = due, waiting for a moment).
    pub fn due_in(&self) -> f32 {
        self.next_in.max(0.0)
    }

    /// Why a due visit can't happen now, if anything.
    pub fn blocked(&self, c: &VisitConditions) -> Option<&'static str> {
        if !self.cfg.enabled {
            Some("disabled")
        } else if c.visitor_present {
            Some("visitor present")
        } else if c.fullscreen {
            Some("fullscreen app")
        } else if c.user_idle_secs >= self.cfg.away_secs {
            Some("user away")
        } else if c.keys_per_sec >= self.cfg.busy_keys_per_sec {
            Some("user typing")
        } else if c.knight_busy {
            Some("knight busy")
        } else {
            None
        }
    }

    /// Advance; `true` when a visit should start now (then the next one is scheduled).
    pub fn tick(&mut self, dt: f32, c: &VisitConditions, rng: &mut Rng) -> bool {
        let present = c.user_idle_secs < self.cfg.away_secs;
        if !self.cfg.enabled || c.visitor_present || !present {
            return false; // the clock only runs while someone is there to see it
        }
        self.next_in -= dt;
        if self.next_in > 0.0 {
            return false;
        }
        self.retry_in -= dt;
        if self.retry_in > 0.0 {
            return false;
        }
        if self.blocked(c).is_some() {
            self.retry_in = self.cfg.retry;
            return false;
        }
        self.schedule_next(rng);
        true
    }

    /// A due visit fell through (nobody eligible, or no spot fit right now): stay due and
    /// try again after `retry`, rather than restarting the whole gap [`Self::tick`] just drew.
    pub fn postpone(&mut self) {
        self.next_in = 0.0;
        self.retry_in = self.cfg.retry;
    }

    /// Restart the countdown (after a visit, also a forced one).
    pub fn schedule_next(&mut self, rng: &mut Rng) {
        self.next_in = rng.range(self.cfg.gap[0], self.cfg.gap[1].max(self.cfg.gap[0]));
        self.retry_in = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> VisitConfig {
        VisitConfig { enabled: true, first: [100.0, 100.0], gap: [300.0, 300.0], busy_keys_per_sec: 2.5, away_secs: 60.0, retry: 10.0 }
    }

    fn run(s: &mut VisitScheduler, secs: f32, c: VisitConditions, rng: &mut Rng) -> Option<f32> {
        let mut t = 0.0;
        while t < secs {
            t += 0.5;
            if s.tick(0.5, &c, rng) {
                return Some(t);
            }
        }
        None
    }

    #[test]
    fn first_visit_after_presence_then_gap() {
        let mut rng = Rng::new(1);
        let mut s = VisitScheduler::new(cfg(), &mut rng);
        let here = VisitConditions { user_idle_secs: 3.0, ..Default::default() };
        assert_eq!(run(&mut s, 200.0, here, &mut rng), Some(100.0));
        assert_eq!(run(&mut s, 400.0, here, &mut rng), Some(300.0));
    }

    #[test]
    fn clock_stops_while_away_or_visited() {
        let mut rng = Rng::new(1);
        let mut s = VisitScheduler::new(cfg(), &mut rng);
        let away = VisitConditions { user_idle_secs: 600.0, ..Default::default() };
        assert_eq!(run(&mut s, 1000.0, away, &mut rng), None);
        assert_eq!(s.due_in(), 100.0);
        let busy = VisitConditions { user_idle_secs: 1.0, visitor_present: true, ..Default::default() };
        assert_eq!(run(&mut s, 1000.0, busy, &mut rng), None);
        assert_eq!(s.due_in(), 100.0);
    }

    #[test]
    fn due_visit_waits_for_a_good_moment() {
        let mut rng = Rng::new(1);
        let mut s = VisitScheduler::new(cfg(), &mut rng);
        let typing = VisitConditions { user_idle_secs: 0.1, keys_per_sec: 6.0, ..Default::default() };
        assert_eq!(s.blocked(&typing), Some("user typing"));
        assert_eq!(run(&mut s, 500.0, typing, &mut rng), None);
        let fullscreen = VisitConditions { user_idle_secs: 0.1, fullscreen: true, ..Default::default() };
        assert_eq!(run(&mut s, 100.0, fullscreen, &mut rng), None);
        // Calm again: within one retry period.
        let calm = VisitConditions { user_idle_secs: 2.0, ..Default::default() };
        let t = run(&mut s, 100.0, calm, &mut rng).expect("visit");
        assert!(t <= 10.5, "{t}");
    }

    #[test]
    fn long_block_then_one_prompt_visit_not_a_burst() {
        let mut rng = Rng::new(3);
        let mut s = VisitScheduler::new(cfg(), &mut rng);
        // Half an hour of fullscreen (the clock runs, the visit is overdue and waits).
        let fullscreen = VisitConditions { user_idle_secs: 0.1, fullscreen: true, ..Default::default() };
        assert_eq!(run(&mut s, 1800.0, fullscreen, &mut rng), None);
        assert_eq!(s.due_in(), 0.0);
        // Free again: it comes within one retry, not after a fresh gap...
        let calm = VisitConditions { user_idle_secs: 2.0, ..Default::default() };
        assert!(run(&mut s, 30.0, calm, &mut rng).expect("visit") <= 10.5);
        // ...and only one: the overdue time doesn't stack visits back to back.
        assert_eq!(s.due_in(), 300.0);
        assert_eq!(run(&mut s, 299.0, calm, &mut rng), None);
    }

    #[test]
    fn a_visit_that_fell_through_stays_due() {
        let mut rng = Rng::new(4);
        let mut s = VisitScheduler::new(cfg(), &mut rng);
        let calm = VisitConditions { user_idle_secs: 2.0, ..Default::default() };
        assert_eq!(run(&mut s, 200.0, calm, &mut rng), Some(100.0));
        // The app found no spot for anyone: try again shortly, not after a whole gap.
        s.postpone();
        assert_eq!(s.due_in(), 0.0);
        let t = run(&mut s, 60.0, calm, &mut rng).expect("retried");
        assert!((9.5..=10.5).contains(&t), "{t}");
    }

    #[test]
    fn disabled_never_visits() {
        let mut rng = Rng::new(1);
        let mut s = VisitScheduler::new(VisitConfig { enabled: false, ..cfg() }, &mut rng);
        assert_eq!(run(&mut s, 10_000.0, VisitConditions { user_idle_secs: 1.0, ..Default::default() }, &mut rng), None);
    }
}
