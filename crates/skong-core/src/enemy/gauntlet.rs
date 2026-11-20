//! The gauntlet: a short arena event on the knight's monitor, like the game's arenas and
//! the Colosseum trials. A quiet announcement (the knight's "!" and his `challenge` bow),
//! then a few waves of enemies, each arriving only after the previous one is cleared and a
//! short breather, and a bigger Geo burst at the end (his `victory`).
//!
//! It ends gracefully when the user types hard, a fullscreen app comes up, a visitor shows
//! up, or the knight dies or is badly hurt: the enemies still about wander off, nothing is
//! lost. Rare on its own (now and then after he has beaten several single enemies), or on
//! request (tray / panel). Tuning: the `gauntlet` section of `data/enemies.json`.
//!
//! [`GauntletDirector`] is the whole logic and is pure: the app reports the world and the
//! enemies' fates each tick and carries out [`GauntletStep`] (spawn this enemy, send the
//! survivors away, burst the reward), placing arrivals with [`super::plan_arrival`] like any
//! other enemy (settled, reachable ledges; never on a game's monitor). The brain sees
//! [`GauntletCue`] (`ai::Brain::see_gauntlet`) for the knight's part.
use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Deserialize;

use super::EnemySet;
use crate::ai::rng::Rng;
use crate::geom::Vec2;

/// One wave: how many, and which enemies (names from `enemies`, with weights).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WaveConfig {
    /// [min, max] enemies in the wave.
    pub count: [usize; 2],
    pub mix: BTreeMap<String, f32>,
}

/// When a gauntlet happens on its own: after he has beaten `after_kills` single enemies,
/// each further kill rolls `chance`; never twice within `cooldown` seconds.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NaturalGauntlet {
    pub enabled: bool,
    pub after_kills: u32,
    pub chance: f32,
    pub cooldown: f32,
}

impl Default for NaturalGauntlet {
    fn default() -> Self {
        Self { enabled: true, after_kills: 5, chance: 0.35, cooldown: 3600.0 }
    }
}

/// The `gauntlet` section of `data/enemies.json` (see its `$comment`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GauntletConfig {
    /// The schema notes in the JSON.
    #[serde(rename = "$comment")]
    pub comment: Option<String>,
    pub waves: Vec<WaveConfig>,
    /// Seconds from the announcement to the first wave.
    pub announce: f32,
    /// Seconds between a cleared wave and the next (random in range).
    pub breather: [f32; 2],
    /// Seconds between the members of a wave arriving.
    pub stagger: f32,
    /// Wave members don't wander off on their own for this long (s).
    pub hold: f32,
    /// A wave still not cleared after this long ends the gauntlet (s).
    pub max_wave_secs: f32,
    /// Nowhere to put the next arrival for this long ends the gauntlet (s).
    pub no_room_secs: f32,
    /// Geo burst on completion: [min, max].
    pub reward: [u32; 2],
    /// Seconds of the closing cheer (the knight's `victory`).
    pub cheer: f32,
    /// Typing at least this fast (keys/s) for `busy_secs` ends it.
    pub busy_keys_per_sec: f32,
    pub busy_secs: f32,
    /// Hurt above this (0..1) ends it.
    pub hurt: f32,
    /// A gauntlet that can't start (a fight still on, a visitor, ...) waits this long (s).
    pub wait: f32,
    pub natural: NaturalGauntlet,
}

impl Default for GauntletConfig {
    fn default() -> Self {
        let wave = |count: [usize; 2], mix: &[(&str, f32)]| WaveConfig { count, mix: mix.iter().map(|(n, w)| (n.to_string(), *w)).collect() };
        Self {
            comment: None,
            waves: vec![
                wave([2, 2], &[("crawlid", 2.0), ("husk", 1.0)]),
                wave([2, 3], &[("crawlid", 1.0), ("vengefly", 1.5), ("husk", 1.0)]),
                wave([3, 4], &[("crawlid", 0.5), ("vengefly", 1.0), ("husk", 1.5)]),
            ],
            announce: 3.0,
            breather: [4.0, 6.0],
            stagger: 1.2,
            hold: 300.0,
            max_wave_secs: 150.0,
            no_room_secs: 12.0,
            reward: [40, 60],
            cheer: 3.0,
            busy_keys_per_sec: 2.5,
            busy_secs: 2.0,
            hurt: 0.6,
            wait: 180.0,
            natural: NaturalGauntlet::default(),
        }
    }
}

