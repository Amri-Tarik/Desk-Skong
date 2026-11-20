//! Hidden achievements for [`Pet`]: the **one place** where what happens in the pet
//! (movement events, the brain's intent, pokes, visitors, enemies, finds, the stakes) is
//! mapped to [`AchievementEvent`]s. The criteria themselves live in
//! [`skong_core::achievements::Tracker`].
//!
//! `app.rs` calls [`Pet::achievements_intent`] after the brain's intent was applied,
//! [`Pet::achievements_poke`] for each click / grab / throw, and [`Pet::achievements_tick`]
//! at the end of every tick. Other modules only expose what happened (`Visitors::take_news`,
//! `Enemies::killed`, `Discoveries::mined`, `Stakes::shattered`).
//!
//! An unlock is announced quietly: a small sparkle above his head, a line in the tray
//! tooltip, a log line, and the panel's Achievements tab (sent only on change). Progress is
//! saved to `%APPDATA%\Desk-Skong\achievements.json`.
use std::path::{Path, PathBuf};

use skong_core::achievements::{self, AchievementEvent as E, Tracker, Totals};
use skong_core::ai::{BehaviourKind, FoeKind, Intent, Poke, StakeEvent};
use skong_core::controller::{BehindPhase, MoveEvent, MoveState};
use skong_core::geom::Vec2;
use skong_core::stakes::{DeathKind, Spell};
use skong_core::terrain::LedgeKind;
use skong_ipc::{AchievementEntry, AchievementsInfo, UnlockedAchievement};

use super::Pet;
use crate::emotes::Emote;

/// A release faster than this is a throw, not a drop (px/s).
const THROW_SPEED: f32 = 500.0;
/// How long the last unlock stays in the tray tooltip (s).
const TIP_SECS: f32 = 15.0 * 60.0;
/// Seconds between saves while progress keeps changing (Geo pickups).
const SAVE_GAP: f32 = 2.0;

fn save_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Desk-Skong").join("achievements.json"))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The tracker plus what the pet looked like last tick (to see edges, not levels).
pub struct Achievements {
    /// `None` when `data/achievements.json` is missing (nothing unlocks).
    tracker: Option<Tracker>,
    path: Option<PathBuf>,
    /// Wallet, Shades beaten and windows shattered as last seen.
    geo: u32,
    shades: u32,
    shattered: u32,
    /// Gauntlets completed since the last tick (set by `tick_enemies`).
    pub(super) gauntlets_cleared: u32,
    /// The troupe's events since the last tick (set by `tick_troupe`).
    pub(super) troupe: Vec<skong_core::enemy::grimm::GrimmEvent>,
    /// Brain switch already looked at (its time).
    switch_at: f32,
    act: Option<String>,
    pos: Vec2,
    state: MoveState,
    behind: bool,
    gaming: bool,
    lost_battle: bool,
    battling: bool,
    /// Last unlock's title for the tray tooltip, and seconds left.
    tip: Option<(String, f32)>,
    /// Something changed for the panel.
    changed: bool,
    since_save: f32,
}

impl Achievements {
    /// Definitions from `data/achievements.json` (next to `data_hk`'s parent) and saved progress.
    pub fn load(data_hk: &Path) -> Self {
        let path = save_path();
        let tracker = match achievements::load_defs(&data_hk.join("../achievements.json")) {
            Ok(defs) => {
                let t = Tracker::new(defs, path.as_deref().map(achievements::read_save).unwrap_or_default());
                log::info!("achievements: {} / {} found", t.unlocked_count(), t.total());
                Some(t)
            }
            Err(e) => {
                log::warn!("achievements disabled: {e:#}");
                None
            }
        };
        Self {
            tracker,
            path,
            geo: 0,
            shades: 0,
            shattered: 0,
            gauntlets_cleared: 0,
            troupe: Vec::new(),
            switch_at: -1.0,
            act: None,
            pos: Vec2::ZERO,
            state: MoveState::Grounded,
            behind: false,
            gaming: false,
            lost_battle: false,
            battling: false,
            tip: None,
            changed: true,
            since_save: 0.0,
        }
    }

