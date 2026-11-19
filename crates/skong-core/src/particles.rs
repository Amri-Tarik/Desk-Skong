//! Procedural particles: the game's small Unity particle systems (run / land dust bits,
//! crystal dash shards, Geo pickup sparkles) were never exported with the tk2d art, so
//! they are recreated here in code. A [`Burst`] is a handful of particles with simple
//! physics (velocity, gravity, drag) that fade out; it draws itself into a small image the
//! size of its live particles' bounding box (one small window per burst on the app side).
//!
//! Everything is deterministic for a given seed, few (tens at most) and short-lived.
//! Specs are in game units (multiplied by `px_per_unit`); angles are degrees, y up,
//! 0 = the burst's `dir` (+1 right / -1 left), 90 = straight up.
use crate::ai::rng::Rng;
use crate::canvas::{Image, over, premul};
use crate::geom::{RectI, Vec2};

/// Hard cap of live particles in one burst (streams included).
pub const MAX_PARTICLES: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A soft round bit of dust.
    Dust,
    /// A thin crystal sliver, pointing along its velocity, with a bright core.
    Shard,
    /// A tiny four-point twinkle.
    Spark,
}

/// How one kind of particle is born and moves.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Particles per emission (inclusive range).
    pub count: [u32; 2],
    /// Launch speed (units/s).
    pub speed: [f32; 2],
    /// Launch angle (degrees, y up, 0 = along `dir`).
    pub angle: [f32; 2],
    /// Each particle picks a random side (the angle mirrored about the vertical).
    pub both_sides: bool,
    /// Spawn offset half-extents around the emission point (units).
    pub jitter: [f32; 2],
    /// Lifetime (s).
    pub life: [f32; 2],
    /// Radius (dust, spark) / half-length (shard), units.
    pub size: [f32; 2],
    /// Size at the end of life relative to the start.
    pub grow: f32,
    /// Downward acceleration (units/s², negative floats up).
    pub gravity: f32,
    /// Exponential velocity damping (1/s).
    pub drag: f32,
    /// Peak opacity.
    pub alpha: f32,
    /// Straight-alpha colours to pick from.
    pub colors: &'static [[u8; 3]],
    pub shape: Shape,
    /// A particle further than this from where it was born dies (units).
    pub reach: f32,
    /// Streams: emissions per second while fed.
    pub rate: f32,
}

const DUST: &[[u8; 3]] = &[[214, 208, 196], [188, 182, 172], [230, 226, 218]];
const CRYSTAL: &[[u8; 3]] = &[[255, 214, 240], [255, 242, 250], [244, 176, 226]];
const GLEAM: &[[u8; 3]] = &[[255, 249, 226], [255, 238, 200]];

/// Bits kicked back from the feet while running fast (a stream).
pub const RUN_DUST: Spec = Spec {
    count: [1, 1],
    speed: [0.8, 1.8],
    angle: [125.0, 165.0],
    both_sides: false,
    jitter: [0.12, 0.02],
    life: [0.28, 0.45],
    size: [0.03, 0.05],
    grow: 1.5,
    gravity: 5.0,
    drag: 3.0,
    alpha: 0.45,
    colors: DUST,
    shape: Shape::Dust,
    reach: 1.2,
    rate: 8.0,
};

/// A soft landing: a few bits rolling out low on both sides.
pub const LAND_DUST: Spec = Spec {
    count: [4, 6],
    speed: [1.2, 3.0],
    angle: [6.0, 35.0],
    both_sides: true,
    jitter: [0.2, 0.02],
    life: [0.32, 0.55],
    size: [0.035, 0.06],
    grow: 1.5,
    gravity: 7.0,
    drag: 4.0,
    alpha: 0.5,
    colors: DUST,
    shape: Shape::Dust,
    reach: 2.0,
    rate: 0.0,
};

/// A hard landing: more bits, a little faster.
pub const HARD_LAND_DUST: Spec = Spec { count: [8, 10], speed: [2.0, 4.2], life: [0.4, 0.65], size: [0.04, 0.07], reach: 2.6, ..LAND_DUST };

/// A jump off the ground: bits pushed out and slightly up from the feet.
pub const JUMP_DUST: Spec = Spec { count: [3, 4], speed: [0.8, 2.0], angle: [12.0, 50.0], life: [0.28, 0.45], ..LAND_DUST };

