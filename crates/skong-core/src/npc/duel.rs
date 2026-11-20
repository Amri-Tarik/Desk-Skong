//! Hornet's duel: a short spar, but she is fast and skilled. Between quick guards she lunges
//! along her ledge (a ground dash), throws her needle out on its thread and back, bursts a
//! silk sphere when he crowds her, and steps back out of reach after a blow. She can only
//! be struck on guard or recovering; his blows count, but she wins: a few touches on him
//! (or the clock) and she ends it with a flourish and a word, then leaves. Non-lethal: the
//! knight fights her like Zote (the brain's `Fight`, a `Sparring` foe), his kit included.
use anyhow::{Result, bail};
use serde::Deserialize;

use crate::ai::{Foe, FoeKind};
use crate::anim::{ClipId, ClipLibrary};
use crate::geom::Vec2;

use super::actor::{NpcEnv, NpcEvent, NpcProp, once_secs};
use super::challenge::{Body, Next, frame_at, pick_line};

/// Clip names for the duel (from Hornet's merged libraries).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuelClipNames {
    pub stance: String,
    pub lunge_antic: String,
    pub lunge: String,
    pub lunge_recover: String,
    pub throw_antic: String,
    pub throw: String,
    pub throw_recover: String,
    pub sphere_antic: String,
    pub sphere: String,
    pub sphere_recover: String,
    /// The silk sphere around her (a separate sprite).
    pub sphere_ball: String,
    pub evade_antic: String,
    pub evade: String,
    /// The needle and its thread, out on a throw.
    pub needle: String,
    pub thread: String,
    /// Her finishing pose.
    pub win: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuelDef {
    pub challenge: Vec<String>,
    /// She ends it, the winner.
    pub win: Vec<String>,
    /// She ends it, the winner, after he landed `respect_after` blows.
    pub respect: Vec<String>,
    /// Called off.
    pub abandon: Vec<String>,
    pub clips: DuelClipNames,
    /// Touches on the knight that win it for her.
    pub wins_after: u32,
    /// His blows that earn a grudging word at the end.
    pub respect_after: u32,
    /// Lunge speed (game units/s) and length (s).
    pub lunge_speed: f32,
    pub lunge_secs: f32,
    /// How far the thrown needle reaches (game units) and how long it is out (s).
    pub throw_reach: f32,
    pub throw_secs: f32,
    /// The silk sphere's radius (game units) and how long it lasts (s).
    pub sphere_radius: f32,
    pub sphere_secs: f32,
    /// Seconds on guard between moves (random in range).
    pub pause: [f32; 2],
    /// She ends it (still the winner) after this long (s).
    pub max_secs: f32,
    /// Body [w, h] in game units.
    pub size: [f32; 2],
}

/// [`DuelClipNames`] resolved.
#[derive(Clone, Debug)]
pub struct DuelClips {
    pub stance: ClipId,
    pub lunge_antic: ClipId,
    pub lunge: ClipId,
    pub lunge_recover: ClipId,
    pub throw_antic: ClipId,
    pub throw: ClipId,
    pub throw_recover: ClipId,
    pub sphere_antic: ClipId,
    pub sphere: ClipId,
    pub sphere_recover: ClipId,
    pub sphere_ball: ClipId,
    pub evade_antic: ClipId,
    pub evade: ClipId,
    pub needle: ClipId,
    pub thread: ClipId,
    pub win: ClipId,
}

impl DuelDef {
    pub fn validate(&self) -> Result<()> {
        let pos = |x: f32| x.is_finite() && x > 0.0;
        if self.wins_after == 0 || self.respect_after == 0 {
            bail!("duel: wins_after and respect_after must be positive");
        }
        let vals = [self.lunge_speed, self.lunge_secs, self.throw_reach, self.throw_secs, self.sphere_radius, self.sphere_secs, self.max_secs, self.size[0], self.size[1]];
        if !vals.iter().all(|v| pos(*v)) {
            bail!("duel: speeds, times, reaches and size must be positive");
        }
        if self.pause[0] < 0.0 || self.pause[1] < self.pause[0] {
            bail!("duel: bad pause range");
        }
        let pools = [&self.challenge, &self.win, &self.respect, &self.abandon];
        if pools.iter().any(|p| p.is_empty()) {
            bail!("duel: needs challenge, win, respect and abandon lines");
        }
        if pools.iter().any(|p| p.iter().any(|l| l.contains('{'))) {
            bail!("duel: lines are plain text (no placeholders)");
        }
        Ok(())
    }

