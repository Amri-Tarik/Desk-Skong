//! Enemies: small Hallownest creatures that wander in now and then (crawl or walk in from
//! a monitor edge, drop onto a ledge, fly in from the side), roam their ledge or hover
//! around, notice the knight and fight back mildly, flinch when struck and die with their
//! death clip (bursting a little Geo). They give the knight a reason to go somewhere and
//! fight; the brain sees them as [`crate::ai::Foe`]s.
//!
//! * [`EnemySet`] — definitions from `data/enemies.json`, validated against their exported
//!   clip libraries (`data/hk/animations/<Library>.json`); enemy art is always vanilla.
//! * [`Enemy`] — one live enemy: footing (rides the window it walks on), the arrive → roam
//!   ⇄ notice / chase / attack → die → corpse lifecycle, hits and contact damage.
//! * [`plan_arrival`] — where (and how) the next one shows up: settled ledges of a monitor
//!   without a fullscreen game, reachable for the knight.
//! * Scheduling reuses [`crate::npc::VisitScheduler`] (presence time, never during
//!   fullscreen apps or fast typing).
//! * [`GauntletDirector`] — the gauntlet: waves of enemies, a reward (see [`gauntlet`]).
//! * [`grimm`] — the secret boss: a scarlet flame, the Troupe Master, his flames.
pub mod gauntlet;
pub mod grimm;
mod sim;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::ai::rng::Rng;
use crate::anim::{ClipId, ClipLibrary};
use crate::assets::{SpriteBank, read_json};
use crate::desktop::{Monitor, WindowId};
use crate::geom::Vec2;
use crate::npc::VisitConfig;
use crate::portal::Footing;

pub use gauntlet::{AbortReason, GauntletConfig, GauntletCue, GauntletDirector, GauntletEvent, GauntletStage, GauntletStep, GauntletTrigger, GauntletWorld};
pub use sim::{Enemy, EnemyEnv, EnemyPhase, EnemyTick};

/// How an enemy gets around.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Moves {
    /// Walks / crawls along ledges (falls when its ledge goes away).
    #[default]
    Ground,
    /// Flies (hovers around a spot; falls only when it dies).
    Fly,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnemyClipNames {
    pub idle: String,
    /// Walking / crawling / flying.
    #[serde(rename = "move")]
    pub moving: String,
    /// Turning around (played once, then the art is mirrored).
    #[serde(default)]
    pub turn: Option<String>,
    /// Noticing the knight (once).
    #[serde(default)]
    pub startle: Option<String>,
    /// Going for the knight (default: `move`).
    #[serde(default)]
    pub chase: Option<String>,
    /// Attack wind-up, the attack itself (moves forward at `lunge`), and the pause after.
    #[serde(default)]
    pub antic: Option<String>,
    #[serde(default)]
    pub attack: Option<String>,
    #[serde(default)]
    pub recover: Option<String>,
    /// Falling (dropping in, or its ledge vanished).
    #[serde(default)]
    pub fall: Option<String>,
    /// Struck dead, flying back / falling.
    pub death_air: String,
    /// Landed dead (the last frame is the corpse).
    pub death_land: String,
}

/// One kind of enemy (see the `$comment` in `data/enemies.json`).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnemyDef {
    /// Id (lowercase), e.g. "crawlid".
    pub name: String,
    pub display: String,
    /// Animation library file stem in `data/hk/animations` (spaces become `_`).
    pub library: String,
    #[serde(default = "one")]
    pub weight: f32,
    #[serde(default)]
    pub moves: Moves,
    pub clips: EnemyClipNames,
    /// Hits it takes (spells count more).
    pub hp: u32,
    /// Roaming speed (game units/s).
    pub speed: f32,
    /// Speed going for the knight (game units/s; default 1.6 × `speed`).
    #[serde(default)]
    pub chase_speed: Option<f32>,
    /// Notices the knight within this many game units (0 = never: a harmless crawler).
    #[serde(default)]
    pub aggro: f32,
    /// Starts its attack this close (game units, horizontal).
    #[serde(default = "reach")]
    pub reach: f32,
    /// Forward speed during the attack (game units/s).
    #[serde(default)]
    pub lunge: f32,
    /// Body size [w, h] in game units (hits, contact, the knight's stand-off).
    pub size: [f32; 2],
    /// Geo dropped when killed: [min, max].
    pub geo: [u32; 2],
    /// Seconds it roams (random in range) before wandering off, unless fighting.
    #[serde(default = "stay")]
    pub stay: [f32; 2],
    /// Flyers hover this high above the ledge they arrived over (game units).
    #[serde(default = "hover")]
    pub hover: f32,
    /// Sprite origin above the feet (game units). Default: measured from the idle frame.
    #[serde(default)]
    pub origin_above_feet: Option<f32>,
}

fn one() -> f32 {
    1.0
}
fn reach() -> f32 {
    2.5
}
fn stay() -> [f32; 2] {
    [60.0, 120.0]
}
fn hover() -> f32 {
    2.2
}

