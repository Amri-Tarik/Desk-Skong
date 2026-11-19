//! Grimm himself: the entrance and bow, the routine of teleports and attacks, his flames,
//! the phase changes, and the three endings (beaten, triumphant, bowing out).
//!
//! Nothing here assumes an arena. Every spot is chosen relative to the knight: the ledge he
//! stands on (or the one under him, else the monitor floor) gives the "ground" line and its
//! span; Grimm floats wherever a spot has no ledge (he is Grimm), and the knight's monitor
//! work area bounds everything. Flames ignore windows.
use crate::ai::rng::Rng;
use crate::ai::{Foe, FoeKind};
use crate::anim::{Animator, ClipId};
use crate::assets::SpriteKey;
use crate::geom::{RectF, Vec2};
use crate::terrain::Terrain;

use super::{GrimmSet, LineKind};

/// Grimm's id among the foes (`FoeKind::Boss`).
pub const GRIMM_ID: u32 = 1;
/// The lantern's id (`FoeKind::Lantern`).
pub const LANTERN_ID: u32 = 2;
/// Hazard ids start here (`FoeKind::Hazard`).
const HAZARD_IDS: u32 = 100;
/// Spikes only cue the knight (a jump) this close to erupting (s): earlier he'd land first.
pub const SPIKE_CUE: f32 = 0.3;

/// The world for one tick.
pub struct GrimmEnv<'a> {
    pub terrain: &'a Terrain,
    /// The knight's body (`None` while he is dead / hidden).
    pub knight: Option<RectF>,
    /// Pixels per game unit (the knight's render scale).
    pub ppu: f32,
    /// Work area of the knight's monitor: the stage.
    pub area: RectF,
}

/// Grimm's attacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Move {
    /// Appears high beside the knight, dives at where he stands, skids along the ground.
    AirDash,
    /// Appears on the knight's ledge, three slashes forward, an uppercut that sheds fireballs.
    Slash,
    /// From afar at the knight's height: a volley of firebats.
    Firebats,
    /// Floats above; flame pillars erupt under the knight's feet, one after another.
    Pillars,
    /// Spikes rise along the knight's ledge (jump!).
    Spikes,
    /// At each phase change: floats up, swells, sprays rings of fireballs.
    Balloon,
}

/// What a flame is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HazardKind {
    /// Flies straight at the knight's height.
    Firebat,
    /// The uppercut's and the balloon's fireballs.
    Fireball,
    /// Erupts from the ground under the knight after a warning glow.
    Pillar,
    /// A row along his ledge, peeking up before they rise.
    Spikes,
}

/// One of Grimm's flames. Harmless while it warns (`warn` > 0), then harmful for `left` s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hazard {
    pub id: u32,
    pub kind: HazardKind,
    /// Centre (screen px).
    pub pos: Vec2,
    /// px/s.
    pub vel: Vec2,
    /// Half its harmful size (px).
    pub half: Vec2,
    /// Seconds of warning left.
    pub warn: f32,
    /// Seconds it stays harmful once armed.
    pub left: f32,
    /// px/s².
    pub gravity: f32,
    /// Seconds since it appeared (animation).
    pub age: f32,
    /// Seconds since it was armed (a pillar's eruption plays from it).
    pub lit: f32,
}

impl Hazard {
    pub fn armed(&self) -> bool {
        self.warn <= 0.0
    }

    pub fn rect(&self) -> RectF {
        RectF::new(self.pos.x - self.half.x, self.pos.y - self.half.y, self.pos.x + self.half.x, self.pos.y + self.half.y)
    }

    fn flies(&self) -> bool {
        matches!(self.kind, HazardKind::Firebat | HazardKind::Fireball)
    }

    /// The brain's view: never a target; it warns (`winding_up`: a pillar or spikes about to
    /// rise under him) or comes at him (`lunging`: a firebat / fireball in flight).
    pub fn foe(&self) -> Foe {
        let cue = match self.kind {
            HazardKind::Spikes => self.warn > 0.0 && self.warn < SPIKE_CUE,
            _ => self.warn > 0.0,
        };
        Foe {
            id: self.id,
            kind: FoeKind::Hazard,
            pos: self.pos,
            feet: self.pos + Vec2::new(0.0, self.half.y),
            half: self.half,
            flying: self.flies(),
            vulnerable: false,
            attacking: true,
            winding_up: cue,
            lunging: self.armed() && self.flies(),
            vel: if self.armed() { self.vel } else { Vec2::ZERO },
        }
    }
}

