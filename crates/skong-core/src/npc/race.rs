//! Hornet's race: she flings one of her spiked silk barbs (the traps of her second fight)
//! to mark the finish on a settled ledge far away, where it sits, then "Go!" — both race
//! there. She moves with her own runs, ground dashes and leaps along a planned [`Course`]
//! (her clips; she is Hornet, so her leaps ignore windows), paced against the knight's
//! planned time so either can win. The knight uses his whole kit (the brain's `Scene`
//! behaviour: a Hurry route, crystal dash and wall jumps included). Whoever touches the barb
//! first wins and it bursts apart; she reacts and leaps away.
//!
//! Pure: [`pick_finish`] (where the barb lands), [`Course`] (her moves and timing) and
//! [`Race`] (the state machine the visitor actor drives).
use anyhow::{Result, bail};
use serde::Deserialize;

use crate::ai::SceneCue;
use crate::anim::{ClipId, ClipLibrary};
use crate::desktop::WindowId;
use crate::geom::{RectF, Vec2};
use crate::portal::Footing;

use super::actor::{DrawnProp, NpcEnv, NpcEvent, NpcProp, once_secs};
use super::challenge::{Body, Next, frame_at, pick_line};

/// Clip names for the race (from Hornet's merged libraries).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaceClipNames {
    /// Pointing at the finish while she says the challenge.
    pub point: String,
    /// Standing ready.
    pub ready: String,
    /// Her barb throw (a held clip may loop while the barb flies).
    pub throw_antic: String,
    pub throw: String,
    pub throw_recover: String,
    /// The barb (a separate sprite): looped as it tumbles through the air, played now and
    /// then as it sits at the finish (its first frame between).
    pub barb: String,
    /// The barb bursting apart (played once) when someone reaches it or the race is off.
    pub burst: String,
    pub run: String,
    pub dash_antic: String,
    pub dash: String,
    pub jump: String,
    pub fall: String,
    /// Mid-leap burst on long leaps.
    pub air_dash: String,
    pub land: String,
    /// Her pose when she wins.
    pub win: String,
    /// The leap away at the end.
    pub depart: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaceDef {
    /// Said as she points out where the barb will go (one picked).
    pub challenge: Vec<String>,
    /// The start.
    pub go: Vec<String>,
    /// She got there first.
    pub win: Vec<String>,
    /// The knight got there first.
    pub lose: Vec<String>,
    /// Called off (fullscreen app, hard typing, the Shade, enemies...).
    pub abandon: Vec<String>,
    pub clips: RaceClipNames,
    /// How far the finish is from her (game units), [min, max].
    pub distance: [f32; 2],
    /// Her running and ground-dash speeds (game units/s) before pacing.
    pub run_speed: f32,
    pub dash_speed: f32,
    /// Her time to the barb as a multiple of the knight's planned time ([min, max], one
    /// picked per race): around 1, so either may win.
    pub pace: [f32; 2],
    /// Barb flight speed (game units/s).
    pub barb_speed: f32,
    /// The knight touches the barb within this many units of it.
    pub reach: f32,
}

/// [`RaceClipNames`] resolved.
#[derive(Clone, Debug)]
pub struct RaceClips {
    pub point: ClipId,
    pub ready: ClipId,
    pub throw_antic: ClipId,
    pub throw: ClipId,
    pub throw_recover: ClipId,
    pub barb: ClipId,
    pub burst: ClipId,
    pub run: ClipId,
    pub dash_antic: ClipId,
    pub dash: ClipId,
    pub jump: ClipId,
    pub fall: ClipId,
    pub air_dash: ClipId,
    pub land: ClipId,
    pub win: ClipId,
    pub depart: ClipId,
}