    pub fn resolve(&self, lib: &ClipLibrary, who: &str) -> Result<DuelClips> {
        let get = |n: &str| match lib.id(n) {
            Some(id) => Ok(id),
            None => bail!("npc '{who}': duel clip '{n}' not in its libraries"),
        };
        let c = &self.clips;
        Ok(DuelClips {
            stance: get(&c.stance)?,
            lunge_antic: get(&c.lunge_antic)?,
            lunge: get(&c.lunge)?,
            lunge_recover: get(&c.lunge_recover)?,
            throw_antic: get(&c.throw_antic)?,
            throw: get(&c.throw)?,
            throw_recover: get(&c.throw_recover)?,
            sphere_antic: get(&c.sphere_antic)?,
            sphere: get(&c.sphere)?,
            sphere_recover: get(&c.sphere_recover)?,
            sphere_ball: get(&c.sphere_ball)?,
            evade_antic: get(&c.evade_antic)?,
            evade: get(&c.evade)?,
            needle: get(&c.needle)?,
            thread: get(&c.thread)?,
            win: get(&c.win)?,
        })
    }
}

/// Her moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelMove {
    Lunge,
    Throw,
    Sphere,
    Evade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelStage {
    Challenge,
    Stance,
    Antic(DuelMove),
    Attack(DuelMove),
    Recover(DuelMove),
    /// She ends it: the flourish and her word.
    Win,
    /// After her word on the duel: a pause, then her parting line (the conversation's
    /// closing line, held back until now; `said` once it's up).
    Farewell { said: bool },
    /// Called off.
    Abandon,
}

/// A duel in progress (see the module docs).
#[derive(Clone, Debug)]
pub struct Duel {
    stage: DuelStage,
    t: f32,
    len: f32,
    elapsed: f32,
    /// Direction of the current move (±1).
    dir: f32,
    /// The current move already caught him.
    hit_done: bool,
    /// Touches on the knight so far.
    pub touches: u32,
    /// His blows that landed.
    pub taken: u32,
    far_for: f32,
    /// Struck on guard: step back next.
    evade_next: bool,
    /// Her parting line, said after the outcome (see [`DuelStage::Farewell`]).
    pub(super) farewell: Option<String>,
}

/// Antics play faster than in the game: she is quick.
const ANTIC_SPEED: f32 = 1.3;
/// Pause between her word on the duel and her parting line (s).
pub(super) const FAREWELL_PAUSE: f32 = 0.6;

impl Duel {
    pub(super) fn start(def: &DuelDef, clips: &DuelClips, body: &mut Body, knight_x: f32, events: &mut Vec<NpcEvent>) -> Duel {
        let text = pick_line(body.rng, &def.challenge);
        let len = crate::bubble::read_secs(&text);
        body.face(knight_x);
        body.play(clips.stance, true);
        events.push(NpcEvent::Say { text, secs: len });
        Duel { stage: DuelStage::Challenge, t: 0.0, len, elapsed: 0.0, dir: 1.0, hit_done: false, touches: 0, taken: 0, far_for: 0.0, evade_next: false, farewell: None }
    }

    pub fn stage(&self) -> DuelStage {
        self.stage
    }

    fn vulnerable(&self) -> bool {
        matches!(self.stage, DuelStage::Stance | DuelStage::Recover(_))
    }

    fn fighting(&self) -> bool {
        !matches!(self.stage, DuelStage::Win | DuelStage::Farewell { .. } | DuelStage::Abandon)
    }