/// Where Grimm is at, coarsely (the app, the director and tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrimmStage {
    /// Appearing, bowing, his opening line.
    Intro,
    /// The fight.
    Fight,
    /// Beaten: stunned, bursting into bats.
    Beaten,
    /// Leaving (a bow and a line, then gone).
    Leaving,
    Gone,
}

/// How it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrimmOutcome {
    /// The knight won.
    Beaten,
    /// The knight fell.
    Triumphed,
    /// Called off (fullscreen app, a visitor, typing, too long): no penalty.
    BowedOut,
}

/// What hurt the knight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitBy {
    /// Grimm himself, mid-attack.
    Body(Move),
    Flame(HazardKind),
}

/// What happened this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrimmTick {
    /// Times the knight got hurt (body or flames).
    pub hits: u32,
    /// What hurt him (logs, balance).
    pub hit_by: Option<HitBy>,
    /// Grimm speaks now (the app picks the line).
    pub say: Option<LineKind>,
    /// Teleport flashes (body centres).
    pub flashes: Vec<Vec2>,
    /// Beaten: he burst into bats here (the Geo goes here).
    pub burst: Option<Vec2>,
    /// A new phase began (2 or 3).
    pub phase: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    Fight,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    /// Fading in (Tele In), then the bow.
    Enter,
    /// The bow, then a line (the intro, or a farewell).
    Bow { t: f32, then: Then },
    /// Standing while his line is read.
    Speak { left: f32, then: Then },
    Roar { left: f32, then: Option<Move> },
    Idle { left: f32 },
    /// A backstep after a blow, then away.
    Evade { left: f32 },
    TeleOut { then: Then },
    Hidden { left: f32 },
    TeleIn { mv: Move },
    Attack { mv: Move, step: u8, left: f32 },
    Stunned { left: f32 },
    Exploding { left: f32 },
    Bats { left: f32 },
    Gone,
}

/// The Troupe Master on stage.
pub struct Grimm {
    /// Feet point (screen px).
    pub pos: Vec2,
    /// His art faces left: mirrored while this is set.
    pub facing_right: bool,
    pub alpha: f32,
    /// White hit flash, 1 → 0.
    pub flash: f32,
    pub hp: u32,
    pub max_hp: u32,
    /// 1, 2, 3.
    pub phase: u8,
    pub hazards: Vec<Hazard>,
    anim: Animator,
    state: State,
    /// Seconds in the current state (tempo-scaled).
    t: f32,
    vel: Vec2,
    /// A pending phase change (handled at the next decision).
    phase_due: Option<u8>,
    last_move: Option<Move>,
    /// The move a teleport is taking him to.
    next: Option<Move>,
    /// Spawned this many of the move's flames.
    count: u32,
    ring: f32,
    /// The knight can't be hurt again for this long (s).
    invuln: f32,
    /// Blows taken while standing still (he evades after a few).
    struck_idle: u32,
    rng: Rng,
    next_hazard: u32,
    outcome: Option<GrimmOutcome>,
    /// A line to say on the next tick (one decided outside it: beaten by a blow).
    say_due: Option<LineKind>,
    ppu: f32,
}

/// The ground line of the knight's spot: the ledge he stands on or the one under him (else
/// the floor of the stage), as (y, x0, x1) within the stage.
pub fn ground_under(terrain: &Terrain, feet: Vec2, area: RectF) -> (f32, f32, f32) {
    let ledge = terrain.ledges.iter().filter(|l| l.x0 <= feet.x && feet.x <= l.x1 && l.y >= feet.y - 2.0 && l.y <= area.bottom + 1.0).min_by(|a, b| a.y.total_cmp(&b.y));
    match ledge {
        Some(l) => (l.y, l.x0.max(area.left), l.x1.min(area.right)),
        None => (area.bottom, area.left, area.right),
    }
}

impl Grimm {
    /// Grimm appearing at `at` (feet), facing `toward` (x).
    pub fn new(set: &GrimmSet, at: Vec2, toward: f32, seed: u32, ppu: f32) -> Self {
        let mut rng = Rng::new(seed);
        let _ = rng.next_u32();
        let clip = set.clips.tele_in;
        let mut anim = Animator::new(clip);
        anim.play(&set.lib, clip, true);
        anim.set_speed(set.cfg.tempo);
        let hp = set.cfg.hp.max(1);
        Self {
            pos: at,
            facing_right: toward > at.x,
            alpha: 0.0,
            flash: 0.0,
            hp,
            max_hp: hp,
            phase: 1,
            hazards: Vec::new(),
            anim,
            state: State::Enter,
            t: 0.0,
            vel: Vec2::ZERO,
            phase_due: None,
            last_move: None,
            next: None,
            count: 0,
            ring: 0.0,
            invuln: 0.0,
            struck_idle: 0,
            rng,
            next_hazard: HAZARD_IDS,
            outcome: None,
            say_due: None,
            ppu,
        }
    }

