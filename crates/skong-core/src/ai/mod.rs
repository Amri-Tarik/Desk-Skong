//! The pet brain: a curious little knight that explores your desktop, naps when you
//! are away, and — if allowed — picks fights with your cursor.
//!
//! ```text
//!  Percepts ──► Senses (diffs, cursor velocity, user presence)
//!                 │
//!                 ├─► reflexes   (pokes, window closed underfoot, wake-ups)
//!                 ├─► Mind       (needs: energy/boredom/loneliness/mischief,
//!                 │               emotions: startle/annoyance/excitement/hurt)
//!                 ├─► InterestMap (POIs with decaying novelty + visit memory)
//!                 ▼
//!   utilities per BehaviourKind ──► selection with commitment + hysteresis
//!                 ▼
//!   Active behaviour state machine ──► Intent (motion / act / signal / attack)
//! ```
//!
//! * **Deterministic**: given the seed and the same percept sequence, the same intents
//!   come out (seeded xorshift, ordered maps, brain-internal clock).
//! * **Navigation is injected** through [`Navigator`] so the brain is testable without
//!   the planner; [`GraphNavigator`] adapts [`crate::nav`].
//! * **Acts are referenced by name** through an [`ActCatalog`]; missing acts degrade
//!   gracefully (the behaviour just skips that flourish).
mod battle;
mod behaviours;
mod config;
mod interest;
mod navigator;
mod needs;
mod percept;
mod repertoire;
mod rng;
#[cfg(test)]
mod tests;

use std::fmt::Write as _;

pub use battle::{BattleScore, Outcome};
pub use behaviours::BehaviourKind;
pub use config::{BattleConfig, BrainConfig, NeedRates, Personality};
pub use interest::{InterestMap, Poi, PoiKey, PoiKind, PoiTarget, ScoredPoi};
pub use navigator::{GraphNavigator, NavStatus, Navigator};
pub use needs::{Emotions, MOOD_TAGS, Mind, Needs};
pub use percept::{ClosedWindow, Cursor, CursorSense, DesktopChanges, Percepts, Poke, RunningAct, Senses, UserActivity};
pub use repertoire::{ActCatalog, ActInfo};

use crate::acts::ActId;
use crate::controller::{MoveEvent, MoveInput, Side};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use battle::Battle;
use behaviours::{Active, Ctx, Explore, Flee, Hide, Idle, KnockedOut, Nap, Ride, Status, Wander, Watch};
use needs::Activity;
use repertoire::Repertoire;
use rng::Rng;

/// How the knight should move this tick.
#[derive(Clone, Copy, Debug, Default)]
pub enum Motion {
    /// Stand still (or let a running act drive).
    #[default]
    Hold,
    /// Use the navigator's input this tick (the brain already called `go`).
    Navigate { goal: NavGoal, style: TravelStyle },
    /// Use this input as-is (chasing / fighting).
    Direct(MoveInput),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackDir {
    Forward,
    Up,
    Down,
}

impl AttackDir {
    /// Act in `data/acts.json` that renders this slash on the ground.
    pub fn act_name(self) -> &'static str {
        match self {
            AttackDir::Forward => "slash",
            AttackDir::Up => "up_slash",
            AttackDir::Down => "down_slash",
        }
    }
}

/// A slash at the cursor, to be rendered by the app (clips `Slash`/`SlashAlt`,
/// `UpSlash`, `DownSlash`; or the acts named by [`AttackDir::act_name`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attack {
    pub dir: AttackDir,
    pub side: Side,
    /// The slash connected (the cursor did not dodge). Nice spot for a hit flash.
    pub hit: bool,
}

/// The brain's output for one tick.
#[derive(Clone, Debug, Default)]
pub struct Intent {
    pub motion: Motion,
    /// Start this act (the brain only asks when no act is playing).
    pub act: Option<ActId>,
    /// Cut the running act short (only requested when it is interruptible).
    pub cancel_act: bool,
    /// Signal for the running act (e.g. "woken").
    pub signal: Option<&'static str>,
    pub attack: Option<Attack>,
    /// Face this way (when standing still).
    pub face: Option<Side>,
    pub behaviour: Option<BehaviourKind>,
    /// "behaviour detail | mood | top-3 utilities", for an overlay.
    pub debug: String,
}

