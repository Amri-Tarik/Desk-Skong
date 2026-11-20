//! The Troupe Master: a secret boss fight. Rare and earned: once enough achievements are
//! unlocked, a dim scarlet flame (a Nightmare Lantern's) now and then appears on a settled
//! ledge the knight can reach. The knight can't summon him: he goes over, lingers by it and
//! wonders ("?"), drawing the user's attention. The flame waits a long while (`lantern.secs`,
//! time to arrange the windows into an arena); when the user clicks it, Grimm steps out of
//! the flame, bows, says a line, and the show begins right there. A debug command summons
//! him (or lights the flame) at once.
//!
//! * [`GrimmConfig`] / [`GrimmSet`]: `data/grimm.json` and his clips (`Grimm Anim`,
//!   `Flameball Anim`, `Grimm Flash Anim`), validated.
//! * [`Grimm`] (`sim`): the routine (teleports, dash / dive, slashes and uppercut, firebats,
//!   flame pillars, spikes, the balloon at each phase change), all placed relative to the
//!   knight so any desktop is a stage; his flames ([`Hazard`]); the endings.
//! * [`Cameo`] (`cameo`): his rare friendly visit (a flourish, a bow, a line, a couple of
//!   teleports around the knight, farewell); sometimes he leaves the flame behind as a hint,
//!   only for a knight who could already find it.
//! * [`GrimmDirector`]: the lantern (schedule, start rules, lifetime, the click), the
//!   cameo's schedule, the summons, the aborts (fullscreen app, visitor, hard typing, a
//!   fight that drags on: he bows out, no penalty), and the outcome events. Pure; the app
//!   (`skong/src/grimm.rs`) places the lantern ([`plan_lantern`]), renders, passes the
//!   user's click on, and applies blows, hits and the reward.
//!
//! The brain sees the flame and the cameo as scenes ([`crate::ai::SceneCue`]: lingering by
//! the flame, watching him perform), Grimm as a foe (`FoeKind::Boss`, fought at the
//! engagement tier, lethal), and his flames (`FoeKind::Hazard`, never targets: dodged,
//! jumped, stepped away from).
mod cameo;
mod sim;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod cameo_tests;

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::ai::rng::Rng;
use crate::ai::{Foe, SceneCue};
use crate::anim::{ClipId, ClipLibrary};
use crate::assets::{SpriteBank, read_json};
use crate::desktop::{Monitor, WindowId};
use crate::geom::Vec2;
use crate::npc::{TalkContext, VisitConditions, VisitConfig, VisitScheduler};
use crate::portal::Footing;

pub use cameo::{Cameo, CameoTick};
pub use sim::{GRIMM_ID, Grimm, GrimmEnv, GrimmOutcome, GrimmStage, GrimmTick, Hazard, HazardKind, HitBy, LANTERN_ID, Move, SPIKE_CUE, ground_under};

/// How the lantern comes about (see `lantern` in `data/grimm.json`).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LanternConfig {
    pub enabled: bool,
    /// Achievements unlocked before the flame ever shows.
    pub min_achievements: usize,
    /// Seconds the flame waits for the user's click (long: time to build an arena).
    pub secs: f32,
    pub schedule: VisitConfig,
}

impl Default for LanternConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_achievements: 8,
            secs: 1800.0,
            schedule: VisitConfig { enabled: true, first: [1800.0, 4800.0], gap: [5400.0, 14400.0], busy_keys_per_sec: 2.5, away_secs: 90.0, retry: 60.0 },
        }
    }
}

/// His rare friendly visit (see `cameo` in `data/grimm.json`).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CameoConfig {
    pub enabled: bool,
    /// Teleports around the knight ([min, max]).
    pub hops: [u32; 2],
    /// Chance he leaves the scarlet flame behind (only once `lantern.min_achievements` are
    /// unlocked: the secret's own rule).
    pub flame_chance: f32,
    pub schedule: VisitConfig,
}

impl Default for CameoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hops: [2, 3],
            flame_chance: 0.4,
            schedule: VisitConfig { enabled: true, first: [2400.0, 6000.0], gap: [7200.0, 18000.0], busy_keys_per_sec: 2.5, away_secs: 90.0, retry: 60.0 },
        }
    }
}

/// What he says, when.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    Intro,
    Phase2,
    Phase3,
    /// He was beaten.
    Defeat,
    /// The knight fell.
    Triumph,
    /// Called off.
    BowOut,
    /// The friendly visit: after his bow.
    Cameo,
    /// The friendly visit: farewell.
    CameoBye,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GrimmLines {
    pub intro: Vec<String>,
    pub phase2: Vec<String>,
    pub phase3: Vec<String>,
    pub defeat: Vec<String>,
    pub triumph: Vec<String>,
    pub bow_out: Vec<String>,
    #[serde(default)]
    pub cameo: Vec<String>,
    #[serde(default)]
    pub cameo_bye: Vec<String>,
}