    pub fn stage(&self) -> GrimmStage {
        match self.state {
            State::Enter => GrimmStage::Intro,
            State::Bow { then: Then::Fight, .. } | State::Speak { then: Then::Fight, .. } => GrimmStage::Intro,
            State::Roar { .. } if self.last_move.is_none() && self.phase == 1 => GrimmStage::Intro,
            State::Bow { then: Then::Leave, .. } | State::Speak { then: Then::Leave, .. } | State::TeleOut { then: Then::Leave } => GrimmStage::Leaving,
            State::Stunned { .. } | State::Exploding { .. } | State::Bats { .. } => GrimmStage::Beaten,
            State::Gone => GrimmStage::Gone,
            _ => GrimmStage::Fight,
        }
    }

    /// The attack under way (wind-up included), if any.
    pub fn current_move(&self) -> Option<Move> {
        match self.state {
            State::Attack { mv, .. } | State::TeleIn { mv } => Some(mv),
            _ => None,
        }
    }

    /// Winding up an attack (the knight can read it).
    pub fn winding_up(&self) -> bool {
        matches!(self.state, State::Attack { step: 0, .. } | State::Attack { mv: Move::Slash, step: 4, .. })
    }

    /// Dashing / slashing / rising at him: his body hurts.
    pub fn lunging(&self) -> bool {
        matches!(self.state, State::Attack { mv: Move::AirDash, step: 1 | 2, .. } | State::Attack { mv: Move::Slash, step: 1..=3 | 5, .. })
    }

    pub fn outcome(&self) -> Option<GrimmOutcome> {
        self.outcome
    }

    pub fn sprite(&self, set: &GrimmSet) -> Option<SpriteKey> {
        self.anim.current(&set.lib)
    }

    pub fn clip(&self) -> ClipId {
        self.anim.clip()
    }

    /// His body: upright, or compact diving and low skidding along the ground (jump it).
    pub fn body(&self, set: &GrimmSet, ppu: f32) -> RectF {
        let size = match self.state {
            State::Attack { mv: Move::AirDash, step: 1, .. } => Vec2::new(1.6, 1.6),
            State::Attack { mv: Move::AirDash, step: 2, .. } => Vec2::new(2.2, 1.0),
            _ => Vec2::new(set.cfg.size[0], set.cfg.size[1]),
        };
        RectF::from_feet(self.pos, size * ppu)
    }

    /// Can be struck right now (visible, not mid-teleport, not ballooned, not bowing).
    pub fn vulnerable(&self) -> bool {
        self.alpha >= 0.5 && matches!(self.state, State::Idle { .. } | State::Evade { .. } | State::Roar { then: Some(_), .. } | State::Attack { mv: Move::AirDash | Move::Slash | Move::Firebats | Move::Pillars | Move::Spikes, .. })
    }

    /// The brain's view of him while the show is on (also while hidden mid-teleport, so the
    /// fight goes on; never once he is beaten or leaving).
    pub fn foe(&self, set: &GrimmSet, ppu: f32) -> Option<Foe> {
        if matches!(self.stage(), GrimmStage::Beaten | GrimmStage::Leaving | GrimmStage::Gone) {
            return None;
        }
        let b = self.body(set, ppu);
        let intro = self.stage() == GrimmStage::Intro;
        Some(Foe {
            id: GRIMM_ID,
            kind: FoeKind::Boss,
            pos: b.center(),
            feet: self.pos,
            half: Vec2::new(b.width(), b.height()) * 0.5,
            flying: self.airborne(),
            vulnerable: self.vulnerable(),
            attacking: !intro,
            winding_up: self.winding_up(),
            lunging: self.lunging(),
            vel: self.vel,
        })
    }

    /// Off the ground line (floating, dashing down, ballooned).
    fn airborne(&self) -> bool {
        matches!(self.state, State::Attack { mv: Move::AirDash, step: 0 | 1, .. } | State::Attack { mv: Move::Pillars | Move::Balloon, .. } | State::Attack { mv: Move::Slash, step: 5 | 6, .. })
    }

