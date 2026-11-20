//! Hidden achievements: things the user can get the knight to do, discovered by trying.
//!
//! * Definitions are data (`data/achievements.json`): id, title and description (shown only
//!   once unlocked), a vague hint category, and a `goal` for counting ones.
//! * The app turns what happens (movement events, the brain's intent, visitors, enemies,
//!   the stakes...) into a small [`AchievementEvent`] stream, in one place
//!   (`skong/src/app/achievements.rs`). Every criterion, including the stateful ones
//!   (counts, "never touched the floor", "every region"), lives in [`Tracker::on`].
//! * Progress persists as an [`AchievementSave`] (unlock times, counts, sets of things
//!   seen); in-the-moment state (a fight in progress, a throw in flight) does not.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Achievement ids the tracker knows (each wired to a criterion in [`Tracker::on`]).
pub mod ids {
    pub const HORNET_TALK: &str = "hornet_talk";
    pub const DREAM_WINDOW: &str = "dream_window";
    pub const DREAM_VISITOR: &str = "dream_visitor";
    pub const FIRST_SPELL: &str = "first_spell";
    pub const POGO: &str = "pogo";
    pub const VENGEFLY_AIRBORNE: &str = "vengefly_airborne";
    pub const FALL_AND_RECLAIM: &str = "fall_and_reclaim";
    pub const WINDOW_RIDE: &str = "window_ride";
    pub const CURSOR_DEFEAT: &str = "cursor_defeat";
    pub const CURSOR_VICTOR: &str = "cursor_victor";
    pub const WALL_CRYSTAL_DASH: &str = "wall_crystal_dash";
    pub const DREAM_GATE: &str = "dream_gate";
    pub const EVERY_REGION: &str = "every_region";
    pub const EVERY_BENCH: &str = "every_bench";
    pub const EVERY_VISITOR: &str = "every_visitor";
    pub const SLY_LESSON: &str = "sly_lesson";
    pub const ZOTE_SPAR: &str = "zote_spar";
    pub const GAUNTLET: &str = "gauntlet";
    pub const UNN_FORM: &str = "unn_form";
    pub const HIGHLIGHTED_TEXT: &str = "highlighted_text";
    pub const SHATTERED_WINDOW: &str = "shattered_window";
    pub const GEO_HOARD: &str = "geo_hoard";
    pub const NAP_POKE: &str = "nap_poke";
    pub const THROW: &str = "throw";
    pub const MIDAIR_CATCH: &str = "midair_catch";
    pub const STEP_BEHIND: &str = "step_behind";
    pub const WALL_JUMP_CHAIN: &str = "wall_jump_chain";
    pub const SUPER_HARD_LANDING: &str = "super_hard_landing";
    pub const EVERY_ENEMY: &str = "every_enemy";
    pub const TROUPE_MASTER: &str = "troupe_master";
    pub const TROUPE_ENCORE: &str = "troupe_encore";
    pub const CORNIFER_MAPS: &str = "cornifer_maps";
    pub const QUIRREL_VIEW: &str = "quirrel_view";
    pub const MYLA_SONG: &str = "myla_song";
    pub const HORNET_RACE: &str = "hornet_race";
    pub const HORNET_DUEL: &str = "hornet_duel";
    pub const GRIMM_CAMEO: &str = "grimm_cameo";

    pub const ALL: &[&str] = &[
        HORNET_TALK, DREAM_WINDOW, DREAM_VISITOR, FIRST_SPELL, POGO, VENGEFLY_AIRBORNE, FALL_AND_RECLAIM, WINDOW_RIDE, CURSOR_DEFEAT, CURSOR_VICTOR, WALL_CRYSTAL_DASH, DREAM_GATE, EVERY_REGION, EVERY_BENCH, EVERY_VISITOR, SLY_LESSON, ZOTE_SPAR, GAUNTLET,
        UNN_FORM, HIGHLIGHTED_TEXT, SHATTERED_WINDOW, GEO_HOARD, NAP_POKE, THROW, MIDAIR_CATCH, STEP_BEHIND, WALL_JUMP_CHAIN, SUPER_HARD_LANDING, EVERY_ENEMY, TROUPE_MASTER, TROUPE_ENCORE,
        CORNIFER_MAPS, QUIRREL_VIEW, MYLA_SONG, HORNET_RACE, HORNET_DUEL, GRIMM_CAMEO,
    ];
}
use ids::*;

/// One achievement as written in `data/achievements.json`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AchievementDef {
    pub id: String,
    /// Shown once unlocked.
    pub title: String,
    /// Shown once unlocked.
    pub description: String,
    /// A vague category shown on the locked entry ("a visitor", "the cursor"), never how.
    #[serde(default)]
    pub hint: Option<String>,
    /// Count needed by counting achievements (default 1).
    #[serde(default)]
    pub goal: Option<u32>,
}

