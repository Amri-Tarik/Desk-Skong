//! How the user handles him ([`super::Poke`]s), and what he makes of it.
//!
//! * **Clicks** are remembered for [`CLICK_MEMORY`] s (his irritation fades with them):
//!   the first one makes him turn and look at the cursor, a second one earns an anger mark,
//!   and [`PROVOKE_CLICKS`] within [`PROVOKE_WINDOW`] s wear his patience out: he picks a
//!   fight with the cursor ([`Handling::provoked`]). Typing fast, the user isn't aiming at
//!   him: he lets [`BUSY_PATIENCE`] more clicks go before each step.
//! * **Shaken** while held (rapid back-and-forth drags): dizzy once he lands.
//! * **Thrown** [`GRUMPY_THROWS`] times within [`THROW_MEMORY`] s: grumpy and wary of the
//!   cursor for [`WARY_SECS`] s (keeps his distance; the mischief it leaves him with comes
//!   out later as pestering).
//! * **Set down gently**: nothing to complain about; now and then a small heart.
//!
//! Battles and pestering read their own clicks (hits from the cursor); this only covers
//! clicks outside a cursor fight. Emotes go through [`super::Intent::emote`] with the usual
//! cooldowns (one anger mark per 30 s at most).
use crate::geom::Vec2;

use super::expression::Expression;

/// Clicks are remembered this long (s): the look / anger ladder climbs within it.
pub const CLICK_MEMORY: f32 = 25.0;
/// Clicks remembered at which he shows an anger mark (the first one only gets a look).
pub const IRKED_CLICKS: usize = 2;
/// This many clicks within [`PROVOKE_WINDOW`] s: he loses patience and fights the cursor.
pub const PROVOKE_CLICKS: usize = 5;
pub const PROVOKE_WINDOW: f32 = 8.0;
/// Extra clicks tolerated at each step while the user types fast.
pub const BUSY_PATIENCE: usize = 2;
/// A provocation stands this long (s), waiting for a higher priority to finish or the user
/// to stop typing; then he lets it go.
pub const PROVOKED_SECS: f32 = 15.0;
/// Utility of a provoked cursor battle: above every pastime and past the commitment wait
/// (it is >= `BrainConfig::urgent`), but still an ambient behaviour, so the Shade, fights,
/// visitors, Geo and a critical need all come first.
pub const PROVOKED_UTILITY: f32 = 1.05;
/// He turns to look at the cursor for this long after a click (s).
pub const LOOK_SECS: f32 = 1.5;

/// A shake: this many direction reversals of at least [`SHAKE_SWING`] px within
/// [`SHAKE_WINDOW`] s while held.
pub const SHAKE_REVERSALS: usize = 4;
pub const SHAKE_WINDOW: f32 = 2.0;
pub const SHAKE_SWING: f32 = 40.0;
/// After a shake he has this long (s) to land and reel; a higher priority may let it pass.
pub const DIZZY_PENDING: f32 = 5.0;
/// Utility of reeling (urgent, below hiding, fleeing and a knock-out).
pub const DIZZY_UTILITY: f32 = 1.15;

/// A release faster than this is a throw (px/s).
pub const THROW_SPEED: f32 = 1200.0;
/// This many throws within [`THROW_MEMORY`] s make him grumpy and wary for [`WARY_SECS`] s.
pub const GRUMPY_THROWS: usize = 3;
pub const THROW_MEMORY: f32 = 60.0;
pub const WARY_SECS: f32 = 90.0;
/// Set down slower than this (px/s), after being held at least [`GENTLE_HOLD`] s: gentle.
pub const GENTLE_SPEED: f32 = 250.0;
pub const GENTLE_HOLD: f32 = 0.3;
/// Chance of a small heart after a gentle carry (when he isn't cross anyway).
pub const CONTENT_CHANCE: f32 = 0.2;

/// What a click outside a fight amounts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    /// He turns to look.
    Look,
    /// A look and an anger mark.
    Irked,
    /// That's enough: a fight with the cursor.
    Provoked,
}

/// How he was set down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    /// Shaken about: dizzy once he lands.
    Shaken,
    /// Thrown; `grumpy` when it was one throw too many.
    Thrown { grumpy: bool },
    /// Picked up and set down softly.
    Gentle,
    /// Anything else (a short lift, a toss).
    Plain,
}

/// Direction reversals along one axis while held.
#[derive(Clone, Debug, Default)]
struct Swing {
    extreme: Option<f32>,
    dir: f32,
}