    /// The knight's blow. `Some(true)`: it beat him; `None`: it didn't land.
    pub fn strike(&mut self, damage: u32, set: &GrimmSet) -> Option<bool> {
        if !self.vulnerable() {
            return None;
        }
        self.hp = self.hp.saturating_sub(damage.max(1));
        self.flash = 1.0;
        if self.hp == 0 {
            self.beaten(set);
            return Some(true);
        }
        let frac = self.hp as f32 / self.max_hp as f32;
        let crossed = set.cfg.phases.iter().filter(|&&p| frac <= p).count() as u8 + 1;
        if crossed > self.phase && self.phase_due.is_none_or(|p| p < crossed) {
            self.phase_due = Some(crossed.min(3));
        }
        if matches!(self.state, State::Idle { .. }) {
            self.struck_idle += 1;
        }
        Some(false)
    }

    fn beaten(&mut self, set: &GrimmSet) {
        self.hazards.clear();
        self.vel = Vec2::ZERO;
        self.alpha = 1.0;
        self.outcome = Some(GrimmOutcome::Beaten);
        self.say_due = Some(LineKind::Defeat);
        self.go(set, State::Stunned { left: 2.0 }, set.clips.death_stun);
    }

    /// The knight fell: a bow, a gracious line, and off he goes.
    pub fn triumph(&mut self, set: &GrimmSet) {
        if self.outcome.is_none() {
            self.outcome = Some(GrimmOutcome::Triumphed);
            self.farewell(set);
        }
    }

    /// Called off: a bow and a line (`instant`: just gone, e.g. a game took the monitor).
    pub fn bow_out(&mut self, set: &GrimmSet, instant: bool) {
        if self.outcome.is_some() && !instant {
            return;
        }
        self.outcome.get_or_insert(GrimmOutcome::BowedOut);
        if instant {
            self.hazards.clear();
            self.state = State::Gone;
            self.alpha = 0.0;
        } else {
            self.farewell(set);
        }
    }

    fn farewell(&mut self, set: &GrimmSet) {
        self.hazards.clear();
        self.vel = Vec2::ZERO;
        self.go(set, State::Bow { t: 0.0, then: Then::Leave }, set.clips.bow);
    }

    fn go(&mut self, set: &GrimmSet, state: State, clip: ClipId) {
        self.state = state;
        self.t = 0.0;
        self.anim.play(&set.lib, clip, true);
    }

    fn face(&mut self, x: f32) {
        if (x - self.pos.x).abs() > 2.0 {
            self.facing_right = x > self.pos.x;
        }
    }

    fn dir(&self) -> f32 {
        if self.facing_right { 1.0 } else { -1.0 }
    }

    /// Keep him on the stage.
    fn clamp(&self, p: Vec2, set: &GrimmSet, area: RectF) -> Vec2 {
        let u = self.ppu;
        let hw = 0.5 * set.cfg.size[0] * u + 0.2 * u;
        let top = area.top + set.cfg.size[1] * u + 0.3 * u;
        Vec2::new(p.x.clamp(area.left + hw, (area.right - hw).max(area.left + hw)), p.y.clamp(top.min(area.bottom), area.bottom))
    }

    #[allow(clippy::too_many_arguments)]
    fn hazard(&mut self, kind: HazardKind, pos: Vec2, vel: Vec2, half: Vec2, warn: f32, left: f32, gravity: f32) {
        let id = self.next_hazard;
        self.next_hazard += 1;
        self.hazards.push(Hazard { id, kind, pos, vel, half, warn, left, gravity, age: 0.0, lit: 0.0 });
    }

    pub fn tick(&mut self, dt: f32, set: &GrimmSet, env: &GrimmEnv) -> GrimmTick {
        let mut out = GrimmTick { say: self.say_due.take(), ..Default::default() };
        self.ppu = env.ppu;
        let tempo = set.cfg.tempo * (1.0 + 0.12 * (self.phase as f32 - 1.0));
        self.anim.set_speed(tempo);
        let mut events = Vec::new();
        self.anim.update(&set.lib, dt, &mut events);
        let td = dt * tempo;
        self.t += td;
        self.flash = (self.flash - dt / 0.2).max(0.0);
        self.invuln = (self.invuln - dt).max(0.0);
        self.tick_hazards(dt, env);
        if let Some(p) = self.step(td, set, env, &mut out) {
            out.phase = Some(p);
        }
        self.hurt_knight(set, env, &mut out);
        out
    }

    fn tick_hazards(&mut self, dt: f32, env: &GrimmEnv) {
        let a = env.area;
        let margin = 4.0 * env.ppu;
        for h in &mut self.hazards {
            h.age += dt;
            if h.warn > 0.0 {
                h.warn -= dt;
                continue;
            }
            h.vel.y += h.gravity * dt;
            h.pos += h.vel * dt;
            h.left -= dt;
            h.lit += dt;
        }
        self.hazards.retain(|h| h.left > 0.0 && h.pos.x > a.left - margin && h.pos.x < a.right + margin && h.pos.y < a.bottom + margin && h.pos.y > a.top - margin);
    }