/// Why the last behaviour switch happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchReason {
    /// The previous behaviour finished.
    Finished,
    /// Outscored after its commitment period (by more than the hysteresis margin).
    Outscored,
    /// An urgent behaviour (flee, hide) cut in.
    Urgent,
}

/// Record of one behaviour switch (for tests and the overlay).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Switch {
    pub at: f32,
    pub from: BehaviourKind,
    pub to: BehaviourKind,
    pub reason: SwitchReason,
}

pub struct Brain {
    cfg: BrainConfig,
    mind: Mind,
    rng: Rng,
    rep: Repertoire,
    interest: InterestMap,
    senses: Senses,
    active: Active,
    /// Brain clock (sum of dt).
    now: f32,
    since_switch: f32,
    think_in: f32,
    utilities: Vec<(BehaviourKind, f32)>,
    best_poi: Option<ScoredPoi>,
    battle_ready_at: f32,
    nap_ready_at: f32,
    /// Something scary happened here; flee from it.
    flee_from: Option<Vec2>,
    /// A huge fall knocked the knight flat.
    knocked_out: bool,
    recent_clicks: Vec<f32>,
    last_switch: Option<Switch>,
    exertion: f32,
}

impl Brain {
    pub fn new(cfg: BrainConfig, catalog: ActCatalog, seed: u32) -> Self {
        let personality = cfg.personality.unwrap_or_else(|| Personality::from_seed(seed));
        Self {
            mind: Mind::new(personality),
            rng: Rng::new(seed),
            rep: Repertoire::new(catalog),
            interest: InterestMap::default(),
            senses: Senses::default(),
            active: Active::Idle(Idle::initial()),
            now: 0.0,
            since_switch: f32::INFINITY,
            think_in: 0.0,
            utilities: Vec::new(),
            best_poi: None,
            battle_ready_at: 0.0,
            nap_ready_at: 0.0,
            flee_from: None,
            knocked_out: false,
            recent_clicks: Vec::new(),
            last_switch: None,
            exertion: 0.0,
            cfg,
        }
    }

    pub fn behaviour(&self) -> BehaviourKind {
        self.active.kind()
    }

    pub fn mind(&self) -> &Mind {
        &self.mind
    }

    /// For debug hotkeys and tests ("make him sleepy").
    pub fn mind_mut(&mut self) -> &mut Mind {
        &mut self.mind
    }

    pub fn config(&self) -> &BrainConfig {
        &self.cfg
    }

    pub fn set_bother_level(&mut self, level: f32) {
        self.cfg.bother_level = level.clamp(0.0, 1.0);
    }

    pub fn senses(&self) -> &Senses {
        &self.senses
    }

    /// Utilities from the last evaluation, best first.
    pub fn utilities(&self) -> &[(BehaviourKind, f32)] {
        &self.utilities
    }

    pub fn last_switch(&self) -> Option<Switch> {
        self.last_switch
    }

    pub fn battle_score(&self) -> Option<BattleScore> {
        match &self.active {
            Active::Battle(b) => Some(b.score),
            _ => None,
        }
    }

    /// The point of interest being explored, if any.
    pub fn exploring(&self) -> Option<Poi> {
        match &self.active {
            Active::Explore(e) => Some(e.poi),
            _ => None,
        }
    }

    /// One tick. Call every frame (or at a fixed rate) before physics.
    pub fn tick(&mut self, p: &Percepts, nav: &mut dyn Navigator) -> Intent {
        let dt = p.dt.max(0.0);
        self.now += dt;
        self.since_switch += dt;
        self.think_in -= dt;
        self.senses.update(p, self.now, &self.cfg);

        let mut out = Intent::default();
        self.reflexes(p, &mut out);
        self.interest.observe(self.now, dt, &self.senses, p.desktop, &self.cfg);

        let asleep = self.asleep(p);
        let with_user = !self.senses.user_away && self.senses.cursor.is_some_and(|c| c.dist() < 300.0);
        self.mind.update(dt, Activity { asleep, exertion: self.exertion, with_user }, self.senses.user_away, &self.cfg);

        if self.senses.held {
            nav.stop();
            out.behaviour = Some(self.active.kind());
            out.debug = format!("held | {}", self.mind.dominant_mood(self.senses.user_away));
            return out;
        }

        if self.think_in <= 0.0 {
            self.think_in = self.cfg.think_interval;
            self.think(p, nav, &mut out, false);
        }
        // A finished behaviour is replaced right away (at most once per tick).
        for _ in 0..2 {
            if self.run_active(p, nav, &mut out) == Status::Running {
                break;
            }
            self.think(p, nav, &mut out, true);
        }

        self.exertion = match out.motion {
            Motion::Hold => 0.0,
            Motion::Direct(_) => 1.0,
            Motion::Navigate { style: TravelStyle::Hurry, .. } => 0.7,
            Motion::Navigate { style: TravelStyle::Normal, .. } => 0.3,
            Motion::Navigate { style: TravelStyle::Stroll, .. } => 0.1,
        };
        out.behaviour = Some(self.active.kind());
        out.debug = self.debug_line();
        out
    }