    /// How many are unlocked (the troupe's lantern waits for enough of them).
    pub fn unlocked(&self) -> usize {
        self.tracker.as_ref().map_or(0, |t| t.unlocked_count())
    }

    /// The tray tooltip, with the latest unlock for a while.
    pub fn tray_tip(&self, base: String) -> String {
        match &self.tip {
            Some((title, _)) => format!("{base}\nAchieved: {title}"),
            None => base,
        }
    }

    /// For the panel: locked entries carry only their hint.
    fn info(&self) -> AchievementsInfo {
        let Some(t) = &self.tracker else { return AchievementsInfo::default() };
        let entries = t
            .defs()
            .iter()
            .map(|d| AchievementEntry {
                hint: d.hint.clone(),
                unlocked: t.unlocked_at(&d.id).map(|at| UnlockedAchievement { id: d.id.clone(), title: d.title.clone(), description: d.description.clone(), at }),
            })
            .collect();
        AchievementsInfo { entries }
    }
}

impl Pet {
    /// Once visitors and enemies are loaded: what "every ..." counts against.
    pub(super) fn achievements_setup(&mut self) {
        let totals = Totals {
            visitors: self.visitors.as_ref().map_or(0, |v| v.kinds().len()),
            regions: crate::discoveries::REGION_COUNT,
            bench_styles: self.discoveries.bench_style_count(&self.benches),
            enemies: self.enemies.as_ref().map_or(0, |e| e.kinds().len()),
        };
        log::debug!("achievements: totals {totals:?}");
        let a = &mut self.achievements;
        if let Some(t) = a.tracker.as_mut() {
            t.set_totals(totals);
        }
        (a.geo, a.shades, a.shattered) = (self.stakes.save.geo, self.stakes.save.shades_defeated, self.stakes.shattered);
        a.pos = self.ch.pos;
    }

    /// Debug: forget everything.
    pub(super) fn reset_achievements(&mut self) {
        if let Some(t) = self.achievements.tracker.as_mut() {
            t.reset();
            log::info!("achievements: reset");
        }
        self.achievements.tip = None;
        self.achievements.changed = true;
        self.achievements.since_save = SAVE_GAP;
    }

    /// One event into the tracker; unlocks are announced quietly.
    fn achieve(&mut self, ev: E) {
        let Some(t) = self.achievements.tracker.as_mut() else { return };
        let fresh: Vec<String> = t.on(&ev, now_unix()).into_iter().map(|d| d.title.clone()).collect();
        if fresh.is_empty() {
            return;
        }
        let (n, total) = (t.unlocked_count(), t.total());
        for title in &fresh {
            log::info!("achievement unlocked: {title} ({n} / {total})");
        }
        self.emotes.show(Emote::Spark);
        self.achievements.tip = fresh.last().map(|t| (t.clone(), TIP_SECS));
        self.achievements.changed = true;
        self.achievements.since_save = SAVE_GAP; // an unlock is saved right away
    }

    /// Index of the monitor under `p` (feet); `usize::MAX` off every monitor.
    fn monitor_index(&self, p: Vec2) -> usize {
        self.watcher.snapshot().monitors.iter().position(|m| m.rect.contains(p.x as i32, (p.y - 1.0) as i32)).unwrap_or(usize::MAX)
    }

    fn on_floor(&self) -> bool {
        self.ch.is_grounded() && self.ch.support.is_some_and(|s| s.kind == LedgeKind::Ground)
    }