    fn hurt_knight(&mut self, set: &GrimmSet, env: &GrimmEnv, out: &mut GrimmTick) {
        let Some(k) = env.knight else { return };
        if self.invuln > 0.0 || self.outcome.is_some() {
            return;
        }
        let u = env.ppu;
        let mut by = self.hazards.iter().find(|h| h.armed() && h.rect().overlaps(&k)).map(|h| HitBy::Flame(h.kind));
        if by.is_none()
            && self.alpha >= 0.8
            && self.lunging()
            && let Some(mv) = self.current_move()
        {
            let mut body = self.body(set, u);
            // The slashes reach past his body.
            if matches!(self.state, State::Attack { mv: Move::Slash, step: 1..=3, .. }) {
                if self.facing_right {
                    body.right += 1.8 * u;
                } else {
                    body.left -= 1.8 * u;
                }
            }
            if body.overlaps(&k) {
                by = Some(HitBy::Body(mv));
            }
        }
        if by.is_some() {
            out.hits += 1;
            out.hit_by = by;
            self.invuln = set.cfg.invuln;
        }
    }

    /// The routine. Returns a new phase when one begins.
    fn step(&mut self, td: f32, set: &GrimmSet, env: &GrimmEnv, out: &mut GrimmTick) -> Option<u8> {
        let c = set.clips.clone();
        let u = env.ppu;
        let knight = env.knight;
        let kx = knight.map_or(self.pos.x, |k| k.center().x);
        match self.state {
            State::Enter => {
                self.alpha = (self.alpha + td / 0.3).min(1.0);
                if self.t >= 0.35 {
                    out.say = Some(LineKind::Intro);
                    self.face(kx);
                    self.go(set, State::Bow { t: 0.0, then: Then::Fight }, c.bow);
                }
            }
            State::Bow { t, then } => {
                self.alpha = (self.alpha + td / 0.3).min(1.0);
                let t = t + td;
                self.state = State::Bow { t, then };
                if then == Then::Leave && t <= td + 1e-6 {
                    out.say = Some(match self.outcome {
                        Some(GrimmOutcome::Triumphed) => LineKind::Triumph,
                        _ => LineKind::BowOut,
                    });
                }
                if t >= 0.9 {
                    self.anim.play(&set.lib, c.bow_return, true);
                    let left = if then == Then::Fight { 3.2 } else { 2.6 };
                    self.state = State::Speak { left, then };
                    self.t = 0.0;
                }
            }
            State::Speak { left, then } => {
                if self.t >= 0.3 && self.anim.clip() != c.idle {
                    self.anim.play(&set.lib, c.idle, false);
                }
                if self.t >= left {
                    match then {
                        Then::Fight => {
                            self.go(set, State::Roar { left: 1.1, then: None }, c.roar_antic);
                        }
                        Then::Leave => self.go(set, State::TeleOut { then: Then::Leave }, c.tele_out),
                    }
                }
            }
            State::Roar { left, then } => {
                if self.t >= 0.15 && self.anim.clip() == c.roar_antic {
                    self.anim.play(&set.lib, c.roar, false);
                }
                if self.t >= left {
                    match then {
                        Some(mv) => self.teleport(set, mv, out),
                        None => self.go(set, State::Idle { left: 0.5 }, c.idle),
                    }
                }
            }
            State::Idle { left } => {
                self.face(kx);
                if self.struck_idle >= 2 && self.rng.chance(0.5) {
                    // Pestered: a quick backstep, then away.
                    self.struck_idle = 0;
                    let away = if kx > self.pos.x { -1.0 } else { 1.0 };
                    self.vel = Vec2::new(away * 14.0 * u, 0.0);
                    self.go(set, State::Evade { left: 0.3 }, c.evade);
                } else if self.t >= left {
                    self.struck_idle = 0;
                    return self.decide(set, out);
                }
            }
            State::Evade { left } => {
                self.pos = self.clamp(self.pos + self.vel * td, set, env.area);
                if self.t >= left {
                    self.vel = Vec2::ZERO;
                    self.anim.play(&set.lib, c.evade_end, true);
                    return self.decide(set, out);
                }
            }
            State::TeleOut { then } => {
                self.alpha = (1.0 - self.t / 0.3).max(0.0);
                if self.t >= 0.3 {
                    self.alpha = 0.0;
                    match then {
                        Then::Leave => {
                            self.state = State::Gone;
                        }
                        Then::Fight => {
                            let left = self.rng.range(0.25, 0.45);
                            self.state = State::Hidden { left };
                            self.t = 0.0;
                        }
                    }
                }
            }
            State::Hidden { left } => {
                if self.t >= left {
                    let mv = self.next.take().unwrap_or(Move::Firebats);
                    match self.spot_for(mv, set, env) {
                        Some(at) => {
                            self.pos = at;
                            self.face(kx);
                            self.go(set, State::TeleIn { mv }, c.tele_in);
                            out.flashes.push(self.body(set, u).center());
                        }
                        None => {
                            // No room for that one here: something else.
                            self.next = Some(self.pick_move(Some(mv)));
                            self.t = 0.0;
                        }
                    }
                }
            }
            State::TeleIn { mv } => {
                self.alpha = (self.t / 0.25).min(1.0);
                self.face(kx);
                if self.t >= 0.25 {
                    self.alpha = 1.0;
                    self.begin(set, mv);
                }
            }
            State::Attack { mv, step, left } => return self.attack(td, set, env, out, mv, step, left),
            State::Stunned { left } => {
                if self.t >= left {
                    out.burst = Some(self.body(set, u).center());
                    self.go(set, State::Exploding { left: 0.35 }, c.explode);
                }
            }
            State::Exploding { left } => {
                if self.t >= left {
                    self.go(set, State::Bats { left: 1.4 }, c.bat_fly);
                }
            }
            State::Bats { left } => {
                self.pos.y -= 3.0 * u * td;
                self.alpha = (1.0 - self.t / left).max(0.0);
                if self.t >= left {
                    self.state = State::Gone;
                }
            }
            State::Gone => {}
        }
        None
    }