    /// Sleeping: the bench "nap" act, or deep sleep ("bow_down") during a nap.
    fn asleep(&self, p: &Percepts) -> bool {
        let Some(act) = p.act else { return false };
        let is = |name: &str| self.rep.catalog.id(name) == Some(act.id);
        is("nap") || (is("bow_down") && matches!(self.active, Active::Nap(_)))
    }

    /// Immediate reactions that bypass deliberation.
    fn reflexes(&mut self, p: &Percepts, out: &mut Intent) {
        let napping = self.asleep(p);
        let fighting = matches!(self.active, Active::Battle(_));
        let mut woken = false;

        self.recent_clicks.retain(|t| self.now - *t < 2.0);
        for poke in p.pokes {
            match *poke {
                Poke::Click { pos } => {
                    if napping {
                        woken = true;
                        self.mind.startle(0.8);
                        self.flee_from = Some(pos);
                    } else if !fighting {
                        self.recent_clicks.push(self.now);
                        self.mind.annoy(0.15);
                        self.mind.excite(0.1 * self.mind.personality.boldness);
                        if self.recent_clicks.len() >= 3 {
                            // Poked hard: that's enough.
                            self.mind.startle(0.7);
                            self.flee_from = Some(pos);
                            self.recent_clicks.clear();
                        }
                    }
                }
                Poke::Grab => {
                    woken |= napping;
                    self.mind.startle(0.3);
                }
                Poke::Release { vel } => {
                    let speed = vel.length();
                    if speed > 1200.0 {
                        self.mind.excite(0.4 * self.mind.personality.boldness);
                        self.mind.startle(0.4);
                        self.mind.hurt(0.2);
                        self.mind.annoy(0.2);
                    }
                }
            }
        }

        if let Some(closed) = self.senses.support_closed {
            self.mind.startle(0.8);
            let r = closed.rect;
            self.flee_from = Some(Vec2::new((r.left + r.right) as f32 * 0.5, r.top as f32));
        }
        if !self.senses.changes.opened.is_empty() {
            self.mind.excite(0.1 * self.mind.personality.curiosity);
            self.mind.adjust(|n| &mut n.boredom, -0.05);
        }
        if self.senses.user_returned {
            self.mind.excite(0.3);
            woken |= napping;
        }
        if p.move_events.contains(&MoveEvent::Landed { hard: true }) {
            self.knocked_out = true;
        }
        if woken {
            self.senses.woken = true;
            out.signal = Some("woken");
            self.nap_ready_at = self.now + 90.0;
        }
        if self.mind.emotions.startle < 0.3 {
            self.flee_from = None; // calmed down before getting round to it
        }
    }

    /// Score every behaviour and switch if warranted.
    fn think(&mut self, p: &Percepts, nav: &mut dyn Navigator, out: &mut Intent, finished: bool) {
        self.best_poi = self.score_pois(p, nav);
        self.utilities = self.score_behaviours(p);
        let current = self.active.kind();
        let Some(&(best, best_u)) = self.utilities.first() else { return };
        let cur_u = self.utilities.iter().find(|(k, _)| *k == current).map_or(0.0, |u| u.1);

        let reason = if finished {
            Some(SwitchReason::Finished)
        } else if best == current {
            None
        } else if best_u >= self.cfg.urgent && best_u > cur_u {
            Some(SwitchReason::Urgent)
        } else if !current.exclusive() && self.since_switch >= self.cfg.min_commit && best_u > cur_u + self.cfg.hysteresis {
            Some(SwitchReason::Outscored)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.switch_to(best, reason, p, nav, out);
        }
    }

