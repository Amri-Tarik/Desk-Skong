//! Pastime props: things the knight amuses himself with when nothing better is going on
//! (the brain side is `ai::pastime`).
//!
//! * **The training dummy** ([`Yard`]): now and then a dummy stands on a settled ledge (the
//!   shinies' placement rules: lasting surfaces, shared occupancy, never on a game monitor,
//!   reachable, not right under his feet). It stays a few minutes, then fades. Struck, it
//!   swings (`Light Hit` / `Heavy Hit`, L or R by the side the blow comes from).
//! * **A lumafly** ([`Sky`]): a small glowing fly that drifts from window to window (a
//!   little above their title bars) on the knight's monitor, bobbing. Swiped at, it may be
//!   caught (it glows bright and slips away upwards) or it darts off. It leaves after a
//!   minute or two.
//!
//! Pure: the app ticks, places and draws them (`glow` for the lumafly).
use crate::canvas::{Image, premul};
use crate::desktop::DesktopSnapshot;
use crate::discovery::Mix;
use crate::geom::{RectF, Vec2};
use crate::shiny::{self, Spot};
use crate::terrain::{Ledge, Terrain};

// --- the training dummy ---------------------------------------------------------------

/// How often a dummy stands somewhere and for how long.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DummyRules {
    /// Seconds between dummies, [min, max] (only counts while allowed).
    pub every_secs: [f32; 2],
    /// Seconds one stays, [min, max].
    pub stay_secs: [f32; 2],
    /// Never closer to the knight than this (px at scale 1).
    pub min_distance: f32,
}

impl Default for DummyRules {
    fn default() -> Self {
        Self { every_secs: [240.0, 600.0], stay_secs: [420.0, 780.0], min_distance: 220.0 }
    }
}

/// Fade in / out (s).
pub const DUMMY_FADE_IN: f32 = 1.2;
pub const DUMMY_FADE_OUT: f32 = 2.5;

/// A blow landing on the dummy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DummyHit {
    /// Nail arts and spells; nail slashes are light.
    pub heavy: bool,
    /// The blow comes from his right (the knight stands right of it).
    pub from_right: bool,
}

impl DummyHit {
    /// The clip of `Training Dummy Real Anim` it plays (knocked away from the blow).
    pub fn clip(self) -> &'static str {
        match (self.heavy, self.from_right) {
            (false, false) => "Light Hit R",
            (false, true) => "Light Hit L",
            (true, false) => "Heavy Hit R",
            (true, true) => "Heavy Hit L",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dummy {
    pub id: u64,
    pub spot: Spot,
    /// [`Yard::clock`] when it turned up and when it starts fading away.
    pub born: f32,
    pub leave_at: f32,
}

/// The dummy on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownDummy {
    pub id: u64,
    /// Bottom centre (where the pole meets the ledge).
    pub pos: Vec2,
    /// Its ledge (x0, x1).
    pub span: (f32, f32),
    /// 0..1 (fading in / out).
    pub alpha: f32,
}

/// The training dummy (at most one) and when the next one turns up.
#[derive(Clone, Debug)]
pub struct Yard {
    dummy: Option<Dummy>,
    /// Blows on their way (clock time they land).
    pending: Vec<(f32, DummyHit)>,
    /// Blows that landed since the app last looked.
    landed: Vec<DummyHit>,
    next_id: u64,
    clock: f32,
    next_at: f32,
    mix: Mix,
}

/// Seed tag ("dumy").
const DUMMY_TAG: u64 = 0x6475_6d79;

impl Yard {
    pub fn new(seed: u64, rules: &DummyRules) -> Self {
        let mut y = Self { dummy: None, pending: Vec::new(), landed: Vec::new(), next_id: 1, clock: 0.0, next_at: 0.0, mix: Mix(seed ^ DUMMY_TAG) };
        y.schedule(rules);
        y
    }

    fn schedule(&mut self, rules: &DummyRules) {
        let [lo, hi] = rules.every_secs;
        self.next_at = self.clock + lo + (hi - lo).max(0.0) * self.mix.f32();
    }

