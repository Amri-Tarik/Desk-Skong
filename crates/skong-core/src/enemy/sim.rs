//! One live enemy: arrive → roam ⇄ notice / chase / attack → (struck) → die → corpse.
use crate::ai::rng::Rng;
use crate::ai::{Foe, FoeKind};
use crate::anim::{Animator, ClipId};
use crate::assets::SpriteKey;
use crate::desktop::WindowId;
use crate::geom::{RectF, Vec2};
use crate::npc::once_secs;
use crate::props::ledge_below;
use crate::terrain::Terrain;

use super::{EnemySet, EntryKind, EnemyPlacement, Moves};

/// Context for one tick.
pub struct EnemyEnv<'a> {
    pub terrain: &'a Terrain,
    /// The knight's body (`None` while he is dead / hidden: nobody to fight).
    pub knight: Option<RectF>,
    /// Pixels per game unit (render scale).
    pub ppu: f32,
    /// Work area of the monitor the enemy lives on: it never leaves it.
    pub area: RectF,
}

/// What happened this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnemyTick {
    /// It bumped into / struck the knight.
    pub hit_knight: bool,
    /// Fully gone (faded corpse, or wandered off): drop it.
    pub gone: bool,
}

/// Coarse phase (for the brain, the app and tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyPhase {
    Arriving,
    Roaming,
    Turning,
    /// Noticed the knight (startle).
    Noticing,
    Chasing,
    Antic,
    Attacking,
    Recovering,
    /// Struck dead, flying back / falling.
    Dying,
    /// Lying dead, fading away.
    Corpse,
    /// Wandering off (nobody came to fight).
    Leaving,
    Gone,
}

pub struct Enemy {
    pub id: u32,
    pub def: usize,
    /// Feet point (flyers: the bottom of their art while hovering).
    pub pos: Vec2,
    /// px/s: knockback, falls, death throws.
    vel: Vec2,
    /// Art is drawn facing left: mirrored while this is set.
    pub facing_right: bool,
    /// Opacity 0..1 (fade in / out).
    pub alpha: f32,
    pub hp: u32,
    /// White hit flash, 1 → 0.
    pub flash: f32,
    anim: Animator,
    phase: EnemyPhase,
    /// Seconds in the phase (or left of it, for timed phases).
    t: f32,
    /// Ground roaming direction (±1) / where an entry or a departure heads.
    dir: f32,
    target: Vec2,
    /// Flyers hover around here.
    home: Vec2,
    /// Ground: falling (no ledge under the feet).
    airborne: bool,
    owner: Option<WindowId>,
    rel: Vec2,
    /// Ground: the walkable span of the current ledge (x0, x1) for the feet point.
    span: (f32, f32),
    rng: Rng,
    /// Seconds of roaming left before wandering off.
    stay: f32,
    contact_cd: f32,
    /// Roaming: seconds until the next pause / of the current pause.
    pause_in: f32,
    paused: f32,
    /// Seconds since it was last struck.
    since_struck: f32,
    /// Pixels per game unit at the last tick.
    ppu: f32,
    /// Sent away (a gauntlet ended early): it wanders off and is no foe any more.
    dismissed: bool,
}

const FADE_IN: f32 = 0.4;
const FADE_OUT: f32 = 0.8;
const CORPSE_SECS: f32 = 2.5;
const CONTACT_CD: f32 = 1.5;
/// Gravity and terminal speed in game units (like the visitors').
const GRAVITY: f32 = 60.0;
const MAX_FALL: f32 = 30.0;