impl GrimmLines {
    pub fn of(&self, kind: LineKind) -> &[String] {
        match kind {
            LineKind::Intro => &self.intro,
            LineKind::Phase2 => &self.phase2,
            LineKind::Phase3 => &self.phase3,
            LineKind::Defeat => &self.defeat,
            LineKind::Triumph => &self.triumph,
            LineKind::BowOut => &self.bow_out,
            LineKind::Cameo => &self.cameo,
            LineKind::CameoBye => &self.cameo_bye,
        }
    }

    /// A line of `kind` whose placeholders can be filled (`None` if none can).
    pub fn pick(&self, kind: LineKind, ctx: &TalkContext, rng: &mut Rng) -> Option<String> {
        let pool = self.of(kind);
        let start = rng.next_u32() as usize;
        (0..pool.len()).find_map(|k| ctx.fill(&pool[(start + k) % pool.len()], rng))
    }
}

/// `data/grimm.json` (see its `$comment`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GrimmConfig {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    pub library: String,
    pub fx_library: String,
    #[serde(default)]
    pub flash_library: Option<String>,
    /// His art's scale relative to the knight's.
    pub art_scale: f32,
    /// Body [w, h] (game units).
    pub size: [f32; 2],
    pub hp: u32,
    /// Fractions of `hp` left where phase 2 and 3 begin.
    pub phases: [f32; 2],
    pub tempo: f32,
    /// Seconds the knight can't be hurt again after a hit.
    pub invuln: f32,
    pub geo: [u32; 2],
    pub max_secs: f32,
    pub busy_keys_per_sec: f32,
    pub busy_secs: f32,
    #[serde(default)]
    pub lantern: LanternConfig,
    #[serde(default)]
    pub cameo: CameoConfig,
    pub lines: GrimmLines,
}

impl GrimmConfig {
    pub fn load(path: &Path) -> Result<GrimmConfig> {
        let cfg: GrimmConfig = read_json(path)?;
        cfg.validate().with_context(|| path.display().to_string())?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        let pos = |x: f32| x.is_finite() && x > 0.0;
        if self.hp == 0 || !pos(self.art_scale) || !pos(self.size[0]) || !pos(self.size[1]) || !pos(self.tempo) || !pos(self.max_secs) {
            bail!("grimm: hp, art_scale, size, tempo and max_secs must be positive");
        }
        let [p2, p3] = self.phases;
        if !(0.0 < p3 && p3 < p2 && p2 < 1.0) {
            bail!("grimm: phases must be descending fractions in (0, 1)");
        }
        let non_neg = |x: f32| x.is_finite() && x >= 0.0;
        if self.geo[1] < self.geo[0] || !non_neg(self.invuln) || !non_neg(self.busy_secs) || !pos(self.lantern.secs) {
            bail!("grimm: bad range");
        }
        if self.cameo.hops[1] < self.cameo.hops[0] || !(0.0..=1.0).contains(&self.cameo.flame_chance) {
            bail!("grimm: bad cameo hops or flame_chance");
        }
        let mut kinds = vec![LineKind::Intro, LineKind::Phase2, LineKind::Phase3, LineKind::Defeat, LineKind::Triumph, LineKind::BowOut];
        if self.cameo.enabled {
            kinds.extend([LineKind::Cameo, LineKind::CameoBye]);
        }
        for kind in kinds {
            let pool = self.lines.of(kind);
            if pool.is_empty() {
                bail!("grimm: no {kind:?} lines");
            }
            for l in pool {
                crate::npc::talk::check_template(l).with_context(|| format!("grimm {kind:?} line '{l}'"))?;
                // A pool must always have something to say, even with nothing to fill in.
            }
            if !pool.iter().any(|l| !l.contains('{')) {
                bail!("grimm: every {kind:?} line has a placeholder; add a plain one");
            }
        }
        Ok(())
    }
}

/// His clips, resolved.
#[derive(Clone, Debug)]
pub struct GrimmClips {
    pub idle: ClipId,
    pub tele_in: ClipId,
    pub tele_out: ClipId,
    pub bow: ClipId,
    pub bow_return: ClipId,
    pub roar_antic: ClipId,
    pub roar: ClipId,
    pub air_dash_antic: ClipId,
    pub air_dash: ClipId,
    pub g_dash: ClipId,
    pub slash_antic: ClipId,
    pub slash1: ClipId,
    pub slash2: ClipId,
    pub slash3: ClipId,
    pub slash_recover: ClipId,
    pub uppercut_antic: ClipId,
    pub uppercut: ClipId,
    pub uppercut_end: ClipId,
    pub cast_antic: ClipId,
    pub cast: ClipId,
    pub cast_return: ClipId,
    pub absorb_start: ClipId,
    pub absorb: ClipId,
    pub absorb_end: ClipId,
    pub spike_ready: ClipId,
    pub spike_antic: ClipId,
    pub spike_up: ClipId,
    pub spike_down: ClipId,
    pub balloon_antic: ClipId,
    pub balloon: ClipId,
    pub balloon_end: ClipId,
    pub evade: ClipId,
    pub evade_end: ClipId,
    pub death_stun: ClipId,
    pub explode: ClipId,
    pub bat_fly: ClipId,
    /// His flames (same library).
    pub firebat: ClipId,
    pub pillar: ClipId,
    /// The friendly visit's flourishes.
    pub tele_to_idle: ClipId,
    pub hand_out: ClipId,
    pub hand_click: ClipId,
    pub look_back: ClipId,
}

