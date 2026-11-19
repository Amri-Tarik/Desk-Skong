//! The Hollow Shade: what is left where the knight died. It floats and drifts around its
//! home, eyes him when he comes close, and — once he comes to fight it — attacks with a
//! simple pattern: fly in, wind up, lunge-slash, back off. Struck enough, it bursts and
//! gives his Geo back.
//!
//! Clip names refer to the `Hollow Shade` animation library; the app renders them.
use crate::geom::{RectF, Vec2};

use super::Xorshift;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadePhase {
    /// Rising out of the knight's remains (`Appear`).
    Appear,
    /// Floating around home (`Idle`).
    Idle,
    /// Noticed the knight (`Startle`).
    Startle,
    /// Flying at the knight (`Fly`).
    Approach,
    /// Winding up (`Slash Antic`).
    Antic,
    /// Lunging slash (`Slash` + `Slash Effect`).
    Slash,
    /// Recovering (`Slash CD`).
    Cooldown,
    /// Backing off before the next pass (`Fly`).
    Retreat,
    /// Beaten (`Death`), then gone.
    Dying,
    Gone,
}

/// What to draw this tick and what happened.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadeTick {
    pub clip: &'static str,
    /// Overlay clip drawn on top (the slash arc).
    pub effect: Option<&'static str>,
    /// The slash connected with the knight this tick.
    pub hit_knight: bool,
    /// The Shade just burst (defeated).
    pub burst: bool,
}

#[derive(Clone, Debug)]
pub struct Shade {
    /// Where it floats when left alone (body centre, screen px).
    pub home: Vec2,
    pub pos: Vec2,
    /// Geo it carries.
    pub geo: u32,
    pub hp: u32,
    pub facing_right: bool,
    phase: ShadePhase,
    /// Seconds in the current phase.
    t: f32,
    clock: f32,
    wander: Vec2,
    wander_in: f32,
    lunge: Vec2,
    hit_done: bool,
    rng: Xorshift,
    /// Render scale (px per game unit / 64).
    pub scale: f32,
}

/// Hits the knight needs to land (spells count more).
pub const SHADE_HP: u32 = 6;
const APPEAR_SECS: f32 = 2.2;
const STARTLE_SECS: f32 = 0.58;
const ANTIC_SECS: f32 = 0.5;
const SLASH_SECS: f32 = 0.25;
const COOLDOWN_SECS: f32 = 0.45;
const RETREAT_SECS: f32 = 0.7;
const DEATH_SECS: f32 = 0.75;

impl Shade {
    /// A new Shade at `at`; `appear` plays the rising animation (fresh death) instead of
    /// starting idle (loaded from the save).
    pub fn new(at: Vec2, geo: u32, appear: bool, seed: u32) -> Self {
        Self {
            home: at,
            pos: at,
            geo,
            hp: SHADE_HP,
            facing_right: false,
            phase: if appear { ShadePhase::Appear } else { ShadePhase::Idle },
            t: 0.0,
            clock: 0.0,
            wander: Vec2::ZERO,
            wander_in: 0.0,
            lunge: Vec2::ZERO,
            hit_done: false,
            rng: Xorshift::new(seed),
            scale: 1.0,
        }
    }

    pub fn phase(&self) -> ShadePhase {
        self.phase
    }

    pub fn alive(&self) -> bool {
        !matches!(self.phase, ShadePhase::Dying | ShadePhase::Gone)
    }

    /// Can be struck right now.
    pub fn vulnerable(&self) -> bool {
        !matches!(self.phase, ShadePhase::Appear | ShadePhase::Dying | ShadePhase::Gone)
    }

    /// Mid-fight (not just floating around).
    pub fn fighting(&self) -> bool {
        matches!(self.phase, ShadePhase::Startle | ShadePhase::Approach | ShadePhase::Antic | ShadePhase::Slash | ShadePhase::Cooldown | ShadePhase::Retreat)
    }

    fn enter(&mut self, phase: ShadePhase) {
        self.phase = phase;
        self.t = 0.0;
    }

    /// Struck by the knight. Returns `true` when this blow defeats it.
    pub fn strike(&mut self, damage: u32, from: Vec2) -> bool {
        if !self.vulnerable() {
            return false;
        }
        self.hp = self.hp.saturating_sub(damage.max(1));
        let away = (self.pos - from).normalize_or(Vec2::X);
        self.pos += away * 30.0 * self.scale;
        if self.hp == 0 {
            self.enter(ShadePhase::Dying);
            return true;
        }
        if matches!(self.phase, ShadePhase::Idle | ShadePhase::Approach) {
            self.enter(ShadePhase::Retreat);
        }
        false
    }