/// A grounded dash: bits thrown back along the floor (`dir` = the dash direction).
pub const DASH_DUST: Spec = Spec { count: [4, 5], speed: [2.0, 4.0], angle: [155.0, 176.0], both_sides: false, life: [0.3, 0.48], ..LAND_DUST };

/// A wall jump: bits off the wall (`dir` = away from the wall).
pub const WALL_DUST: Spec = Spec { count: [3, 4], speed: [1.0, 2.4], angle: [-40.0, 40.0], both_sides: false, jitter: [0.02, 0.25], gravity: 5.0, ..LAND_DUST };

/// Crystal heart charging: slivers lifting off around the body (a stream).
pub const SD_CHARGE_SHARDS: Spec = Spec {
    count: [1, 1],
    speed: [0.6, 1.6],
    angle: [70.0, 110.0],
    both_sides: false,
    jitter: [0.45, 0.4],
    life: [0.3, 0.5],
    size: [0.05, 0.085],
    grow: 0.5,
    gravity: -1.0,
    drag: 2.0,
    alpha: 0.75,
    colors: CRYSTAL,
    shape: Shape::Shard,
    reach: 1.5,
    rate: 9.0,
};

/// Crystal dash launch: shards sprayed out behind (`dir` = the flight direction).
pub const SD_BURST_SHARDS: Spec = Spec {
    count: [9, 11],
    speed: [4.0, 9.0],
    angle: [145.0, 215.0],
    jitter: [0.1, 0.3],
    life: [0.28, 0.5],
    size: [0.06, 0.11],
    gravity: 6.0,
    drag: 6.0,
    reach: 3.5,
    rate: 0.0,
    ..SD_CHARGE_SHARDS
};

/// Crystal dash into a wall: shards bouncing back and falling (`dir` = the flight direction).
pub const SD_BREAK_SHARDS: Spec = Spec { count: [7, 9], speed: [2.5, 6.0], angle: [120.0, 240.0], life: [0.4, 0.65], gravity: 16.0, drag: 3.0, ..SD_BURST_SHARDS };

/// Geo pickup: a few warm sparkles drifting up.
pub const GEO_SPARKLE: Spec = Spec {
    count: [4, 6],
    speed: [0.6, 1.8],
    angle: [45.0, 135.0],
    both_sides: false,
    jitter: [0.25, 0.2],
    life: [0.4, 0.7],
    size: [0.05, 0.08],
    grow: 0.6,
    gravity: -0.6,
    drag: 2.5,
    alpha: 0.85,
    colors: GLEAM,
    shape: Shape::Spark,
    reach: 2.0,
    rate: 0.0,
};

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    /// Screen px (y down) and px/s.
    pub pos: Vec2,
    pub vel: Vec2,
    pub born: Vec2,
    pub age: f32,
    pub life: f32,
    /// px at birth.
    pub size: f32,
    pub color: [u8; 3],
}

impl Particle {
    /// Life used, 0..1.
    pub fn progress(&self) -> f32 {
        (self.age / self.life).clamp(0.0, 1.0)
    }

    /// Opacity now: a quick rise over the first 8 % of life, then an eased fade to zero.
    pub fn alpha(&self, peak: f32) -> f32 {
        let u = self.progress();
        peak * (u / 0.08).min(1.0) * (1.0 - u).powf(1.5)
    }
}

/// A few particles from one spec, emitted once or fed as a stream.
pub struct Burst {
    pub spec: Spec,
    ppu: f32,
    rng: Rng,
    parts: Vec<Particle>,
    /// Stream: emissions owed (fractional).
    owed: f32,
}

impl Burst {
    pub fn new(spec: Spec, px_per_unit: f32, seed: u32) -> Burst {
        Burst { spec, ppu: px_per_unit, rng: Rng::new(seed), parts: Vec::with_capacity(16), owed: 0.0 }
    }

    pub fn particles(&self) -> &[Particle] {
        &self.parts
    }