impl GauntletConfig {
    /// Check numbers and resolve each wave's mix against the roster: per wave, (def, weight).
    pub fn resolve(&self, set: &EnemySet) -> Result<Vec<Vec<(usize, f32)>>> {
        let ok = |x: f32| x.is_finite() && x >= 0.0;
        if !ok(self.announce) || !ok(self.stagger) || !ok(self.hold) || !ok(self.cheer) || !ok(self.busy_secs) || !ok(self.wait) {
            bail!("gauntlet: times must be >= 0");
        }
        if !ok(self.breather[0]) || self.breather[1] < self.breather[0] || self.reward[1] < self.reward[0] {
            bail!("gauntlet: bad range");
        }
        if self.max_wave_secs <= 0.0 || self.no_room_secs <= 0.0 || !(0.0..=1.0).contains(&self.natural.chance) {
            bail!("gauntlet: bad limits");
        }
        self.waves
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if w.count[0] == 0 || w.count[1] < w.count[0] {
                    bail!("gauntlet wave {}: bad count", i + 1);
                }
                let mix = w
                    .mix
                    .iter()
                    .map(|(name, &weight)| match set.id(name) {
                        Some(def) if ok(weight) => Ok((def, weight)),
                        Some(_) => bail!("gauntlet wave {}: bad weight for '{name}'", i + 1),
                        None => bail!("gauntlet wave {}: no enemy '{name}'", i + 1),
                    })
                    .collect::<Result<Vec<_>>>()?;
                if mix.iter().all(|(_, w)| *w <= 0.0) {
                    bail!("gauntlet wave {}: empty mix", i + 1);
                }
                Ok(mix)
            })
            .collect()
    }
}

/// Where the gauntlet is at (for the brain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GauntletStage {
    /// The announcement: he readies himself.
    Announce,
    /// A wave is coming in / being fought.
    Wave,
    /// A short breather before the next wave.
    Breather,
    /// All waves cleared: the reward and his cheer.
    Reward,
}

/// The gauntlet as the brain sees it (see `ai::Brain::see_gauntlet`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GauntletCue {
    /// Different for every gauntlet.
    pub serial: u32,
    pub stage: GauntletStage,
    /// The current (or next) wave, 1-based, of `waves`.
    pub wave: usize,
    pub waves: usize,
    /// The arena: where the knight stood when it began.
    pub at: Vec2,
}

/// Why a gauntlet started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GauntletTrigger {
    /// On its own (after he had beaten several single enemies).
    Natural,
    /// The user asked (tray / panel).
    Requested,
}

/// Why a gauntlet ended early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortReason {
    /// The user is typing hard.
    Typing,
    /// A fullscreen app came up.
    Fullscreen,
    /// A visitor showed up.
    Visitor,
    /// The knight died.
    KnightDown,
    /// The knight is badly hurt (or backed off, knocked about).
    Hurt,
    /// Nowhere good for the next enemy to arrive.
    NoRoom,
    /// A wave dragged on too long.
    TooLong,
}

impl AbortReason {
    pub fn label(self) -> &'static str {
        match self {
            AbortReason::Typing => "the user is typing",
            AbortReason::Fullscreen => "a fullscreen app",
            AbortReason::Visitor => "a visitor",
            AbortReason::KnightDown => "the knight died",
            AbortReason::Hurt => "the knight is badly hurt",
            AbortReason::NoRoom => "no room for the next wave",
            AbortReason::TooLong => "a wave dragged on",
        }
    }
}

/// What happened in a gauntlet (logged; for achievements and the like).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GauntletEvent {
    /// `gauntlet_started`: the announcement.
    Started { serial: u32, trigger: GauntletTrigger, waves: usize },
    /// `gauntlet_wave_started`: wave `wave` (1-based) of `count` enemies is on its way.
    WaveStarted { serial: u32, wave: usize, count: usize },
    /// `gauntlet_wave_cleared`: none of wave `wave` (1-based) of `waves` is left.
    WaveCleared { serial: u32, wave: usize, waves: usize },
    /// `gauntlet_completed`: every wave cleared; `geo` is the reward burst, `beaten` the
    /// enemies he struck down.
    Completed { serial: u32, waves: usize, beaten: u32, geo: u32 },
    /// `gauntlet_aborted`: ended early during wave `wave` (0 = the announcement).
    Aborted { serial: u32, wave: usize, reason: AbortReason },
}

impl GauntletEvent {
    /// Stable event name (log lines, achievements).
    pub fn name(&self) -> &'static str {
        match self {
            GauntletEvent::Started { .. } => "gauntlet_started",
            GauntletEvent::WaveStarted { .. } => "gauntlet_wave_started",
            GauntletEvent::WaveCleared { .. } => "gauntlet_wave_cleared",
            GauntletEvent::Completed { .. } => "gauntlet_completed",
            GauntletEvent::Aborted { .. } => "gauntlet_aborted",
        }
    }
}

