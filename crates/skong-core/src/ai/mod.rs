//! The pet brain: a curious little knight that keeps you company, watches whatever is
//! happening on your screen, explores new windows, sits down when tired, naps when you
//! are away, and — if allowed — picks fights with your cursor.
//!
//! ```text
//!  Percepts ──► Senses (diffs, cursor velocity, user presence, stuck detection)
//!                 │
//!                 ├─► reflexes   (pokes, window closed underfoot, wake-ups, wall pushing)
//!                 ├─► Mind       (needs: energy/boredom/loneliness/mischief,
//!                 │               emotions: startle/annoyance/excitement/hurt)
//!                 ├─► ActivityTracker (screen-activity spots with identity + satiation)
//!                 ├─► InterestMap (window POIs with decaying novelty + visit memory)
//!                 ▼
//!   utilities per BehaviourKind ──► selection with commitment + hysteresis
//!                 ▼
//!   Active behaviour state machine ──► Intent (motion / act / signal / attack / reason)
//! ```
//!
//! **Every act has a reason.** Acts are not drawn from a random pool: each behaviour
//! starts the act that fits its situation (gaze at the activity it watches, look up at a
//! cursor above, map a never-visited window, sit when settling, …). Only a couple of
//! `fidget` acts keep a random weight, used rarely between purposeful actions. The current
//! reason heads `Intent::debug`.
//!
//! * **Deterministic**: given the seed and the same percept sequence, the same intents
//!   come out (seeded xorshift, ordered maps, brain-internal clock).
//! * **Navigation is injected** through [`Navigator`] so the brain is testable without
//!   the planner; [`GraphNavigator`] adapts [`crate::nav`].
//! * **Acts are referenced by name** through an [`ActCatalog`]; missing acts degrade
//!   gracefully (the behaviour just skips that flourish).
mod activity;
mod battle;
mod behaviours;
mod companion;
mod config;
mod greet;
mod death;
mod interest;
mod navigator;
mod needs;
mod percept;
mod places;
mod repertoire;
pub mod rng;
mod shade_hunt;
mod spots;
mod stakes;
mod watch;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod stakes_tests;

use std::collections::BTreeMap;
use std::fmt::Write as _;

pub use activity::{Activity, ActivitySpot, ActivityTracker, QUIET, TrackedSpot};
pub use battle::{BattleScore, Outcome};
pub use behaviours::{BehaviourKind, Tier};
pub use config::{BattleConfig, BoredomConfig, BrainConfig, CompanionConfig, NeedRates, Personality, TierConfig, WatchConfig};
pub use greet::{STAND_OFF_UNITS, Visitor, VisitorState};
pub use interest::{InterestMap, Poi, PoiKey, PoiKind, PoiTarget, ScoredPoi};
pub use navigator::{GraphNavigator, NavStatus, Navigator, Reach};
pub use needs::{Emotions, MOOD_TAGS, Mind, Needs};
pub use percept::{ClosedWindow, Cursor, CursorSense, DesktopChanges, Percepts, Poke, RunningAct, STUCK_SECS, Senses, UserActivity};
pub use places::{DiscoveryEvent, KnownPlace, PlaceId, PlaceKind, PlaceMemory};
pub use repertoire::{ActCatalog, ActInfo};
pub use spots::{Vantage, View};
pub use stakes::{Lethality, ShadeSense, StakeEvent, StakesConfig, StakesSense};