impl GrimmClips {
    fn resolve(lib: &ClipLibrary, name: &str) -> Result<GrimmClips> {
        let get = |n: &str| lib.id(n).with_context(|| format!("grimm: no clip '{n}' in {name}"));
        Ok(GrimmClips {
            idle: get("Idle")?,
            tele_in: get("Tele In")?,
            tele_out: get("Tele Out")?,
            bow: get("Bow")?,
            bow_return: get("Bow Return")?,
            roar_antic: get("Roar Antic")?,
            roar: get("Roar")?,
            air_dash_antic: get("Air Dash Antic")?,
            air_dash: get("Air Dash")?,
            g_dash: get("G Dash")?,
            slash_antic: get("Slash Antic")?,
            slash1: get("Slash 1")?,
            slash2: get("Slash 2")?,
            slash3: get("Slash 3")?,
            slash_recover: get("Slash Recover")?,
            uppercut_antic: get("Uppercut Antic")?,
            uppercut: get("Uppercut")?,
            uppercut_end: get("Uppercut End")?,
            cast_antic: get("Cast Antic")?,
            cast: get("Cast")?,
            cast_return: get("Cast Return")?,
            absorb_start: get("Absorb Start")?,
            absorb: get("Absorb")?,
            absorb_end: get("Absorb End")?,
            spike_ready: get("Spike Ready")?,
            spike_antic: get("Spike Antic")?,
            spike_up: get("Spike Up")?,
            spike_down: get("Spike Down")?,
            balloon_antic: get("Balloon Antic")?,
            balloon: get("Balloon")?,
            balloon_end: get("Balloon End")?,
            evade: get("Evade")?,
            evade_end: get("Evade End")?,
            death_stun: get("Death Stun")?,
            explode: get("Explode")?,
            bat_fly: get("Bat Fly")?,
            firebat: get("Firebat")?,
            pillar: get("Pillar")?,
            tele_to_idle: get("TeleToIdle")?,
            hand_out: get("Hand Out")?,
            hand_click: get("Hand Click")?,
            look_back: get("LookBack")?,
        })
    }
}

/// Grimm's data and clip libraries.
pub struct GrimmSet {
    pub cfg: GrimmConfig,
    pub lib: ClipLibrary,
    pub clips: GrimmClips,
    /// `Flameball Anim`: the fireballs (and the lantern's flame).
    pub fx: ClipLibrary,
    pub fireball: ClipId,
    /// `Grimm Flash Anim` (optional): the teleport flash.
    pub flash: Option<(ClipLibrary, ClipId)>,
}

fn lib_path(anim_dir: &Path, name: &str) -> std::path::PathBuf {
    anim_dir.join(format!("{}.json", name.replace(' ', "_")))
}

impl GrimmSet {
    pub fn load(json: &Path, anim_dir: &Path, bank: &SpriteBank) -> Result<GrimmSet> {
        let cfg = GrimmConfig::load(json)?;
        Self::with_config(cfg, anim_dir, bank)
    }

    pub fn with_config(cfg: GrimmConfig, anim_dir: &Path, bank: &SpriteBank) -> Result<GrimmSet> {
        let lib = ClipLibrary::load(&lib_path(anim_dir, &cfg.library), bank).with_context(|| format!("grimm: {}", cfg.library))?;
        let clips = GrimmClips::resolve(&lib, &cfg.library)?;
        let fx = ClipLibrary::load(&lib_path(anim_dir, &cfg.fx_library), bank).with_context(|| format!("grimm: {}", cfg.fx_library))?;
        let fireball = fx.id("Idle").with_context(|| format!("grimm: no clip 'Idle' in {}", cfg.fx_library))?;
        let flash = cfg.flash_library.as_deref().and_then(|n| {
            let l = ClipLibrary::load(&lib_path(anim_dir, n), bank).ok()?;
            let id = l.id("Flash")?;
            Some((l, id))
        });
        Ok(GrimmSet { cfg, lib, clips, fx, fireball, flash })
    }
}