impl EnemyDef {
    pub fn chase_speed(&self) -> f32 {
        self.chase_speed.unwrap_or(self.speed * 1.6)
    }
}

/// When enemies come, and how many at once.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EnemyRules {
    /// At most this many alive at once (a world event may add one more).
    pub max_alive: usize,
    /// A visit brings this many (random in range, up to `max_alive`).
    pub group: [usize; 2],
    /// Seconds between the members of a group arriving.
    pub stagger: f32,
}

impl Default for EnemyRules {
    fn default() -> Self {
        Self { max_alive: 2, group: [1, 2], stagger: 4.0 }
    }
}

#[derive(Debug, Deserialize)]
struct EnemiesFile {
    #[serde(default)]
    schedule: VisitConfig,
    #[serde(default)]
    rules: EnemyRules,
    #[serde(default)]
    gauntlet: GauntletConfig,
    enemies: Vec<EnemyDef>,
}

/// Clip ids of one enemy, resolved in its library.
#[derive(Clone, Debug)]
pub struct EnemyClips {
    pub idle: ClipId,
    pub moving: ClipId,
    pub turn: Option<ClipId>,
    pub startle: Option<ClipId>,
    pub chase: ClipId,
    pub antic: Option<ClipId>,
    pub attack: Option<ClipId>,
    pub recover: Option<ClipId>,
    pub fall: Option<ClipId>,
    pub death_air: ClipId,
    pub death_land: ClipId,
}

/// Every enemy definition with its clip library.
pub struct EnemySet {
    pub defs: Vec<EnemyDef>,
    pub clips: Vec<EnemyClips>,
    /// Per def: index into `libs`.
    pub lib_of: Vec<usize>,
    pub libs: Vec<ClipLibrary>,
    pub schedule: VisitConfig,
    pub rules: EnemyRules,
    /// The gauntlet (validated against the roster).
    pub gauntlet: GauntletConfig,
}

impl EnemySet {
    /// Load `enemies.json`, reading each library from `<anim_dir>/<Library>.json`. Unknown
    /// clips or bad numbers are errors.
    pub fn load(json: &Path, anim_dir: &Path, bank: &SpriteBank) -> Result<EnemySet> {
        let file: EnemiesFile = read_json(json)?;
        let mut libs: Vec<ClipLibrary> = Vec::new();
        let mut lib_idx: HashMap<String, usize> = HashMap::new();
        let mut lib_of = Vec::new();
        let mut clips = Vec::new();
        for d in &file.enemies {
            validate(d)?;
            let li = match lib_idx.get(&d.library) {
                Some(&i) => i,
                None => {
                    let path = anim_dir.join(format!("{}.json", d.library.replace(' ', "_")));
                    libs.push(ClipLibrary::load(&path, bank).with_context(|| format!("enemy '{}'", d.name))?);
                    lib_idx.insert(d.library.clone(), libs.len() - 1);
                    libs.len() - 1
                }
            };
            clips.push(resolve(d, &libs[li])?);
            lib_of.push(li);
        }
        if file.enemies.is_empty() {
            bail!("{}: no enemies", json.display());
        }
        if file.rules.max_alive == 0 || file.rules.group[0] == 0 || file.rules.group[1] < file.rules.group[0] {
            bail!("{}: bad rules", json.display());
        }
        let set = EnemySet { defs: file.enemies, clips, lib_of, libs, schedule: file.schedule, rules: file.rules, gauntlet: file.gauntlet };
        set.gauntlet.resolve(&set).with_context(|| format!("{}", json.display()))?;
        Ok(set)
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.defs.iter().position(|d| d.name.eq_ignore_ascii_case(name))
    }

    pub fn lib(&self, def: usize) -> &ClipLibrary {
        &self.libs[self.lib_of[def]]
    }
}

fn resolve(d: &EnemyDef, lib: &ClipLibrary) -> Result<EnemyClips> {
    let get = |n: &str| lib.id(n).with_context(|| format!("enemy '{}': no clip '{n}' in {}", d.name, d.library));
    let opt = |n: &Option<String>| n.as_deref().map(get).transpose();
    let c = &d.clips;
    let moving = get(&c.moving)?;
    Ok(EnemyClips {
        idle: get(&c.idle)?,
        moving,
        turn: opt(&c.turn)?,
        startle: opt(&c.startle)?,
        chase: opt(&c.chase)?.unwrap_or(moving),
        antic: opt(&c.antic)?,
        attack: opt(&c.attack)?,
        recover: opt(&c.recover)?,
        fall: opt(&c.fall)?,
        death_air: get(&c.death_air)?,
        death_land: get(&c.death_land)?,
    })
}