    pub fn clock(&self) -> f32 {
        self.clock
    }

    pub fn dummy(&self) -> Option<&Dummy> {
        self.dummy.as_ref()
    }

    /// Advance the clock and land due blows. `true` when a dummy should turn up now (then
    /// [`place`] one, or [`Self::retry_soon`]). The timer only runs while `allowed`.
    pub fn due(&mut self, dt: f32, allowed: bool, rules: &DummyRules) -> bool {
        self.clock += dt;
        let now = self.clock;
        let (due, later): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|(t, _)| *t <= now);
        self.pending = later;
        if self.dummy.is_some() {
            self.landed.extend(due.into_iter().map(|(_, h)| h));
        }
        if self.dummy.is_some() {
            return false;
        }
        if !allowed {
            self.next_at += dt;
            return false;
        }
        if self.clock < self.next_at {
            return false;
        }
        self.schedule(rules);
        true
    }

    /// No room just now: try again in a little while.
    pub fn retry_soon(&mut self) {
        self.next_at = self.clock + 30.0;
    }

    /// A dummy now stands at `spot`.
    pub fn add(&mut self, spot: Spot, rules: &DummyRules) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let [lo, hi] = rules.stay_secs;
        let stay = lo + (hi - lo).max(0.0) * self.mix.f32();
        self.dummy = Some(Dummy { id, spot, born: self.clock, leave_at: self.clock + stay });
        self.pending.clear();
        self.landed.clear();
        id
    }

    /// A blow lands after `delay` seconds (the swing / the fireball's flight).
    pub fn strike(&mut self, hit: DummyHit, delay: f32) {
        if self.dummy.is_some() {
            self.pending.push((self.clock + delay.max(0.0), hit));
        }
    }

    /// Blows that landed since the last call (the app plays their clips).
    pub fn take_landed(&mut self) -> Vec<DummyHit> {
        std::mem::take(&mut self.landed)
    }

    /// Leave now (fade out): a game came up on its monitor, debug.
    pub fn dismiss(&mut self) {
        if let Some(d) = &mut self.dummy {
            d.leave_at = d.leave_at.min(self.clock);
        }
    }

    pub fn clear(&mut self) {
        self.dummy = None;
        self.pending.clear();
        self.landed.clear();
    }

    /// The dummy on screen (`None` if there is none or its ledge is covered right now). One
    /// past its stay, or whose window closed or went fullscreen, is gone.
    pub fn shown(&mut self, desk: &DesktopSnapshot, terrain: &Terrain, width: f32, settled: &dyn Fn(&Ledge) -> bool) -> Option<ShownDummy> {
        let clock = self.clock;
        let d = self.dummy?;
        let open = match d.spot {
            Spot::Window { id, .. } => desk.window(id).is_some_and(|w| !w.fullscreen),
            Spot::Ground { .. } => true,
        };
        if !open || clock >= d.leave_at + DUMMY_FADE_OUT {
            self.dummy = None;
            self.pending.clear();
            return None;
        }
        let (pos, span) = shiny::resolve(terrain, d.spot, width, settled)?;
        let alpha = ((clock - d.born) / DUMMY_FADE_IN).min((d.leave_at + DUMMY_FADE_OUT - clock) / DUMMY_FADE_OUT).clamp(0.0, 1.0);
        Some(ShownDummy { id: d.id, pos, span, alpha })
    }
}