/// Where the lantern's flame burns: a feet point on a footing near the knight (5–16 units
/// away, on his monitor, room for it), that `reachable` (a route check) accepts.
pub fn plan_lantern(footings: &[Footing], knight: Vec2, monitors: &[Monitor], ppu: f32, rng: &mut Rng, reachable: &mut dyn FnMut(Vec2) -> bool) -> Option<(Vec2, Option<WindowId>)> {
    let mon = monitors.iter().find(|m| m.work.contains(knight.x as i32, (knight.y - 1.0) as i32)).or_else(|| monitors.iter().find(|m| m.primary)).or(monitors.first())?;
    let area = mon.work.to_f();
    let (near, far) = (5.0 * ppu, 16.0 * ppu);
    let mut cands: Vec<(f32, Vec2, Option<WindowId>)> = Vec::new();
    for f in footings.iter().filter(|f| f.y > area.top + 3.0 * ppu && f.y <= area.bottom + 1.0) {
        let (x0, x1) = (f.x0.max(area.left) + 0.8 * ppu, f.x1.min(area.right) - 0.8 * ppu);
        if x1 <= x0 {
            continue;
        }
        let (a, b) = (x0.max(knight.x - far), x1.min(knight.x + far));
        if b < a {
            continue;
        }
        for _ in 0..5 {
            let p = Vec2::new(a + rng.f32() * (b - a), f.y);
            let d = p.distance(knight);
            if (near..=far).contains(&d) {
                cands.push((d + 0.4 * far * rng.f32(), p, f.owner));
            }
        }
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    cands.into_iter().take(8).find(|c| reachable(c.1)).map(|c| (c.1, c.2))
}

/// The scarlet flame waiting to be struck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lantern {
    /// Feet point (on its ledge).
    pub at: Vec2,
    pub owner: Option<WindowId>,
    rel: Vec2,
    /// 0..1 (fades in, flickers dim, fades out).
    pub alpha: f32,
    /// Seconds since it lit (flicker).
    pub age: f32,
    fading: bool,
}

impl Lantern {
    /// Burning bright enough to be clicked (not fading).
    pub fn lit(&self) -> bool {
        self.alpha >= 0.5 && !self.fading
    }

    /// Where its (clickable) flame is drawn, about: a box over its feet point.
    pub fn rect(&self, ppu: f32) -> crate::geom::RectF {
        crate::geom::RectF::from_feet(self.at, Vec2::new(0.8, 1.2) * ppu)
    }
}

/// How Grimm came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summon {
    /// The user clicked the scarlet flame.
    Lantern,
    /// The debug command, straight to the show.
    Direct,
}

/// Why he bowed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BowOutReason {
    Fullscreen,
    Visitor,
    Typing,
    TooLong,
    /// A game took the monitor (he just vanishes).
    Game,
}

/// What happened (logged by stable name; achievements key on them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GrimmEvent {
    /// `grimm_lantern_lit`.
    LanternLit { natural: bool },
    /// `grimm_lantern_faded`: nobody struck it in time (or the moment passed).
    LanternFaded,
    /// `grimm_summoned`.
    Summoned { via: Summon },
    /// `grimm_phase`: phase 2 or 3 began.
    Phase { phase: u8 },
    /// `grimm_defeated`: the knight won; `geo` bursts where Grimm was.
    Defeated { geo: u32 },
    /// `grimm_triumphed`: the knight fell.
    KnightFell,
    /// `grimm_bowed_out`.
    BowedOut { reason: BowOutReason },
    /// `grimm_cameo`: his friendly visit began.
    CameoArrived,
    /// `grimm_cameo_over`: it ended (`finished`: seen through to his farewell; `flame`: he
    /// left the scarlet flame behind).
    CameoOver { finished: bool, flame: bool },
}

impl GrimmEvent {
    pub fn name(&self) -> &'static str {
        match self {
            GrimmEvent::LanternLit { .. } => "grimm_lantern_lit",
            GrimmEvent::LanternFaded => "grimm_lantern_faded",
            GrimmEvent::Summoned { .. } => "grimm_summoned",
            GrimmEvent::Phase { .. } => "grimm_phase",
            GrimmEvent::Defeated { .. } => "grimm_defeated",
            GrimmEvent::KnightFell => "grimm_triumphed",
            GrimmEvent::BowedOut { .. } => "grimm_bowed_out",
            GrimmEvent::CameoArrived => "grimm_cameo",
            GrimmEvent::CameoOver { .. } => "grimm_cameo_over",
        }
    }
}

/// The world this tick, as the troupe cares.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrimmWorld {
    /// Achievements unlocked so far.
    pub achievements: usize,
    pub keys_per_sec: f32,
    pub user_idle_secs: f32,
    pub fullscreen: bool,
    pub visitor_present: bool,
    /// A gauntlet is on (or waiting to start).
    pub gauntlet: bool,
    /// Other enemies alive.
    pub others_alive: usize,
    /// Asleep, hiding, held...
    pub knight_busy: bool,
    pub knight_down: bool,
    /// A Shade is waiting for him (it comes first).
    pub shade: bool,
    /// The knight's feet.
    pub knight_at: Vec2,
    /// Placements can be route-checked (a nav graph is ready) and the art is loaded.
    pub ready: bool,
}