    pub fn len(&self) -> usize {
        self.parts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// One emission at screen point `at`; `dir` is +1 (right) or -1 (left).
    pub fn emit(&mut self, at: Vec2, dir: f32) {
        let s = self.spec;
        let n = s.count[0] + (self.rng.f32() * (s.count[1] - s.count[0] + 1) as f32) as u32 % (s.count[1] - s.count[0] + 1);
        for _ in 0..n {
            if self.parts.len() >= MAX_PARTICLES {
                break;
            }
            let side = if s.both_sides && self.rng.f32() < 0.5 { -dir } else { dir };
            let a = self.rng.range(s.angle[0], s.angle[1]).to_radians();
            let speed = self.rng.range(s.speed[0], s.speed[1]) * self.ppu;
            let vel = Vec2::new(a.cos() * side, -a.sin()) * speed;
            let pos = at + Vec2::new(self.rng.range(-s.jitter[0], s.jitter[0]), self.rng.range(-s.jitter[1], s.jitter[1])) * self.ppu;
            let life = self.rng.range(s.life[0], s.life[1]);
            let size = self.rng.range(s.size[0], s.size[1]) * self.ppu;
            let color = s.colors[(self.rng.next_u32() as usize) % s.colors.len().max(1)];
            self.parts.push(Particle { pos, vel, born: pos, age: 0.0, life, size, color });
        }
    }

    /// Stream: emit at `spec.rate` per second for `dt` seconds.
    pub fn feed(&mut self, dt: f32, at: Vec2, dir: f32) {
        self.owed += self.spec.rate * dt;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            self.emit(at, dir);
        }
    }

    /// Advance the physics; particles past their life or reach are dropped.
    pub fn step(&mut self, dt: f32) {
        let g = self.spec.gravity * self.ppu;
        let damp = (-self.spec.drag * dt).exp();
        let reach = self.spec.reach * self.ppu;
        for p in &mut self.parts {
            p.vel.y += g * dt;
            p.vel *= damp;
            p.pos += p.vel * dt;
            p.age += dt;
        }
        self.parts.retain(|p| p.age < p.life && p.pos.distance(p.born) <= reach);
    }

    fn radius(&self, p: &Particle) -> f32 {
        p.size * (1.0 + (self.spec.grow - 1.0) * p.progress())
    }

    /// Pixel rectangle covering every live particle (with its antialiased edge).
    pub fn bounds(&self) -> Option<RectI> {
        let mut r: Option<RectI> = None;
        for p in &self.parts {
            let e = self.radius(p).max(1.0) + 2.0;
            let b = RectI::new((p.pos.x - e).floor() as i32, (p.pos.y - e).floor() as i32, (p.pos.x + e).ceil() as i32, (p.pos.y + e).ceil() as i32);
            r = Some(r.map_or(b, |a| RectI::new(a.left.min(b.left), a.top.min(b.top), a.right.max(b.right), a.bottom.max(b.bottom))));
        }
        r
    }

    /// Draw into `img` (resized to [`Self::bounds`]); returns the image's screen top-left.
    pub fn draw(&self, img: &mut Image) -> Option<(i32, i32)> {
        let b = self.bounds()?;
        img.resize(b.width().max(1) as u32, b.height().max(1) as u32);
        for p in &self.parts {
            let a = p.alpha(self.spec.alpha);
            if a <= 0.004 {
                continue;
            }
            let local = p.pos - Vec2::new(b.left as f32, b.top as f32);
            stamp(img, self.spec.shape, local, p.vel, self.radius(p), p.color, a);
        }
        Some((b.left, b.top))
    }
}