impl Swing {
    /// `true` when this sample reverses the direction after a swing of [`SHAKE_SWING`] px.
    fn track(&mut self, v: f32) -> bool {
        let Some(ext) = self.extreme else {
            self.extreme = Some(v);
            return false;
        };
        if self.dir == 0.0 {
            if (v - ext).abs() >= SHAKE_SWING {
                self.dir = (v - ext).signum();
                self.extreme = Some(v);
            }
            return false;
        }
        if (v - ext) * self.dir > 0.0 {
            self.extreme = Some(v);
            false
        } else if (ext - v) * self.dir >= SHAKE_SWING {
            self.dir = -self.dir;
            self.extreme = Some(v);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Handling {
    clicks: Vec<f64>,
    throws: Vec<f64>,
    /// Look at the cursor (or this click) until then.
    look: Option<(Vec2, f64)>,
    provoked_until: f64,
    wary_until: f64,
    dizzy_until: f64,
    /// While held: when he was picked up, the swings, and the most reversals in a window.
    held_at: Option<f64>,
    swings: [Swing; 2],
    reversals: Vec<f64>,
    shake_peak: usize,
    emote: Option<Expression>,
}

impl Default for Handling {
    fn default() -> Self {
        Self {
            clicks: Vec::new(),
            throws: Vec::new(),
            look: None,
            provoked_until: f64::NEG_INFINITY,
            wary_until: f64::NEG_INFINITY,
            dizzy_until: f64::NEG_INFINITY,
            held_at: None,
            swings: Default::default(),
            reversals: Vec::new(),
            shake_peak: 0,
            emote: None,
        }
    }
}

impl Handling {
    /// A click outside a cursor fight. `busy`: the user is typing fast.
    pub fn click(&mut self, now: f64, pos: Vec2, busy: bool) -> Click {
        self.clicks.retain(|t| ((now - t) as f32) < CLICK_MEMORY);
        self.clicks.push(now);
        self.look = Some((pos, now + LOOK_SECS as f64));
        let patience = if busy { BUSY_PATIENCE } else { 0 };
        let quick = self.clicks.iter().filter(|t| ((now - **t) as f32) < PROVOKE_WINDOW).count();
        if quick >= PROVOKE_CLICKS + patience {
            self.clicks.clear();
            self.provoked_until = now + PROVOKED_SECS as f64;
            self.emote = Some(Expression::Angry);
            Click::Provoked
        } else if self.clicks.len() >= IRKED_CLICKS + patience {
            self.emote = Some(Expression::Angry);
            Click::Irked
        } else {
            Click::Look
        }
    }

    /// Clicks he still remembers.
    #[cfg(test)]
    pub fn clicks(&self, now: f64) -> usize {
        self.clicks.iter().filter(|t| ((now - **t) as f32) < CLICK_MEMORY).count()
    }

    /// Show this bubble at the end of the tick (cooldowns permitting).
    pub fn emote(&mut self, e: Expression) {
        self.emote = Some(e);
    }

    pub fn take_emote(&mut self) -> Option<Expression> {
        self.emote.take()
    }

    /// Where he is looking after a click, while it lasts.
    pub fn look(&self, now: f64) -> Option<Vec2> {
        self.look.filter(|(_, until)| now < *until).map(|(at, _)| at)
    }

    /// He has had enough of the clicking: fight the cursor (when nothing more important
    /// is going on).
    pub fn provoked(&self, now: f64) -> bool {
        now < self.provoked_until
    }

    /// The provoked fight started (or he let it go).
    pub fn settle_provocation(&mut self) {
        self.provoked_until = f64::NEG_INFINITY;
    }

    /// Grumpy after being thrown around: keeps his distance from the cursor.
    pub fn wary(&self, now: f64) -> bool {
        now < self.wary_until
    }

    /// Shaken up and not yet reeled (see [`DIZZY_PENDING`]).
    pub fn dizzy(&self, now: f64) -> bool {
        now < self.dizzy_until
    }

    pub fn reeling(&mut self) {
        self.dizzy_until = f64::NEG_INFINITY;
    }

    pub fn grabbed(&mut self, now: f64) {
        self.held_at = Some(now);
        self.swings = Default::default();
        self.reversals.clear();
        self.shake_peak = 0;
    }

    /// While held: where he is (the user's drag).
    pub fn held(&mut self, now: f64, pos: Vec2) {
        let x = self.swings[0].track(pos.x);
        let y = self.swings[1].track(pos.y);
        if x || y {
            self.reversals.push(now);
        }
        self.reversals.retain(|t| ((now - t) as f32) <= SHAKE_WINDOW);
        self.shake_peak = self.shake_peak.max(self.reversals.len());
    }

    /// Let go with velocity `vel`.
    pub fn released(&mut self, now: f64, vel: Vec2) -> Release {
        let held_for = self.held_at.take().map_or(0.0, |t| (now - t) as f32);
        if std::mem::take(&mut self.shake_peak) >= SHAKE_REVERSALS {
            self.dizzy_until = now + DIZZY_PENDING as f64;
            return Release::Shaken;
        }
        let speed = vel.length();
        if speed > THROW_SPEED {
            self.throws.retain(|t| ((now - t) as f32) < THROW_MEMORY);
            self.throws.push(now);
            let grumpy = self.throws.len() >= GRUMPY_THROWS;
            if grumpy {
                self.throws.clear();
                self.wary_until = now + WARY_SECS as f64;
                self.emote = Some(Expression::Angry);
            }
            return Release::Thrown { grumpy };
        }
        if speed < GENTLE_SPEED && held_for >= GENTLE_HOLD { Release::Gentle } else { Release::Plain }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    #[test]
    fn the_click_ladder_climbs_and_fades() {
        let mut h = Handling::default();
        let at = Vec2::new(10.0, 10.0);
        assert_eq!(h.click(0.0, at, false), Click::Look);
        assert_eq!(h.take_emote(), None);
        assert_eq!(h.look(1.0), Some(at));
        assert_eq!(h.look(2.0), None);
        assert_eq!(h.click(3.0, at, false), Click::Irked);
        assert_eq!(h.take_emote(), Some(Expression::Angry));
        // Forgotten after a while: back to a look.
        assert_eq!(h.click(40.0, at, false), Click::Look);
        // Five quick ones: provoked.
        for t in [41.0, 42.0, 43.0] {
            assert_eq!(h.click(t, at, false), Click::Irked);
        }
        assert_eq!(h.click(44.0, at, false), Click::Provoked);
        assert!(h.provoked(44.0 + PROVOKED_SECS as f64 - 0.1));
        assert!(!h.provoked(44.0 + PROVOKED_SECS as f64 + 0.1), "the provocation lapses");
        assert_eq!(h.clicks(44.0), 0, "a fresh start after a provocation");
    }

    #[test]
    fn slow_clicks_never_provoke() {
        let mut h = Handling::default();
        for i in 0..20 {
            let got = h.click(i as f64 * 3.0, Vec2::ZERO, false);
            assert_ne!(got, Click::Provoked, "click {i}");
        }
    }

    #[test]
    fn typing_buys_patience() {
        let mut h = Handling::default();
        assert_eq!(h.click(0.0, Vec2::ZERO, true), Click::Look);
        assert_eq!(h.click(0.5, Vec2::ZERO, true), Click::Look, "busy: no anger yet");
        for t in [1.0, 1.5, 2.0, 2.5] {
            assert_ne!(h.click(t, Vec2::ZERO, true), Click::Provoked);
        }
        assert_eq!(h.click(3.0, Vec2::ZERO, true), Click::Provoked, "7 quick clicks while typing");
    }

    fn hold(h: &mut Handling, from: f64, secs: f64, path: impl Fn(f64) -> Vec2) -> f64 {
        let n = (secs / DT) as usize;
        for i in 0..n {
            let t = from + i as f64 * DT;
            h.held(t, path(t - from));
        }
        from + secs
    }

    #[test]
    fn a_shake_makes_him_dizzy_a_carry_does_not() {
        let mut h = Handling::default();
        h.grabbed(0.0);
        // Back and forth, 100 px, ~3 times a second.
        let end = hold(&mut h, 0.0, 1.5, |t| Vec2::new(500.0 + 100.0 * (t * 18.0).sin() as f32, 300.0));
        assert_eq!(h.released(end, Vec2::ZERO), Release::Shaken);
        assert!(h.dizzy(end + 1.0));
        h.reeling();
        assert!(!h.dizzy(end + 1.0));

        // A long carry across the screen, a little wobble: gentle.
        h.grabbed(10.0);
        let end = hold(&mut h, 10.0, 2.0, |t| Vec2::new(200.0 + 400.0 * t as f32, 300.0 + 10.0 * (t * 20.0).sin() as f32));
        assert_eq!(h.released(end, Vec2::new(60.0, 0.0)), Release::Gentle);
        assert!(!h.dizzy(end));

        // One slow sway back and forth isn't a shake either.
        h.grabbed(20.0);
        let end = hold(&mut h, 20.0, 3.0, |t| Vec2::new(500.0 + 150.0 * (t * 2.0).sin() as f32, 300.0));
        assert_ne!(h.released(end, Vec2::ZERO), Release::Shaken);
    }

    #[test]
    fn three_throws_in_a_minute_make_him_wary() {
        let mut h = Handling::default();
        let fling = Vec2::new(2000.0, -500.0);
        let throw = |h: &mut Handling, t: f64| {
            h.grabbed(t);
            h.released(t + 0.5, fling)
        };
        assert_eq!(throw(&mut h, 0.0), Release::Thrown { grumpy: false });
        assert_eq!(throw(&mut h, 70.0), Release::Thrown { grumpy: false }, "the first one is forgotten");
        assert_eq!(throw(&mut h, 80.0), Release::Thrown { grumpy: false });
        assert!(!h.wary(81.0));
        assert_eq!(throw(&mut h, 90.0), Release::Thrown { grumpy: true });
        assert_eq!(h.take_emote(), Some(Expression::Angry));
        assert!(h.wary(90.5 + WARY_SECS as f64 - 1.0));
        assert!(!h.wary(90.5 + WARY_SECS as f64 + 1.0));
    }

    #[test]
    fn a_quick_lift_is_neither_gentle_nor_a_throw() {
        let mut h = Handling::default();
        h.grabbed(0.0);
        assert_eq!(h.released(0.1, Vec2::ZERO), Release::Plain);
        h.grabbed(1.0);
        assert_eq!(h.released(2.0, Vec2::new(600.0, 0.0)), Release::Plain);
    }
}