fn validate(d: &EnemyDef) -> Result<()> {
    let range_ok = |r: [f32; 2]| r[0] >= 0.0 && r[1] >= r[0];
    let pos = |x: f32| x.is_finite() && x > 0.0;
    let non_neg = |x: f32| x.is_finite() && x >= 0.0;
    if d.hp == 0 {
        bail!("enemy '{}': hp must be at least 1", d.name);
    }
    if !pos(d.speed) || !pos(d.size[0]) || !pos(d.size[1]) || !non_neg(d.weight) {
        bail!("enemy '{}': speed, size and weight must be positive", d.name);
    }
    if !range_ok(d.stay) || d.geo[1] < d.geo[0] || !non_neg(d.aggro) || !non_neg(d.reach) || !non_neg(d.lunge) {
        bail!("enemy '{}': bad range", d.name);
    }
    if d.clips.attack.is_some() != d.clips.antic.is_some() {
        bail!("enemy '{}': an attack needs both `antic` and `attack` clips", d.name);
    }
    Ok(())
}

/// How an enemy enters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntryKind {
    /// Walks / flies in from `from` (just inside a monitor edge) to the placement point.
    Edge { from: Vec2 },
    /// Falls from above onto the placement point.
    Drop,
}

/// Where an enemy shows up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyPlacement {
    /// Feet point it walks / flies to (flyers: the bottom of their art while hovering).
    pub at: Vec2,
    pub owner: Option<WindowId>,
    pub entry: EntryKind,
}

/// Pick where an enemy of `def` arrives. `footings` must already be settled and on
/// monitors without games; `monitors` are those monitors. Ground enemies prefer a ledge
/// that runs into the edge of the knight's monitor (they walk in from there), else they drop
/// onto a ledge near him; flyers come in from the nearer side edge to hover over a ledge.
/// `reachable` route-checks a feet point for the knight. Arrivals keep 4–14 units from the
/// knight (not on top of him, not miles away) and stay on his monitor when possible.
pub fn plan_arrival(
    def: &EnemyDef,
    footings: &[Footing],
    knight: Vec2,
    monitors: &[Monitor],
    ppu: f32,
    rng: &mut Rng,
    reachable: &mut dyn FnMut(Vec2) -> bool,
) -> Option<EnemyPlacement> {
    let mon = monitors.iter().find(|m| m.work.contains(knight.x as i32, (knight.y - 1.0) as i32)).or_else(|| monitors.iter().find(|m| m.primary)).or(monitors.first())?;
    let area = mon.work.to_f();
    let half = 0.5 * def.size[0] * ppu;
    let (near, far) = (4.0 * ppu, 14.0 * ppu);
    // Usable part of each footing on this monitor, wide enough to roam.
    let spans: Vec<(Footing, f32, f32)> = footings
        .iter()
        .filter(|f| f.y > area.top + (def.size[1] + def.hover) * ppu && f.y <= area.bottom + 1.0)
        .filter_map(|f| {
            let (x0, x1) = (f.x0.max(area.left) + half, f.x1.min(area.right) - half);
            (x1 - x0 >= 2.0 * def.size[0] * ppu).then_some((*f, x0, x1))
        })
        .collect();
    if spans.is_empty() {
        return None;
    }
    // Candidate points: a few per span, at a comfortable distance from the knight.
    let mut cands: Vec<(Footing, f32, f32, f32, Vec2)> = Vec::new();
    for &(f, x0, x1) in &spans {
        // Sample the part of the span within reach of the knight's distance band.
        let (a, b) = (x0.max(knight.x - far), x1.min(knight.x + far));
        if b < a {
            continue;
        }
        for _ in 0..6 {
            let x = a + rng.f32() * (b - a);
            let p = Vec2::new(x, f.y);
            let d = p.distance(knight);
            if (near..=far).contains(&d) {
                cands.push((f, x0, x1, d + 0.3 * far * rng.f32(), p));
            }
        }
    }
    cands.sort_by(|a, b| a.3.total_cmp(&b.3));
    let edge_tol = 4.0;
    for (f, x0, x1, _, p) in cands.into_iter().take(8) {
        if !reachable(p) {
            continue;
        }
        let entry = match def.moves {
            Moves::Fly => {
                // In from the nearer side of the monitor, at hovering height.
                let y = f.y - def.hover * ppu;
                let from_left = p.x - area.left < area.right - p.x;
                let x = if from_left { area.left + half } else { area.right - half };
                return Some(EnemyPlacement { at: Vec2::new(p.x, y), owner: None, entry: EntryKind::Edge { from: Vec2::new(x, y) } });
            }
            Moves::Ground => {
                let at_left = f.x0 <= area.left + edge_tol;
                let at_right = f.x1 >= area.right - edge_tol;
                match (at_left, at_right) {
                    (true, true) => EntryKind::Edge { from: Vec2::new(if p.x - x0 < x1 - p.x { x0 } else { x1 }, f.y) },
                    (true, false) => EntryKind::Edge { from: Vec2::new(x0, f.y) },
                    (false, true) => EntryKind::Edge { from: Vec2::new(x1, f.y) },
                    (false, false) => EntryKind::Drop,
                }
            }
        };
        return Some(EnemyPlacement { at: p, owner: f.owner, entry });
    }
    None
}