impl RaceDef {
    pub fn validate(&self) -> Result<()> {
        let pos = |x: f32| x.is_finite() && x > 0.0;
        if !pos(self.run_speed) || !pos(self.dash_speed) || !pos(self.barb_speed) || !pos(self.reach) {
            bail!("race: speeds and reach must be positive");
        }
        if !(pos(self.distance[0]) && self.distance[1] >= self.distance[0]) || !(pos(self.pace[0]) && self.pace[1] >= self.pace[0]) {
            bail!("race: bad distance or pace range");
        }
        let pools = [&self.challenge, &self.go, &self.win, &self.lose, &self.abandon];
        if pools.iter().any(|p| p.is_empty()) {
            bail!("race: needs challenge, go, win, lose and abandon lines");
        }
        if pools.iter().any(|p| p.iter().any(|l| l.contains('{'))) {
            bail!("race: lines are plain text (no placeholders)");
        }
        Ok(())
    }

    pub fn resolve(&self, lib: &ClipLibrary, who: &str) -> Result<RaceClips> {
        let get = |n: &str| match lib.id(n) {
            Some(id) => Ok(id),
            None => bail!("npc '{who}': race clip '{n}' not in its libraries"),
        };
        let c = &self.clips;
        Ok(RaceClips {
            point: get(&c.point)?,
            ready: get(&c.ready)?,
            throw_antic: get(&c.throw_antic)?,
            throw: get(&c.throw)?,
            throw_recover: get(&c.throw_recover)?,
            barb: get(&c.barb)?,
            burst: get(&c.burst)?,
            run: get(&c.run)?,
            dash_antic: get(&c.dash_antic)?,
            dash: get(&c.dash)?,
            jump: get(&c.jump)?,
            fall: get(&c.fall)?,
            air_dash: get(&c.air_dash)?,
            land: get(&c.land)?,
            win: get(&c.win)?,
            depart: get(&c.depart)?,
        })
    }
}

/// Where the barb lands: a point on a footing (settled ledges only, on `area` — the
/// monitor she stands on, never a game's) `distance` units from `start`, preferring far and
/// high ones (a climb, a dash), that `reachable` (a route check for the knight) accepts.
pub fn pick_finish(
    footings: &[Footing],
    start: Vec2,
    area: RectF,
    ppu: f32,
    distance: [f32; 2],
    random: &mut dyn FnMut() -> f32,
    reachable: &mut dyn FnMut(Vec2) -> bool,
) -> Option<(Vec2, Option<WindowId>)> {
    let (near, far) = (distance[0] * ppu, distance[1] * ppu);
    let margin = 0.8 * ppu;
    let mut cands: Vec<(f32, Vec2, Option<WindowId>)> = Vec::new();
    for f in footings.iter().filter(|f| f.y > area.top + 3.0 * ppu && f.y <= area.bottom + 1.0) {
        let (x0, x1) = (f.x0.max(area.left) + margin, f.x1.min(area.right) - margin);
        if x1 <= x0 {
            continue;
        }
        for _ in 0..6 {
            let p = Vec2::new(x0 + random() * (x1 - x0), f.y);
            let d = p.distance(start);
            if !(near..=far).contains(&d) {
                continue;
            }
            // Far, and above her: a climb makes a race.
            let climb = ((start.y - p.y) / ppu).clamp(0.0, 8.0);
            cands.push((d / ppu + 0.8 * climb + 4.0 * random(), p, f.owner));
        }
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    cands.into_iter().take(10).find(|c| reachable(c.1)).map(|c| (c.1, c.2))
}

/// One of her moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LegKind {
    Run,
    /// The wind-up of a ground dash (standing).
    DashAntic,
    Dash,
    /// A leap (px above the straight line at its middle); long ones burst forward mid-air.
    Leap { height: f32, air_dash: bool },
    Land,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Leg {
    kind: LegKind,
    from: Vec2,
    to: Vec2,
    secs: f32,
}

/// Her way to the barb: runs along her ledge (a ground dash on long stretches), a leap to
/// the finish's ledge (she is Hornet: through windows if need be), a landing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Course {
    legs: Vec<Leg>,
}

/// Where she is on her course.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoursePose {
    pub pos: Vec2,
    pub kind: LegKind,
    /// 0..1 through the current leg.
    pub progress: f32,
    /// Moving right.
    pub right: bool,
    pub done: bool,
}