use crate::acts::{ActId, Requirement};
use crate::controller::{MoveEvent, MoveInput, Side};
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use battle::Battle;
use behaviours::{Active, Ctx, Explore, Flee, Hide, Idle, KnockedOut, Nap, Recover, Rest, Ride, Status};
use companion::Companion;
use needs::Activity as Exertion;
use repertoire::Repertoire;
use rng::Rng;
use watch::WatchSpot;
use death::Dead;
use shade_hunt::{Loot, ShadeHunt};
use stakes::StakesMind;

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
    /// End the running act gracefully: replace it with this short exit act (`get_up` from a
    /// seat, `look_up_end`, ...) when another behaviour takes over. Exit acts are
    /// uninterruptible and only a fraction of a second long.
    pub exit_act: Option<ActId>,
    /// Signal for the running act (e.g. "woken").
    pub signal: Option<&'static str>,
    pub attack: Option<Attack>,
    /// Death / respawn / Shade / Geo / spell events for the app.
    pub stakes: Vec<StakeEvent>,
    /// Face this way (when standing still).
    pub face: Option<Side>,
    pub behaviour: Option<BehaviourKind>,
    /// Why the knight does what he does, e.g. "watch activity @ (812,430) 0.8 [...]".
    pub reason: String,
    /// Plain standing with nothing in mind (a short gap between purposeful actions).
    pub waiting: bool,
    /// Standing beside this visitor: start the conversation.
    pub greet: Option<u32>,
    /// The dream nail just requested is aimed at this visitor (their inner thoughts).
    pub dream_visitor: Option<u32>,
    /// "reason | mood + needs | top-3 utilities", for an overlay.
    pub debug: String,
    /// Discovery traffic for the app (explored a new window, used / struck a found place).
    pub discovery: Option<DiscoveryEvent>,
    /// Bored for a while with nothing interesting around: please make something happen.
    /// Rate-limited (`BoredomConfig::event_gap`); never while the user types, is away, or a
    /// fullscreen app is up.
    pub request: Option<WorldEvent>,
}

/// Something the bored knight asks the world for (see [`Intent::request`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldEvent {
    /// Rain some Geo for him to collect.
    GeoShower,
    /// Invite an NPC visitor.
    Visitor,
    /// A playful chase of the cursor. The brain handles this one itself (it invites a
    /// battle); the app may show a cue.
    CursorChase,
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
    /// The oscillation guard stopped a ping-pong between targets.
    Calming,
    /// A higher priority tier took over (e.g. loot appeared while he sat on a ledge).
    Preempted,
}

/// Record of one behaviour switch (for tests and the overlay).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Switch {
    pub at: f32,
    pub from: BehaviourKind,
    pub to: BehaviourKind,
    pub reason: SwitchReason,
}

/// The best activity spot to watch right now.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SpotChoice {
    id: u32,
    vantage: Vantage,
    interest: f32,
    score: f32,
}

/// A behaviour that ends without success is not picked again for this long (s).
const FAIL_COOLDOWN: f32 = 15.0;
/// Oscillation guard: this many direction reversals within `OSC_WINDOW` s ...
const OSC_REVERSALS: usize = 3;
const OSC_WINDOW: f32 = 6.0;
/// ... make the knight calm down (idle only) for this long.
const CALM_SECS: f32 = 4.0;

pub struct Brain {
    cfg: BrainConfig,
    mind: Mind,
    rng: Rng,
    rep: Repertoire,
    interest: InterestMap,
    spots: ActivityTracker,
    senses: Senses,
    active: Active,
    /// Brain clock (sum of dt).
    now: f32,
    since_switch: f32,
    think_in: f32,
    utilities: Vec<(BehaviourKind, f32)>,
    best_poi: Option<ScoredPoi>,
    best_spot: Option<SpotChoice>,
    battle_ready_at: f32,
    nap_ready_at: f32,
    /// Something scary happened here; flee from it.
    flee_from: Option<Vec2>,
    /// A huge fall knocked the knight flat.
    knocked_out: bool,
    recent_clicks: Vec<f32>,
    last_switch: Option<Switch>,
    last_motion: Motion,
    exertion: f32,
    /// Times the knight caught himself pushing into a wall.
    stuck_count: u32,
    visitors: greet::VisitorMemory,
    stakes: StakesMind,
    /// Behaviours that just failed, and when they may be picked again.
    cooldown: BTreeMap<BehaviourKind, f32>,
    /// Oscillation guard: times the travel direction flipped, current direction, the
    /// last navigation goals, and the end of the current calm-down.
    reversals: Vec<f32>,
    travel_dir: f32,
    recent_goals: Vec<NavGoal>,
    calm_until: f32,
    /// Seconds bored (above `BoredomConfig::bored`) in a row.
    bored_for: f32,
    /// Earliest time for the next requested world event.
    next_event_at: f32,
    /// A requested cursor chase invites a battle until then.
    chase_until: f32,
    /// Highest tier with an eligible behaviour at the last evaluation.
    top_tier: Tier,
    oscillations: u32,
}