    /// After the brain's intent was applied (strikes landed, stakes handled).
    pub(super) fn achievements_intent(&mut self, intent: &Intent) {
        if self.achievements.tracker.is_none() {
            return;
        }
        let on_floor = self.on_floor();
        for s in intent.strikes.iter().filter(|s| s.kind == FoeKind::Enemy) {
            self.achieve(E::EnemyStruck { foe: s.id, on_floor });
        }
        let battling = self.achievements.battling;
        for e in &intent.stakes {
            match *e {
                StakeEvent::Cast { spell: Spell::Fireball | Spell::Scream, .. } => self.achieve(E::SpellCast),
                StakeEvent::Died { kind, .. } => {
                    if kind == DeathKind::Fight && battling {
                        self.achieve(E::CursorBattleLost);
                    }
                    self.achieve(E::Died { shade: kind.leaves_shade(), fall: kind == DeathKind::Fall });
                }
                StakeEvent::Victory { .. } => self.achieve(E::CursorBattleWon),
                _ => {}
            }
        }
        if let Some(skong_core::ai::DiscoveryEvent::Used(id)) = intent.discovery
            && let Some(style) = self.discoveries.bench_style(id)
        {
            self.achieve(E::BenchRested { style });
        }
    }

    /// A click, grab or throw on the knight.
    pub(super) fn achievements_poke(&mut self, poke: &Poke) {
        match *poke {
            Poke::Click { .. } if self.brain.behaviour() == BehaviourKind::Nap => self.achieve(E::PokedWhileNapping),
            Poke::Click { .. } => {}
            Poke::Grab => self.achieve(E::Grabbed),
            Poke::Release { vel } if vel.length() > THROW_SPEED => {
                let monitor = self.monitor_index(self.ch.pos);
                self.achieve(E::Thrown { monitor });
            }
            Poke::Release { .. } => {}
        }
    }