#[derive(Deserialize)]
struct DefsFile {
    achievements: Vec<AchievementDef>,
}

/// Parse the definitions (`data/achievements.json`'s content).
pub fn parse_defs(json: &str) -> Result<Vec<AchievementDef>> {
    let f: DefsFile = serde_json::from_str(json).context("achievements: bad json")?;
    let mut seen = BTreeSet::new();
    for d in &f.achievements {
        anyhow::ensure!(seen.insert(d.id.as_str()), "achievements: duplicate id '{}'", d.id);
        if !ALL.contains(&d.id.as_str()) {
            log::warn!("achievements: '{}' has no criterion; it can never unlock", d.id);
        }
    }
    Ok(f.achievements)
}

pub fn load_defs(path: &Path) -> Result<Vec<AchievementDef>> {
    parse_defs(&std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?)
}

/// Persistent progress (`%APPDATA%\Desk-Skong\achievements.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AchievementSave {
    /// Unlocked ids and when (unix seconds).
    pub unlocked: BTreeMap<String, u64>,
    /// Running counts of counting achievements (cursor wins, Geo picked up).
    pub counts: BTreeMap<String, u32>,
    /// What each "every ..." achievement has seen so far (visitor names, regions...).
    pub seen: BTreeMap<String, BTreeSet<String>>,
    /// The Shade waiting now came from a fatal fall (it outlives a restart, so this does too).
    pub shade_from_fall: bool,
}

/// Read a save; a missing file is a fresh start. So is a malformed one, but it is kept
/// beside it first ([`crate::persist::backup_path`]): the next save would overwrite the
/// progress in it.
pub fn read_save(path: &Path) -> AchievementSave {
    crate::persist::load_or_fresh(path, |s| serde_json::from_str(s), AchievementSave::default)
}

/// Write a save atomically ([`crate::persist::write_atomic`]: a crash or a full disk
/// mid-write leaves the old save whole, never a truncated one).
pub fn write_save(path: &Path, save: &AchievementSave) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(save).map_err(std::io::Error::other)?;
    crate::persist::write_atomic(path, json.as_bytes())
}

/// How many there are of the things "every ..." achievements ask for (set by the app from
/// what it loaded; 0 = unknown, that achievement can't unlock).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub visitors: usize,
    pub regions: usize,
    pub bench_styles: usize,
    pub enemies: usize,
}

/// What happened, as far as achievements care. `monitor` = index into the desktop's
/// monitors (only compared with other indices).
#[derive(Clone, Debug, PartialEq)]
pub enum AchievementEvent {
    /// A conversation with visitor `who` (definition name, e.g. "hornet") started.
    VisitorTalked { who: String },
    /// A dream-nailed visitor's thought showed.
    VisitorDreamed { who: String },
    /// A dream-nailed window's thought showed.
    WindowDreamed,
    /// A teaching visitor's lesson ended with the final bow (not cut short).
    LessonFinished { who: String },
    /// A sparring visitor went down.
    SparWon { who: String },
    /// A visitor's calm scene (maps, the view, a song) ran until they parted.
    CompanyKept { who: String },
    /// The brain cast a spell or a nail art.
    SpellCast,
    /// A down-strike in mid-air connected and bounced him up.
    Pogo,
    /// His blow landed on enemy `foe`; `on_floor`: he stood on the desktop floor (monitor
    /// ground) as he struck.
    EnemyStruck { foe: u32, on_floor: bool },
    /// Enemy `foe` of kind `kind` (definition name, e.g. "vengefly") was beaten.
    EnemyKilled { foe: u32, kind: String },
    /// He landed (`floor`: on the desktop floor, the monitor ground).
    Landed { monitor: usize, floor: bool },
    /// He died; `shade`: a Shade was left; `fall`: a fatal fall.
    Died { shade: bool, fall: bool },
    /// The Shade was beaten and its Geo won back.
    ShadeReclaimed,
    /// The window under him moved.
    WindowRode,
    CursorBattleLost,
    CursorBattleWon,
    /// A crystal dash launched (`from_wall`: charged on a wall).
    SuperdashLaunched { monitor: usize, from_wall: bool },
    /// Mid crystal dash, over `monitor`.
    SuperdashFlying { monitor: usize },
    SuperdashOver,
    /// He warped through a Dream Gate.
    DreamGateWarp,
    /// A Geo rock of region `region` broke.
    DepositMined { region: usize },
    /// He sat on a found bench of style `style`.
    BenchRested { style: usize },
    /// A gauntlet was cleared.
    GauntletCleared,
    /// He melted into the slug form.
    SlugForm,
    /// He went to look at highlighted text.
    TextHighlighted,
    /// A fullscreen game made him leave its monitor (or hide).
    HidFromGame,
    /// A closed / minimized window shattered into Geo.
    WindowShattered,
    /// He picked up this much Geo.
    GeoCollected(u32),
    /// The user clicked him while he napped.
    PokedWhileNapping,
    /// The user threw him (a real throw, not a drop) from over `monitor`.
    Thrown { monitor: usize },
    /// The user picked him up.
    Grabbed,
    WallJumped,
    SuperHardLanding,
    /// He stepped behind a drawn line.
    SteppedBehind,
    /// He beat the Troupe Master.
    TroupeMasterBeaten,
    /// The Troupe Master beat him.
    FellToTroupeMaster,
    /// He beat Hornet to her needle.
    HornetRaceWon,
    /// Hornet's duel was seen through to her final word.
    HornetDuelDone,
    /// The Troupe Master's friendly visit was seen through to his farewell.
    GrimmCameoSeen,
}