/// Stand-in drawing when the dummy's art isn't exported: a post with a crossbar and a
/// round stuffed head, `w`×`h` px, feet at the bottom centre. `swing` (−1..1) tilts it.
pub fn dummy_stand_in(w: u32, h: u32, swing: f32, alpha: f32) -> Image {
    let (w, h) = (w.max(8), h.max(16));
    let mut img = Image::new(w, h);
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    if a == 0 {
        return img;
    }
    let (wf, hf) = (w as f32, h as f32);
    let tilt = swing.clamp(-1.0, 1.0) * 0.18;
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Lean the whole thing around its foot.
            let up = hf - py;
            let cx = wf * 0.5 + tilt * up;
            let dx = px - cx;
            let pole = dx.abs() < (wf * 0.05).max(1.0) && py > hf * 0.2;
            let bar = (py - hf * 0.45).abs() < (hf * 0.025).max(1.0) && dx.abs() < wf * 0.42;
            let body = (dx / (wf * 0.24)).powi(2) + ((py - hf * 0.5) / (hf * 0.2)).powi(2) < 1.0;
            let head = dx.hypot(py - hf * 0.2) < wf * 0.2;
            let mask = head && dx.hypot(py - hf * 0.2) < wf * 0.13 && (py - hf * 0.2) > -wf * 0.1;
            let c = if mask {
                Some((236, 236, 230))
            } else if head || body {
                Some((112, 88, 120))
            } else if bar || pole {
                Some((70, 60, 54))
            } else {
                None
            };
            if let Some((r, g, b)) = c {
                img.px[(y * w + x) as usize] = premul(r, g, b, a);
            }
        }
    }
    img
}

// --- the lumafly ----------------------------------------------------------------------

/// How often a lumafly drifts by, how it flies, and how catchable it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LumaflyRules {
    /// Seconds between lumaflies, [min, max] (only counts while allowed).
    pub every_secs: [f32; 2],
    /// Seconds one stays before drifting off, [min, max].
    pub life_secs: [f32; 2],
    /// Cruising speed (px/s at scale 1).
    pub speed: f32,
    /// A swipe this close to it (px at scale 1, from his body centre) may catch it ...
    pub catch_reach: f32,
    /// ... with this chance; otherwise it darts away.
    pub catch_chance: f32,
}