/// Why the lantern (`natural`) or a summon can't happen now (`None`: it can).
pub fn start_blocked(cfg: &GrimmConfig, w: &GrimmWorld, natural: bool) -> Option<&'static str> {
    if w.fullscreen {
        Some("a fullscreen app")
    } else if w.visitor_present {
        Some("a visitor")
    } else if w.gauntlet {
        Some("a gauntlet")
    } else if w.others_alive > 0 {
        Some("another fight")
    } else if w.knight_down {
        Some("the knight is down")
    } else if w.keys_per_sec >= cfg.busy_keys_per_sec {
        Some("the user is typing")
    } else if natural && w.shade {
        Some("his Shade waits")
    } else if natural && (w.knight_busy || w.user_idle_secs >= cfg.lantern.schedule.away_secs) {
        Some("the knight is busy or the user away")
    } else if !w.ready {
        Some("not ready yet (nav graph, art)")
    } else if natural && w.achievements < cfg.lantern.min_achievements {
        Some("not enough achievements")
    } else {
        None
    }
}

/// What the app should do this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrimmStep {
    /// Find a spot for the lantern now (answer with [`GrimmDirector::lantern_placed`]).
    pub place_lantern: bool,
    /// Grimm speaks (pick the text with [`GrimmLines::pick`]).
    pub say: Option<LineKind>,
    /// Times the knight got hurt this tick, and by what.
    pub hits: u32,
    pub hit_by: Option<HitBy>,
    /// Teleport flashes (body centres).
    pub flashes: Vec<Vec2>,
    /// Beaten: burst this much Geo here.
    pub reward: Option<(Vec2, u32)>,
    pub events: Vec<GrimmEvent>,
}

/// A lantern wanted (natural or asked for), waiting for a spot.
#[derive(Clone, Copy, Debug)]
struct Want {
    natural: bool,
    tries: u32,
    retry_in: f32,
    waited: f32,
}

/// Lantern, summons, the show and its endings. Pure; see the module docs.
pub struct GrimmDirector {
    sched: VisitScheduler,
    rng: Rng,
    lantern: Option<Lantern>,
    grimm: Option<Grimm>,
    /// A summon asked for (debug), and how long it has waited.
    pending: Option<(Summon, f32)>,
    want: Option<Want>,
    /// `place_lantern` was asked this tick.
    asked: bool,
    busy_for: f32,
    /// Seconds of the show so far.
    secs: f32,
    held_by: Option<&'static str>,
    /// The user clicked the flame: Grimm steps out at the next good moment.
    clicked: bool,
    /// His friendly visit, its schedule, a debug request for one (and how long it waited).
    cameo: Option<Cameo>,
    cameo_sched: VisitScheduler,
    cameo_pending: Option<f32>,
    /// The cameo was cut short.
    cameo_cut: bool,
    cameos: u32,
}

/// A requested summon waits this long for a good moment (s).
const SUMMON_WAIT: f32 = 120.0;

impl GrimmDirector {
    pub fn new(cfg: &GrimmConfig, seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let sched = VisitScheduler::new(cfg.lantern.schedule, &mut rng);
        let cameo_sched = VisitScheduler::new(cfg.cameo.schedule, &mut rng);
        Self {
            sched,
            rng,
            lantern: None,
            grimm: None,
            pending: None,
            want: None,
            asked: false,
            busy_for: 0.0,
            secs: 0.0,
            held_by: None,
            clicked: false,
            cameo: None,
            cameo_sched,
            cameo_pending: None,
            cameo_cut: false,
            cameos: 0,
        }
    }

    /// Debug: his friendly visit at the next good moment.
    pub fn request_cameo(&mut self) {
        if self.grimm.is_some() || self.cameo.is_some() {
            log::info!("grimm: he's already here");
            return;
        }
        self.cameo_pending = Some(0.0);
    }

    pub fn cameo(&self) -> Option<&Cameo> {
        self.cameo.as_ref()
    }

    /// Grimm on stage (the show or his visit), or about to be: no visitors or enemies now.
    /// A scarlet flame alone doesn't hold anything up: it waits for the user's click.
    pub fn busy(&self) -> bool {
        self.grimm.is_some() || self.cameo.is_some() || self.pending.is_some() || self.cameo_pending.is_some()
    }

    /// The user clicked the scarlet flame: `true` when one is lit (Grimm steps out of it at
    /// the next good moment).
    pub fn click_lantern(&mut self) -> bool {
        match &self.lantern {
            Some(l) if l.lit() => {
                if !self.clicked {
                    log::info!("grimm: the scarlet flame is clicked");
                }
                self.clicked = true;
                true
            }
            _ => false,
        }
    }