/// Unlock state and every criterion.
#[derive(Clone, Debug)]
pub struct Tracker {
    defs: Vec<AchievementDef>,
    save: AchievementSave,
    totals: Totals,
    /// Enemies he has struck, and whether every blow so far was off the floor (and he
    /// hasn't touched it since the first).
    fights: BTreeMap<u32, bool>,
    /// Wall jumps since he last landed.
    wall_jumps: u32,
    /// Monitor he was thrown from (while he flies).
    thrown_from: Option<usize>,
    dirty: bool,
    /// The sandbox is on: every event is ignored (nothing unlocks, no progress counts).
    paused: bool,
}

impl Tracker {
    pub fn new(defs: Vec<AchievementDef>, save: AchievementSave) -> Self {
        Self { defs, save, totals: Totals::default(), fights: BTreeMap::new(), wall_jumps: 0, thrown_from: None, dirty: false, paused: false }
    }

    /// Pause (the sandbox) or resume counting. Paused, [`Tracker::on`] ignores every event:
    /// nothing unlocks and no counter, "seen" set or streak moves.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            // Half-done streaks (an off-the-floor fight, wall jumps, a throw in flight)
            // started before the pause mustn't finish with help from the sandbox.
            self.fights.clear();
            self.wall_jumps = 0;
            self.thrown_from = None;
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_totals(&mut self, totals: Totals) {
        self.totals = totals;
    }

    pub fn defs(&self) -> &[AchievementDef] {
        &self.defs
    }

    pub fn save(&self) -> &AchievementSave {
        &self.save
    }

    /// When `id` was unlocked (unix seconds).
    pub fn unlocked_at(&self, id: &str) -> Option<u64> {
        self.save.unlocked.get(id).copied()
    }

    /// Unlocked achievements that are still defined.
    pub fn unlocked_count(&self) -> usize {
        self.defs.iter().filter(|d| self.save.unlocked.contains_key(&d.id)).count()
    }

    pub fn total(&self) -> usize {
        self.defs.len()
    }

    /// Progress changed since the last call (time to save).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Forget everything (debug).
    pub fn reset(&mut self) {
        self.save = AchievementSave::default();
        self.fights.clear();
        self.wall_jumps = 0;
        self.thrown_from = None;
        self.dirty = true;
    }