impl Default for LumaflyRules {
    fn default() -> Self {
        Self { every_secs: [180.0, 480.0], life_secs: [70.0, 130.0], speed: 42.0, catch_reach: 70.0, catch_chance: 0.35 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlyState {
    /// Drifting to its next waypoint (and hovering a moment there).
    Drift,
    /// Swiped at and missed: darting away until then.
    Darting { until: f32 },
    /// Caught: a bright flare, then it slips out of his hands and floats up and away.
    Caught { at: f32 },
    /// Its time is up: floating up and away, fading.
    Leaving { since: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lumafly {
    pub id: u64,
    /// Centre of the glow (screen px), without the bob.
    pub pos: Vec2,
    pub vel: Vec2,
    pub target: Vec2,
    /// Hover at the waypoint until then.
    pub hover_until: f32,
    pub born: f32,
    pub leave_at: f32,
    pub state: FlyState,
    /// Bob / flicker phase (0..1).
    pub phase: f32,
    /// The monitor work area it keeps to.
    pub area: RectF,
}

/// A lumafly on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownFly {
    pub id: u64,
    /// Where it glows (bob included).
    pub pos: Vec2,
    pub alpha: f32,
    /// 1 = normal glow; up to ~2.5 in the catch flare.
    pub glow: f32,
    /// Leaving or caught: not worth chasing any more.
    pub leaving: bool,
    pub phase: f32,
}

/// What a swipe did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swipe {
    Caught,
    Missed,
    /// Nowhere near it (or it's gone / leaving).
    TooFar,
}

/// Seconds the leaving fade takes; the catch flare.
const LEAVE_SECS: f32 = 6.0;
const FLARE_SECS: f32 = 0.8;
/// Fade in (s).
const FLY_FADE_IN: f32 = 2.0;
/// Waypoints sit this far above a title bar (px at scale 1), [min, max].
const ABOVE_BAR: [f32; 2] = [50.0, 130.0];

/// The lumaflies (at most one) and when the next drifts by.
#[derive(Clone, Debug)]
pub struct Sky {
    flies: Vec<Lumafly>,
    next_id: u64,
    clock: f32,
    next_at: f32,
    mix: Mix,
}

/// Seed tag ("luma").
const FLY_TAG: u64 = 0x6c75_6d61;

impl Sky {
    pub fn new(seed: u64, rules: &LumaflyRules) -> Self {
        let mut s = Self { flies: Vec::new(), next_id: 1, clock: 0.0, next_at: 0.0, mix: Mix(seed ^ FLY_TAG) };
        s.schedule(rules);
        s
    }

    fn schedule(&mut self, rules: &LumaflyRules) {
        let [lo, hi] = rules.every_secs;
        self.next_at = self.clock + lo + (hi - lo).max(0.0) * self.mix.f32();
    }

    pub fn clock(&self) -> f32 {
        self.clock
    }

    pub fn flies(&self) -> &[Lumafly] {
        &self.flies
    }

    /// Advance the spawn timer: `true` when one should drift in now (then [`Self::spawn`]).
    pub fn due(&mut self, dt: f32, allowed: bool, rules: &LumaflyRules) -> bool {
        if !self.flies.is_empty() {
            return false;
        }
        if !allowed {
            self.next_at += dt;
            return false;
        }
        if self.clock < self.next_at {
            return false;
        }
        self.schedule(rules);
        true
    }

    /// Waypoints: a little above the given title bars / ledges (`bars`: (x0, x1, y)), kept
    /// inside `area`; the middle of the area if there are none.
    fn waypoint(&mut self, area: RectF, bars: &[(f32, f32, f32)], scale: f32) -> Vec2 {
        let inside: Vec<&(f32, f32, f32)> = bars.iter().filter(|b| b.2 > area.top + 80.0 * scale && b.2 <= area.bottom + 1.0 && b.1 > area.left && b.0 < area.right).collect();
        let (x, y) = if inside.is_empty() {
            (area.left + area.width() * (0.2 + 0.6 * self.mix.f32()), area.top + area.height() * (0.3 + 0.4 * self.mix.f32()))
        } else {
            let b = inside[(self.mix.next() % inside.len() as u64) as usize];
            let x = b.0 + (b.1 - b.0) * (0.15 + 0.7 * self.mix.f32());
            let up = (ABOVE_BAR[0] + (ABOVE_BAR[1] - ABOVE_BAR[0]) * self.mix.f32()) * scale;
            (x, b.2 - up)
        };
        let m = 30.0 * scale;
        Vec2::new(x.clamp(area.left + m, area.right - m), y.clamp(area.top + m, area.bottom - m))
    }

    /// A lumafly drifts in from the side of `area` farther from `knight`, heading for a
    /// title bar. `bars`: (x0, x1, y) of ledges it may hover over.
    pub fn spawn(&mut self, area: RectF, bars: &[(f32, f32, f32)], knight: Vec2, rules: &LumaflyRules, scale: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let from_left = knight.x > area.center().x;
        let x = if from_left { area.left + 10.0 * scale } else { area.right - 10.0 * scale };
        let y = area.top + area.height() * (0.25 + 0.35 * self.mix.f32());
        let target = self.waypoint(area, bars, scale);
        let [lo, hi] = rules.life_secs;
        let life = lo + (hi - lo).max(0.0) * self.mix.f32();
        let phase = self.mix.f32();
        self.flies.push(Lumafly {
            id,
            pos: Vec2::new(x, y),
            vel: Vec2::ZERO,
            target,
            hover_until: 0.0,
            born: self.clock,
            leave_at: self.clock + life,
            state: FlyState::Drift,
            phase,
            area,
        });
        log::info!("a lumafly drifts in at ({x:.0},{y:.0}) (#{id})");
        id
    }

    /// Move them. `bars` as in [`Self::spawn`].
    pub fn step(&mut self, dt: f32, bars: &[(f32, f32, f32)], rules: &LumaflyRules, scale: f32) {
        self.clock += dt;
        let now = self.clock;
        let mut flies = std::mem::take(&mut self.flies);
        for f in &mut flies {
            if matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) && now >= f.leave_at {
                f.state = FlyState::Leaving { since: now };
            }
            if let FlyState::Darting { until } = f.state
                && now >= until
            {
                f.state = FlyState::Drift;
                f.target = self.waypoint(f.area, bars, scale);
            }
            let speed = rules.speed * scale;
            let (desired, agility) = match f.state {
                FlyState::Drift => {
                    let off = f.target - f.pos;
                    if off.length() < 6.0 * scale {
                        if f.hover_until <= 0.0 {
                            f.hover_until = now + 2.0 + 4.0 * self.mix.f32();
                        } else if now >= f.hover_until {
                            f.hover_until = 0.0;
                            f.target = self.waypoint(f.area, bars, scale);
                        }
                        (Vec2::ZERO, 2.0)
                    } else {
                        // Slow down on arrival.
                        let s = speed * (off.length() / (40.0 * scale)).min(1.0);
                        (off.normalize_or_zero() * s, 1.2)
                    }
                }
                FlyState::Darting { .. } => {
                    let off = f.target - f.pos;
                    (off.normalize_or_zero() * speed * 3.5, 6.0)
                }
                FlyState::Caught { .. } | FlyState::Leaving { .. } => (Vec2::new(0.0, -speed * 0.9), 1.0),
            };
            f.vel += (desired - f.vel) * (agility * dt).min(1.0);
            f.pos += f.vel * dt;
            if matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) {
                let m = 20.0 * scale;
                f.pos.x = f.pos.x.clamp(f.area.left + m, f.area.right - m);
                f.pos.y = f.pos.y.clamp(f.area.top + m, f.area.bottom - m);
            }
        }
        // Gone once faded out (or flown off the top).
        flies.retain(|f| match f.state {
            FlyState::Leaving { since } => now - since < LEAVE_SECS,
            FlyState::Caught { at } => now - at < FLARE_SECS + LEAVE_SECS,
            _ => true,
        });
        self.flies = flies;
    }

    /// The knight swipes (jumps, cups his hands) at fly `id` from `from` (his body centre).
    pub fn swipe(&mut self, id: u64, from: Vec2, rules: &LumaflyRules, scale: f32) -> Swipe {
        let now = self.clock;
        let roll = self.mix.f32();
        let away_x = self.mix.f32();
        let Some(f) = self.flies.iter_mut().find(|f| f.id == id) else { return Swipe::TooFar };
        if !matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) || f.pos.distance(from) > rules.catch_reach * scale {
            return Swipe::TooFar;
        }
        if roll < rules.catch_chance {
            f.state = FlyState::Caught { at: now };
            f.vel = Vec2::ZERO;
            log::info!("the knight caught the lumafly (#{id}); it slips away");
            return Swipe::Caught;
        }
        // Darts off, away from him and up.
        let side = if f.pos.x >= from.x { 1.0 } else { -1.0 };
        let to = f.pos + Vec2::new(side * (120.0 + 120.0 * away_x) * scale, -(90.0 + 60.0 * away_x) * scale);
        let m = 30.0 * scale;
        f.target = Vec2::new(to.x.clamp(f.area.left + m, f.area.right - m), to.y.clamp(f.area.top + m, f.area.bottom - m));
        f.state = FlyState::Darting { until: now + 0.9 };
        Swipe::Missed
    }