impl Enemy {
    pub fn new(id: u32, def: usize, set: &EnemySet, at: EnemyPlacement, seed: u32, ppu: f32) -> Self {
        let d = &set.defs[def];
        let c = &set.clips[def];
        let mut rng = Rng::new(seed);
        let _ = rng.next_u32();
        let (pos, airborne, clip) = match (at.entry, d.moves) {
            (EntryKind::Edge { from }, _) => (from, false, c.moving),
            (EntryKind::Drop, Moves::Ground) => (at.at - Vec2::new(0.0, 3.0 * ppu), true, c.fall.unwrap_or(c.idle)),
            (EntryKind::Drop, Moves::Fly) => (at.at - Vec2::new(0.0, 2.0 * ppu), false, c.moving),
        };
        let mut anim = Animator::new(clip);
        anim.play(set.lib(def), clip, true);
        let stay = rng.range(d.stay[0], d.stay[1]);
        let dir = if at.at.x >= pos.x { 1.0 } else { -1.0 };
        Self {
            id,
            def,
            pos,
            vel: Vec2::ZERO,
            facing_right: dir > 0.0,
            alpha: 0.0,
            hp: d.hp,
            flash: 0.0,
            anim,
            phase: EnemyPhase::Arriving,
            t: 0.0,
            dir,
            target: at.at,
            home: at.at,
            airborne,
            owner: at.owner,
            rel: Vec2::ZERO,
            span: (f32::NEG_INFINITY, f32::INFINITY),
            rng,
            stay,
            contact_cd: 0.0,
            pause_in: 3.0,
            paused: 0.0,
            since_struck: f32::INFINITY,
            ppu,
            dismissed: false,
        }
    }

    /// Stay at least `secs` before wandering off on its own (gauntlet waves wait for him).
    pub fn hold(&mut self, secs: f32) {
        self.stay = self.stay.max(secs);
    }