impl Course {
    /// Her natural course from `start` (standing on a ledge spanning `span`) to `finish`.
    pub fn plan(start: Vec2, span: (f32, f32), finish: Vec2, ppu: f32, def: &RaceDef) -> Course {
        let u = ppu;
        let run = def.run_speed * u;
        let dash = def.dash_speed * u;
        let dir = if finish.x >= start.x { 1.0 } else { -1.0 };
        let mut legs = Vec::new();
        let level = (finish.y - start.y).abs() < 0.3 * u && span.0 - 1.0 <= finish.x && finish.x <= span.1 + 1.0;
        // Where she leaves her ledge: right before the finish when it's above her ledge
        // (or a short hop when level), else the ledge's end toward it.
        let launch_x = if level {
            finish.x
        } else {
            let within = span.0 <= finish.x && finish.x <= span.1;
            let x = if within { finish.x - dir * (finish.x - start.x).abs().min(3.5 * u) } else { finish.x };
            let x = x.clamp(span.0 + 0.3 * u, (span.1 - 0.3 * u).max(span.0 + 0.3 * u));
            // Never backwards.
            if (x - start.x) * dir < 0.0 { start.x } else { x }
        };
        let launch = Vec2::new(launch_x, start.y);
        // The ground stretch: run, and on a long one a dash in the middle.
        let ground = (launch.x - start.x).abs();
        if ground > 7.0 * u {
            let a = Vec2::new(start.x + dir * 0.35 * ground, start.y);
            let dash_len = (0.4 * ground).min(9.0 * u);
            let b = Vec2::new(a.x + dir * dash_len, start.y);
            legs.push(Leg { kind: LegKind::Run, from: start, to: a, secs: (a.x - start.x).abs() / run });
            legs.push(Leg { kind: LegKind::DashAntic, from: a, to: a, secs: 0.22 });
            legs.push(Leg { kind: LegKind::Dash, from: a, to: b, secs: dash_len / dash });
            legs.push(Leg { kind: LegKind::Run, from: b, to: launch, secs: (launch.x - b.x).abs() / run });
        } else if ground > 1.0 {
            legs.push(Leg { kind: LegKind::Run, from: start, to: launch, secs: ground / run });
        }
        if !level {
            let chord = launch.distance(finish);
            let rise = (launch.y - finish.y).max(0.0);
            let height = if rise > 0.0 { 0.75 * rise + 1.5 * u } else { 1.4 * u + 0.08 * (finish.y - launch.y) };
            let secs = (chord / (1.3 * run) + 0.25).clamp(0.5, 2.4);
            let air_dash = (finish.x - launch.x).abs() > 10.0 * u;
            legs.push(Leg { kind: LegKind::Leap { height, air_dash }, from: launch, to: finish, secs });
        }
        legs.push(Leg { kind: LegKind::Land, from: finish, to: finish, secs: 0.25 });
        legs.retain(|l| l.secs > 0.0);
        Course { legs }
    }

    /// Every leg `k` times as long (slower when > 1).
    pub fn paced(mut self, k: f32) -> Course {
        for l in &mut self.legs {
            l.secs *= k;
        }
        self
    }

    /// Seconds from start to the landing's end.
    pub fn secs(&self) -> f32 {
        self.legs.iter().map(|l| l.secs).sum()
    }

    /// Seconds until she touches the finish (before the landing pose).
    pub fn arrival(&self) -> f32 {
        self.secs() - self.legs.last().filter(|l| l.kind == LegKind::Land).map_or(0.0, |l| l.secs)
    }

    /// Any leap at all (she leaves her ledge).
    pub fn leaps(&self) -> bool {
        self.legs.iter().any(|l| matches!(l.kind, LegKind::Leap { .. }))
    }