    /// The brain's view: a sparring partner (none once it's over). While she is still
    /// saying her challenge it is `waiting`: he squares up, but nothing is struck yet.
    pub fn foe(&self, id: u32, feet: Vec2, def: &DuelDef, ppu: f32) -> Option<Foe> {
        if !self.fighting() {
            return None;
        }
        let half = Vec2::new(def.size[0], def.size[1]) * 0.5 * ppu;
        let lunging = self.stage == DuelStage::Attack(DuelMove::Lunge);
        Some(Foe {
            id,
            kind: FoeKind::Sparring,
            pos: feet - Vec2::new(0.0, half.y),
            feet,
            half,
            flying: false,
            vulnerable: self.vulnerable(),
            attacking: matches!(self.stage, DuelStage::Antic(_) | DuelStage::Attack(_)),
            winding_up: matches!(self.stage, DuelStage::Antic(_)),
            lunging,
            vel: if lunging { Vec2::new(self.dir * def.lunge_speed * ppu, 0.0) } else { Vec2::ZERO },
            waiting: self.stage == DuelStage::Challenge,
        })
    }

    /// His blow lands (from `from_x`): a flash and a nudge; she doesn't go down. Returns
    /// whether it landed.
    pub(super) fn strike(&mut self, damage: u32, from_x: f32, body: &mut Body, span: (f32, f32), ppu: f32) -> bool {
        if !self.vulnerable() {
            return false;
        }
        self.taken += damage.max(1);
        *body.flash = 1.0;
        let away = if body.pos.x >= from_x { 1.0 } else { -1.0 };
        body.pos.x = (body.pos.x + away * 0.25 * ppu).clamp(span.0, span.1.max(span.0));
        // Skilled: a blow on guard and she's out of there next.
        if self.stage == DuelStage::Stance && body.rng.chance(0.6) {
            self.evade_next = true;
            self.t = self.len;
        }
        true
    }

    /// The needle and thread while a throw is out.
    pub fn props(&self, pos: Vec2, looking_right: bool, lib: &ClipLibrary, def: &DuelDef, clips: &DuelClips, ppu: f32) -> Vec<NpcProp> {
        if self.stage == DuelStage::Attack(DuelMove::Sphere) {
            let at = pos - Vec2::new(0.0, 1.0 * ppu);
            return frame_at(lib, clips.sphere_ball, self.t, false).map(|key| NpcProp { key, at, flip: !looking_right, alpha: 1.0 }).into_iter().collect();
        }
        if self.stage != DuelStage::Attack(DuelMove::Throw) {
            return Vec::new();
        }
        let dir = if looking_right { 1.0 } else { -1.0 };
        let at = needle_tip(pos, dir, self.t, def, ppu);
        let flip = dir > 0.0;
        [frame_at(lib, clips.thread, self.t, false), frame_at(lib, clips.needle, self.t, true)]
            .into_iter()
            .flatten()
            .map(|key| NpcProp { key, at, flip, alpha: 1.0 })
            .collect()
    }

    pub(super) fn abandon(&mut self, def: &DuelDef, body: &mut Body, events: &mut Vec<NpcEvent>) {
        if !self.fighting() {
            return;
        }
        events.push(NpcEvent::LineDone);
        events.push(NpcEvent::DuelOver { finished: false });
        // Called off: her word on it is the parting line.
        self.farewell = None;
        let text = pick_line(body.rng, &def.abandon);
        self.len = crate::bubble::read_secs(&text);
        events.push(NpcEvent::Say { text, secs: self.len });
        self.stage = DuelStage::Abandon;
        self.t = 0.0;
    }

