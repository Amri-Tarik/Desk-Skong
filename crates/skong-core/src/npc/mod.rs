//! NPC visitors: Hollow Knight characters who drop by now and then (walk in from a screen
//! edge, appear perched on a window edge, step out of a dream portal or fall out of one),
//! hang around for a while, talk to the knight when he comes over, and leave.
//!
//! * [`NpcSet`] — definitions from `data/npcs.json`, validated against their exported clip
//!   libraries (`data/hk/animations/<Library>.json`); NPC art is always vanilla.
//! * [`Npc`] — one live visitor: footing (rides the window it stands on), animation, the
//!   arrive → wait → talk → leave state machine, and [`NpcEvent`]s for the app (lines to
//!   show in a bubble, portals to open).
//! * [`VisitScheduler`] — when the next visit happens (organic, never during fullscreen
//!   apps or fast typing).
//! * [`talk`] — conversation lines with desktop-aware templates; [`dream`] — the knight's
//!   dream-nail thoughts about windows.
//! * [`lesson`] — teachers (Sly) follow the talk with a training session: drills,
//!   demonstrations with their own clips, praise and corrections.
//! * [`company`] — calm scenes after the talk: sitting together with maps, taking in the
//!   view, a song while mining.
//!
//! * [`challenge`] — Hornet's challenges after the talk: a [`race`] to where her needle
//!   lands, or a short [`duel`] she wins.
//!
//! The brain sees visitors as [`crate::ai::Visitor`] percepts (see [`Npc::percept`]).
mod actor;
pub mod company;
pub mod challenge;
pub mod dream;
pub mod duel;
pub mod forced;
pub mod lesson;
pub mod race;
pub mod spar;
pub mod talk;
mod visits;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod challenge_tests;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::anim::{ClipId, ClipLibrary};
use crate::ai::rng::Rng;
use crate::assets::{SpriteBank, read_json};

pub use actor::{Npc, NpcEnv, NpcEvent, NpcPhase, NpcProp, Placement, once_secs};
pub use challenge::{ChallengeClips, ChallengeDef, ChallengeKind, ChallengePlan};
pub use dream::DreamRules;
pub use lesson::{DemoStep, DrillDef, Lesson, LessonDef, PlannedDrill};
pub use spar::{SparClips, SparDef};
pub use company::{CompanyClips, CompanyDef, CompanyNeeds};
pub use talk::{Line, TalkContext};
pub use visits::{VisitConditions, VisitConfig, VisitScheduler};

/// How a visitor shows up.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Arrival {
    /// Walks in from the edge of the knight's monitor along the bottom of the screen.
    Walk,
    /// Appears sitting on the edge of a window top (legs dangling), fading in.
    Perch,
    /// Steps out of a dream portal somewhere on the desktop.
    #[default]
    Portal,
    /// Falls out of a dream portal (fall → land → stand clips).
    Drop,
}

/// How a visitor goes away.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Leave {
    /// Walks off the nearest screen edge.
    Walk,
    #[default]
    Fade,
    /// A dream portal opens and takes them.
    Portal,
}