    /// Where she is `t` seconds in.
    pub fn at(&self, t: f32) -> CoursePose {
        let mut left = t.max(0.0);
        for (i, l) in self.legs.iter().enumerate() {
            if left <= l.secs || i + 1 == self.legs.len() {
                let s = (left / l.secs.max(1e-4)).clamp(0.0, 1.0);
                let mut pos = l.from + (l.to - l.from) * s;
                if let LegKind::Leap { height, .. } = l.kind {
                    pos.y -= 4.0 * height * s * (1.0 - s);
                }
                let right = if (l.to.x - l.from.x).abs() > 0.5 { l.to.x > l.from.x } else { self.heading(i) };
                return CoursePose { pos, kind: l.kind, progress: s, right, done: i + 1 == self.legs.len() && left >= l.secs };
            }
            left -= l.secs;
        }
        CoursePose { pos: Vec2::ZERO, kind: LegKind::Land, progress: 1.0, right: true, done: true }
    }

    /// The overall heading around leg `i` (standing legs face where she goes).
    fn heading(&self, i: usize) -> bool {
        let moving = self.legs[i..].iter().chain(self.legs[..i].iter().rev()).find(|l| (l.to.x - l.from.x).abs() > 0.5);
        moving.is_none_or(|l| l.to.x > l.from.x)
    }
}

/// Who touched the barb first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Winner {
    Knight,
    Hornet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Pointing out the finish, saying the challenge.
    Challenge,
    ThrowAntic,
    /// The barb flies; she holds the throw until it lands.
    Throw,
    Recover,
    /// A breath, then "Go!".
    Set,
    Run,
    /// At the finish (the barb burst as it was reached): the winner's reaction (hers, or
    /// her grudging word).
    Finish,
    /// Then a pause and her parting line (the conversation's closing line, held back until
    /// now; `said` once it's up).
    Farewell { said: bool },
    /// Called off: a parting word.
    Abandon,
    /// Leaps away (and fades).
    Depart,
}

/// The barb marking the finish.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Barb {
    Held,
    /// Tumbling along its arc from her hand (see [`barb_arc`]).
    Flying { from: Vec2, to: Vec2, lift: f32, t: f32, secs: f32 },
    /// Sitting on the finish ledge (`at`: its centre).
    Resting { at: Vec2, t: f32 },
    /// Bursting apart where it was.
    Bursting { at: Vec2, t: f32 },
    Gone,
}

/// The barb's centre above the finish (game units): its spike tips just touch the ledge.
const BARB_REST: f32 = 0.82;
/// Its reach (tip to centre, game units) when the app draws it itself.
const BARB_RADIUS: f32 = 0.8;
/// Sitting, it stirs (its clip played once) this often (s).
const BARB_STIR: f32 = 2.6;

/// A race in progress (see the module docs).
#[derive(Clone, Debug)]
pub struct Race {
    stage: Stage,
    t: f32,
    len: f32,
    pub finish: Vec2,
    /// The knight's planned time to the finish when the race was set (s), if known.
    knight_eta: Option<f32>,
    /// Picked from `pace`.
    pace: f32,
    course: Course,
    run_t: f32,
    winner: Option<Winner>,
    barb: Barb,
    /// The barb was thrown this way (+1 right).
    barb_dir: f32,
    go: bool,
    abandoned: bool,
    depart_from: Vec2,
    /// Her parting line, said after the outcome (see `Stage::Farewell`).
    pub(super) farewell: Option<String>,
}

/// The barb's resting point: its centre, sitting on the ledge.
fn barb_rest(finish: Vec2, ppu: f32) -> Vec2 {
    finish - Vec2::new(0.0, BARB_REST * ppu)
}

/// Where the barb is `t` seconds into a `secs` flight from `from` to `to`, lobbed `lift`
/// px above the straight line at its middle.
fn barb_arc(from: Vec2, to: Vec2, lift: f32, t: f32, secs: f32) -> Vec2 {
    let s = (t / secs.max(1e-3)).clamp(0.0, 1.0);
    let mut p = from + (to - from) * s;
    p.y -= 4.0 * lift * s * (1.0 - s);
    p
}