    /// Feed one event; returns the definitions it unlocked (usually none).
    pub fn on(&mut self, ev: &AchievementEvent, now: u64) -> Vec<&AchievementDef> {
        use AchievementEvent as E;
        if self.paused {
            return Vec::new();
        }
        let mut new: Vec<&'static str> = Vec::new();
        let mut unlock = |id: &'static str| new.push(id);
        match ev {
            E::VisitorTalked { who } => {
                if who == "hornet" {
                    unlock(HORNET_TALK);
                }
                if self.see(EVERY_VISITOR, who, self.totals.visitors) {
                    unlock(EVERY_VISITOR);
                }
            }
            E::VisitorDreamed { .. } => unlock(DREAM_VISITOR),
            E::WindowDreamed => unlock(DREAM_WINDOW),
            E::LessonFinished { .. } => unlock(SLY_LESSON),
            E::SparWon { who } if who == "zote" => unlock(ZOTE_SPAR),
            E::SparWon { .. } => {}
            E::CompanyKept { who } => match who.as_str() {
                "cornifer" => unlock(CORNIFER_MAPS),
                "quirrel" => unlock(QUIRREL_VIEW),
                "myla" => unlock(MYLA_SONG),
                _ => {}
            },
            E::SpellCast => unlock(FIRST_SPELL),
            E::Pogo => unlock(POGO),
            E::EnemyStruck { foe, on_floor } => {
                let clean = self.fights.entry(*foe).or_insert(true);
                *clean &= !on_floor;
            }
            E::EnemyKilled { foe, kind } => {
                let clean = self.fights.remove(foe).unwrap_or(false);
                if kind == "vengefly" && clean {
                    unlock(VENGEFLY_AIRBORNE);
                }
                if self.see(EVERY_ENEMY, kind, self.totals.enemies) {
                    unlock(EVERY_ENEMY);
                }
            }
            E::Landed { floor, .. } => {
                if *floor {
                    self.fights.values_mut().for_each(|c| *c = false);
                }
                self.wall_jumps = 0;
                self.thrown_from = None;
            }
            E::Died { shade, fall } => {
                self.fights.clear();
                self.thrown_from = None;
                if *shade {
                    self.save.shade_from_fall = *fall;
                    self.dirty = true;
                }
            }
            E::ShadeReclaimed => {
                if std::mem::take(&mut self.save.shade_from_fall) {
                    unlock(FALL_AND_RECLAIM);
                }
                self.dirty = true;
            }
            E::WindowRode => unlock(WINDOW_RIDE),
            E::CursorBattleLost => unlock(CURSOR_DEFEAT),
            E::CursorBattleWon => {
                if self.count(CURSOR_VICTOR, 1) {
                    unlock(CURSOR_VICTOR);
                }
            }
            E::SuperdashLaunched { from_wall, .. } => {
                if *from_wall {
                    unlock(WALL_CRYSTAL_DASH);
                }
            }
            // (Achievements never need a second monitor: everyone can reach 100 %.)
            E::SuperdashFlying { .. } | E::SuperdashOver => {}
            E::DreamGateWarp => unlock(DREAM_GATE),
            E::DepositMined { region } => {
                if self.see(EVERY_REGION, &region.to_string(), self.totals.regions) {
                    unlock(EVERY_REGION);
                }
            }
            E::BenchRested { style } => {
                if self.see(EVERY_BENCH, &style.to_string(), self.totals.bench_styles) {
                    unlock(EVERY_BENCH);
                }
            }
            E::GauntletCleared => unlock(GAUNTLET),
            E::SlugForm => unlock(UNN_FORM),
            E::TextHighlighted => unlock(HIGHLIGHTED_TEXT),
            // (Not everyone plays fullscreen games: no achievement for it.)
            E::HidFromGame => {}
            E::WindowShattered => unlock(SHATTERED_WINDOW),
            E::GeoCollected(n) => {
                if self.count(GEO_HOARD, *n) {
                    unlock(GEO_HOARD);
                }
            }
            E::PokedWhileNapping => unlock(NAP_POKE),
            E::Thrown { monitor } => {
                unlock(THROW);
                self.thrown_from = Some(*monitor);
            }
            E::Grabbed => {
                if self.thrown_from.take().is_some() {
                    unlock(MIDAIR_CATCH);
                }
            }
            E::WallJumped => {
                self.wall_jumps += 1;
                if self.wall_jumps >= self.goal(WALL_JUMP_CHAIN) {
                    unlock(WALL_JUMP_CHAIN);
                }
            }
            E::SuperHardLanding => unlock(SUPER_HARD_LANDING),
            E::SteppedBehind => unlock(STEP_BEHIND),
            E::TroupeMasterBeaten => unlock(TROUPE_MASTER),
            E::FellToTroupeMaster => unlock(TROUPE_ENCORE),
            E::HornetRaceWon => unlock(HORNET_RACE),
            E::HornetDuelDone => unlock(HORNET_DUEL),
            E::GrimmCameoSeen => unlock(GRIMM_CAMEO),
        }
        let mut fresh = Vec::new();
        for id in new {
            if let Some(i) = self.defs.iter().position(|d| d.id == id)
                && !self.save.unlocked.contains_key(id)
            {
                self.save.unlocked.insert(id.to_string(), now);
                self.dirty = true;
                fresh.push(i);
            }
        }
        fresh.into_iter().map(|i| &self.defs[i]).collect()
    }

    /// The defined goal of counting achievement `id` (1 when not given or not defined).
    fn goal(&self, id: &str) -> u32 {
        self.defs.iter().find(|d| d.id == id).and_then(|d| d.goal).unwrap_or(1).max(1)
    }

    /// Add `n` to `id`'s count (not past its goal); true when the goal is reached.
    fn count(&mut self, id: &str, n: u32) -> bool {
        let goal = self.goal(id);
        let c = self.save.counts.entry(id.to_string()).or_insert(0);
        let before = *c;
        *c = c.saturating_add(n).min(goal);
        if *c != before {
            self.dirty = true;
        }
        *c >= goal
    }

    /// `id` has seen `what`; true once it has seen `total` different things.
    fn see(&mut self, id: &str, what: &str, total: usize) -> bool {
        let set = self.save.seen.entry(id.to_string()).or_default();
        if set.insert(what.to_string()) {
            self.dirty = true;
        }
        total > 0 && set.len() >= total
    }
}

#[cfg(test)]
#[path = "achievements_tests.rs"]
mod tests;