/// Composite one particle centred at `c` (image px).
fn stamp(img: &mut Image, shape: Shape, c: Vec2, vel: Vec2, r: f32, rgb: [u8; 3], alpha: f32) {
    let e = r.max(1.0) + 1.5;
    let (x0, y0) = (((c.x - e).floor() as i32).max(0), ((c.y - e).floor() as i32).max(0));
    let (x1, y1) = (((c.x + e).ceil() as i32).min(img.width as i32), ((c.y + e).ceil() as i32).min(img.height as i32));
    // Shards point along their motion (sideways when nearly still).
    let axis = if vel.length_squared() > 1.0 { vel.normalize() } else { Vec2::new(1.0, 0.0) };
    for y in y0..y1 {
        for x in x0..x1 {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c;
            let (cover, white) = match shape {
                Shape::Dust => {
                    // Soft disc, never smaller than about a pixel.
                    let rr = r.max(0.8);
                    ((rr + 0.5 - d.length()).clamp(0.0, 1.0) * (1.0 - 0.35 * (d.length() / rr).min(1.0)), 0.0)
                }
                Shape::Shard => {
                    let (len, wid) = (r.max(1.2), (r * 0.32).max(0.6));
                    let (u, v) = (d.dot(axis).abs(), d.perp_dot(axis).abs());
                    let s = u / len + v / wid;
                    (((1.0 - s) * wid + 0.6).clamp(0.0, 1.0), (1.0 - s).max(0.0))
                }
                Shape::Spark => {
                    let rr = r.max(0.8);
                    let core = (-(d.length() / (rr * 0.6)).powi(2)).exp();
                    let arm = |along: f32, across: f32| if along.abs() < rr * 2.0 { (1.0 - along.abs() / (rr * 2.0)) * (-(across / 0.55).powi(2)).exp() } else { 0.0 };
                    ((core + 0.7 * (arm(d.x, d.y) + arm(d.y, d.x))).min(1.0), core)
                }
            };
            let a = cover * alpha;
            if a <= 0.004 {
                continue;
            }
            let mix = |ch: u8| (ch as f32 + (255.0 - ch as f32) * white * 0.6) as u8;
            let s = premul(mix(rgb[0]), mix(rgb[1]), mix(rgb[2]), (a * 255.0).round().min(255.0) as u8);
            let px = &mut img.px[(y as u32 * img.width + x as u32) as usize];
            *px = over(s, *px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPU: f32 = 38.4; // default scale 0.6

    fn run(spec: Spec, seed: u32, secs: f32) -> Vec<usize> {
        let mut b = Burst::new(spec, PPU, seed);
        b.emit(Vec2::new(500.0, 300.0), 1.0);
        let mut counts = vec![b.len()];
        let mut t = 0.0;
        while t < secs {
            b.step(1.0 / 120.0);
            t += 1.0 / 120.0;
            counts.push(b.len());
        }
        counts
    }

    #[test]
    fn spawn_counts_stay_in_range() {
        for (spec, name) in [(LAND_DUST, "land"), (HARD_LAND_DUST, "hard"), (JUMP_DUST, "jump"), (DASH_DUST, "dash"), (WALL_DUST, "wall"), (SD_BURST_SHARDS, "burst"), (SD_BREAK_SHARDS, "break"), (GEO_SPARKLE, "geo")] {
            for seed in 0..200 {
                let mut b = Burst::new(spec, PPU, seed);
                b.emit(Vec2::ZERO, -1.0);
                assert!((spec.count[0] as usize..=spec.count[1] as usize).contains(&b.len()), "{name} seed {seed}: {}", b.len());
            }
        }
    }

    #[test]
    fn every_particle_dies_within_its_lifetime() {
        for spec in [LAND_DUST, HARD_LAND_DUST, SD_BURST_SHARDS, SD_BREAK_SHARDS, GEO_SPARKLE] {
            let counts = run(spec, 7, spec.life[1] + 0.02);
            assert!(counts[0] > 0);
            assert_eq!(*counts.last().unwrap(), 0, "all gone after the longest life");
            assert!(counts.windows(2).all(|w| w[1] <= w[0]), "a one-shot burst never grows");
        }
    }

    #[test]
    fn fades_in_quickly_then_out_to_nothing() {
        let p = Particle { pos: Vec2::ZERO, vel: Vec2::ZERO, born: Vec2::ZERO, age: 0.0, life: 1.0, size: 2.0, color: [255; 3] };
        let at = |age: f32| Particle { age, ..p }.alpha(0.8);
        assert_eq!(at(0.0), 0.0);
        assert!((at(0.08) - 0.8 * 0.92f32.powf(1.5)).abs() < 1e-4);
        let mut prev = f32::INFINITY;
        for i in 8..=100 {
            let a = at(i as f32 / 100.0);
            assert!(a <= prev + 1e-6, "monotonic fade after the rise");
            prev = a;
        }
        assert_eq!(at(1.0), 0.0);
    }

    #[test]
    fn particles_stay_within_reach_and_bounds_cover_them() {
        for spec in [HARD_LAND_DUST, SD_BURST_SHARDS, SD_BREAK_SHARDS, GEO_SPARKLE, DASH_DUST] {
            let mut b = Burst::new(spec, PPU, 3);
            b.emit(Vec2::new(200.0, 200.0), 1.0);
            let mut img = Image::default();
            for _ in 0..90 {
                b.step(1.0 / 120.0);
                for p in b.particles() {
                    assert!(p.pos.distance(p.born) <= spec.reach * PPU);
                }
                if let Some(r) = b.bounds() {
                    for p in b.particles() {
                        assert!(r.contains(p.pos.x as i32, p.pos.y as i32));
                    }
                    // Small: well under a fifth of a 1080p screen either way.
                    assert!(r.width() < 400 && r.height() < 300, "{r:?}");
                    let (x, y) = b.draw(&mut img).expect("drawn");
                    assert_eq!((x, y, img.width as i32, img.height as i32), (r.left, r.top, r.width(), r.height()));
                }
            }
        }
    }

    #[test]
    fn deterministic_for_a_seed() {
        let pos = |seed| {
            let mut b = Burst::new(SD_BURST_SHARDS, PPU, seed);
            b.emit(Vec2::new(10.0, 10.0), -1.0);
            b.step(0.1);
            b.particles().iter().map(|p| (p.pos.x.to_bits(), p.pos.y.to_bits())).collect::<Vec<_>>()
        };
        assert_eq!(pos(42), pos(42));
        assert_ne!(pos(42), pos(43));
    }

    #[test]
    fn directions_follow_the_spec() {
        // Dash dust flies back (against `dir`), the burst shards behind the flight.
        for (spec, dir) in [(DASH_DUST, 1.0), (SD_BURST_SHARDS, -1.0), (RUN_DUST, 1.0)] {
            let mut b = Burst::new(spec, PPU, 9);
            b.emit(Vec2::ZERO, dir);
            assert!(b.particles().iter().all(|p| p.vel.x * dir < 0.0));
        }
        // Landing dust spreads both ways and starts upwards; sparkles rise.
        let mut b = Burst::new(HARD_LAND_DUST, PPU, 1);
        b.emit(Vec2::ZERO, 1.0);
        assert!(b.particles().iter().any(|p| p.vel.x > 0.0) && b.particles().iter().any(|p| p.vel.x < 0.0));
        assert!(b.particles().iter().all(|p| p.vel.y < 0.0));
        let mut g = Burst::new(GEO_SPARKLE, PPU, 1);
        g.emit(Vec2::ZERO, 1.0);
        for _ in 0..30 {
            g.step(1.0 / 120.0);
        }
        assert!(g.particles().iter().all(|p| p.pos.y < p.born.y));
    }

    #[test]
    fn streams_emit_at_their_rate_and_stay_capped() {
        let mut b = Burst::new(RUN_DUST, PPU, 5);
        let mut emitted = 0;
        let mut x = 0.0;
        for _ in 0..120 {
            let before = b.len();
            b.feed(1.0 / 120.0, Vec2::new(x, 0.0), 1.0);
            emitted += b.len() - before;
            b.step(1.0 / 120.0);
            x += 9.0 * PPU / 120.0;
        }
        assert!((7..=8).contains(&emitted), "one second of running: {emitted}");
        assert!(b.len() <= 5, "only the last ~0.45 s are alive: {}", b.len());
        let mut c = Burst::new(Spec { rate: 2000.0, ..SD_CHARGE_SHARDS }, PPU, 5);
        c.feed(1.0, Vec2::ZERO, 1.0);
        assert_eq!(c.len(), MAX_PARTICLES);
    }

    #[test]
    fn draws_something_faint_and_in_colour() {
        let mut b = Burst::new(SD_BURST_SHARDS, PPU, 11);
        b.emit(Vec2::new(100.0, 100.0), 1.0);
        b.step(0.05);
        let mut img = Image::default();
        b.draw(&mut img).expect("live");
        let lit: Vec<u32> = img.px.iter().copied().filter(|p| p >> 24 > 0).collect();
        assert!(!lit.is_empty());
        assert!(lit.iter().all(|p| p >> 24 <= 255));
        // Pinkish: red channel at least the green one (premultiplied).
        assert!(lit.iter().all(|p| (p >> 16) & 0xff >= (p >> 8) & 0xff));
        // Mostly empty canvas: a few sparse bits, not a cloud.
        assert!(lit.len() * 4 < img.px.len(), "{} of {}", lit.len(), img.px.len());
    }
}