impl Race {
    pub(super) fn start(finish: Vec2, knight_eta: Option<f32>, def: &RaceDef, clips: &RaceClips, body: &mut Body, events: &mut Vec<NpcEvent>) -> Race {
        let pace = body.rng.range(def.pace[0], def.pace[1]);
        let text = pick_line(body.rng, &def.challenge);
        let len = crate::bubble::read_secs(&text);
        body.face(finish.x);
        body.play(clips.point, true);
        events.push(NpcEvent::Say { text, secs: len });
        let barb_dir = if finish.x >= body.pos.x { 1.0 } else { -1.0 };
        let depart_from = *body.pos;
        Race {
            stage: Stage::Challenge,
            t: 0.0,
            len,
            finish,
            knight_eta,
            pace,
            course: Course::default(),
            run_t: 0.0,
            winner: None,
            barb: Barb::Held,
            barb_dir,
            go: false,
            abandoned: false,
            depart_from,
            farewell: None,
        }
    }

    pub fn winner(&self) -> Option<Winner> {
        self.winner
    }

    /// Her course (once the race started).
    pub fn course(&self) -> &Course {
        &self.course
    }

    /// Mid-leap or leaping away: no footing (the actor doesn't make her fall).
    pub fn airborne(&self) -> bool {
        match self.stage {
            Stage::Run => matches!(self.course.at(self.run_t).kind, LegKind::Leap { .. }),
            Stage::Depart => true,
            _ => false,
        }
    }

    /// The knight's part: ready, then "Go!", then decided (or called off).
    pub fn cue(&self, serial: u32) -> SceneCue {
        let over = self.winner.is_some() || self.abandoned;
        SceneCue::Race { serial, finish: self.finish, go: self.go && !over, over, won: self.winner.map(|w| w == Winner::Knight) }
    }

    /// The barb (while it's out): tumbling in flight, sitting (stirring now and then),
    /// bursting.
    pub fn props(&self, lib: &ClipLibrary, clips: &RaceClips, ppu: f32) -> Vec<NpcProp> {
        let (at, key, burst) = match self.barb {
            Barb::Held | Barb::Gone => return Vec::new(),
            Barb::Flying { from, to, lift, t, secs } => (barb_arc(from, to, lift, t, secs), frame_at(lib, clips.barb, t, true), 0.0),
            Barb::Resting { at, t } => {
                let stir = t % BARB_STIR;
                let t = if stir < once_secs(lib, clips.barb) { stir } else { 0.0 };
                (at, frame_at(lib, clips.barb, t, false), 0.0)
            }
            Barb::Bursting { at, t } => (at, frame_at(lib, clips.burst, t, false), (t / once_secs(lib, clips.burst).max(0.1)).clamp(0.0, 1.0)),
        };
        let Some(key) = key else { return Vec::new() };
        // Mirrored when thrown right (it tumbles the way it flies).
        let drawn = Some(DrawnProp::Barb { radius: BARB_RADIUS * ppu, burst });
        vec![NpcProp { key, at, flip: self.barb_dir > 0.0, alpha: 1.0, drawn }]
    }

    /// The barb bursts where it is (reached, or the race is off); one still in her hand
    /// is simply put away.
    fn burst_barb(&mut self) {
        self.barb = match self.barb {
            Barb::Flying { from, to, lift, t, secs } => Barb::Bursting { at: barb_arc(from, to, lift, t, secs), t: 0.0 },
            Barb::Resting { at, .. } => Barb::Bursting { at, t: 0.0 },
            Barb::Held => Barb::Gone,
            b => b,
        };
    }

    /// Call it off (the user needs the screen, the Shade, enemies...): a parting word, the
    /// barb bursts, she leaves.
    pub(super) fn abandon(&mut self, def: &RaceDef, body: &mut Body, events: &mut Vec<NpcEvent>) {
        if self.abandoned || matches!(self.stage, Stage::Depart | Stage::Abandon) {
            return;
        }
        self.abandoned = true;
        events.push(NpcEvent::LineDone);
        if self.winner.is_none() {
            events.push(NpcEvent::RaceOver { knight_won: None });
        }
        self.burst_barb();
        self.farewell = None;
        let text = pick_line(body.rng, &def.abandon);
        self.len = crate::bubble::read_secs(&text).min(3.0);
        events.push(NpcEvent::Say { text, secs: self.len });
        self.stage = Stage::Abandon;
        self.t = 0.0;
    }