    /// Wander off now (from its next tick), harmless and ignored by the knight.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    pub fn dismissed(&self) -> bool {
        self.dismissed
    }

    /// Call once after creation so an enemy placed on a window rides it.
    pub fn attach(&mut self, env: &EnemyEnv) {
        self.resync_rel(env);
    }

    pub fn phase(&self) -> EnemyPhase {
        self.phase
    }

    pub fn alive(&self) -> bool {
        !matches!(self.phase, EnemyPhase::Dying | EnemyPhase::Corpse | EnemyPhase::Gone)
    }

    /// Can be struck right now.
    pub fn vulnerable(&self) -> bool {
        self.alive() && self.alpha >= 0.5
    }

    pub fn sprite(&self, set: &EnemySet) -> Option<SpriteKey> {
        self.anim.current(set.lib(self.def))
    }

    pub fn clip(&self) -> ClipId {
        self.anim.clip()
    }

    /// Body rect (screen px).
    pub fn body(&self, set: &EnemySet, ppu: f32) -> RectF {
        let s = set.defs[self.def].size;
        RectF::from_feet(self.pos, Vec2::new(s[0], s[1]) * ppu)
    }

    /// The brain's view of this enemy (alive only).
    pub fn foe(&self, set: &EnemySet, ppu: f32) -> Option<Foe> {
        if !self.alive() || self.dismissed || self.phase == EnemyPhase::Leaving && self.alpha < 0.5 {
            return None;
        }
        let d = &set.defs[self.def];
        let b = self.body(set, ppu);
        let flying = d.moves == Moves::Fly;
        Some(Foe {
            id: self.id,
            kind: FoeKind::Enemy,
            pos: b.center(),
            feet: self.pos,
            half: Vec2::new(b.width(), b.height()) * 0.5,
            flying,
            vulnerable: self.vulnerable(),
            attacking: matches!(self.phase, EnemyPhase::Antic | EnemyPhase::Attacking | EnemyPhase::Chasing),
        })
    }

    /// Struck by the knight from `from` (his body centre). Returns `true` when this blow
    /// kills it (the app bursts its Geo then).
    pub fn strike(&mut self, damage: u32, from: Vec2, set: &EnemySet, ppu: f32) -> bool {
        if !self.vulnerable() {
            return false;
        }
        let d = &set.defs[self.def];
        let c = &set.clips[self.def];
        let lib = set.lib(self.def);
        self.hp = self.hp.saturating_sub(damage.max(1));
        self.flash = 1.0;
        self.since_struck = 0.0;
        self.stay = self.stay.max(20.0);
        let centre = self.body(set, ppu).center();
        let away = (centre - from).normalize_or(Vec2::X);
        let side = if away.x >= 0.0 { 1.0 } else { -1.0 };
        if self.hp == 0 {
            // Thrown back and up; death clip in the air, then it lands dead.
            self.phase = EnemyPhase::Dying;
            self.t = 0.0;
            self.vel = Vec2::new(side * 5.0 * ppu, -7.0 * ppu);
            self.airborne = true;
            self.facing_right = side < 0.0; // looks back at the one who struck it
            self.anim.play(lib, c.death_air, true);
            return true;
        }
        // Recoil: knocked back (flyers in any direction), a wind-up is cut short.
        self.vel = match d.moves {
            Moves::Ground => Vec2::new(side * 7.0 * ppu, 0.0),
            Moves::Fly => away * 8.0 * ppu,
        };
        match self.phase {
            EnemyPhase::Antic | EnemyPhase::Attacking => self.recover(set, false),
            EnemyPhase::Leaving | EnemyPhase::Roaming | EnemyPhase::Turning => {
                // Struck: it turns to face its attacker.
                self.facing_right = side < 0.0;
                self.dir = -side;
                if self.phase != EnemyPhase::Roaming {
                    self.roam(set);
                }
            }
            _ => {}
        }
        false
    }

    pub fn tick(&mut self, dt: f32, set: &EnemySet, env: &EnemyEnv) -> EnemyTick {
        let d = &set.defs[self.def];
        let c = set.clips[self.def].clone();
        let lib = set.lib(self.def);
        let ppu = env.ppu;
        let mut out = EnemyTick::default();
        self.ppu = ppu;
        self.t += dt;
        self.contact_cd -= dt;
        self.since_struck += dt;
        self.flash = (self.flash - dt / 0.2).max(0.0);
        let mut events = Vec::new();
        self.anim.update(lib, dt, &mut events);
        if !matches!(self.phase, EnemyPhase::Leaving | EnemyPhase::Corpse | EnemyPhase::Gone) {
            self.alpha = (self.alpha + dt / FADE_IN).min(1.0);
        }

        // Footing / flight first, then what it wants to do.
        let flying = d.moves == Moves::Fly && self.alive();
        if flying {
            self.fly_physics(dt, env);
        } else {
            self.ground_physics(dt, env, set);
        }
        let knight = env.knight;
        let me = self.body(set, ppu);
        let to_knight = knight.map(|k| k.center() - me.center());
        let dist = to_knight.map_or(f32::INFINITY, |v| v.length());
        let level = knight.is_some_and(|k| (k.bottom - self.pos.y).abs() < 0.8 * ppu && k.center().x > self.span.0 - 2.0 * ppu && k.center().x < self.span.1 + 2.0 * ppu);
        let notices = d.aggro > 0.0 && dist < d.aggro * ppu && (flying || level);
        // Sent away: off it goes as soon as it has footing (or wings).
        if self.dismissed && self.alive() && self.phase != EnemyPhase::Leaving && (flying || !self.airborne) {
            self.leave(set, env);
        }

        match self.phase {
            EnemyPhase::Arriving => {
                if flying {
                    if self.steer(self.target, d.speed * 1.5 * ppu, dt) {
                        self.home = self.target;
                        self.roam(set);
                    }
                } else if !self.airborne {
                    let step = d.speed * ppu * dt;
                    let dx = self.target.x - self.pos.x;
                    if dx.abs() <= step || self.alpha >= 1.0 && dx.abs() < 1.0 {
                        self.roam(set);
                    } else {
                        self.dir = dx.signum();
                        self.facing_right = self.dir > 0.0;
                        self.walk(step * self.dir, env);
                        if self.anim.clip() != c.moving {
                            self.anim.play(lib, c.moving, false);
                        }
                    }
                }
            }
            EnemyPhase::Roaming => {
                self.stay -= dt;
                if notices {
                    self.notice(set, to_knight.unwrap_or(Vec2::X));
                } else if self.stay <= 0.0 && self.since_struck > 10.0 {
                    self.leave(set, env);
                } else if flying {
                    self.hover(set, dt);
                } else {
                    self.patrol(set, env, dt);
                }
            }
            EnemyPhase::Turning => {
                if self.anim.is_finished() || self.t >= once_secs(lib, self.anim.clip()) {
                    self.dir = -self.dir;
                    self.roam(set);
                }
            }
            EnemyPhase::Noticing => {
                if let Some(v) = to_knight {
                    self.face(v.x);
                }
                if self.anim.is_finished() || self.t >= once_secs(lib, self.anim.clip()) {
                    self.phase = EnemyPhase::Chasing;
                    self.t = 0.0;
                    self.anim.play(lib, c.chase, false);
                }
            }
            EnemyPhase::Chasing => match (knight, to_knight) {
                (Some(k), Some(v)) if dist < 1.7 * d.aggro * ppu && (flying || level) => {
                    self.face(v.x);
                    if flying {
                        // Body centre onto his body centre.
                        self.steer(k.center() + Vec2::new(0.0, 0.5 * me.height()), d.chase_speed() * ppu, dt);
                    } else if c.antic.is_some() && v.x.abs() < d.reach * ppu {
                        self.phase = EnemyPhase::Antic;
                        self.t = 0.0;
                        self.anim.play(lib, c.antic.unwrap_or(c.idle), true);
                    } else if v.x.abs() > 0.5 * me.width() {
                        let step = d.chase_speed() * ppu * dt * v.x.signum();
                        self.walk(step, env);
                    }
                }
                _ => self.roam(set),
            },
            EnemyPhase::Antic => {
                if self.anim.is_finished() || self.t >= once_secs(lib, self.anim.clip()).max(0.3) {
                    self.phase = EnemyPhase::Attacking;
                    self.t = 0.0;
                    self.anim.play(lib, c.attack.unwrap_or(c.chase), true);
                }
            }
            EnemyPhase::Attacking => {
                let len = c.attack.map_or(0.5, |a| once_secs(lib, a)).max(0.3);
                let dir = if self.facing_right { 1.0 } else { -1.0 };
                self.walk(dir * d.lunge * ppu * dt, env);
                if self.t >= len {
                    self.recover(set, false);
                }
            }
            EnemyPhase::Recovering => {
                if flying && self.t < 0.6 {
                    // Backing off after a bump.
                    let away = Vec2::new(-self.dir_x(), -0.4).normalize_or(Vec2::NEG_Y);
                    self.pos += away * d.speed * ppu * dt;
                    self.pos = self.clamp_fly(self.pos, env);
                }
                if self.t >= 0.6 {
                    if notices || (dist < 1.7 * d.aggro * ppu && (flying || level)) {
                        self.phase = EnemyPhase::Chasing;
                        self.t = 0.0;
                        self.anim.play(lib, c.chase, false);
                    } else {
                        self.home = self.pos;
                        self.roam(set);
                    }
                }
            }
            EnemyPhase::Dying => {
                if !self.airborne {
                    self.phase = EnemyPhase::Corpse;
                    self.t = 0.0;
                    self.vel = Vec2::ZERO;
                    self.anim.play(lib, c.death_land, true);
                }
            }
            EnemyPhase::Corpse => {
                if self.t > CORPSE_SECS {
                    self.alpha -= dt / FADE_OUT;
                    if self.alpha <= 0.0 {
                        self.alpha = 0.0;
                        self.phase = EnemyPhase::Gone;
                    }
                }
            }
            EnemyPhase::Leaving => {
                let speed = d.speed * ppu * dt;
                if flying {
                    self.steer(self.target, d.speed * 1.5 * ppu, dt);
                } else {
                    self.walk(self.dir * speed, env);
                }
                let at_end = if flying { self.pos.distance(self.target) < 6.0 } else { (self.pos.x - self.target.x).abs() < 2.0 || self.blocked_at_span_end() };
                if at_end || self.alpha < 1.0 {
                    self.alpha -= dt / FADE_OUT;
                }
                if self.alpha <= 0.0 {
                    self.alpha = 0.0;
                    self.phase = EnemyPhase::Gone;
                }
            }
            EnemyPhase::Gone => {}
        }

        // Contact: bumping into the knight hurts him (now and then, never while dying).
        let harmful = matches!(
            self.phase,
            EnemyPhase::Roaming | EnemyPhase::Turning | EnemyPhase::Noticing | EnemyPhase::Chasing | EnemyPhase::Antic | EnemyPhase::Attacking | EnemyPhase::Recovering
        );
        if harmful
            && self.alpha >= 0.9
            && self.contact_cd <= 0.0
            && let Some(k) = knight
            && k.overlaps(&self.body(set, ppu))
        {
            out.hit_knight = true;
            self.contact_cd = CONTACT_CD;
            if flying {
                self.recover(set, true);
            } else {
                let side = if k.center().x >= self.pos.x { 1.0 } else { -1.0 };
                self.vel.x = -side * 4.0 * ppu;
            }
        }
        out.gone = self.phase == EnemyPhase::Gone;
        out
    }

    fn dir_x(&self) -> f32 {
        if self.facing_right { 1.0 } else { -1.0 }
    }

    fn face(&mut self, dx: f32) {
        if dx.abs() > 2.0 {
            self.facing_right = dx > 0.0;
            self.dir = dx.signum();
        }
    }

    fn roam(&mut self, set: &EnemySet) {
        let c = &set.clips[self.def];
        self.phase = EnemyPhase::Roaming;
        self.t = 0.0;
        self.paused = 0.0;
        self.pause_in = self.rng.range(2.0, 6.0);
        self.facing_right = self.dir > 0.0;
        self.anim.play(set.lib(self.def), c.moving, false);
    }

    fn notice(&mut self, set: &EnemySet, to_knight: Vec2) {
        let c = &set.clips[self.def];
        let lib = set.lib(self.def);
        self.face(to_knight.x);
        self.t = 0.0;
        match c.startle {
            Some(s) => {
                self.phase = EnemyPhase::Noticing;
                self.anim.play(lib, s, true);
            }
            None => {
                self.phase = EnemyPhase::Chasing;
                self.anim.play(lib, c.chase, false);
            }
        }
    }

    /// The pause after an attack (or a bump; `bumped` flyers back off first).
    fn recover(&mut self, set: &EnemySet, bumped: bool) {
        let c = &set.clips[self.def];
        self.phase = EnemyPhase::Recovering;
        self.t = 0.0;
        if bumped {
            self.dir = self.dir_x();
        }
        let clip = c.recover.unwrap_or(c.idle);
        let moves = set.defs[self.def].moves;
        self.anim.play(set.lib(self.def), if moves == Moves::Fly { c.moving } else { clip }, true);
    }

    /// Wander off: ground enemies walk to the end of their ledge nearer the monitor edge,
    /// flyers fly out the nearer side; both fade out there.
    fn leave(&mut self, set: &EnemySet, env: &EnemyEnv) {
        let c = &set.clips[self.def];
        self.phase = EnemyPhase::Leaving;
        self.t = 0.0;
        let left = self.pos.x - env.area.left < env.area.right - self.pos.x;
        if set.defs[self.def].moves == Moves::Fly {
            let x = if left { env.area.left } else { env.area.right };
            self.target = Vec2::new(x, self.pos.y - 1.0 * env.ppu);
        } else {
            let (x0, x1) = self.span;
            let x = if left { x0.max(env.area.left) } else { x1.min(env.area.right) };
            self.target = Vec2::new(x, self.pos.y);
        }
        self.dir = if self.target.x >= self.pos.x { 1.0 } else { -1.0 };
        self.facing_right = self.dir > 0.0;
        self.anim.play(set.lib(self.def), c.moving, false);
    }

    /// Ground roaming: walk the ledge, turn at its ends, pause now and then.
    fn patrol(&mut self, set: &EnemySet, env: &EnemyEnv, dt: f32) {
        let d = &set.defs[self.def];
        let c = &set.clips[self.def];
        let lib = set.lib(self.def);
        if self.paused > 0.0 {
            self.paused -= dt;
            if self.paused <= 0.0 {
                self.anim.play(lib, c.moving, false);
                self.pause_in = self.rng.range(2.5, 6.0);
            }
            return;
        }
        self.pause_in -= dt;
        if self.pause_in <= 0.0 {
            self.paused = self.rng.range(0.8, 2.0);
            self.anim.play(lib, c.idle, false);
            return;
        }
        let before = self.pos.x;
        self.walk(self.dir * d.speed * env.ppu * dt, env);
        let blocked = (self.pos.x - before).abs() < 0.25 * d.speed * env.ppu * dt;
        if blocked && !self.airborne {
            self.turn(set);
        }
    }

    fn turn(&mut self, set: &EnemySet) {
        let c = &set.clips[self.def];
        match c.turn {
            Some(t) => {
                self.phase = EnemyPhase::Turning;
                self.t = 0.0;
                self.anim.play(set.lib(self.def), t, true);
                // The turn clip is drawn mid-turn; mirror at its end (`Turning` above).
            }
            None => {
                self.dir = -self.dir;
                self.facing_right = self.dir > 0.0;
            }
        }
    }

    /// Flyer roaming: drift between points around home, bobbing.
    fn hover(&mut self, set: &EnemySet, dt: f32) {
        let d = &set.defs[self.def];
        self.pause_in -= dt;
        if self.pause_in <= 0.0 || self.t < dt * 1.5 {
            let off = Vec2::new(self.rng.range(-2.5, 2.5), self.rng.range(-0.8, 0.8)) * self.ppu;
            self.target = self.home + off;
            self.pause_in = self.rng.range(1.5, 3.0);
        }
        let to = self.target - self.pos;
        if to.length() > 2.0 {
            self.face(to.x);
            let step = to.normalize_or(Vec2::ZERO) * (d.speed * self.ppu * dt).min(to.length());
            self.pos += step;
        }
    }

    /// Fly toward `to` at `speed` px/s; `true` once there.
    fn steer(&mut self, to: Vec2, speed: f32, dt: f32) -> bool {
        let v = to - self.pos;
        let step = speed * dt;
        if v.length() <= step.max(1.0) {
            self.pos = to;
            return true;
        }
        self.face(v.x);
        self.pos += v.normalize_or(Vec2::ZERO) * step;
        false
    }

    fn clamp_fly(&self, p: Vec2, env: &EnemyEnv) -> Vec2 {
        let m = 0.6 * env.ppu;
        let a = env.area;
        let top = a.top + 2.0 * env.ppu;
        Vec2::new(p.x.clamp(a.left + m, (a.right - m).max(a.left + m)), p.y.clamp(top, a.bottom.max(top)))
    }

    fn fly_physics(&mut self, dt: f32, env: &EnemyEnv) {
        self.pos += self.vel * dt;
        self.vel *= (-8.0 * dt).exp();
        if self.phase != EnemyPhase::Arriving && self.phase != EnemyPhase::Leaving {
            self.pos = self.clamp_fly(self.pos, env);
        }
    }

    /// Ground enemies ride their window, walk their ledge, fall when it goes away; dying
    /// bodies (flyers too) fly back and drop onto the ledge below.
    fn ground_physics(&mut self, dt: f32, env: &EnemyEnv, set: &EnemySet) {
        let ppu = env.ppu;
        let half = 0.5 * set.defs[self.def].size[0] * ppu;
        if !self.airborne {
            if let Some(r) = self.owner.and_then(|id| env.terrain.window_rect(id)) {
                self.pos = Vec2::new(r.left, r.top) + self.rel;
            }
            match env.terrain.ledges.iter().find(|l| (l.y - self.pos.y).abs() < 1.5 && l.x0 - 1.0 <= self.pos.x && self.pos.x <= l.x1 + 1.0) {
                Some(l) => {
                    let leaving = self.phase == EnemyPhase::Leaving;
                    let (a0, a1) = if leaving { (l.x0, l.x1) } else { (l.x0.max(env.area.left) + half, l.x1.min(env.area.right) - half) };
                    self.span = if a1 > a0 { (a0, a1) } else { (self.pos.x, self.pos.x) };
                    self.owner = l.owner;
                    // Knockback slides along the ledge, never off it.
                    if self.vel.x != 0.0 {
                        self.pos.x = (self.pos.x + self.vel.x * dt).clamp(self.span.0, self.span.1);
                        self.vel.x *= (-8.0 * dt).exp();
                        if self.vel.x.abs() < 1.0 {
                            self.vel.x = 0.0;
                        }
                    }
                    self.resync_rel(env);
                    return;
                }
                None => {
                    // Its ledge went away: fall.
                    self.airborne = true;
                    self.vel.y = 0.0;
                }
            }
        }
        // Falling.
        self.vel.y = (self.vel.y + GRAVITY * ppu * dt).min(MAX_FALL * ppu);
        self.vel.x *= (-2.0 * dt).exp();
        let a = env.area;
        let nx = (self.pos.x + self.vel.x * dt).clamp(a.left + half, (a.right - half).max(a.left + half));
        let ny = self.pos.y + self.vel.y * dt;
        let below = if self.vel.y > 0.0 { ledge_below(env.terrain, Vec2::new(nx, self.pos.y)) } else { None };
        match below {
            Some((y, owner)) if ny >= y => {
                self.pos = Vec2::new(nx, y);
                self.land(owner, env);
            }
            None if self.vel.y > 0.0 && ny >= env.terrain.bounds.bottom => {
                self.pos = Vec2::new(nx, env.terrain.bounds.bottom);
                self.land(None, env);
            }
            _ => self.pos = Vec2::new(nx, ny),
        }
    }

    fn land(&mut self, owner: Option<WindowId>, env: &EnemyEnv) {
        self.airborne = false;
        self.vel = Vec2::ZERO;
        self.owner = owner;
        self.span = (f32::NEG_INFINITY, f32::INFINITY);
        self.resync_rel(env);
    }

    /// Ground: move `dx` along the current ledge (stops at its walkable ends).
    fn walk(&mut self, dx: f32, env: &EnemyEnv) {
        if self.airborne {
            return;
        }
        let (x0, x1) = self.span;
        let nx = self.pos.x + dx;
        self.pos.x = if self.phase == EnemyPhase::Arriving { nx } else { nx.clamp(x0.min(self.pos.x), x1.max(self.pos.x)) };
        self.resync_rel(env);
    }

    fn blocked_at_span_end(&self) -> bool {
        (self.dir > 0.0 && self.pos.x >= self.span.1 - 0.5) || (self.dir < 0.0 && self.pos.x <= self.span.0 + 0.5)
    }

    fn resync_rel(&mut self, env: &EnemyEnv) {
        if let Some(r) = self.owner.and_then(|id| env.terrain.window_rect(id)) {
            self.rel = self.pos - Vec2::new(r.left, r.top);
        }
    }
}