    /// Send them off (a game came up, debug).
    pub fn dismiss(&mut self) {
        let now = self.clock;
        for f in &mut self.flies {
            if matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) {
                f.state = FlyState::Leaving { since: now };
            }
        }
    }

    pub fn clear(&mut self) {
        self.flies.clear();
    }

    /// What to draw.
    pub fn shown(&self, scale: f32) -> Vec<ShownFly> {
        use std::f32::consts::TAU;
        let now = self.clock;
        self.flies
            .iter()
            .map(|f| {
                let bob = Vec2::new((TAU * (now / 3.1 + f.phase)).sin() * 3.0, (TAU * (now / 1.7 + f.phase)).sin() * 5.0) * scale;
                let fade_in = ((now - f.born) / FLY_FADE_IN).clamp(0.0, 1.0);
                let (alpha, glow, leaving) = match f.state {
                    FlyState::Drift | FlyState::Darting { .. } => (fade_in, 1.0, false),
                    FlyState::Caught { at } => {
                        let t = now - at;
                        let flare = if t < FLARE_SECS { 1.0 + 1.5 * (1.0 - t / FLARE_SECS) } else { 1.0 };
                        (fade_in * (1.0 - ((t - FLARE_SECS).max(0.0) / LEAVE_SECS)).clamp(0.0, 1.0), flare, true)
                    }
                    FlyState::Leaving { since } => (fade_in * (1.0 - (now - since) / LEAVE_SECS).clamp(0.0, 1.0), 1.0, true),
                };
                ShownFly { id: f.id, pos: f.pos + bob, alpha, glow, leaving, phase: f.phase }
            })
            .collect()
    }
}