    /// The knight's part: watching the friendly visit, or lingering by a lit flame.
    pub fn scene_cue(&self, set: &GrimmSet) -> Option<SceneCue> {
        if self.grimm.is_some() {
            return None;
        }
        if let Some(c) = &self.cameo {
            return (!c.leaving()).then(|| c.cue(set));
        }
        self.lantern.filter(|l| !l.fading).map(|l| SceneCue::Flame { at: l.at })
    }

    /// Seconds of presence until the next lantern may show (0 = due).
    pub fn lantern_due_in(&self) -> f32 {
        self.sched.due_in()
    }

    /// The debug command: Grimm now (or the lantern first), at the next good moment.
    pub fn request(&mut self, via: Summon) {
        if self.grimm.is_some() {
            log::info!("grimm: the show is already on");
            return;
        }
        self.pending = Some((via, 0.0));
        self.held_by = None;
    }

    /// The flame, the show or his visit is on (or wanted): his art stays loaded.
    pub fn active(&self) -> bool {
        self.busy() || self.lantern.is_some() || self.want.is_some()
    }

    /// Grimm is on stage.
    pub fn fighting(&self) -> bool {
        self.grimm.is_some()
    }

    pub fn grimm(&self) -> Option<&Grimm> {
        self.grimm.as_ref()
    }

    pub fn lantern(&self) -> Option<&Lantern> {
        self.lantern.as_ref()
    }