impl Brain {
    pub fn new(cfg: BrainConfig, catalog: ActCatalog, seed: u32) -> Self {
        let personality = cfg.personality.unwrap_or_else(|| Personality::from_seed(seed));
        Self {
            mind: Mind::new(personality),
            rng: Rng::new(seed),
            rep: Repertoire::with_repeat_window(catalog, cfg.boredom.repeat_window),
            interest: InterestMap::default(),
            spots: ActivityTracker::default(),
            senses: Senses::default(),
            active: Active::Idle(Idle::placeholder()),
            now: 0.0,
            since_switch: f32::INFINITY,
            think_in: 0.0,
            utilities: Vec::new(),
            best_poi: None,
            best_spot: None,
            battle_ready_at: 0.0,
            nap_ready_at: 0.0,
            flee_from: None,
            knocked_out: false,
            recent_clicks: Vec::new(),
            last_switch: None,
            last_motion: Motion::Hold,
            exertion: 0.0,
            stuck_count: 0,
            visitors: Default::default(),
            stakes: StakesMind::new(&cfg.stakes),
            cooldown: BTreeMap::new(),
            reversals: Vec::new(),
            travel_dir: 0.0,
            recent_goals: Vec::new(),
            calm_until: f32::NEG_INFINITY,
            bored_for: 0.0,
            next_event_at: 0.0,
            chase_until: f32::NEG_INFINITY,
            top_tier: Tier::Ambient,
            oscillations: 0,
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

    /// Tracked screen-activity spots.
    pub fn activity(&self) -> &ActivityTracker {
        &self.spots
    }

    /// Utilities from the last evaluation, best first.
    pub fn utilities(&self) -> &[(BehaviourKind, f32)] {
        &self.utilities
    }

    pub fn last_switch(&self) -> Option<Switch> {
        self.last_switch
    }

    /// Times the oscillation guard had to calm the knight down.
    pub fn oscillations(&self) -> u32 {
        self.oscillations
    }

    /// Times the knight caught himself pushing into a wall.
    pub fn stuck_count(&self) -> u32 {
        self.stuck_count
    }

    /// The NPC visitors on screen (call before [`Self::tick`]; empty when none).
    pub fn see_visitors(&mut self, visitors: Vec<Visitor>) {
        for v in self.visitors.arrivals(&visitors) {
            self.mind.excite(0.6 * v.excitement);
        }
        self.senses.visitors = visitors;
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

    /// The activity spot being watched, if any.
    pub fn watching(&self) -> Option<&TrackedSpot> {
        match &self.active {
            Active::Watch(w) => self.spots.get(w.spot),
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
        self.spots.update(p.activity, dt);

        let mut out = Intent::default();
        self.reflexes(p, &mut out);
        self.unstick(nav);
        if !self.senses.held {
            self.guard_oscillation(p, nav, &mut out);
        }
        self.interest.observe(self.now, dt, &self.senses, p.desktop, &self.cfg);

        let asleep = self.asleep(p);
        let seated = !asleep && p.act.and_then(|a| self.rep.catalog.get(a.id)).is_some_and(|a| a.requires == Requirement::Seat);
        let with_user = !self.senses.user_away && self.senses.cursor.is_some_and(|c| c.dist() < 400.0);
        self.mind.update(dt, Exertion { asleep, seated, exertion: self.exertion, with_user }, self.senses.user_away, &self.cfg);

        if self.senses.held {
            nav.stop();
            out.behaviour = Some(self.active.kind());
            out.reason = "held by the user".into();
            out.debug = format!("held | {}", self.mind.dominant_mood(self.senses.user_away));
            self.last_motion = Motion::Hold;
            return out;
        }

        let first = self.since_switch.is_infinite();
        if self.think_in <= 0.0 || first {
            self.think_in = self.cfg.think_interval;
            self.think(p, nav, &mut out, first);
        }
        // A finished behaviour is replaced right away (at most once per tick).
        for _ in 0..2 {
            out.waiting = false;
            let status = self.run_active(p, nav, &mut out);
            if status == Status::Running {
                break;
            }
            if status == Status::Failed {
                // Don't re-pick what just failed (e.g. the same unreachable seat) at once.
                let kind = self.active.kind();
                log::debug!("brain: {} failed; cooling down for {FAIL_COOLDOWN} s", kind.label());
                self.cooldown.insert(kind, self.now + FAIL_COOLDOWN);
            }
            self.think(p, nav, &mut out, true);
        }
        if p.act.is_some() || out.act.is_some() || !matches!(out.motion, Motion::Hold) {
            out.waiting = false;
        }

        if self.senses.stuck {
            // Turn away from the wall (behaviours may have faced the old goal).
            out.face = Side::of(-p.last_input.x).or(Some(p.knight.facing.opposite()));
        }
        self.exertion = match out.motion {
            Motion::Hold => 0.0,
            Motion::Direct(_) => 1.0,
            Motion::Navigate { style: TravelStyle::Hurry, .. } => 0.7,
            Motion::Navigate { style: TravelStyle::Normal, .. } => 0.3,
            Motion::Navigate { style: TravelStyle::Stroll, .. } => 0.1,
        };
        self.maybe_request_event(p, &mut out);
        self.last_motion = out.motion;
        out.behaviour = Some(self.active.kind());
        out.reason = self.reason(p, nav);
        if self.now < self.calm_until {
            out.reason = format!("calming down (was dithering) | {}", out.reason);
        }
        out.debug = self.debug_line(&out.reason);
        out
    }

    /// Bored for a while with nothing interesting around: ask the world for an event
    /// (rate-limited, never while the user types, is away, or a fullscreen app is up).
    fn maybe_request_event(&mut self, p: &Percepts, out: &mut Intent) {
        let b = self.cfg.boredom;
        let s = &self.senses;
        self.bored_for = if self.mind.needs.boredom >= b.bored { self.bored_for + p.dt.max(0.0) } else { 0.0 };
        let interesting = self.best_poi.is_some_and(|sp| sp.score > 0.2)
            || self.best_spot.is_some()
            || self.visit_utility() > 0.2
            || !self.stakes.sense.coins.is_empty()
            || matches!(self.active.kind().tier(), Tier::Engagement | Tier::Urgent);
        if interesting || self.bored_for < b.event_after || self.now < self.next_event_at || s.user_busy || s.user_away || s.fullscreen {
            return;
        }
        let chase_ok = self.cfg.bother_level > 0.0 && self.mind.needs.mischief > 0.3 && s.cursor.is_some();
        let options = [(WorldEvent::GeoShower, 1.0), (WorldEvent::Visitor, 1.0), (WorldEvent::CursorChase, if chase_ok { 1.0 } else { 0.0 })];
        let Some(ev) = self.rng.pick_weighted(&options) else { return };
        log::info!("brain: bored stiff with nothing going on; asking the world for {ev:?}");
        if ev == WorldEvent::CursorChase {
            self.chase_until = self.now + 20.0;
            self.mind.excite(0.3);
        }
        out.request = Some(ev);
        self.next_event_at = self.now + b.event_gap;
        self.bored_for = 0.0;
    }

    /// Sleeping: the seated "nap" act, or deep sleep ("bow_down") during a nap.
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
                    if vel.length() > 1200.0 {
                        self.mind.excite(0.4 * self.mind.personality.boldness);
                        self.mind.startle(0.4);
                        self.mind.hurt(0.4);
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
        if std::mem::take(&mut self.visitors.knock) {
            woken |= napping || matches!(self.active, Active::Nap(_)); // a visitor! up you get
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

    /// Anti-dither guard: the knight reversing direction ≥ [`OSC_REVERSALS`] times within
    /// [`OSC_WINDOW`] s while travelling, without reaching where he was going, means the
    /// brain is ping-ponging between targets. Stop, mark the recent targets bad and calm
    /// down (idle only) for [`CALM_SECS`].
    fn guard_oscillation(&mut self, p: &Percepts, nav: &mut dyn Navigator, out: &mut Intent) {
        let k = p.knight;
        if let Motion::Navigate { goal, .. } = self.last_motion {
            if self.recent_goals.last() != Some(&goal) {
                self.recent_goals.push(goal);
                if self.recent_goals.len() > 6 {
                    self.recent_goals.remove(0);
                }
            }
            if nav.status() == NavStatus::Arrived {
                let reached = match goal {
                    NavGoal::Point(g) => behaviours::on_point(k, g, 30.0),
                    NavGoal::OnWindow(id) => k.support.and_then(|s| s.owner) == Some(id),
                };
                if reached {
                    self.reversals.clear();
                    self.recent_goals.clear();
                }
            }
            if k.vel.x.abs() > 20.0 {
                let dir = k.vel.x.signum();
                if self.travel_dir != 0.0 && dir != self.travel_dir {
                    self.reversals.push(self.now);
                }
                self.travel_dir = dir;
            }
        }
        self.reversals.retain(|t| self.now - *t <= OSC_WINDOW);
        if self.reversals.len() < OSC_REVERSALS {
            return;
        }
        log::debug!(
            "brain: dithering ({} reversals in {OSC_WINDOW} s, {} behaviour); calming down, blocking {} recent targets",
            self.reversals.len(),
            self.active.kind().label(),
            self.recent_goals.len()
        );
        nav.stop();
        for goal in self.recent_goals.drain(..) {
            let target = match goal {
                NavGoal::Point(g) => PoiTarget::Spot(g),
                NavGoal::OnWindow(id) => PoiTarget::Window(id),
            };
            self.interest.block(target, self.now + behaviours::FAILED_TARGET_SECS);
        }
        self.reversals.clear();
        self.travel_dir = 0.0;
        self.calm_until = self.now + CALM_SECS;
        self.oscillations += 1;
        self.switch_to(BehaviourKind::Idle, SwitchReason::Calming, p, nav, out);
    }

    /// Walking into a wall: pressing a direction without moving, or sliding down a wall on
    /// a stroll (strolls never wall-jump). Stop, give up that goal for a while, turn away.
    fn unstick(&mut self, nav: &mut dyn Navigator) {
        let unintended_wall = self.senses.wall_slide
            && match self.last_motion {
                Motion::Navigate { style, .. } => style == TravelStyle::Stroll,
                Motion::Direct(_) => !matches!(self.active, Active::Battle(_)),
                Motion::Hold => false,
            };
        let moving = !matches!(self.last_motion, Motion::Hold);
        if !(moving && (self.senses.stuck || unintended_wall)) {
            self.senses.stuck = false;
            return;
        }
        self.senses.stuck = true;
        self.stuck_count += 1;
        if let Motion::Navigate { goal, .. } = self.last_motion {
            nav.stop();
            let target = match goal {
                NavGoal::Point(pt) => PoiTarget::Spot(pt),
                NavGoal::OnWindow(id) => PoiTarget::Window(id),
            };
            self.interest.block(target, self.now + 20.0);
        }
        log::debug!("brain: stuck against a wall ({:?}); giving up that goal", self.last_motion);
    }

    /// Score every behaviour and switch if warranted. Tiers first: the best eligible
    /// behaviour of the highest non-empty tier is the candidate, and a higher tier than the
    /// current one takes over at once. Within a tier, commitment and hysteresis apply.
    fn think(&mut self, p: &Percepts, nav: &mut dyn Navigator, out: &mut Intent, finished: bool) {
        self.best_poi = self.score_pois(p, nav);
        self.best_spot = self.score_spots(p, nav, finished);
        self.score_places(p, nav);
        self.utilities = self.score_behaviours(p);
        let eligible = self.cfg.tiers.eligible;
        let tiered: Vec<(BehaviourKind, f32, Tier)> = self.utilities.iter().map(|&(k, u)| (k, u, self.tier_at(k, u, p))).collect();
        let top = tiered.iter().filter(|t| t.1 > eligible).map(|t| t.2).max();
        // Utilities are sorted best first, so the first of the top tier is its best.
        let Some(&(best, best_u, best_tier)) = tiered.iter().find(|t| Some(t.2) == top).or(tiered.first()) else { return };
        let current = self.active.kind();
        let (cur_u, cur_tier) = tiered.iter().find(|t| t.0 == current).map_or((0.0, Tier::Ambient), |t| (t.1, t.2));
        // A current behaviour that is no longer eligible holds no tier — except a running
        // Urgent one (flee, hide…), whose trigger is spent once it starts: it keeps its tier
        // until it finishes.
        let cur_tier = if current.tier() == Tier::Urgent {
            Tier::Urgent
        } else if cur_u > eligible {
            cur_tier
        } else {
            Tier::Ambient
        };
        self.top_tier = top.unwrap_or(Tier::Ambient);

        let reason = if finished {
            Some(SwitchReason::Finished)
        } else if best == current {
            None
        } else if best_tier > cur_tier {
            Some(SwitchReason::Preempted)
        } else if best_tier < cur_tier {
            None
        } else if best_u >= self.cfg.urgent && best_u > cur_u {
            Some(SwitchReason::Urgent)
        } else if !current.exclusive()
            && (self.since_switch >= self.cfg.min_commit || best == BehaviourKind::Greet) // a visitor: drop everything
            && best_u > cur_u + self.cfg.hysteresis
        {
            Some(SwitchReason::Outscored)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.switch_to(best, reason, p, nav, out);
        }
    }

    /// Tier at utility `u`: high tiers need a minimum score, otherwise it competes as Ambient.
    fn tier_at(&self, kind: BehaviourKind, u: f32, p: &Percepts) -> Tier {
        let t = self.tier_of(kind, p);
        let min = match t {
            Tier::Engagement => self.cfg.tiers.engagement_min,
            Tier::Needs => self.cfg.tiers.needs_min,
            _ => 0.0,
        };
        if u >= min { t } else { Tier::Ambient }
    }

    /// The tier `kind` competes in right now (its base tier refined by context).
    fn tier_of(&self, kind: BehaviourKind, p: &Percepts) -> Tier {
        let t = &self.cfg.tiers;
        let critical = self.mind.needs.energy < t.critical_energy;
        match kind {
            // Resting is a pastime unless he's exhausted / badly hurt.
            BehaviourKind::Rest | BehaviourKind::Nap if !critical => Tier::Ambient,
            BehaviourKind::Recover if self.mind.emotions.hurt < t.critical_hurt => Tier::Ambient,
            // An exhausted (or sleeping) knight lets far prizes wait; a coin next to him still
            // beats sitting. A Shade hunt the user asked for stays an engagement.
            BehaviourKind::Loot if critical || self.asleep(p) => {
                let k = p.knight.pos;
                let near = self.stakes.sense.coins.iter().any(|c| c.distance(k) <= t.tired_loot_reach);
                if near && !self.asleep(p) { Tier::Engagement } else { Tier::Ambient }
            }
            BehaviourKind::ShadeHunt if (critical || self.asleep(p)) && !self.stakes.force_hunt => Tier::Ambient,
            _ => kind.tier(),
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
        if let Active::Greet(g) = &self.active
            && g.failed
        {
            self.visitors.snub(g.id, self.now + 90.0);
        }
        let visitor = self.visitors.target(&self.senses.visitors, p.knight.pos, self.now).cloned();
        nav.stop();
        if reason == SwitchReason::Preempted {
            // A deliberate change of plan, not dithering.
            self.reversals.clear();
            self.travel_dir = 0.0;
            self.recent_goals.clear();
        }
        // End a running (interruptible) act gracefully rather than snapping out of it.
        if let Some(a) = p.act.filter(|a| a.interruptible)
            && let Some(exit) = self.rep.catalog.get(a.id).and_then(behaviours::exit_act_for)
        {
            out.exit_act = self.rep.catalog.id(exit);
        }
        let poi = self.best_poi.map(|s| s.poi);
        let spot = self.best_spot;
        let place = self.interest.places.best;
        let flee_from = if kind == BehaviourKind::Flee { self.flee_from.take().unwrap_or(p.knight.pos) } else { p.knight.pos };
        let death = if kind == BehaviourKind::Dead { self.stakes.pending_death.take() } else { None };
        let mut ctx = self.ctx(p, nav, out);
        let active = match kind {
            BehaviourKind::Explore => match poi {
                Some(poi) => Active::Explore(Explore::start(&mut ctx, poi)),
                None => Active::Idle(Idle::start(&mut ctx)),
            },
            BehaviourKind::Watch => match spot {
                Some(s) => Active::Watch(WatchSpot::start(&mut ctx, s.id, s.vantage, s.interest)),
                None => Active::Idle(Idle::start(&mut ctx)),
            },
            BehaviourKind::Idle => Active::Idle(Idle::start(&mut ctx)),
            BehaviourKind::Companion => Active::Companion(Box::new(Companion::start(&mut ctx))),
            BehaviourKind::Rest => Active::Rest(Rest::start(&mut ctx)),
            BehaviourKind::Nap => Active::Nap(Nap::start(&mut ctx)),
            BehaviourKind::Battle => Active::Battle(Box::new(Battle::start(&mut ctx))),
            BehaviourKind::Flee => Active::Flee(Flee::start(&mut ctx, flee_from)),
            BehaviourKind::Hide => Active::Hide(Hide::start(&mut ctx)),
            BehaviourKind::Ride => Active::Ride(Ride::start(&mut ctx)),
            BehaviourKind::KnockedOut => Active::KnockedOut(KnockedOut::start(&mut ctx)),
            BehaviourKind::Recover => Active::Recover(Recover::start(&mut ctx)),
            BehaviourKind::Greet => match &visitor {
                Some(v) => Active::Greet(Box::new(greet::Greet::start(&mut ctx, v))),
                None => Active::Idle(Idle::start(&mut ctx)),
            },
            BehaviourKind::Dead => {
                let (k, cracked) = death.unwrap_or_default();
                Active::Dead(Dead::start(&mut ctx, k, cracked))
            }
            BehaviourKind::ShadeHunt => Active::ShadeHunt(Box::new(ShadeHunt::start(&mut ctx))),
            BehaviourKind::Loot => Active::Loot(Loot::start(&mut ctx)),
            BehaviourKind::Visit => match place {
                Some(c) => Active::Visit(places::Visit::start(&mut ctx, c)),
                None => Active::Idle(Idle::start(&mut ctx)),
            },
        };
        let kind = active.kind();
        log::debug!("brain: {} -> {} ({reason:?})", from.label(), kind.label());
        self.active = active;
        self.since_switch = 0.0;
        self.last_switch = Some(Switch { at: self.now, from, to: kind, reason });
    }

    fn run_active(&mut self, p: &Percepts, nav: &mut dyn Navigator, out: &mut Intent) -> Status {
        let Brain { cfg, mind, rng, rep, interest, spots, senses, active, now, stakes, .. } = self;
        let mut ctx = Ctx { p, now: *now, cfg, senses, mind, nav, rng, rep, interest, spots, stakes, out };
        active.tick(&mut ctx)
    }

    fn reason(&mut self, p: &Percepts, nav: &mut dyn Navigator) -> String {
        let mut scratch = Intent::default();
        let Brain { cfg, mind, rng, rep, interest, spots, senses, active, now, stakes, .. } = self;
        let ctx = Ctx { p, now: *now, cfg, senses, mind, nav, rng, rep, interest, spots, stakes, out: &mut scratch };
        active.reason(&ctx)
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
            spots: &mut self.spots,
            stakes: &mut self.stakes,
            out,
        }
    }

    /// Rank window points of interest: cheap interest first, then route-plan the top few.
    fn score_pois(&self, p: &Percepts, nav: &mut dyn Navigator) -> Option<ScoredPoi> {
        let style = self.mind.travel_style();
        let knight = p.knight;
        let speed = knight.params.gait_speed_px(style.gait()).max(1.0);
        let mut ranked: Vec<(Poi, f32)> = self
            .interest
            .candidates(self.now, knight, (p.desktop, p.terrain), &self.cfg)
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

    /// Rank active screen spots: interest = intensity × curiosity × (1 − satiation), minus
    /// travel to the best vantage point.
    fn score_spots(&self, p: &Percepts, nav: &mut dyn Navigator, finished: bool) -> Option<SpotChoice> {
        let knight = p.knight;
        let m = &self.mind;
        let watching = match &self.active {
            Active::Watch(w) if !finished => Some(w.spot),
            _ => None,
        };
        let curious = 0.6 + 0.6 * m.needs.boredom * (0.5 + m.personality.curiosity);
        let style = m.travel_style();
        let speed = knight.params.gait_speed_px(style.gait()).max(1.0);
        let mut ranked: Vec<(SpotChoice, f32)> = self
            .spots
            .spots()
            .iter()
            .filter(|t| t.active() && t.intensity >= self.cfg.watch.min_intensity)
            .filter_map(|t| {
                // The spot being watched is judged without its satiation: the watch ends
                // itself once satisfied, while other spots must clearly beat it.
                let sat = if Some(t.id) == watching { 0.0 } else { t.satiation };
                if sat >= 0.95 {
                    return None;
                }
                let interest = (t.intensity * curious * (1.0 - sat)).min(1.0);
                let vantage = spots::vantage(p.terrain, t.rect, knight.pos)?;
                if self.interest.is_blocked(PoiTarget::Spot(vantage.point)) {
                    return None;
                }
                let guess = interest - self.cfg.travel_cost_per_sec * knight.pos.distance(vantage.point) / speed;
                Some((SpotChoice { id: t.id, vantage, interest, score: guess }, guess))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked
            .into_iter()
            .take(2)
            .filter_map(|(mut c, _)| {
                let here = c.vantage.point.distance(knight.pos) < 30.0;
                // The route must end at the vantage point, not merely near it.
                let eta = if here {
                    0.0
                } else {
                    let r = nav.reach(knight, NavGoal::Point(c.vantage.point), style)?;
                    if !r.lands_on(c.vantage.point, watch::VANTAGE_TOL) {
                        return None;
                    }
                    r.eta
                };
                c.score = c.interest - self.cfg.travel_cost_per_sec * eta;
                Some(c)
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

        let idle = 0.2;
        // 0 = content, 1 = bored stiff: pastimes near the user lose to anything new.
        let bored = smooth(cfg.boredom.bored, 1.0, n.boredom);
        let companion = match s.cursor {
            Some(_) if !s.user_away && !s.fullscreen => ((0.42 + 0.15 * n.loneliness + 0.1 * (pers.sociability - 0.5)) * (1.0 - 0.7 * bored)).max(0.25),
            _ => 0.0,
        };
        let watch = self.best_spot.map_or(0.0, |c| (c.score.clamp(0.0, 0.95) * (0.7 + 0.3 * n.energy) + 0.3 * bored).min(0.95));
        let explore = self.best_poi.map_or(0.0, |sp| (sp.score.clamp(0.0, 0.9) * (0.6 + 0.4 * n.energy) + 0.35 * bored).min(0.9));

        let rest = if s.fullscreen {
            0.0
        } else {
            let settle = if s.user_busy { 0.12 } else { 0.0 } + if s.user_away { 0.25 } else { 0.0 };
            let restless = if n.energy < cfg.tiers.critical_energy { 1.0 } else { 1.0 - 0.8 * bored };
            ((0.75 * smooth(0.35, 0.7, tired) + settle) * (1.0 - 0.7 * arousal) * restless).min(0.85)
        };
        let nap = if self.now < self.nap_ready_at || s.fullscreen {
            0.0
        } else {
            let away = if s.user_away { 0.35 + 0.25 * tired } else { 0.0 };
            ((0.95 * smooth(0.7, 0.92, tired) + away) * (1.0 - 0.8 * arousal)).min(0.99)
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
                    && e.hurt < 0.35 =>
            {
                let invite = if self.now < self.chase_until || (c.near_for >= b.park_secs && c.still_for >= b.park_secs) {
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
        let recover = if e.hurt > 0.35 { (0.5 + 0.4 * e.hurt).min(0.9) } else { 0.0 };
        // A visitor! Drop (almost) everything and go say hello.
        let greet = match self.visitors.target(&s.visitors, p.knight.pos, self.now) {
            // Mid-conversation nothing else (but real emergencies) may cut in.
            Some(v) if v.state == VisitorState::Talking => 0.99,
            Some(v) if !s.fullscreen => (0.86 + 0.08 * v.excitement + 0.04 * e.excitement).min(0.97),
            _ => 0.0,
        };
        let visit = self.visit_utility();
        let visit = if visit > 0.0 { (visit + 0.25 * bored).min(1.1) } else { 0.0 };

        let mut u = vec![
            (BehaviourKind::Idle, idle),
            (BehaviourKind::Companion, companion),
            (BehaviourKind::Watch, watch),
            (BehaviourKind::Explore, explore),
            (BehaviourKind::Rest, rest),
            (BehaviourKind::Nap, nap),
            (BehaviourKind::Battle, battle),
            (BehaviourKind::Flee, flee),
            (BehaviourKind::Hide, hide),
            (BehaviourKind::Ride, ride),
            (BehaviourKind::KnockedOut, knocked),
            (BehaviourKind::Recover, recover),
            (BehaviourKind::Greet, greet),
            (BehaviourKind::Visit, visit),
        ];
        u.extend(self.stakes_utilities(p));
        let calm = self.now < self.calm_until;
        for (kind, v) in &mut u {
            let cooling = self.cooldown.get(kind).is_some_and(|t| self.now < *t);
            if *v < cfg.urgent && (cooling || (calm && *kind != BehaviourKind::Idle)) {
                *v = 0.0;
            }
        }
        // Stable sort: ties keep declaration order, so selection is deterministic.
        u.sort_by(|a, b| b.1.total_cmp(&a.1));
        u
    }

    fn debug_line(&self, reason: &str) -> String {
        let mut s = String::from(reason);
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