    /// End of every tick: movement, acts, visitors, enemies, finds and the stakes; then
    /// save and tell the panel when something changed.
    pub(super) fn achievements_tick(&mut self, dt: f32) {
        if self.achievements.tracker.is_none() {
            return;
        }
        let mut evs = Vec::new();

        // Movement.
        let here = self.monitor_index(self.ch.pos);
        let mut dash_over = false;
        for e in &self.move_events {
            match e {
                MoveEvent::Landed { .. } => evs.push(E::Landed { monitor: here, floor: self.on_floor() }),
                MoveEvent::WallJumped { .. } => evs.push(E::WallJumped),
                MoveEvent::SuperHardLanded => evs.push(E::SuperHardLanding),
                MoveEvent::PlatformMoved { .. } => evs.push(E::WindowRode),
                MoveEvent::Pogo => evs.push(E::Pogo),
                MoveEvent::SuperdashLaunched => {
                    let from_wall = matches!(self.achievements.state, MoveState::SuperdashCharge { wall: Some(_) });
                    evs.push(E::SuperdashLaunched { monitor: here, from_wall });
                }
                MoveEvent::SuperdashHitWall | MoveEvent::SuperdashEnded | MoveEvent::SuperdashCancelled => dash_over = true,
                _ => {}
            }
        }
        let flying = self.ch.state == MoveState::Superdash;
        if flying || dash_over {
            evs.push(E::SuperdashFlying { monitor: here });
        }
        if dash_over || (self.achievements.state == MoveState::Superdash && !flying) {
            evs.push(E::SuperdashOver);
        }
        let a = &mut self.achievements;
        a.state = self.ch.state;
        let behind = self.ch.behind_phase() == Some(BehindPhase::Behind);
        if behind && !a.behind {
            evs.push(E::SteppedBehind);
        }
        a.behind = behind;

        // Acts: the slug form, a Dream Gate warp (the arrival far from the warp's spot).
        let act = self.act.as_ref().map(|p| self.data.acts.get(p.act()).name.clone());
        if act != a.act {
            match act.as_deref() {
                Some("slug_crawl") => evs.push(E::SlugForm),
                Some(skong_core::dreamgate::ARRIVE_ACT) if a.act.as_deref() == Some(skong_core::dreamgate::WARP_ACT) && self.ch.pos.distance(a.pos) > 2.0 * self.ch.size.x => evs.push(E::DreamGateWarp),
                _ => {}
            }
            a.act = act;
        }
        a.pos = self.ch.pos;

        // The brain: highlighted text, a lost cursor battle.
        if let Some(sw) = self.brain.last_switch()
            && sw.at != a.switch_at
        {
            a.switch_at = sw.at;
            if sw.to == BehaviourKind::Highlight {
                evs.push(E::TextHighlighted);
            }
        }
        a.battling = self.brain.behaviour() == BehaviourKind::Battle;
        let lost = self.brain.battle_score().is_some_and(|s| s.cursor >= self.brain.config().battle.lose_hits);
        if lost && !a.lost_battle {
            evs.push(E::CursorBattleLost);
        }
        a.lost_battle = lost;

        // A fullscreen game sent him away (or into hiding).
        let gaming = self.leaving_for.is_some() || self.game_hidden;
        if gaming && !a.gaming {
            evs.push(E::HidFromGame);
        }
        a.gaming = gaming;

        // The stakes: Geo picked up (not won back from a Shade), Shades beaten, shattering.
        let save = &self.stakes.save;
        if save.shades_defeated != a.shades {
            a.shades = save.shades_defeated;
            evs.push(E::ShadeReclaimed);
        } else if save.geo > a.geo {
            evs.push(E::GeoCollected(save.geo - a.geo));
        }
        a.geo = save.geo;
        if self.stakes.shattered != a.shattered {
            a.shattered = self.stakes.shattered;
            evs.push(E::WindowShattered);
        }

        // Visitors, enemies, finds.
        for n in self.visitors.as_mut().map(|v| v.take_news()).unwrap_or_default() {
            use crate::npcs::VisitorNews as N;
            evs.push(match n {
                N::Talked { who } => E::VisitorTalked { who },
                N::LessonOver { who, finished: true } => E::LessonFinished { who },
                N::SparOver { who, lost: true } => E::SparWon { who },
                N::Dreamed { who: Some(who) } => E::VisitorDreamed { who },
                N::Dreamed { who: None } => E::WindowDreamed,
                N::Company { who, finished: true } => E::CompanyKept { who },
                N::LessonOver { .. } | N::SparOver { .. } | N::Company { .. } => continue,
            });
        }
        for (foe, kind) in self.enemies.as_mut().map(|e| std::mem::take(&mut e.killed)).unwrap_or_default() {
            evs.push(E::EnemyKilled { foe, kind });
        }
        for region in std::mem::take(&mut self.discoveries.mined) {
            evs.push(E::DepositMined { region });
        }
        for _ in 0..std::mem::take(&mut self.achievements.gauntlets_cleared) {
            evs.push(E::GauntletCleared);
        }
        for ev in std::mem::take(&mut self.achievements.troupe) {
            use skong_core::enemy::grimm::GrimmEvent as G;
            match ev {
                G::Defeated { .. } => evs.push(E::TroupeMasterBeaten),
                G::KnightFell => evs.push(E::FellToTroupeMaster),
                _ => {}
            }
        }

        for ev in evs {
            self.achieve(ev);
        }
        self.achievements_housekeeping(dt);
    }

    /// Tray tip timeout, throttled saves, the panel's copy.
    fn achievements_housekeeping(&mut self, dt: f32) {
        let a = &mut self.achievements;
        if let Some((_, secs)) = a.tip.as_mut() {
            *secs -= dt;
            if *secs <= 0.0 {
                a.tip = None;
            }
        }
        a.since_save += dt;
        if a.since_save >= SAVE_GAP
            && let (Some(t), Some(p)) = (a.tracker.as_mut(), a.path.as_deref())
            && t.take_dirty()
        {
            a.since_save = 0.0;
            if let Err(e) = achievements::write_save(p, t.save()) {
                log::warn!("could not save achievements to {}: {e}", p.display());
            }
        }
        if std::mem::take(&mut a.changed) {
            let info = a.info();
            self.control.publish_achievements(info);
        }
    }
}