    /// What a summon (or a clicked flame) is waiting for, if it waits.
    pub fn held_by(&self) -> Option<&'static str> {
        self.held_by
    }

    /// The brain's foes: Grimm and his flames (the scarlet flame is the user's to click).
    pub fn foes(&self, set: &GrimmSet, ppu: f32) -> Vec<Foe> {
        let mut v: Vec<Foe> = Vec::new();
        if let Some(g) = &self.grimm {
            v.extend(g.foe(set, ppu));
            v.extend(g.hazards.iter().map(Hazard::foe));
        }
        v
    }

    /// The app answered [`GrimmStep::place_lantern`] (`None`: no good spot right now; it is
    /// asked again a little later, a few times).
    pub fn lantern_placed(&mut self, at: Option<(Vec2, Option<WindowId>)>, env: &GrimmEnv) -> Option<GrimmEvent> {
        if !std::mem::take(&mut self.asked) {
            return None;
        }
        let want = self.want.as_mut()?;
        match at {
            Some((at, owner)) => {
                let natural = want.natural;
                self.want = None;
                let rel = owner.and_then(|id| env.terrain.window_rect(id)).map_or(Vec2::ZERO, |r| at - Vec2::new(r.left, r.top));
                self.lantern = Some(Lantern { at, owner, rel, alpha: 0.0, age: 0.0, fading: false });
                log::info!("grimm: a scarlet flame flickers at ({:.0},{:.0})", at.x, at.y);
                Some(GrimmEvent::LanternLit { natural })
            }
            None => {
                want.tries = want.tries.saturating_sub(1);
                want.retry_in = 20.0;
                if want.tries == 0 {
                    log::info!("grimm: no good spot for the lantern; maybe later");
                    self.want = None;
                }
                None
            }
        }
    }

    fn spawn(&mut self, set: &GrimmSet, at: Vec2, toward: f32, ppu: f32) {
        self.grimm = Some(Grimm::new(set, at, toward, self.rng.next_u32(), ppu));
        self.secs = 0.0;
        self.busy_for = 0.0;
        self.pending = None;
        self.want = None;
        self.asked = false;
        self.lantern = None;
        self.clicked = false;
        self.sched.schedule_next(&mut self.rng);
    }

    /// His friendly visit begins beside the knight (on his ground line, the roomier side).
    fn spawn_cameo(&mut self, set: &GrimmSet, w: &GrimmWorld, env: &GrimmEnv) {
        let (gy, x0, x1) = ground_under(env.terrain, w.knight_at, env.area);
        let side = if w.knight_at.x - x0 > x1 - w.knight_at.x { -1.0 } else { 1.0 };
        let u = env.ppu;
        let x = (w.knight_at.x + side * 4.0 * u).clamp(env.area.left + u, (env.area.right - u).max(env.area.left + u));
        let c = &set.cfg.cameo;
        let hops = c.hops[0] + self.rng.next_u32() % (c.hops[1] - c.hops[0] + 1);
        // The hint only for a knight who could already find the secret.
        let flame = set.cfg.lantern.enabled && w.achievements >= set.cfg.lantern.min_achievements && self.rng.chance(c.flame_chance);
        self.cameos += 1;
        log::info!("grimm: a friendly visit ({hops} teleports{})", if flame { ", leaving his flame" } else { "" });
        self.cameo = Some(Cameo::new(set, Vec2::new(x, gy), w.knight_at.x, hops, flame, self.cameos, self.rng.next_u32(), u));
        self.cameo_pending = None;
        self.cameo_cut = false;
        self.cameo_sched.schedule_next(&mut self.rng);
    }

    /// The knight's blow on Grimm: `Some(beaten)` when it landed.
    pub fn strike(&mut self, damage: u32, set: &GrimmSet) -> Option<bool> {
        self.grimm.as_mut()?.strike(damage, set)
    }

    /// A game took the stage: the flame and the show vanish at once.
    pub fn vanish(&mut self) -> Vec<GrimmEvent> {
        let mut ev = Vec::new();
        if self.lantern.take().is_some() {
            ev.push(GrimmEvent::LanternFaded);
        }
        if let Some(g) = self.grimm.take()
            && g.outcome().is_none()
        {
            ev.push(GrimmEvent::BowedOut { reason: BowOutReason::Game });
        }
        if self.cameo.take().is_some() {
            ev.push(GrimmEvent::CameoOver { finished: false, flame: false });
        }
        self.want = None;
        self.asked = false;
        self.clicked = false;
        ev
    }

    pub fn tick(&mut self, dt: f32, w: &GrimmWorld, env: &GrimmEnv, set: &GrimmSet) -> GrimmStep {
        let mut step = GrimmStep::default();
        let cfg = &set.cfg;
        self.asked = false;

        // 1. The show.
        if let Some(g) = self.grimm.as_mut() {
            self.secs += dt;
            self.busy_for = if w.keys_per_sec >= cfg.busy_keys_per_sec { self.busy_for + dt } else { 0.0 };
            if g.outcome().is_none() {
                let reason = if w.fullscreen {
                    Some(BowOutReason::Fullscreen)
                } else if w.visitor_present {
                    Some(BowOutReason::Visitor)
                } else if self.busy_for >= cfg.busy_secs {
                    Some(BowOutReason::Typing)
                } else if self.secs > cfg.max_secs {
                    Some(BowOutReason::TooLong)
                } else {
                    None
                };
                if w.knight_down {
                    log::info!("grimm: the knight fell; a bow, and the troupe moves on");
                    g.triumph(set);
                    step.events.push(GrimmEvent::KnightFell);
                } else if let Some(reason) = reason {
                    log::info!("grimm: bowing out ({reason:?})");
                    g.bow_out(set, false);
                    step.events.push(GrimmEvent::BowedOut { reason });
                }
            }
            let t = g.tick(dt, set, env);
            step.hits = t.hits;
            step.hit_by = t.hit_by;
            step.say = t.say;
            step.flashes = t.flashes;
            if let Some(phase) = t.phase {
                log::info!("grimm: phase {phase}");
                step.events.push(GrimmEvent::Phase { phase });
            }
            if let Some(at) = t.burst {
                let geo = cfg.geo[0] + self.rng.next_u32() % (cfg.geo[1] - cfg.geo[0] + 1);
                log::info!("grimm: beaten; {geo} geo");
                step.reward = Some((at, geo));
                step.events.push(GrimmEvent::Defeated { geo });
            }
            if g.stage() == GrimmStage::Gone {
                self.grimm = None;
            }
            return step;
        }

        // 1b. His friendly visit.
        if let Some(c) = self.cameo.as_mut() {
            self.busy_for = if w.keys_per_sec >= cfg.busy_keys_per_sec { self.busy_for + dt } else { 0.0 };
            let abort = w.fullscreen || w.visitor_present || w.gauntlet || w.others_alive > 0 || w.knight_down || w.shade || self.busy_for >= cfg.busy_secs;
            if abort && !c.leaving() {
                log::info!("grimm: his visit is cut short");
                c.bow_out(set);
                self.cameo_cut = true;
            }
            let t = c.tick(dt, set, env);
            step.say = t.say;
            step.flashes = t.flashes;
            if let Some(flame) = t.left {
                step.events.push(GrimmEvent::CameoOver { finished: !self.cameo_cut, flame });
                if flame {
                    // The hint: his flame, placed like a natural one (settled, reachable).
                    self.want = Some(Want { natural: true, tries: 3, retry_in: 0.5, waited: 0.0 });
                }
            }
            if c.gone() {
                self.cameo = None;
            }
            return step;
        }

        // 2. The flame, waiting for the user's click (a long while: time to build an arena).
        if let Some(l) = self.lantern.as_mut() {
            l.age += dt;
            match l.owner.map(|id| env.terrain.window_rect(id)) {
                Some(Some(r)) => l.at = Vec2::new(r.left, r.top) + l.rel,
                Some(None) => l.fading = true,
                None => {}
            }
            // Only hard aborts put it out early (a fullscreen app); visitors, fights and
            // the Shade come and go while it waits.
            if w.fullscreen || l.age >= cfg.lantern.secs {
                l.fading = true;
            }
            if self.clicked && !l.fading {
                match start_blocked(cfg, w, false) {
                    None => {
                        let at = l.at;
                        log::info!("grimm: the troupe master steps out of the clicked flame");
                        self.spawn(set, at, w.knight_at.x, env.ppu);
                        self.held_by = None;
                        step.events.push(GrimmEvent::Summoned { via: Summon::Lantern });
                        return step;
                    }
                    Some(why) => {
                        if self.held_by != Some(why) {
                            log::info!("grimm: clicked; he waits for a good moment ({why})");
                            self.held_by = Some(why);
                        }
                    }
                }
            }
            if l.fading {
                l.alpha -= dt / 1.5;
                if l.alpha <= 0.0 {
                    log::info!("grimm: the scarlet flame goes out");
                    self.lantern = None;
                    self.clicked = false;
                    step.events.push(GrimmEvent::LanternFaded);
                }
            } else {
                l.alpha = (l.alpha + dt / 2.0).min(1.0);
            }
            return step;
        }

        // 3. A summon asked for (debug).
        if let Some((via, waited)) = self.pending.as_mut() {
            *waited += dt;
            let via = *via;
            if *waited > SUMMON_WAIT {
                log::info!("grimm: no good moment came ({}); maybe later", self.held_by.unwrap_or("?"));
                self.pending = None;
            } else if let Some(why) = start_blocked(cfg, w, false) {
                if self.held_by != Some(why) {
                    log::info!("grimm: waiting ({why})");
                    self.held_by = Some(why);
                }
            } else {
                self.pending = None;
                self.held_by = None;
                match via {
                    Summon::Direct => {
                        // Beside him on his ground line, on the roomier side.
                        let (gy, x0, x1) = ground_under(env.terrain, w.knight_at, env.area);
                        let side = if w.knight_at.x - x0 > x1 - w.knight_at.x { -1.0 } else { 1.0 };
                        let u = env.ppu;
                        let x = (w.knight_at.x + side * 5.0 * u).clamp(env.area.left + u, (env.area.right - u).max(env.area.left + u));
                        log::info!("grimm: summoned");
                        self.spawn(set, Vec2::new(x, gy), w.knight_at.x, u);
                        step.events.push(GrimmEvent::Summoned { via: Summon::Direct });
                        return step;
                    }
                    Summon::Lantern => self.want = Some(Want { natural: false, tries: 5, retry_in: 0.0, waited: 0.0 }),
                }
            }
        }

        // 4. The natural flame: only for a knight with enough achievements, now and then.
        if cfg.lantern.enabled && w.achievements >= cfg.lantern.min_achievements && self.want.is_none() {
            let c = VisitConditions {
                user_idle_secs: w.user_idle_secs,
                keys_per_sec: w.keys_per_sec,
                fullscreen: w.fullscreen,
                visitor_present: w.visitor_present || w.gauntlet || w.others_alive > 0 || w.shade,
                knight_busy: w.knight_busy || w.knight_down,
            };
            if self.sched.tick(dt, &c, &mut self.rng) {
                log::info!("grimm: a lantern is due");
                self.want = Some(Want { natural: true, tries: 3, retry_in: 0.0, waited: 0.0 });
            }
        }

        // 4b. His friendly visit: rare, for everyone (or asked for).
        if self.want.is_none() {
            if self.cameo_pending.is_none() && cfg.cameo.enabled && self.lantern.is_none() {
                let c = VisitConditions {
                    user_idle_secs: w.user_idle_secs,
                    keys_per_sec: w.keys_per_sec,
                    fullscreen: w.fullscreen,
                    visitor_present: w.visitor_present || w.gauntlet || w.others_alive > 0 || w.shade,
                    knight_busy: w.knight_busy || w.knight_down,
                };
                if self.cameo_sched.tick(dt, &c, &mut self.rng) {
                    log::info!("grimm: a friendly visit is due");
                    self.cameo_pending = Some(0.0);
                }
            }
            if let Some(waited) = self.cameo_pending.as_mut() {
                // Waits (art loading, a good moment) a little, like a summon.
                *waited += dt;
                let blocked = start_blocked(cfg, w, false).or(if w.shade || w.knight_busy { Some("the knight is busy") } else { None });
                if *waited > SUMMON_WAIT {
                    log::info!("grimm: no good moment for a visit came; maybe later");
                    self.cameo_pending = None;
                } else if let Some(why) = blocked {
                    if self.held_by != Some(why) {
                        log::info!("grimm: a visit waits ({why})");
                        self.held_by = Some(why);
                    }
                } else {
                    self.held_by = None;
                    self.spawn_cameo(set, w, env);
                    step.events.push(GrimmEvent::CameoArrived);
                    return step;
                }
            }
        }

        // 5. Ask the app for a spot when the moment is right.
        if let Some(want) = self.want.as_mut() {
            want.retry_in -= dt;
            want.waited += dt;
            let limit = if want.natural { 600.0 } else { SUMMON_WAIT };
            if want.waited > limit {
                log::info!("grimm: the lantern's moment passed");
                self.want = None;
            } else if want.retry_in <= 0.0 && start_blocked(cfg, w, want.natural).is_none() {
                step.place_lantern = true;
                self.asked = true;
            }
        }
        step
    }
}