    fn switch_to(&mut self, kind: BehaviourKind, reason: SwitchReason, p: &Percepts, nav: &mut dyn Navigator, out: &mut Intent) {
        let from = self.active.kind();
        if let Active::Battle(b) = &self.active {
            let base = self.cfg.battle.cooldown / self.cfg.bother_level.max(0.05);
            let after = if b.outcome() == Some(Outcome::GaveUp) { base * 0.5 } else { base };
            self.battle_ready_at = self.now + after;
        }
        if from == BehaviourKind::Nap {
            self.nap_ready_at = self.nap_ready_at.max(self.now + 30.0);
        }
        if kind == BehaviourKind::KnockedOut {
            self.knocked_out = false;
        }
        nav.stop();
        let poi = self.best_poi.map(|s| s.poi);
        let flee_from = if kind == BehaviourKind::Flee { self.flee_from.take().unwrap_or(p.knight.pos) } else { p.knight.pos };
        let mut ctx = self.ctx(p, nav, out);
        let active = match (kind, poi) {
            (BehaviourKind::Explore, Some(poi)) => Active::Explore(Explore::start(&mut ctx, poi)),
            (BehaviourKind::Wander, _) | (BehaviourKind::Explore, None) => Active::Wander(Wander::start(&mut ctx)),
            (BehaviourKind::Idle, _) => Active::Idle(Idle::start(&mut ctx)),
            (BehaviourKind::Nap, _) => Active::Nap(Nap::start(&mut ctx)),
            (BehaviourKind::Watch, _) => Active::Watch(Watch::start(&mut ctx)),
            (BehaviourKind::Battle, _) => Active::Battle(Box::new(Battle::start(&mut ctx))),
            (BehaviourKind::Flee, _) => Active::Flee(Flee::start(&mut ctx, flee_from)),
            (BehaviourKind::Hide, _) => Active::Hide(Hide::start(&mut ctx)),
            (BehaviourKind::Ride, _) => Active::Ride(Ride::start(&mut ctx)),
            (BehaviourKind::KnockedOut, _) => Active::KnockedOut(KnockedOut::start(&mut ctx)),
        };
        log::debug!("brain: {} -> {} ({reason:?})", from.label(), kind.label());
        self.active = active;
        self.since_switch = 0.0;
        self.last_switch = Some(Switch { at: self.now, from, to: kind, reason });
    }

    fn run_active(&mut self, p: &Percepts, nav: &mut dyn Navigator, out: &mut Intent) -> Status {
        let Brain { cfg, mind, rng, rep, interest, senses, active, now, .. } = self;
        let mut ctx = Ctx { p, now: *now, cfg, senses, mind, nav, rng, rep, interest, out };
        active.tick(&mut ctx)
    }

    fn ctx<'a, 'p>(&'a mut self, p: &'a Percepts<'p>, nav: &'a mut dyn Navigator, out: &'a mut Intent) -> Ctx<'a, 'p> {
        Ctx {
            p,
            now: self.now,
            cfg: &self.cfg,
            senses: &self.senses,
            mind: &mut self.mind,
            nav,
            rng: &mut self.rng,
            rep: &mut self.rep,
            interest: &mut self.interest,
            out,
        }
    }