    pub(super) fn tick(&mut self, dt: f32, def: &DuelDef, clips: &DuelClips, body: &mut Body, env: &NpcEnv, events: &mut Vec<NpcEvent>) -> Next {
        let u = env.ppu;
        self.t += dt;
        self.elapsed += dt;
        let k = env.knight;
        let dx = k.x - body.pos.x;
        let far = dx.abs() > 14.0 * u || (k.y - body.pos.y).abs() > 5.0 * u;
        self.far_for = if far { self.far_for + dt } else { 0.0 };
        if self.fighting() && self.stage != DuelStage::Challenge {
            if self.far_for > 8.0 {
                self.abandon(def, body, events);
                return Next::Stay;
            }
            if self.touches >= def.wins_after || self.elapsed > def.max_secs {
                self.win(def, clips, body, k.x, events);
                return Next::Stay;
            }
        }
        let span = span_of(env, *body.pos, def, u);
        body.anim.set_speed(if matches!(self.stage, DuelStage::Antic(_)) { ANTIC_SPEED } else { 1.0 });
        match self.stage {
            DuelStage::Challenge => {
                body.face(k.x);
                if self.t >= self.len {
                    events.push(NpcEvent::LineDone);
                    self.guard(def, clips, body);
                }
            }
            DuelStage::Stance => {
                body.face(k.x);
                if self.t >= self.len {
                    let mv = self.choose(dx.abs() / u, body);
                    let antic = match mv {
                        DuelMove::Lunge => clips.lunge_antic,
                        DuelMove::Throw => clips.throw_antic,
                        DuelMove::Sphere => clips.sphere_antic,
                        DuelMove::Evade => clips.evade_antic,
                    };
                    self.dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                    if mv == DuelMove::Evade {
                        self.dir = -self.dir;
                    } else {
                        *body.looking_right = self.dir > 0.0;
                    }
                    body.play(antic, true);
                    self.go(DuelStage::Antic(mv), (once_secs(body.lib, antic) / ANTIC_SPEED).max(0.12));
                }
            }
            DuelStage::Antic(mv) => {
                if self.t >= self.len {
                    let (clip, len) = match mv {
                        // Through him and a little past (at most `lunge_secs` long).
                        DuelMove::Lunge => (clips.lunge, ((dx.abs() / u + 1.5) / def.lunge_speed).clamp(0.15, def.lunge_secs)),
                        DuelMove::Throw => (clips.throw, def.throw_secs),
                        DuelMove::Sphere => (clips.sphere, def.sphere_secs),
                        DuelMove::Evade => (clips.evade, 0.3),
                    };
                    self.hit_done = false;
                    body.play(clip, true);
                    self.go(DuelStage::Attack(mv), len);
                }
            }
            DuelStage::Attack(mv) => {
                match mv {
                    DuelMove::Lunge | DuelMove::Evade => {
                        let speed = if mv == DuelMove::Lunge { def.lunge_speed } else { 16.0 };
                        body.pos.x = (body.pos.x + self.dir * speed * u * dt).clamp(span.0, span.1.max(span.0));
                    }
                    DuelMove::Throw | DuelMove::Sphere => {}
                }
                if !self.hit_done && self.catches(mv, *body.pos, k, def, u) {
                    self.hit_done = true;
                    self.touches += 1;
                    events.push(NpcEvent::HitKnight);
                }
                if self.t >= self.len {
                    let recover = match mv {
                        DuelMove::Lunge => Some(clips.lunge_recover),
                        DuelMove::Throw => Some(clips.throw_recover),
                        DuelMove::Sphere => Some(clips.sphere_recover),
                        DuelMove::Evade => None,
                    };
                    match recover {
                        Some(c) => {
                            body.play(c, true);
                            self.go(DuelStage::Recover(mv), once_secs(body.lib, c).max(0.25));
                        }
                        None => self.guard(def, clips, body),
                    }
                    return Next::Landed;
                }
            }
            DuelStage::Recover(_) => {
                if self.t >= self.len {
                    self.guard(def, clips, body);
                }
            }
            DuelStage::Win => {
                if self.t >= self.len {
                    events.push(NpcEvent::LineDone);
                    body.play(clips.stance, false);
                    if self.farewell.is_none() {
                        return Next::Linger(body.rng.range(1.5, 3.0));
                    }
                    self.go(DuelStage::Farewell { said: false }, FAREWELL_PAUSE);
                }
            }
            DuelStage::Farewell { said } => {
                body.face(k.x);
                if self.t >= self.len {
                    match self.farewell.take() {
                        Some(text) if !said => {
                            let len = crate::bubble::read_secs(&text);
                            events.push(NpcEvent::Say { text, secs: len });
                            self.go(DuelStage::Farewell { said: true }, len);
                        }
                        _ => {
                            events.push(NpcEvent::LineDone);
                            return Next::Linger(body.rng.range(1.0, 2.0));
                        }
                    }
                }
            }
            DuelStage::Abandon => {
                if self.t >= self.len {
                    events.push(NpcEvent::LineDone);
                    body.play(clips.stance, false);
                    return Next::Linger(body.rng.range(1.0, 2.0));
                }
            }
        }
        Next::Stay
    }