    /// One step. `knight` = the knight's body centre; `hunted` = he came to fight (the
    /// Shade only attacks then; otherwise it just watches him). `bounds` keeps it on screen.
    pub fn step(&mut self, dt: f32, knight: Option<Vec2>, hunted: bool, bounds: RectF) -> ShadeTick {
        self.t += dt;
        self.clock += dt;
        let s = self.scale.max(0.1);
        let mut out = ShadeTick::default();
        let near = knight.filter(|k| k.distance(self.pos) < 320.0 * s);
        if let Some(k) = knight.filter(|_| !matches!(self.phase, ShadePhase::Slash | ShadePhase::Dying | ShadePhase::Gone)) {
            if self.phase != ShadePhase::Retreat || near.is_none() {
                self.facing_right = k.x > self.pos.x;
            }
        }
        let leashed = knight.is_none_or(|k| k.distance(self.home) > 1100.0 * s);
        let calm_down = !hunted || leashed;

        match self.phase {
            ShadePhase::Appear => {
                out.clip = "Appear";
                if self.t >= APPEAR_SECS {
                    self.enter(ShadePhase::Idle);
                }
            }
            ShadePhase::Idle => {
                out.clip = "Idle";
                self.float_home(dt, s);
                if hunted && near.is_some() {
                    self.enter(ShadePhase::Startle);
                }
            }
            ShadePhase::Startle => {
                out.clip = "Startle";
                if self.t >= STARTLE_SECS {
                    self.enter(if calm_down { ShadePhase::Idle } else { ShadePhase::Approach });
                }
            }
            ShadePhase::Approach => {
                out.clip = "Fly";
                match knight {
                    Some(k) if !calm_down => {
                        let side = if self.pos.x < k.x { -1.0 } else { 1.0 };
                        let goal = k + Vec2::new(side * 100.0 * s, -20.0 * s);
                        self.fly_to(goal, 230.0 * s, dt);
                        if self.pos.distance(k) < 150.0 * s && self.t > 0.3 {
                            self.enter(ShadePhase::Antic);
                        }
                    }
                    _ => self.enter(ShadePhase::Idle),
                }
            }
            ShadePhase::Antic => {
                out.clip = "Slash Antic";
                if self.t >= ANTIC_SECS {
                    let k = knight.unwrap_or(self.pos + Vec2::X);
                    self.lunge = (k - self.pos).normalize_or(Vec2::X);
                    self.hit_done = false;
                    self.enter(ShadePhase::Slash);
                }
            }
            ShadePhase::Slash => {
                out.clip = "Slash";
                out.effect = Some("Slash Effect");
                self.pos += self.lunge * 720.0 * s * dt;
                if !self.hit_done && knight.is_some_and(|k| k.distance(self.pos) < 75.0 * s) {
                    self.hit_done = true;
                    out.hit_knight = true;
                }
                if self.t >= SLASH_SECS {
                    self.enter(ShadePhase::Cooldown);
                }
            }
            ShadePhase::Cooldown => {
                out.clip = "Slash CD";
                if self.t >= COOLDOWN_SECS {
                    self.enter(ShadePhase::Retreat);
                }
            }
            ShadePhase::Retreat => {
                out.clip = "Fly";
                if let Some(k) = knight {
                    let away = (self.pos - k).normalize_or(Vec2::NEG_Y);
                    let up = Vec2::new(away.x, away.y.min(0.0) - 0.4).normalize_or(Vec2::NEG_Y);
                    self.pos += up * 170.0 * s * dt;
                }
                if self.t >= RETREAT_SECS {
                    self.enter(if calm_down { ShadePhase::Idle } else { ShadePhase::Approach });
                }
            }
            ShadePhase::Dying => {
                out.clip = "Death";
                if self.t >= DEATH_SECS {
                    self.enter(ShadePhase::Gone);
                    out.burst = true;
                }
            }
            ShadePhase::Gone => out.clip = "Death",
        }
        let m = 30.0 * s;
        if bounds.width() > 2.0 * m && bounds.height() > 2.0 * m {
            self.pos.x = self.pos.x.clamp(bounds.left + m, bounds.right - m);
            self.pos.y = self.pos.y.clamp(bounds.top + m, bounds.bottom - m);
        }
        out
    }

    /// Bob and drift a little around home (returning there first if it strayed).
    fn float_home(&mut self, dt: f32, s: f32) {
        self.wander_in -= dt;
        if self.wander_in <= 0.0 {
            self.wander = Vec2::new(self.rng.range(-50.0, 50.0), self.rng.range(-12.0, 12.0)) * s;
            self.wander_in = self.rng.range(3.0, 6.0);
        }
        let bob = Vec2::new(0.0, (self.clock * std::f32::consts::TAU / 2.6).sin() * 6.0 * s);
        let goal = self.home + self.wander + bob;
        let far = self.pos.distance(goal) > 80.0 * s;
        self.fly_to(goal, if far { 160.0 * s } else { 30.0 * s }, dt);
    }

    fn fly_to(&mut self, goal: Vec2, speed: f32, dt: f32) {
        let d = goal - self.pos;
        let step = speed * dt;
        self.pos = if d.length() <= step { goal } else { self.pos + d.normalize() * step };
    }
}