/// Which way the art looks.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Faces {
    /// Side view drawn facing left: mirrored to face the knight.
    #[default]
    Left,
    /// Side view drawn facing right: mirrored to look left.
    Right,
    /// Front view (or with its own left/right talk clips): never mirrored.
    Front,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipNames {
    pub idle: String,
    /// Talking while the knight stands on the left (or the only talk clip).
    pub talk: String,
    #[serde(default)]
    pub talk_right: Option<String>,
    #[serde(default)]
    pub talk_end: Option<String>,
    #[serde(default)]
    pub talk_end_right: Option<String>,
    #[serde(default)]
    pub walk: Option<String>,
    /// Drop arrivals: looping fall clip, then the landing clips in order.
    #[serde(default)]
    pub fall: Option<String>,
    #[serde(default)]
    pub land: Vec<String>,
    /// Played once as they start to leave (a bow...), before fading / walking off.
    #[serde(default)]
    pub leave: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcDef {
    /// Id (lowercase), e.g. "cornifer".
    pub name: String,
    /// Shown in logs and the panel.
    pub display: String,
    /// Animation library file stem in `data/hk/animations` (spaces become `_`).
    pub library: String,
    #[serde(default = "one")]
    pub weight: f32,
    #[serde(default)]
    pub faces: Faces,
    pub clips: ClipNames,
    #[serde(default)]
    pub arrive: Arrival,
    #[serde(default)]
    pub leave: Leave,
    /// Seconds the visitor waits around (random in range) if nobody comes to talk.
    #[serde(default = "stay")]
    pub stay: [f32; 2],
    /// Walking speed (game units/s) for walk arrivals / departures.
    #[serde(default = "walk_speed")]
    pub walk_speed: f32,
    /// Sprite origin height above the feet (game units). Default: measured from the idle
    /// frame's bottom edge (feet at the bottom of the art).
    #[serde(default)]
    pub origin_above_feet: Option<f32>,
    /// The talk clips face the other way than the idle art (Hornet points to the right).
    #[serde(default)]
    pub talk_mirrored: bool,
    /// How thrilled the knight is to see this one (0..1).
    #[serde(default = "one")]
    pub excitement: f32,
    /// Where the knight stands to talk: this far from the feet point (game units).
    #[serde(default = "stand_off")]
    pub stand_off: f32,
    /// Lines said to nobody while waiting (humming...).
    #[serde(default)]
    pub idle_lines: Vec<String>,
    #[serde(default = "hum_every")]
    pub idle_every: [f32; 2],
    /// What the knight hears when he dream-nails them after their talk (original lines;
    /// same placeholders as conversations).
    #[serde(default)]
    pub dream_thoughts: Vec<String>,
    /// Knight act when he arrives next to them, before the first line.
    #[serde(default)]
    pub greet_react: Option<String>,
    /// Conversations: one is picked per visit. Lines may use `{title}`, `{app}`, `{daypart}`.
    pub conversations: Vec<Vec<Line>>,
    /// Named clip sequences the visitor can perform (demonstrations during a lesson).
    #[serde(default)]
    pub demos: BTreeMap<String, Vec<DemoStep>>,
    /// A teacher: after the conversation, a short training session (see [`lesson`]).
    #[serde(default)]
    pub lesson: Option<LessonDef>,
    /// Only visits within this many seconds after the knight fought (the cursor, the
    /// Shade, spells and nail arts). Invitations by name ignore it.
    #[serde(default)]
    pub after_fight: Option<f32>,
    /// More clip libraries merged into this visitor's, their clip names prefixed (e.g.
    /// Zote's fight clips as "Fight Idle", "Fight Antic"...).
    #[serde(default)]
    pub extra_libraries: Vec<ExtraLibrary>,
    /// A fighter: after the conversation he may challenge the knight to a spar (see [`spar`]).
    #[serde(default)]
    pub spar: Option<SparDef>,
    /// A calm scene that may follow the conversation (see [`company`]).
    #[serde(default)]
    pub company: Option<CompanyDef>,
    /// Hornet: after the conversation she may challenge the knight to a race or a duel (see
    /// [`challenge`]).
    #[serde(default)]
    pub challenge: Option<ChallengeDef>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraLibrary {
    pub library: String,
    pub prefix: String,
    /// Only these clips (unprefixed names) of the library; all when empty.
    #[serde(default)]
    pub clips: Vec<String>,
}

fn one() -> f32 {
    1.0
}
fn stay() -> [f32; 2] {
    [90.0, 180.0]
}
fn walk_speed() -> f32 {
    6.0
}
fn stand_off() -> f32 {
    crate::ai::STAND_OFF_UNITS
}
fn hum_every() -> [f32; 2] {
    [10.0, 20.0]
}

#[derive(Debug, Deserialize)]
struct NpcsFile {
    #[serde(default)]
    visits: VisitConfig,
    npcs: Vec<NpcDef>,
    #[serde(default)]
    dream: DreamRules,
    #[serde(default)]
    portal_style: crate::portal::PortalStyle,
}

/// Clip ids of one NPC, resolved in its library.
#[derive(Clone, Debug)]
pub struct NpcClips {
    pub idle: ClipId,
    pub talk: ClipId,
    pub talk_right: Option<ClipId>,
    pub talk_end: Option<ClipId>,
    pub talk_end_right: Option<ClipId>,
    pub walk: Option<ClipId>,
    pub fall: Option<ClipId>,
    pub land: Vec<ClipId>,
    pub leave: Option<ClipId>,
    /// `demos` resolved: (clip, seconds; `None` = play it once).
    pub demos: BTreeMap<String, Vec<(ClipId, Option<f32>)>>,
    /// The lesson stance (idle when unset).
    pub stance: Option<ClipId>,
    pub spar: Option<SparClips>,
    pub company: Option<CompanyClips>,
    pub challenge: Option<ChallengeClips>,
}

/// Every visitor definition with its clip library.
pub struct NpcSet {
    pub defs: Vec<NpcDef>,
    pub clips: Vec<NpcClips>,
    /// Per def: index into `libs`.
    pub lib_of: Vec<usize>,
    pub libs: Vec<ClipLibrary>,
    pub visits: VisitConfig,
    pub dream: DreamRules,
    /// How portal arrivals / departures look (dust by default).
    pub portal_style: crate::portal::PortalStyle,
}

impl NpcSet {
    /// Load `npcs.json`, reading each library from `<anim_dir>/<Library>.json`. Unknown
    /// clips, empty conversations or bad ranges are errors.
    pub fn load(npcs_json: &Path, anim_dir: &Path, bank: &SpriteBank) -> Result<NpcSet> {
        let file: NpcsFile = read_json(npcs_json)?;
        let mut libs: Vec<ClipLibrary> = Vec::new();
        let mut lib_idx: HashMap<String, usize> = HashMap::new();
        let mut lib_of = Vec::new();
        let mut clips = Vec::new();
        let load = |name: &str, who: &str| {
            let path = anim_dir.join(format!("{}.json", name.replace(' ', "_")));
            ClipLibrary::load(&path, bank).with_context(|| format!("npc '{who}'"))
        };
        for d in &file.npcs {
            // A library merged with extras is its own entry.
            let key = d.extra_libraries.iter().fold(d.library.clone(), |k, e| format!("{k}+{}:{}", e.prefix, e.library));
            let li = match lib_idx.get(&key) {
                Some(&i) => i,
                None => {
                    let mut lib = load(&d.library, &d.name)?;
                    for e in &d.extra_libraries {
                        lib.merge_only(load(&e.library, &d.name)?, &e.prefix, &e.clips);
                    }
                    libs.push(lib);
                    lib_idx.insert(key, libs.len() - 1);
                    libs.len() - 1
                }
            };
            clips.push(resolve(d, &libs[li])?);
            lib_of.push(li);
            validate(d)?;
        }
        if file.npcs.is_empty() {
            bail!("{}: no npcs", npcs_json.display());
        }
        Ok(NpcSet { defs: file.npcs, clips, lib_of, libs, visits: file.visits, dream: file.dream, portal_style: file.portal_style })
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.defs.iter().position(|d| d.name.eq_ignore_ascii_case(name))
    }

    pub fn lib(&self, def: usize) -> &ClipLibrary {
        &self.libs[self.lib_of[def]]
    }
}

fn resolve(d: &NpcDef, lib: &ClipLibrary) -> Result<NpcClips> {
    let get = |n: &str| lib.id(n).with_context(|| format!("npc '{}': no clip '{n}' in {}", d.name, d.library));
    let opt = |n: &Option<String>| n.as_deref().map(get).transpose();
    let c = &d.clips;
    Ok(NpcClips {
        idle: get(&c.idle)?,
        talk: get(&c.talk)?,
        talk_right: opt(&c.talk_right)?,
        talk_end: opt(&c.talk_end)?,
        talk_end_right: opt(&c.talk_end_right)?,
        walk: opt(&c.walk)?,
        fall: opt(&c.fall)?,
        land: c.land.iter().map(|n| get(n)).collect::<Result<_>>()?,
        leave: opt(&c.leave)?,
        demos: d
            .demos
            .iter()
            .map(|(k, steps)| Ok((k.clone(), steps.iter().map(|s| Ok((get(s.clip())?, s.secs()))).collect::<Result<Vec<_>>>()?)))
            .collect::<Result<_>>()?,
        stance: d.lesson.as_ref().and_then(|l| l.stance.as_deref()).map(get).transpose()?,
        spar: d.spar.as_ref().map(|s| s.resolve(lib, &d.name)).transpose()?,
        company: d.company.as_ref().map(|s| s.resolve(lib, get(&c.idle)?, &d.name)).transpose()?,
        challenge: d.challenge.as_ref().map(|c| c.resolve(lib, &d.name)).transpose()?,
    })
}

fn validate(d: &NpcDef) -> Result<()> {
    let range_ok = |r: [f32; 2]| r[0] >= 0.0 && r[1] >= r[0];
    if !range_ok(d.stay) || !range_ok(d.idle_every) {
        bail!("npc '{}': bad range", d.name);
    }
    if d.conversations.is_empty() || d.conversations.iter().any(|c| c.is_empty()) {
        bail!("npc '{}': needs at least one non-empty conversation", d.name);
    }
    if (d.arrive == Arrival::Walk || d.leave == Leave::Walk) && d.clips.walk.is_none() {
        bail!("npc '{}': walk arrival/leave needs a walk clip", d.name);
    }
    if d.arrive == Arrival::Drop && d.clips.fall.is_none() {
        bail!("npc '{}': drop arrival needs a fall clip", d.name);
    }
    for line in d.conversations.iter().flatten() {
        talk::check_template(line.text()).with_context(|| format!("npc '{}'", d.name))?;
    }
    for t in &d.dream_thoughts {
        talk::check_template(t).with_context(|| format!("npc '{}' dream thought", d.name))?;
    }
    if let Some(l) = &d.lesson {
        l.validate(&|n| d.demos.contains_key(n)).with_context(|| format!("npc '{}'", d.name))?;
    }
    if d.demos.values().flatten().any(|s| s.secs().is_some_and(|t| t.is_nan() || t <= 0.0)) {
        bail!("npc '{}': demo step seconds must be positive", d.name);
    }
    if d.after_fight.is_some_and(|s| s.is_nan() || s <= 0.0) {
        bail!("npc '{}': after_fight must be positive", d.name);
    }
    if let Some(s) = &d.company {
        s.validate().with_context(|| format!("npc '{}'", d.name))?;
    }
    if let Some(s) = &d.spar {
        s.validate().with_context(|| format!("npc '{}'", d.name))?;
    }
    if let Some(c) = &d.challenge {
        c.validate().with_context(|| format!("npc '{}'", d.name))?;
    }
    Ok(())
}

/// Where a visitor of this kind should appear. `footings` are standable surfaces (nav
/// surfaces when available), `reachable` route-checks a point for the knight.
pub fn plan_placement(
    arrive: Arrival,
    footings: &[crate::portal::Footing],
    knight: crate::geom::Vec2,
    monitors: &[crate::desktop::Monitor],
    rng: &mut Rng,
    reachable: &mut dyn FnMut(crate::geom::Vec2) -> bool,
) -> Option<Placement> {
    use crate::geom::Vec2;
    use crate::portal::{SpotOptions, pick_spot};
    let mut random = || rng.f32();
    match arrive {
        Arrival::Portal | Arrival::Drop => {
            let s = pick_spot(footings, knight, monitors, &SpotOptions::default(), &mut random, reachable)?;
            Some(Placement { feet: s.at, owner: s.owner, walk_to: None })
        }
        Arrival::Perch => {
            let tops: Vec<_> = footings.iter().copied().filter(|f| f.owner.is_some()).collect();
            let opts = SpotOptions { min_distance: 220.0, ..SpotOptions::default() };
            let s = pick_spot(&tops, knight, monitors, &opts, &mut random, reachable)?;
            Some(Placement { feet: s.at, owner: s.owner, walk_to: None })
        }
        Arrival::Walk => {
            let m = monitors.iter().find(|m| m.work.contains(knight.x as i32, (knight.y - 1.0) as i32)).or_else(|| monitors.iter().find(|m| m.primary))?;
            let w = m.work.to_f();
            // The ground: the lowest footing spanning most of the monitor's bottom.
            let ground = footings.iter().filter(|f| (f.y - w.bottom).abs() < 4.0 && f.x1 - f.x0 > w.width() * 0.5).min_by(|a, b| b.y.total_cmp(&a.y))?;
            let from_left = random() < 0.5;
            let x0 = ground.x0.max(w.left) + 1.0;
            let x1 = ground.x1.min(w.right) - 1.0;
            let start = if from_left { x0 } else { x1 };
            let off = 250.0 + 200.0 * random();
            let target = if from_left { knight.x - off } else { knight.x + off };
            let to = target.clamp(x0 + 120.0, (x1 - 120.0).max(x0 + 121.0));
            let to = if from_left { to.max(x0 + 120.0) } else { to.min(x1 - 120.0) };
            reachable(Vec2::new(to, ground.y)).then_some(Placement { feet: Vec2::new(start, ground.y), owner: ground.owner, walk_to: Some(to) })
        }
    }
}