    /// Her pick, by how far he is (units): crowded → a sphere or a step back; mid-range →
    /// the needle or a lunge; far → a lunge to close in.
    fn choose(&mut self, d: f32, body: &mut Body) -> DuelMove {
        if std::mem::take(&mut self.evade_next) {
            return DuelMove::Evade;
        }
        let r = body.rng.f32();
        if d < 2.5 {
            if r < 0.45 {
                DuelMove::Sphere
            } else if r < 0.8 {
                DuelMove::Evade
            } else {
                DuelMove::Lunge
            }
        } else if d < 6.0 {
            if r < 0.5 { DuelMove::Throw } else { DuelMove::Lunge }
        } else {
            DuelMove::Lunge
        }
    }

    /// Does `mv`, right now, catch the knight (feet `k`)?
    fn catches(&self, mv: DuelMove, at: Vec2, k: Vec2, def: &DuelDef, u: f32) -> bool {
        let dy = (k.y - at.y).abs();
        match mv {
            DuelMove::Lunge => (k.x - at.x).abs() < (0.5 * def.size[0] + 0.6) * u && dy < 1.5 * u,
            DuelMove::Throw => {
                let tip = needle_tip(at, self.dir, self.t, def, u).x + self.dir * 2.0 * u;
                let (a, b) = if self.dir > 0.0 { (at.x, tip) } else { (tip, at.x) };
                (a..=b).contains(&k.x) && dy < 1.4 * u
            }
            DuelMove::Sphere => Vec2::new(k.x, k.y - 0.6 * u).distance(at - Vec2::new(0.0, 1.0 * u)) < def.sphere_radius * u,
            DuelMove::Evade => false,
        }
    }

    fn guard(&mut self, def: &DuelDef, clips: &DuelClips, body: &mut Body) {
        body.play(clips.stance, false);
        let len = body.rng.range(def.pause[0], def.pause[1]);
        self.go(DuelStage::Stance, len);
    }

    fn win(&mut self, def: &DuelDef, clips: &DuelClips, body: &mut Body, knight_x: f32, events: &mut Vec<NpcEvent>) {
        events.push(NpcEvent::LineDone);
        events.push(NpcEvent::DuelOver { finished: true });
        let pool = if self.taken >= def.respect_after { &def.respect } else { &def.win };
        let text = pick_line(body.rng, pool);
        let len = crate::bubble::read_secs(&text).max(once_secs(body.lib, clips.win));
        log::info!("duel: Hornet wins ({} touches, {} blows taken, {:.0} s)", self.touches, self.taken, self.elapsed);
        body.face(knight_x);
        body.anim.set_speed(1.0);
        body.play(clips.win, true);
        events.push(NpcEvent::Say { text, secs: len });
        self.go(DuelStage::Win, len);
    }

    fn go(&mut self, stage: DuelStage, len: f32) {
        self.stage = stage;
        self.t = 0.0;
        self.len = len;
    }
}

/// Where the thrown needle is (its middle): out from her hand to `throw_reach` and back.
fn needle_tip(feet: Vec2, dir: f32, t: f32, def: &DuelDef, u: f32) -> Vec2 {
    let s = (t / def.throw_secs).clamp(0.0, 1.0);
    let out = if s < 0.5 { s * 2.0 } else { (1.0 - s) * 2.0 };
    feet + Vec2::new(dir * (1.0 + out * def.throw_reach) * u, -1.1 * u)
}

/// Where she may move: her ledge, inside the monitor.
pub(super) fn span_of(env: &NpcEnv, at: Vec2, def: &DuelDef, u: f32) -> (f32, f32) {
    let half = 0.5 * def.size[0] * u;
    env.terrain
        .ledges
        .iter()
        .find(|l| (l.y - at.y).abs() < 1.5 && l.x0 - 1.0 <= at.x && at.x <= l.x1 + 1.0)
        .map_or((at.x, at.x), |l| (l.x0.max(env.area.left) + half, l.x1.min(env.area.right) - half))
}