/// The lumafly's glow, `size` px square: a soft cyan halo, a bright pale core and two
/// faint flickering wings. `glow` > 1 brightens and widens it (the catch flare).
pub fn glow(size: u32, t: f32, phase: f32, glow: f32, alpha: f32) -> Image {
    use std::f32::consts::TAU;
    let size = size.max(7);
    let s = size as f32;
    let mut img = Image::new(size, size);
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return img;
    }
    let flicker = 0.85 + 0.15 * (TAU * (t / 0.9 + phase)).sin();
    let g = glow.max(0.0);
    let (cx, cy) = (s * 0.5, s * 0.5);
    let halo_r = s * 0.22 * (0.8 + 0.2 * g);
    let core_r = (s * 0.05).max(0.7);
    let wing = (TAU * (t * 9.0 + phase)).sin().abs();
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = dx.hypot(dy);
            let halo = (-(d / halo_r).powi(2)).exp() * 0.55 * flicker * g.min(2.5);
            let core = (-(d / core_r).powi(2)).exp();
            // Wings: two small ellipses above-left and above-right of the body.
            let wing_at = |sx: f32| {
                let (u, v) = ((dx - sx * s * 0.07) / (s * 0.06), (dy + s * 0.05) / (s * 0.035 * (0.5 + 0.5 * wing)));
                if u * u + v * v < 1.0 { 0.35 } else { 0.0 }
            };
            let w = wing_at(-1.0) + wing_at(1.0);
            let a = (halo + core + w).min(1.0);
            let a = (a * alpha * 255.0) as u8;
            if a > 0 {
                // Cool cyan halo, near-white core.
                let warm = core.min(1.0);
                let r = (150.0 + 105.0 * warm) as u8;
                let gr = (235.0 + 20.0 * warm) as u8;
                img.px[(y * size + x) as usize] = premul(r, gr, 255, a);
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{Monitor, WindowInfo};
    use crate::geom::RectI;
    use crate::terrain::TerrainOptions;

    fn desk(windows: Vec<WindowInfo>) -> (DesktopSnapshot, Terrain) {
        let d = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows,
        };
        let t = Terrain::from_desktop(&d, &TerrainOptions::default());
        (d, t)
    }

    #[test]
    fn a_dummy_turns_up_on_schedule_stays_a_while_and_fades() {
        let rules = DummyRules { every_secs: [10.0, 10.0], stay_secs: [60.0, 60.0], min_distance: 0.0 };
        let mut yard = Yard::new(1, &rules);
        let (d, t) = desk(vec![]);
        let mut due = false;
        for _ in 0..(9.0 / 0.1) as usize {
            due |= yard.due(0.1, true, &rules);
        }
        assert!(!due, "not before its time");
        // Not while held back: the timer waits.
        for _ in 0..100 {
            assert!(!yard.due(0.1, false, &rules));
        }
        assert!((0..30).any(|_| yard.due(0.1, true, &rules)), "due now");
        yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        let s = yard.shown(&d, &t, 40.0, &|_| true).expect("shown");
        assert_eq!(s.pos, Vec2::new(500.0, 1040.0));
        assert!(s.alpha < 0.2, "fading in");
        // A blow lands after its delay, once.
        yard.strike(DummyHit { heavy: true, from_right: false }, 0.3);
        yard.due(0.2, true, &rules);
        assert!(yard.take_landed().is_empty());
        yard.due(0.2, true, &rules);
        assert_eq!(yard.take_landed(), vec![DummyHit { heavy: true, from_right: false }]);
        assert!(yard.take_landed().is_empty());
        for _ in 0..(70.0 / 0.1) as usize {
            assert!(!yard.due(0.1, true, &rules), "one at a time");
        }
        assert!(yard.shown(&d, &t, 40.0, &|_| true).is_none(), "gone after its stay");
        assert!(yard.dummy().is_none());
    }

    #[test]
    fn a_dummy_on_a_window_goes_with_it() {
        let rules = DummyRules::default();
        let mut w = WindowInfo { id: 7, rect: RectI::new(300, 400, 1200, 900), title: String::new(), class: String::new(), process: "x.exe".into(), pid: 1, maximized: false, fullscreen: false, foreground: false, topmost: false };
        let (d, t) = desk(vec![w.clone()]);
        let mut yard = Yard::new(2, &rules);
        yard.add(Spot::Window { id: 7, anchor: crate::discovery::Anchor { fx: 0.5, dy: 0.0 } }, &rules);
        let s = yard.shown(&d, &t, 40.0, &|_| true).expect("on the title bar");
        assert_eq!(s.pos.y, 400.0);
        // The window goes fullscreen: the dummy is gone.
        w.fullscreen = true;
        let (d, t) = desk(vec![w]);
        assert!(yard.shown(&d, &t, 40.0, &|_| true).is_none());
        assert!(yard.dummy().is_none());
    }

    #[test]
    fn the_dummy_stands_only_on_settled_ledges_and_never_on_a_game_monitor() {
        use crate::discovery::{Footprint, LASTING_SECS};
        use crate::lasting::LedgeAges;
        use crate::occupancy::Occupancy;
        use crate::shiny::{ShinySite, place};
        // Two monitors; the right one runs a game. A window on each.
        let d = DesktopSnapshot {
            generation: 1,
            monitors: vec![
                Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 },
                Monitor { rect: RectI::new(1920, 0, 3840, 1080), work: RectI::new(1920, 0, 3840, 1040), primary: false, scale_milli: 1000 },
            ],
            windows: vec![
                WindowInfo { id: 1, rect: RectI::new(300, 400, 1200, 900), title: String::new(), class: String::new(), process: "a.exe".into(), pid: 1, maximized: false, fullscreen: false, foreground: false, topmost: false },
                WindowInfo { id: 2, rect: RectI::new(2200, 400, 3200, 900), title: String::new(), class: String::new(), process: "b.exe".into(), pid: 2, maximized: false, fullscreen: false, foreground: false, topmost: false },
            ],
        };
        let t = Terrain::from_desktop(&d, &TerrainOptions::default());
        let aged = |secs: f32| {
            let mut a = LedgeAges::default();
            let mut now = 0.0;
            while now <= secs {
                a.observe(&t, now);
                now += 1.0;
            }
            a
        };
        let occ = Occupancy::default();
        let game = [RectI::new(1920, 0, 3840, 1080)];
        let fp = Footprint { w: 30.0, h: 160.0, room: 140.0 };
        for (age, window_ok) in [(10.0, false), (60.0, true)] {
            let ages = aged(age);
            let mut on_window = 0;
            for salt in 0..60 {
                let site = ShinySite { terrain: &t, ages: &ages, now: age, min_age: LASTING_SECS, occupancy: &occ, footprint: fp, knight: Vec2::new(100.0, 1040.0), min_distance: 220.0, area: None, avoid: &game, reachable: &|_| true };
                let (spot, pos) = place(&site, &[], salt).expect("somewhere");
                assert!(pos.x < 1920.0, "never on the game's monitor: {pos:?}");
                if let Spot::Window { .. } = spot {
                    on_window += 1;
                }
            }
            assert_eq!(on_window > 0, window_ok, "window ledges {age} s old: {on_window}");
        }
    }

    #[test]
    fn hit_clips_swing_away_from_the_blow() {
        assert_eq!(DummyHit { heavy: false, from_right: true }.clip(), "Light Hit L");
        assert_eq!(DummyHit { heavy: true, from_right: false }.clip(), "Heavy Hit R");
    }

    fn sky() -> (Sky, LumaflyRules, RectF) {
        let rules = LumaflyRules::default();
        (Sky::new(3, &rules), rules, RectF::new(0.0, 0.0, 1920.0, 1040.0))
    }

    #[test]
    fn a_lumafly_drifts_over_the_title_bars_and_leaves_in_time() {
        let (mut sky, rules, area) = sky();
        let bars = [(200.0, 900.0, 400.0), (1000.0, 1700.0, 250.0)];
        sky.spawn(area, &bars, Vec2::new(1500.0, 1040.0), &rules, 1.0);
        assert!(sky.flies()[0].pos.x < 100.0, "comes in from the side away from him");
        let mut hovered = 0;
        for _ in 0..(60.0 / 0.05) as usize {
            sky.step(0.05, &bars, &rules, 1.0);
            let f = sky.flies()[0];
            assert!(area.contains(f.pos), "keeps to its monitor: {:?}", f.pos);
            let over_a_bar = bars.iter().any(|b| f.pos.x >= b.0 - 5.0 && f.pos.x <= b.1 + 5.0 && f.pos.y < b.2 - 40.0 && f.pos.y > b.2 - 140.0);
            if f.vel.length() < 5.0 && over_a_bar {
                hovered += 1;
            }
        }
        assert!(hovered > 0, "hovers above title bars");
        for _ in 0..(160.0 / 0.05) as usize {
            sky.step(0.05, &bars, &rules, 1.0);
        }
        assert!(sky.flies().is_empty(), "drifted off after its life");
    }

    #[test]
    fn a_swipe_close_by_catches_it_sometimes_otherwise_it_darts_off() {
        let (mut sky, rules, area) = sky();
        let mut caught = 0;
        let mut missed = 0;
        for i in 0..40 {
            sky.clear();
            sky.spawn(area, &[], Vec2::ZERO, &rules, 1.0);
            let f = sky.flies()[0];
            assert_eq!(sky.swipe(f.id, f.pos + Vec2::new(400.0, 0.0), &rules, 1.0), Swipe::TooFar, "out of reach");
            match sky.swipe(f.id, f.pos + Vec2::new(20.0, 10.0 + i as f32), &rules, 1.0) {
                Swipe::Caught => {
                    caught += 1;
                    assert!(sky.shown(1.0)[0].leaving, "caught: no longer chased");
                    assert!(sky.shown(1.0)[0].glow > 2.0, "the flare");
                }
                Swipe::Missed => {
                    missed += 1;
                    let before = sky.flies()[0].pos;
                    for _ in 0..10 {
                        sky.step(0.05, &[], &rules, 1.0);
                    }
                    assert!(sky.flies()[0].pos.distance(before) > 40.0, "darts away");
                }
                Swipe::TooFar => panic!("within reach"),
            }
        }
        assert!(caught >= 5 && missed >= 15, "caught {caught}, missed {missed}");
    }

    #[test]
    fn the_glow_is_soft_and_brighter_in_the_flare() {
        let calm = glow(24, 0.0, 0.0, 1.0, 1.0);
        let flare = glow(24, 0.0, 0.0, 2.5, 1.0);
        let sum = |img: &Image| img.px.iter().map(|p| (p >> 24) as u64).sum::<u64>();
        assert!(sum(&flare) > sum(&calm));
        let c = calm.px[(12 * 24 + 12) as usize] >> 24;
        assert!(c > 200, "bright core: {c}");
        assert_eq!(calm.px[0] >> 24, 0, "transparent corners");
        assert!(dummy_stand_in(40, 120, 0.0, 1.0).px.iter().any(|p| p >> 24 > 0));
    }
}