/// The world this tick, as the gauntlet cares.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GauntletWorld {
    pub keys_per_sec: f32,
    pub user_idle_secs: f32,
    /// Seconds idle that count as "away" (no natural gauntlet then).
    pub away_secs: f32,
    pub fullscreen: bool,
    pub visitor_present: bool,
    /// Live enemies that are not part of a gauntlet (another fight).
    pub others_alive: usize,
    /// Asleep, hiding, held, fleeing...: no gauntlet now.
    pub knight_busy: bool,
    /// Fighting (enemies, a spar, the cursor) or in a lesson: no gauntlet now.
    pub knight_fighting: bool,
    pub knight_down: bool,
    pub knight_hurt: bool,
    /// His Shade waits to be reclaimed: that comes first (no gauntlet, requested or not).
    pub shade: bool,
    /// The knight's feet (the arena).
    pub knight_at: Vec2,
    /// Arrivals can be placed (a nav graph is ready).
    pub ready: bool,
}

/// Why a gauntlet can't start right now (`None`: it can).
pub fn start_blocked(cfg: &GauntletConfig, w: &GauntletWorld, trigger: GauntletTrigger) -> Option<&'static str> {
    let natural = trigger == GauntletTrigger::Natural;
    if cfg.waves.is_empty() {
        Some("no waves")
    } else if w.fullscreen {
        Some("a fullscreen app")
    } else if w.visitor_present {
        Some("a visitor")
    } else if w.others_alive > 0 || w.knight_fighting {
        Some("another fight")
    } else if w.knight_down || w.knight_hurt {
        Some("the knight is hurt")
    } else if w.shade {
        Some("his Shade waits")
    } else if w.keys_per_sec >= cfg.busy_keys_per_sec {
        Some("the user is typing")
    } else if natural && (w.knight_busy || w.user_idle_secs >= w.away_secs) {
        Some("the knight is busy or the user away")
    } else if !w.ready {
        Some("no nav graph yet")
    } else {
        None
    }
}

/// What the app should do this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GauntletStep {
    /// Let an enemy of this def arrive now (report back with [`GauntletDirector::spawned`]).
    pub spawn: Option<usize>,
    /// The gauntlet ended early: send these enemies (every one that came for it) away;
    /// they wander off.
    pub dismiss: Vec<u32>,
    /// Burst this much Geo by the knight (completion).
    pub reward: Option<u32>,
    pub events: Vec<GauntletEvent>,
}

/// One running gauntlet.
#[derive(Clone, Debug)]
struct Run {
    serial: u32,
    stage: GauntletStage,
    /// The rolled line-up, per wave.
    waves: Vec<Vec<usize>>,
    /// Current (or next) wave, 0-based.
    wave: usize,
    /// Members of the current wave still to arrive (front first).
    queue: Vec<usize>,
    /// Ids of every enemy that arrived for this gauntlet, and of the current wave.
    all: Vec<u32>,
    members: Vec<u32>,
    timer: f32,
    spawn_in: f32,
    wave_secs: f32,
    starved: f32,
    busy_for: f32,
    beaten: u32,
    at: Vec2,
}

/// Runs gauntlets: natural and requested triggers, the start conditions, the waves, the
/// reward, the aborts. Pure; see the module docs for the app's part.
#[derive(Clone, Debug)]
pub struct GauntletDirector {
    pub cfg: GauntletConfig,
    mix: Vec<Vec<(usize, f32)>>,
    now: f64,
    /// Single enemies beaten since the last gauntlet.
    kills: u32,
    /// No natural gauntlet before this time.
    ready_at: f64,
    /// A gauntlet waiting for a good moment, and how long it has waited.
    pending: Option<(GauntletTrigger, f32)>,
    run: Option<Run>,
    serial: u32,
    /// Last reason a pending gauntlet was held back (logged on change).
    held_by: Option<&'static str>,
}

impl GauntletDirector {
    pub fn new(cfg: GauntletConfig, set: &EnemySet) -> Result<Self> {
        let mix = cfg.resolve(set)?;
        Ok(Self { cfg, mix, now: 0.0, kills: 0, ready_at: 0.0, pending: None, run: None, serial: 0, held_by: None })
    }

    /// Tests: days of uptime pass in an instant.
    #[cfg(test)]
    pub(crate) fn skip_clock(&mut self, secs: f64) {
        self.now += secs;
    }