    /// After an attack or a pause: a phase change (roar, then the balloon) or the next move.
    fn decide(&mut self, set: &GrimmSet, out: &mut GrimmTick) -> Option<u8> {
        if let Some(due) = self.phase_due.filter(|&d| d > self.phase) {
            // One act at a time, even when a flurry crossed two thresholds at once.
            let p = self.phase + 1;
            self.phase = p;
            if p >= due {
                self.phase_due = None;
            }
            out.say = Some(if p >= 3 { LineKind::Phase3 } else { LineKind::Phase2 });
            self.go(set, State::Roar { left: 1.3, then: Some(Move::Balloon) }, set.clips.roar_antic);
            return Some(p);
        }
        let mv = self.pick_move(self.last_move);
        self.teleport(set, mv, out);
        None
    }

    fn teleport(&mut self, set: &GrimmSet, mv: Move, out: &mut GrimmTick) {
        self.next = Some(mv);
        self.vel = Vec2::ZERO;
        out.flashes.push(self.body(set, self.ppu).center());
        self.go(set, State::TeleOut { then: Then::Fight }, set.clips.tele_out);
    }

    /// The next attack: never the same twice in a row; spikes from phase 2 on.
    fn pick_move(&mut self, not: Option<Move>) -> Move {
        let spikes = if self.phase >= 2 { 1.0 } else { 0.0 };
        let items = [(Move::AirDash, 1.2), (Move::Slash, 1.2), (Move::Firebats, 1.0), (Move::Pillars, 0.8), (Move::Spikes, spikes)];
        let items: Vec<(Move, f32)> = items.into_iter().filter(|(m, _)| Some(*m) != not).collect();
        self.rng.pick_weighted(&items).unwrap_or(Move::Firebats)
    }

    /// Where `mv` starts, relative to the knight (None: no room for it here).
    fn spot_for(&mut self, mv: Move, set: &GrimmSet, env: &GrimmEnv) -> Option<Vec2> {
        let u = env.ppu;
        let k = env.knight?;
        let feet = Vec2::new(k.center().x, k.bottom);
        let (gy, x0, x1) = ground_under(env.terrain, feet, env.area);
        let a = env.area;
        // The side with more room, now and then the other one.
        let room = |s: f32| if s > 0.0 { a.right - feet.x } else { feet.x - a.left };
        let mut side = if room(1.0) >= room(-1.0) { 1.0 } else { -1.0 };
        if self.rng.chance(0.35) && room(-side) > 6.0 * u {
            side = -side;
        }
        let at = |dx: f32, y: f32| Vec2::new(feet.x + side * dx * u, y);
        let p = match mv {
            Move::AirDash => at(5.0, gy - 5.5 * u),
            Move::Slash => {
                // On his ledge, a few steps away (the other side if this one is too short).
                let on = |s: f32| {
                    let x = (feet.x + s * 4.5 * u).clamp(x0 + 0.7 * u, x1 - 0.7 * u);
                    ((x - feet.x).abs() >= 2.5 * u).then_some(Vec2::new(x, gy))
                };
                on(side).or_else(|| on(-side)).unwrap_or(at(4.5, gy))
            }
            Move::Firebats => at(9.0, gy),
            Move::Pillars => at(6.0, gy - 3.5 * u),
            Move::Spikes => {
                if x1 - x0 < 4.0 * u {
                    return None;
                }
                at(6.0, gy)
            }
            Move::Balloon => Vec2::new(feet.x, (gy - 6.5 * u).min(k.top - 4.0 * u)),
        };
        Some(self.clamp(p, set, env.area))
    }