    pub(super) fn tick(&mut self, dt: f32, def: &RaceDef, clips: &RaceClips, body: &mut Body, env: &NpcEnv, events: &mut Vec<NpcEvent>) -> Next {
        let u = env.ppu;
        self.t += dt;
        let burst_secs = once_secs(body.lib, clips.burst);
        self.barb = match self.barb {
            Barb::Flying { from, to, lift, t, secs } if t + dt < secs => Barb::Flying { from, to, lift, t: t + dt, secs },
            Barb::Flying { to, .. } => Barb::Resting { at: to, t: 0.0 },
            Barb::Resting { at, t } => Barb::Resting { at, t: t + dt },
            Barb::Bursting { at, t } if t + dt < burst_secs => Barb::Bursting { at, t: t + dt },
            Barb::Bursting { .. } => Barb::Gone,
            b => b,
        };
        // The knight at the barb first?
        if self.go && self.winner.is_none() && !self.abandoned && self.stage == Stage::Run {
            let k = env.knight;
            if (k.x - self.finish.x).abs() <= def.reach * u && (k.y - self.finish.y).abs() <= 0.6 * u {
                log::info!("race: the knight reached the barb first");
                self.winner = Some(Winner::Knight);
                self.burst_barb();
                events.push(NpcEvent::RaceOver { knight_won: Some(true) });
            }
        }
        match self.stage {
            Stage::Challenge => {
                if self.t >= self.len {
                    events.push(NpcEvent::LineDone);
                    self.next(Stage::ThrowAntic, once_secs(body.lib, clips.throw_antic).max(0.2));
                    body.face(self.finish.x);
                    body.play(clips.throw_antic, true);
                }
            }
            Stage::ThrowAntic => {
                if self.t >= self.len {
                    body.play(clips.throw, true);
                    // Flung up from over her shoulder.
                    let hand = *body.pos + Vec2::new(self.barb_dir * 0.4 * u, -2.0 * u);
                    let to = barb_rest(self.finish, u);
                    let secs = (hand.distance(to) / (def.barb_speed * u)).clamp(0.4, 1.6);
                    // A lob, higher on longer throws.
                    let lift = (0.8 * u + 0.12 * hand.distance(to)).min(3.5 * u);
                    self.barb = Barb::Flying { from: hand, to, lift, t: 0.0, secs };
                    self.next(Stage::Throw, secs);
                }
            }
            Stage::Throw => {
                if self.t >= self.len {
                    body.play(clips.throw_recover, true);
                    self.next(Stage::Recover, once_secs(body.lib, clips.throw_recover).max(0.2));
                }
            }
            Stage::Recover => {
                if self.t >= self.len {
                    body.play(clips.ready, false);
                    self.next(Stage::Set, 0.7);
                }
            }
            Stage::Set => {
                if self.t >= self.len {
                    // Her course from where she stands, paced against the knight.
                    let span = ledge_span(env, *body.pos);
                    let natural = Course::plan(*body.pos, span, self.finish, u, def);
                    let target = match self.knight_eta {
                        // He needs a moment to get going too.
                        Some(eta) => self.pace * eta + 0.4,
                        None => self.pace * natural.secs(),
                    };
                    let k = (target / natural.arrival().max(0.1)).clamp(0.45, 2.2);
                    self.course = natural.paced(k);
                    log::info!(
                        "race: go! her course {:.1} s (pace {:.2}, knight's plan {}), {} legs",
                        self.course.arrival(),
                        self.pace,
                        self.knight_eta.map_or("?".into(), |e| format!("{e:.1} s")),
                        self.course.legs.len()
                    );
                    let text = pick_line(body.rng, &def.go);
                    events.push(NpcEvent::Say { secs: crate::bubble::read_secs(&text).min(1.6), text });
                    self.go = true;
                    self.run_t = 0.0;
                    self.next(Stage::Run, self.course.secs());
                }
            }
            Stage::Run => {
                self.run_t += dt;
                let p = self.course.at(self.run_t);
                *body.pos = p.pos;
                *body.looking_right = p.right;
                let clip = match p.kind {
                    LegKind::Run => clips.run,
                    LegKind::DashAntic => clips.dash_antic,
                    LegKind::Dash => clips.dash,
                    LegKind::Leap { air_dash: true, .. } if (0.35..0.7).contains(&p.progress) => clips.air_dash,
                    LegKind::Leap { .. } if p.progress < 0.45 => clips.jump,
                    LegKind::Leap { .. } => clips.fall,
                    LegKind::Land => clips.land,
                };
                body.play(clip, false);
                if self.winner.is_none() && self.run_t >= self.course.arrival() {
                    log::info!("race: Hornet reached her barb first");
                    self.winner = Some(Winner::Hornet);
                    self.burst_barb();
                    events.push(NpcEvent::RaceOver { knight_won: Some(false) });
                }
                if p.done {
                    *body.pos = self.finish;
                    events.push(NpcEvent::LineDone);
                    body.face(env.knight.x);
                    let (pool, clip) = match self.winner {
                        Some(Winner::Knight) => (&def.lose, clips.ready),
                        _ => (&def.win, clips.win),
                    };
                    let text = pick_line(body.rng, pool);
                    let len = crate::bubble::read_secs(&text).max(once_secs(body.lib, clip));
                    events.push(NpcEvent::Say { text, secs: len });
                    body.play(clip, true);
                    self.next(Stage::Finish, len);
                    return Next::Landed;
                }
            }
            Stage::Finish => {
                if self.t >= self.len {
                    events.push(NpcEvent::LineDone);
                    body.face(env.knight.x);
                    body.play(clips.ready, true);
                    // A breath, then her parting line (if any) and away.
                    self.next(Stage::Farewell { said: self.farewell.is_none() }, 0.4);
                }
            }
            Stage::Farewell { said } => {
                body.face(env.knight.x);
                if self.t >= self.len {
                    match self.farewell.take() {
                        Some(text) if !said => {
                            let len = crate::bubble::read_secs(&text);
                            events.push(NpcEvent::Say { text, secs: len });
                            self.next(Stage::Farewell { said: true }, len);
                        }
                        _ => {
                            events.push(NpcEvent::LineDone);
                            body.play(clips.depart, true);
                            self.depart_from = *body.pos;
                            self.next(Stage::Depart, 0.8);
                        }
                    }
                }
            }
            Stage::Abandon => {
                if self.t >= self.len {
                    events.push(NpcEvent::LineDone);
                    body.play(clips.depart, true);
                    self.depart_from = *body.pos;
                    self.next(Stage::Depart, 0.8);
                }
            }
            Stage::Depart => {
                // Up and away, fading.
                let s = (self.t / self.len).clamp(0.0, 1.0);
                let dir = if *body.looking_right { 1.0 } else { -1.0 };
                *body.pos = self.depart_from + Vec2::new(dir * 3.0 * u * s, -5.0 * u * s + 2.0 * u * s * s);
                *body.alpha = 1.0 - s;
                if s >= 1.0 {
                    return Next::Gone;
                }
            }
        }
        Next::Stay
    }

    fn next(&mut self, stage: Stage, len: f32) {
        self.stage = stage;
        self.t = 0.0;
        self.len = len;
    }
}

/// The span of the ledge under `at` (just `at` when there's none).
fn ledge_span(env: &NpcEnv, at: Vec2) -> (f32, f32) {
    env.terrain
        .ledges
        .iter()
        .filter(|l| (l.y - at.y).abs() < 1.5 && l.x0 - 1.0 <= at.x && at.x <= l.x1 + 1.0)
        .max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)))
        .map_or((at.x, at.x), |l| (l.x0.max(env.area.left), l.x1.min(env.area.right)))
}