    /// The user asked for one (tray / panel): it starts at the next good moment.
    pub fn request(&mut self) {
        if self.run.is_some() {
            log::info!("gauntlet: one is already on");
        } else {
            self.pending = Some((GauntletTrigger::Requested, 0.0));
            self.held_by = None;
        }
    }

    /// A gauntlet is running.
    pub fn active(&self) -> bool {
        self.run.is_some()
    }

    /// A gauntlet is waiting to start.
    pub fn pending(&self) -> Option<GauntletTrigger> {
        self.pending.map(|p| p.0)
    }

    /// Single enemies beaten towards the next natural gauntlet.
    pub fn kills(&self) -> u32 {
        self.kills
    }

    /// Enemy `id` arrived for the gauntlet (any wave).
    pub fn is_member(&self, id: u32) -> bool {
        self.run.as_ref().is_some_and(|r| r.all.contains(&id))
    }

    /// The gauntlet as the brain sees it.
    pub fn cue(&self) -> Option<GauntletCue> {
        self.run.as_ref().map(|r| GauntletCue { serial: r.serial, stage: r.stage, wave: r.wave + 1, waves: r.waves.len(), at: r.at })
    }

    /// The knight beat enemy `id`: a gauntlet member counts for the run, any other one
    /// towards a natural gauntlet.
    pub fn on_kill(&mut self, id: u32, rng: &mut Rng) {
        if let Some(r) = self.run.as_mut()
            && r.all.contains(&id)
        {
            r.beaten += 1;
            return;
        }
        self.kills += 1;
        let n = self.cfg.natural;
        if n.enabled && self.run.is_none() && self.pending.is_none() && self.kills >= n.after_kills && self.now >= self.ready_at && rng.chance(n.chance) {
            log::info!("gauntlet: {} enemies beaten; a gauntlet is coming", self.kills);
            self.pending = Some((GauntletTrigger::Natural, 0.0));
            self.held_by = None;
        }
    }

    /// The app placed the enemy asked for in [`GauntletStep::spawn`] (`Some(id)`), or found
    /// no good spot for it (`None`: asked again a moment later).
    pub fn spawned(&mut self, id: Option<u32>) {
        let Some(r) = self.run.as_mut() else { return };
        match id {
            Some(id) => {
                if !r.queue.is_empty() {
                    r.queue.remove(0);
                }
                r.members.push(id);
                r.all.push(id);
                r.starved = 0.0;
                r.spawn_in = self.cfg.stagger;
            }
            None => r.spawn_in = 1.0,
        }
    }

    /// Seconds wave members stay before wandering off on their own.
    pub fn hold_secs(&self) -> f32 {
        self.cfg.hold
    }

    /// Advance. `alive(id)`: is that enemy still alive (not dying, dead or gone)?
    pub fn tick(&mut self, dt: f32, w: &GauntletWorld, alive: &dyn Fn(u32) -> bool, rng: &mut Rng) -> GauntletStep {
        self.now += dt as f64;
        let mut step = GauntletStep::default();
        if self.run.is_none() {
            self.try_start(dt, w, rng, &mut step);
            return step;
        }
        if let Some(reason) = self.abort_reason(dt, w) {
            self.abort(reason, &mut step);
            return step;
        }
        let cfg = &self.cfg;
        let Some(r) = self.run.as_mut() else { return step };
        // Some(reason): ends early; Some(None): over (after the cheer).
        let mut end: Option<Option<AbortReason>> = None;
        match r.stage {
            GauntletStage::Announce | GauntletStage::Breather => {
                r.timer -= dt;
                if r.timer <= 0.0 {
                    r.stage = GauntletStage::Wave;
                    r.queue = r.waves[r.wave].clone();
                    r.members.clear();
                    r.wave_secs = 0.0;
                    r.starved = 0.0;
                    r.spawn_in = 0.0;
                    log::info!("gauntlet: wave {}/{} ({} enemies)", r.wave + 1, r.waves.len(), r.queue.len());
                    step.events.push(GauntletEvent::WaveStarted { serial: r.serial, wave: r.wave + 1, count: r.queue.len() });
                }
            }
            GauntletStage::Wave => {
                r.wave_secs += dt;
                r.spawn_in -= dt;
                if !r.queue.is_empty() {
                    r.starved += dt;
                }
                if r.wave_secs > cfg.max_wave_secs {
                    end = Some(Some(AbortReason::TooLong));
                } else if r.starved > cfg.no_room_secs {
                    end = Some(Some(AbortReason::NoRoom));
                } else if let Some(&def) = r.queue.first() {
                    if r.spawn_in <= 0.0 {
                        step.spawn = Some(def);
                    }
                } else if !r.members.iter().any(|&id| alive(id)) {
                    let (serial, wave, waves) = (r.serial, r.wave + 1, r.waves.len());
                    log::info!("gauntlet: wave {wave}/{waves} cleared");
                    step.events.push(GauntletEvent::WaveCleared { serial, wave, waves });
                    if wave == waves {
                        let geo = cfg.reward[0] + rng.next_u32() % (cfg.reward[1] - cfg.reward[0] + 1);
                        log::info!("gauntlet: completed ({} beaten); {geo} geo", r.beaten);
                        step.events.push(GauntletEvent::Completed { serial, waves, beaten: r.beaten, geo });
                        step.reward = Some(geo);
                        r.stage = GauntletStage::Reward;
                        r.timer = cfg.cheer;
                    } else {
                        r.wave += 1;
                        r.stage = GauntletStage::Breather;
                        r.timer = rng.range(cfg.breather[0], cfg.breather[1]);
                    }
                }
            }
            GauntletStage::Reward => {
                r.timer -= dt;
                if r.timer <= 0.0 {
                    end = Some(None);
                }
            }
        }
        match end {
            Some(Some(reason)) => self.abort(reason, &mut step),
            Some(None) => self.finish(),
            None => {}
        }
        step
    }