    fn begin(&mut self, set: &GrimmSet, mv: Move) {
        let c = &set.clips;
        let (clip, left) = match mv {
            Move::AirDash => (c.air_dash_antic, 0.5),
            Move::Slash => (c.slash_antic, 0.6),
            Move::Firebats => (c.cast_antic, 0.5),
            Move::Pillars => (c.absorb_start, 0.6),
            Move::Spikes => (c.spike_ready, 0.7),
            Move::Balloon => (c.balloon_antic, 0.45),
        };
        self.last_move = Some(mv);
        self.count = 0;
        self.go(set, State::Attack { mv, step: 0, left }, clip);
    }

    #[allow(clippy::too_many_arguments)]
    fn attack(&mut self, td: f32, set: &GrimmSet, env: &GrimmEnv, out: &mut GrimmTick, mv: Move, step: u8, left: f32) -> Option<u8> {
        let c = set.clips.clone();
        let u = env.ppu;
        let k = env.knight;
        let feet = k.map_or(self.pos, |k| Vec2::new(k.center().x, k.bottom));
        let (gy, x0, x1) = ground_under(env.terrain, feet, env.area);
        let next = |g: &mut Self, step: u8, left: f32, clip: ClipId| {
            g.state = State::Attack { mv, step, left };
            g.t = 0.0;
            g.anim.play(&set.lib, clip, true);
        };
        let done = self.t >= left;
        match (mv, step) {
            // --- Air dash: dive at where he stands, then skid along the ground.
            (Move::AirDash, 0) => {
                self.face(feet.x);
                if done {
                    let target = Vec2::new(feet.x, gy);
                    self.vel = (target - self.pos).normalize_or(Vec2::Y) * 26.0 * u;
                    self.face(target.x);
                    next(self, 1, 0.8, c.air_dash);
                }
            }
            (Move::AirDash, 1) => {
                self.pos += self.vel * td;
                if self.pos.y >= gy || done {
                    self.pos.y = self.pos.y.min(gy);
                    let dir = if self.vel.x.abs() > 1.0 { self.vel.x.signum() } else { self.dir() };
                    self.vel = Vec2::new(dir * 22.0 * u, 0.0);
                    next(self, 2, 0.35, c.g_dash);
                }
                self.pos = self.clamp(self.pos, set, env.area);
            }
            (Move::AirDash, 2) => {
                self.pos = self.clamp(self.pos + self.vel * td, set, env.area);
                if done {
                    self.vel = Vec2::ZERO;
                    next(self, 3, 0.45, c.slash_recover);
                }
            }
            // --- Slash: three cuts forward, an uppercut shedding fireballs.
            (Move::Slash, 0) => {
                self.face(feet.x);
                if done {
                    self.vel = Vec2::new(self.dir() * 10.0 * u, 0.0);
                    next(self, 1, 0.12, c.slash1);
                }
            }
            (Move::Slash, 1..=3) => {
                self.pos.x += self.vel.x * td;
                self.pos.x = self.pos.x.clamp(x0.min(self.pos.x), x1.max(self.pos.x));
                self.pos = self.clamp(self.pos, set, env.area);
                if done {
                    match step {
                        1 => next(self, 2, 0.12, c.slash2),
                        2 => next(self, 3, 0.14, c.slash3),
                        _ => {
                            self.vel = Vec2::ZERO;
                            next(self, 4, 0.25, c.uppercut_antic);
                        }
                    }
                }
            }
            (Move::Slash, 4) => {
                if done {
                    // Up he goes, and two fireballs are flung far out to the sides, high (a
                    // flourish more than a threat: they burn out before they come down).
                    let centre = self.body(set, u).center() - Vec2::new(0.0, 0.8 * u);
                    for vx in [-10.0, 10.0] {
                        self.hazard(HazardKind::Fireball, centre, Vec2::new(vx, -8.0) * u, Vec2::splat(0.35 * u), 0.0, 1.4, 18.0 * u);
                    }
                    self.vel = Vec2::new(0.0, -20.0 * u);
                    next(self, 5, 0.3, c.uppercut);
                }
            }
            (Move::Slash, 5) => {
                self.pos = self.clamp(self.pos + self.vel * td, set, env.area);
                if done {
                    self.vel = Vec2::ZERO;
                    next(self, 6, 0.2, c.uppercut_end);
                }
            }
            (Move::Slash, _) => {
                if done {
                    return self.decide(set, out);
                }
            }
            // --- Firebats at his height.
            (Move::Firebats, 0) => {
                self.face(feet.x);
                if done {
                    next(self, 1, 0.0, c.cast);
                }
            }
            (Move::Firebats, 1) => {
                let bats = if self.phase >= 3 { 4 } else { 3 };
                let due = (self.t / 0.4) as u32 + 1;
                while self.count < due.min(bats) {
                    // Low (jump it) and high (stay down: it passes over his head), in turn.
                    let ky = k.map_or(self.pos.y - u, |k| k.center().y);
                    let dy = [0.25, -1.4, 0.25, -1.4][self.count as usize % 4] * u;
                    let from = Vec2::new(self.pos.x + self.dir() * 0.9 * u, ky + dy);
                    self.hazard(HazardKind::Firebat, from, Vec2::new(self.dir() * 13.0 * u, 0.0), Vec2::new(0.6 * u, 0.4 * u), 0.0, 4.0, 0.0);
                    self.count += 1;
                }
                if self.count >= bats && self.t >= 0.4 * bats as f32 {
                    next(self, 2, 0.4, c.cast_return);
                }
            }
            // --- Flame pillars under his feet.
            (Move::Pillars, 0) => {
                self.face(feet.x);
                if done {
                    next(self, 1, 0.0, c.absorb);
                }
            }
            (Move::Pillars, 1) => {
                let pillars = if self.phase >= 2 { 4 } else { 3 };
                let due = (self.t / 0.85) as u32 + 1;
                if self.count < due.min(pillars) {
                    self.hazard(HazardKind::Pillar, Vec2::new(feet.x, gy - 3.5 * u), Vec2::ZERO, Vec2::new(0.7 * u, 3.5 * u), 0.65, 0.4, 0.0);
                    self.count += 1;
                }
                if self.count >= pillars && self.t >= 0.85 * pillars as f32 + 0.3 {
                    next(self, 2, 0.45, c.absorb_end);
                }
            }
            // --- Spikes along his ledge.
            (Move::Spikes, 0) => {
                self.face(feet.x);
                if self.t >= 0.25 && self.anim.clip() == c.spike_ready {
                    self.anim.play(&set.lib, c.spike_antic, true);
                }
                if done {
                    let (a, b) = ((feet.x - 7.0 * u).max(x0), (feet.x + 7.0 * u).min(x1));
                    if b - a > u {
                        let half = Vec2::new(0.5 * (b - a), 0.55 * u);
                        self.hazard(HazardKind::Spikes, Vec2::new(0.5 * (a + b), gy - half.y), Vec2::ZERO, half, 0.95, 0.5, 0.0);
                    }
                    next(self, 1, 1.45, c.spike_up);
                }
            }
            (Move::Spikes, 1) => {
                if done {
                    next(self, 2, 0.3, c.spike_down);
                }
            }
            // --- The balloon: rings of fireballs.
            (Move::Balloon, 0) => {
                if done {
                    self.ring = self.rng.range(0.0, std::f32::consts::TAU);
                    next(self, 1, 4.5, c.balloon);
                }
            }
            (Move::Balloon, 1) => {
                // Rings with wide gaps, each turned a little: keep your distance and they pass.
                let due = (self.t / 0.9) as u32 + 1;
                if self.count < due && !done {
                    let centre = self.body(set, u).center();
                    let n = 5 + self.phase as u32;
                    for i in 0..n {
                        let a = self.ring + i as f32 * std::f32::consts::TAU / n as f32;
                        self.hazard(HazardKind::Fireball, centre, Vec2::from_angle(a) * 4.5 * u, Vec2::splat(0.35 * u), 0.0, 3.0, 0.0);
                    }
                    self.ring += 0.4;
                    self.count += 1;
                }
                if done {
                    next(self, 2, 0.35, c.balloon_end);
                }
            }
            (Move::Balloon, _) => {
                if done {
                    return self.decide(set, out);
                }
            }
            // --- Recoveries end in a short pause.
            _ => {
                if done || left <= 0.0 {
                    let pause = match self.phase {
                        1 => self.rng.range(0.9, 1.4),
                        2 => self.rng.range(0.6, 1.0),
                        _ => self.rng.range(0.4, 0.8),
                    };
                    self.go(set, State::Idle { left: pause }, c.idle);
                }
            }
        }
        None
    }
}