    /// Rank points of interest: cheap interest first, then route-plan the top few.
    fn score_pois(&self, p: &Percepts, nav: &mut dyn Navigator) -> Option<ScoredPoi> {
        let style = self.mind.travel_style();
        let knight = p.knight;
        let speed = knight.params.gait_speed_px(style.gait()).max(1.0);
        let mut ranked: Vec<(Poi, f32)> = self
            .interest
            .candidates(self.now, knight, (p.desktop, p.terrain, p.benches), &self.cfg)
            .into_iter()
            .filter_map(|poi| {
                let at = poi.target.position(p.terrain)?;
                let guess = poi.interest(&self.mind) - self.cfg.travel_cost_per_sec * knight.pos.distance(at) / speed;
                Some((poi, guess))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked
            .into_iter()
            .take(self.cfg.max_planned_pois)
            .filter_map(|(poi, _)| {
                let eta = nav.estimate(knight, poi.target.goal(), style)?;
                let score = poi.interest(&self.mind) - self.cfg.travel_cost_per_sec * eta;
                Some(ScoredPoi { poi, eta, score })
            })
            .max_by(|a, b| a.score.total_cmp(&b.score))
    }

    fn score_behaviours(&self, p: &Percepts) -> Vec<(BehaviourKind, f32)> {
        let m = &self.mind;
        let n = m.needs;
        let e = m.emotions;
        let pers = m.personality;
        let s = &self.senses;
        let cfg = &self.cfg;
        let arousal = m.arousal();
        let tired = m.tiredness();

        let smooth = |lo: f32, hi: f32, x: f32| {
            let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };

        let idle = 0.3 + 0.15 * (1.0 - arousal);
        let wander = (0.12 + 0.25 * n.boredom + 0.1 * pers.vigour) * (0.5 + 0.5 * n.energy);
        let explore = self.best_poi.map_or(0.0, |sp| sp.score.clamp(0.0, 0.95) * (0.6 + 0.4 * n.energy));

        let nap = if self.now < self.nap_ready_at || s.fullscreen {
            0.0
        } else {
            let away = if s.user_away { 0.35 + 0.25 * tired } else { 0.0 };
            // Without a bench it sleeps face-down on the spot: a bit less inviting.
            let spot = if p.benches.is_empty() && !s.user_away { 0.8 } else { 1.0 };
            ((0.95 * smooth(0.5, 0.9, tired) + away) * (1.0 - 0.8 * arousal) * spot).min(0.99)
        };

        let watch = match s.cursor {
            Some(c) if !s.user_away => {
                let parked = if c.near_for > 1.0 && c.still_for > 1.0 { 0.15 } else { 0.0 };
                (0.6 * n.loneliness * (0.5 + pers.sociability) + parked + 0.1 * e.excitement).min(0.9)
            }
            _ => 0.0,
        };

        let b = &cfg.battle;
        let battle = match s.cursor {
            Some(c)
                if cfg.bother_level > 0.0
                    && self.now >= self.battle_ready_at
                    && !s.user_busy
                    && !s.user_away
                    && !s.fullscreen
                    && n.energy > 0.25
                    && e.startle < 0.5
                    && e.hurt < 0.5 =>
            {
                let invite = if c.near_for >= b.park_secs && c.still_for >= b.park_secs {
                    1.0
                } else if c.near_for > 0.3 && c.speed() > 600.0 {
                    0.8 // waving it in the knight's face
                } else {
                    0.25
                };
                let drive = 0.75 * n.mischief + 0.25 * e.excitement + 0.15 * pers.boldness;
                (drive * invite * (0.6 + 0.4 * cfg.bother_level)).min(0.95)
            }
            _ => 0.0,
        };

        let flee = if self.flee_from.is_some() && e.startle > 0.3 { 1.2 } else { 0.0 };
        let hide = if s.fullscreen { 1.3 } else { 0.0 };
        let ride = if s.since_carried(self.now) < 0.6 && p.knight.is_grounded() { 0.65 + 0.2 * e.excitement } else { 0.0 };
        let knocked = if self.knocked_out { 1.25 } else { 0.0 };

        let mut u = vec![
            (BehaviourKind::Idle, idle),
            (BehaviourKind::Wander, wander),
            (BehaviourKind::Explore, explore),
            (BehaviourKind::Nap, nap),
            (BehaviourKind::Watch, watch),
            (BehaviourKind::Battle, battle),
            (BehaviourKind::Flee, flee),
            (BehaviourKind::Hide, hide),
            (BehaviourKind::Ride, ride),
            (BehaviourKind::KnockedOut, knocked),
        ];
        // Stable sort: ties keep declaration order, so selection is deterministic.
        u.sort_by(|a, b| b.1.total_cmp(&a.1));
        u
    }

    fn debug_line(&self) -> String {
        let mut s = String::from(self.active.kind().label());
        let detail = self.active.detail();
        if !detail.is_empty() {
            let _ = write!(s, " [{detail}]");
        }
        let n = self.mind.needs;
        let _ = write!(
            s,
            " | {} E{:.0} B{:.0} L{:.0} M{:.0} |",
            self.mind.dominant_mood(self.senses.user_away),
            n.energy * 100.0,
            n.boredom * 100.0,
            n.loneliness * 100.0,
            n.mischief * 100.0
        );
        for (k, u) in self.utilities.iter().take(3) {
            let _ = write!(s, " {} {u:.2}", k.label());
        }
        s
    }
}