    fn try_start(&mut self, dt: f32, w: &GauntletWorld, rng: &mut Rng, step: &mut GauntletStep) {
        let Some((trigger, waited)) = self.pending.as_mut() else { return };
        *waited += dt;
        let trigger = *trigger;
        if *waited > self.cfg.wait {
            log::info!("gauntlet: no good moment came ({}); maybe later", self.held_by.unwrap_or("?"));
            self.pending = None;
            return;
        }
        if let Some(why) = start_blocked(&self.cfg, w, trigger) {
            if self.held_by != Some(why) {
                log::info!("gauntlet: waiting ({why})");
                self.held_by = Some(why);
            }
            return;
        }
        self.pending = None;
        self.held_by = None;
        self.serial += 1;
        let waves: Vec<Vec<usize>> = self
            .mix
            .iter()
            .zip(&self.cfg.waves)
            .map(|(mix, wc)| {
                let n = wc.count[0] + rng.next_u32() as usize % (wc.count[1] - wc.count[0] + 1);
                (0..n).filter_map(|_| rng.pick_weighted(mix)).collect()
            })
            .collect();
        let n = waves.len();
        log::info!("gauntlet: begins ({trigger:?}, {n} waves: {:?})", waves.iter().map(Vec::len).collect::<Vec<_>>());
        self.run = Some(Run {
            serial: self.serial,
            stage: GauntletStage::Announce,
            waves,
            wave: 0,
            queue: Vec::new(),
            all: Vec::new(),
            members: Vec::new(),
            timer: self.cfg.announce,
            spawn_in: 0.0,
            wave_secs: 0.0,
            starved: 0.0,
            busy_for: 0.0,
            beaten: 0,
            at: w.knight_at,
        });
        step.events.push(GauntletEvent::Started { serial: self.serial, trigger, waves: n });
    }

    fn abort_reason(&mut self, dt: f32, w: &GauntletWorld) -> Option<AbortReason> {
        let r = self.run.as_mut()?;
        if r.stage == GauntletStage::Reward {
            return None; // won: nothing to lose any more
        }
        r.busy_for = if w.keys_per_sec >= self.cfg.busy_keys_per_sec { r.busy_for + dt } else { 0.0 };
        if w.knight_down {
            Some(AbortReason::KnightDown)
        } else if w.fullscreen {
            Some(AbortReason::Fullscreen)
        } else if w.visitor_present {
            Some(AbortReason::Visitor)
        } else if w.knight_hurt {
            Some(AbortReason::Hurt)
        } else if r.busy_for >= self.cfg.busy_secs {
            Some(AbortReason::Typing)
        } else {
            None
        }
    }

    fn abort(&mut self, reason: AbortReason, step: &mut GauntletStep) {
        let Some(r) = &self.run else { return };
        let wave = if r.stage == GauntletStage::Announce { 0 } else { r.wave + 1 };
        log::info!("gauntlet: ends early in wave {wave} ({}); the enemies wander off", reason.label());
        step.events.push(GauntletEvent::Aborted { serial: r.serial, wave, reason });
        step.dismiss = r.all.clone();
        self.finish();
    }

    fn finish(&mut self) {
        self.run = None;
        self.kills = 0;
        self.ready_at = self.now + self.cfg.natural.cooldown as f64;
    }
}
