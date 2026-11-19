//! Behaviours: small state machines that turn the brain's choice into per-tick intent.
//! Selection (utilities, commitment) lives in [`super::Brain`]; this module only knows
//! how to *do* each behaviour once chosen.
//!
//! Every act a behaviour starts has a reason that the behaviour can name (it shows up in
//! `Intent::debug`). Plain standing between acts is short (a [`Beat`] gap) and flagged as
//! `Intent::waiting`.
use crate::acts::ActId;
use crate::controller::Side;
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::activity::ActivityTracker;
use super::battle::Battle;
use super::companion::Companion;
use super::config::BrainConfig;
use super::interest::{InterestMap, Poi, PoiKind, PoiTarget};
use super::navigator::{NavStatus, Navigator};
use super::needs::Mind;
use super::percept::{Percepts, Senses};
use super::repertoire::Repertoire;
use super::rng::Rng;
use super::spots::{self, View};
use super::watch::WatchSpot;
use super::death::Dead;
use super::shade_hunt::{Loot, ShadeHunt};
use super::stakes::StakesMind;
use super::{Intent, Motion};

/// What the knight is up to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BehaviourKind {
    /// Nothing to do and nobody around: short fidgets, sits if he can.
    Idle,
    /// Default with a user present: stay near the cursor, react to it.
    Companion,
    /// Watch a region of the screen that is changing.
    Watch,
    /// Travel to a point of interest (window, ledge) and do the fitting thing there.
    Explore,
    /// Tired or settling: find a seat and sit.
    Rest,
    /// Sleep (bench / perch nap, or face-down deep sleep).
    Nap,
    /// Challenge and fight the cursor.
    Battle,
    /// Startled: jump, then run away from the threat.
    Flee,
    /// A fullscreen app is up: retreat to a corner and keep quiet.
    Hide,
    /// Enjoy riding a window that is being moved.
    Ride,
    /// Knocked flat by a huge fall.
    KnockedOut,
    /// Hurt: nurse it, then focus to heal.
    Recover,
    /// Died: death act, gone, respawn at a bench / safe ledge.
    Dead,
    /// Go fight the Shade to get the Geo back.
    ShadeHunt,
    /// Walk over loose Geo to pick it up.
    Loot,
}

impl BehaviourKind {
    pub const ALL: [BehaviourKind; 15] = [
        BehaviourKind::Idle,
        BehaviourKind::Companion,
        BehaviourKind::Watch,
        BehaviourKind::Explore,
        BehaviourKind::Rest,
        BehaviourKind::Nap,
        BehaviourKind::Battle,
        BehaviourKind::Flee,
        BehaviourKind::Hide,
        BehaviourKind::Ride,
        BehaviourKind::KnockedOut,
        BehaviourKind::Recover,
        BehaviourKind::Dead,
        BehaviourKind::ShadeHunt,
        BehaviourKind::Loot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BehaviourKind::Idle => "idle",
            BehaviourKind::Companion => "companion",
            BehaviourKind::Watch => "watch",
            BehaviourKind::Explore => "explore",
            BehaviourKind::Rest => "rest",
            BehaviourKind::Nap => "nap",
            BehaviourKind::Battle => "battle",
            BehaviourKind::Flee => "flee",
            BehaviourKind::Hide => "hide",
            BehaviourKind::Ride => "ride",
            BehaviourKind::KnockedOut => "knocked out",
            BehaviourKind::Recover => "recover",
            BehaviourKind::Dead => "dead",
            BehaviourKind::ShadeHunt => "shade hunt",
            BehaviourKind::Loot => "loot",
        }
    }

    /// Exclusive behaviours end themselves; only urgent challengers may interrupt.
    pub fn exclusive(self) -> bool {
        matches!(self, BehaviourKind::Nap | BehaviourKind::Battle | BehaviourKind::Flee | BehaviourKind::Hide | BehaviourKind::KnockedOut | BehaviourKind::Dead | BehaviourKind::ShadeHunt | BehaviourKind::Loot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Running,
    Done,
    /// Ended without achieving its goal (no seat, unreachable target, ...). The brain
    /// puts the behaviour on a short cooldown so it is not re-picked at once.
    Failed,
}

/// Everything a behaviour may read or poke during its tick.
pub(crate) struct Ctx<'a, 'p> {
    pub p: &'a Percepts<'p>,
    pub now: f32,
    pub cfg: &'a BrainConfig,
    pub senses: &'a Senses,
    pub mind: &'a mut Mind,
    pub nav: &'a mut dyn Navigator,
    pub rng: &'a mut Rng,
    pub rep: &'a mut Repertoire,
    pub interest: &'a mut InterestMap,
    pub spots: &'a mut ActivityTracker,
    pub stakes: &'a mut StakesMind,
    pub out: &'a mut Intent,
}

impl Ctx<'_, '_> {
    pub fn dt(&self) -> f32 {
        self.p.dt
    }

    pub fn grounded(&self) -> bool {
        self.p.knight.is_grounded()
    }

    pub fn act_id(&self, name: &str) -> Option<ActId> {
        self.rep.catalog.id(name)
    }

    /// May the knight move this tick? Cancels an interruptible act in the way.
    pub fn free_to_move(&mut self) -> bool {
        match self.p.act {
            None => true,
            Some(a) if a.interruptible => {
                self.out.cancel_act = true;
                true
            }
            Some(_) => false,
        }
    }

    /// Ask the app to start `id` (only when nothing plays and requirements hold).
    pub fn request_act(&mut self, id: ActId) {
        let ok = self.rep.catalog.get(id).is_some_and(|a| a.startable(self.grounded(), self.senses.can_sit()));
        if self.p.act.is_none() && ok {
            self.out.act = Some(id);
            self.rep.used(id, self.now);
        }
    }

    /// A plain fidget (the only acts with a random-pick weight).
    pub fn fidget(&mut self) -> Option<ActWait> {
        let situation = (self.grounded(), self.senses.can_sit());
        self.rep.pick(self.mind, self.senses.user_away, situation, Some("fidget"), self.now, self.rng).map(ActWait::new)
    }

    /// The seated act that fits: "rest" when tired, otherwise "lean_back".
    pub fn seated_act(&mut self) -> Option<ActWait> {
        let name = if self.mind.tiredness() > 0.5 { "rest" } else { "lean_back" };
        ActWait::named(self, name)
    }

    pub fn face_toward(&mut self, x: f32) {
        self.out.face = Side::of(x - self.p.knight.pos.x).or(self.out.face);
    }

    pub fn face_cursor(&mut self) {
        if let Some(c) = self.senses.cursor {
            self.out.face = Side::of(c.offset.x);
        }
    }

    pub fn range(&mut self, r: [f32; 2]) -> f32 {
        self.rng.range(r[0], r[1].max(r[0]))
    }
}

/// Plays one act to completion. Gives up if it cannot start within [`ActWait::PATIENCE`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActWait {
    pub id: ActId,
    started: bool,
    waited: f32,
}

impl ActWait {
    const PATIENCE: f32 = 1.5;

    pub fn new(id: ActId) -> Self {
        Self { id, started: false, waited: 0.0 }
    }

    pub fn named(ctx: &Ctx, name: &str) -> Option<Self> {
        ctx.act_id(name).map(Self::new)
    }

    /// `true` once the act has finished (or could not start).
    pub fn tick(&mut self, ctx: &mut Ctx) -> bool {
        match ctx.p.act {
            Some(a) if a.id == self.id => {
                self.started = true;
                false
            }
            _ if self.started => true,
            other => {
                self.waited += ctx.dt();
                if self.waited > Self::PATIENCE {
                    return true;
                }
                match other {
                    None => ctx.request_act(self.id),
                    Some(a) if a.interruptible => ctx.out.cancel_act = true,
                    Some(_) => {}
                }
                false
            }
        }
    }
}

/// Rhythm of purposeful acts separated by short gaps of plain standing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Beat {
    gap: f32,
    act: Option<ActWait>,
    /// Name of the act playing (for the reason line).
    pub doing: Option<ActId>,
}

impl Beat {
    pub fn new(first_gap: f32) -> Self {
        Self { gap: first_gap, act: None, doing: None }
    }

    /// Run the current act or count down the gap; `true` when it is time to pick the next
    /// act. `gap` is the range of the next pause.
    pub fn tick(&mut self, ctx: &mut Ctx, gap: [f32; 2]) -> bool {
        if let Some(w) = &mut self.act {
            if w.tick(ctx) {
                self.act = None;
                self.doing = None;
                self.gap = ctx.range(gap);
            }
            return false;
        }
        self.gap -= ctx.dt();
        if self.gap > 0.0 {
            ctx.out.waiting = true;
            return false;
        }
        true
    }

    /// Play this act (if any); otherwise wait another gap.
    pub fn play(&mut self, ctx: &mut Ctx, act: Option<ActWait>, gap: [f32; 2]) {
        match act {
            Some(w) => {
                self.doing = Some(w.id);
                self.act = Some(w);
            }
            None => self.gap = ctx.range(gap),
        }
    }

    pub fn busy(&self) -> bool {
        self.act.is_some()
    }

    pub fn doing_name<'c>(&self, ctx: &'c Ctx) -> Option<&'c str> {
        self.doing.and_then(|id| ctx.rep.catalog.get(id)).map(|a| a.name.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TravelStep {
    Moving,
    Arrived,
    Failed,
}

/// Navigation towards one goal with a timeout. Fails at once when the brain notices the
/// knight pushing into a wall (the brain also blocks the goal for a while).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Travel {
    pub goal: NavGoal,
    pub style: TravelStyle,
    started: bool,
    elapsed: f32,
    timeout: f32,
    /// Arrival must be on the goal point (seats, vantage points), within this many px.
    exact: Option<f32>,
}

impl Travel {
    pub fn new(goal: NavGoal, style: TravelStyle, timeout: f32) -> Self {
        Self { goal, style, started: false, elapsed: 0.0, timeout, exact: None }
    }

    /// Travel with a timeout estimated from the straight-line distance.
    pub fn to(ctx: &Ctx, to: Vec2, style: TravelStyle) -> Self {
        Self::new(NavGoal::Point(to), style, timeout_for(ctx, to, style))
    }

    /// Travel that only counts as arrived when the knight stands on `to` (within `tol` px).
    /// A follower that "arrives" next to it (e.g. under a ledge it cannot mount) fails,
    /// and the target is blocked for a while.
    pub fn exact(ctx: &Ctx, to: Vec2, style: TravelStyle, tol: f32) -> Self {
        Self { exact: Some(tol), ..Self::to(ctx, to, style) }
    }

    /// Change destination; the next tick re-issues `go`.
    pub fn retarget(&mut self, goal: NavGoal) {
        self.goal = goal;
        self.started = false;
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> TravelStep {
        if ctx.senses.stuck {
            return TravelStep::Failed;
        }
        if !ctx.free_to_move() {
            return TravelStep::Moving;
        }
        if !self.started {
            if !ctx.nav.go(ctx.p.knight, self.goal, self.style) {
                block(ctx, self.goal, 30.0);
                return TravelStep::Failed;
            }
            self.started = true;
        }
        self.elapsed += ctx.dt();
        if self.elapsed > self.timeout {
            ctx.nav.stop();
            block(ctx, self.goal, 30.0);
            return TravelStep::Failed;
        }
        match ctx.nav.status() {
            NavStatus::Arrived => match (self.exact, self.goal) {
                (Some(tol), NavGoal::Point(p)) if !on_point(ctx.p.knight, p, tol) => {
                    log::debug!("brain: arrived at ({:.0},{:.0}) instead of ({:.0},{:.0}); target blocked", ctx.p.knight.pos.x, ctx.p.knight.pos.y, p.x, p.y);
                    block(ctx, self.goal, FAILED_TARGET_SECS);
                    TravelStep::Failed
                }
                _ => TravelStep::Arrived,
            },
            NavStatus::Failed | NavStatus::Idle => TravelStep::Failed,
            NavStatus::Moving => {
                ctx.out.motion = Motion::Navigate { goal: self.goal, style: self.style };
                TravelStep::Moving
            }
        }
    }
}

/// How long a target that could not be reached (or stood on) stays blocked.
pub(crate) const FAILED_TARGET_SECS: f32 = 60.0;

/// Is the knight standing on `p` (grounded, same height, within `tol` px)?
pub(crate) fn on_point(k: &crate::controller::Character, p: Vec2, tol: f32) -> bool {
    k.is_grounded() && (k.pos.x - p.x).abs() <= tol && (k.pos.y - p.y).abs() <= 3.0
}

/// Remember a goal as unreachable for `secs`.
pub(crate) fn block(ctx: &mut Ctx, goal: NavGoal, secs: f32) {
    let target = match goal {
        NavGoal::Point(p) => PoiTarget::Spot(p),
        NavGoal::OnWindow(id) => PoiTarget::Window(id),
    };
    ctx.interest.block(target, ctx.now + secs);
}

/// Travel timeout from a straight-line guess (planner etas are better but optional).
fn timeout_for(ctx: &Ctx, to: Vec2, style: TravelStyle) -> f32 {
    let speed = ctx.p.knight.params.gait_speed_px(style.gait()).max(1.0);
    4.0 + 3.0 * ctx.p.knight.pos.distance(to) / speed
}

/// Travel style by mood and distance: short hops are strolls, long trips at least a run.
pub(crate) fn style_for(ctx: &Ctx, to: Vec2) -> TravelStyle {
    let d = ctx.p.knight.pos.distance(to);
    match ctx.mind.travel_style() {
        TravelStyle::Stroll if d > 1200.0 && ctx.mind.needs.energy > 0.3 => TravelStyle::Normal,
        TravelStyle::Hurry if d < 250.0 => TravelStyle::Normal,
        s => s,
    }
}

/// Arrival tolerance on a seat (px): benches need the knight right at them.
pub(crate) const SEAT_TOL: f32 = 6.0;
/// At most this many seat candidates are route-planned per search.
const SEAT_PLANS: usize = 4;

/// Nearest seat (bench or perch) around `near` that a route actually gets the knight *onto*,
/// with the gentlest style that does (stroll, else a normal run). Seats whose route ends
/// elsewhere (e.g. a content ledge a stroll can't mount: the planner stops underneath)
/// are blocked for a while so they aren't tried again and again.
pub(crate) fn find_seat(ctx: &mut Ctx, near: Vec2, range: f32) -> Option<(Vec2, TravelStyle)> {
    let k = ctx.p.knight;
    let candidates: Vec<Vec2> = spots::seats(ctx.p.terrain, k, near, ctx.p.benches, range)
        .into_iter()
        .filter(|s| !ctx.interest.is_blocked(PoiTarget::Spot(*s)) && !on_point(k, *s, SEAT_TOL))
        .take(SEAT_PLANS)
        .collect();
    let styles: &[TravelStyle] = if ctx.mind.needs.energy > 0.15 { &[TravelStyle::Stroll, TravelStyle::Normal] } else { &[TravelStyle::Stroll] };
    for s in candidates {
        if let Some(&style) = styles.iter().find(|&&st| ctx.nav.reaches(k, s, st, SEAT_TOL)) {
            return Some((s, style));
        }
        ctx.interest.block(PoiTarget::Spot(s), ctx.now + FAILED_TARGET_SECS);
    }
    None
}

// --- Idle ---------------------------------------------------------------------------

/// Nobody around and nothing to look at: sit if possible, else the odd fidget (or a
/// lantern walk at night).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Idle {
    beat: Beat,
}

impl Idle {
    /// Placeholder before the first decision.
    pub fn placeholder() -> Self {
        Self { beat: Beat::new(0.0) }
    }

    pub fn start(ctx: &mut Ctx) -> Self {
        let gap = ctx.range(ctx.cfg.idle_gap);
        Self { beat: Beat::new(gap) }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let gap = ctx.cfg.idle_gap;
        if self.beat.tick(ctx, gap) {
            let act = if ctx.senses.can_sit() {
                ctx.seated_act()
            } else if ctx.senses.dark && ctx.rng.chance(0.3) {
                ActWait::named(ctx, "lantern_patrol")
            } else {
                ctx.fidget()
            };
            self.beat.play(ctx, act, gap);
        }
        Status::Running
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        match self.beat.doing_name(ctx) {
            Some("lantern_patrol") => "idle: dark, lantern walk".into(),
            Some(n) => format!("idle: {n}"),
            None => "idle: nothing going on".into(),
        }
    }
}

// --- Explore ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum ExplorePhase {
    Travel(Travel),
    Inspect { act: Option<ActWait>, left: f32 },
}

/// Go to a point of interest and do the thing that fits it there.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Explore {
    pub poi: Poi,
    phase: ExplorePhase,
    what: &'static str,
}

impl Explore {
    pub fn start(ctx: &mut Ctx, poi: Poi) -> Self {
        let to = poi.target.position(ctx.p.terrain).unwrap_or(ctx.p.knight.pos);
        let style = style_for(ctx, to);
        let travel = Travel::new(poi.target.goal(), style, timeout_for(ctx, to, style));
        Self { poi, phase: ExplorePhase::Travel(travel), what: "" }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        match &mut self.phase {
            ExplorePhase::Travel(t) => {
                if let PoiTarget::Window(id) = self.poi.target
                    && ctx.p.desktop.window(id).is_none()
                {
                    ctx.nav.stop();
                    return Status::Done;
                }
                match t.tick(ctx) {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Failed => {
                        ctx.interest.block(self.poi.target, ctx.now + 60.0);
                        Status::Failed
                    }
                    TravelStep::Arrived => {
                        self.arrive(ctx);
                        Status::Running
                    }
                }
            }
            ExplorePhase::Inspect { act, left } => {
                *left -= ctx.dt();
                let act_done = act.as_mut().is_none_or(|w| w.tick(ctx));
                if act_done && *left <= 0.0 { Status::Done } else { Status::Running }
            }
        }
    }

    /// The act that fits the place (each with its reason).
    fn arrive(&mut self, ctx: &mut Ctx) {
        let novelty = self.poi.novelty;
        ctx.interest.visit(self.poi.target, ctx.now, ctx.p.desktop);
        ctx.mind.adjust(|n| &mut n.boredom, -(0.15 + 0.45 * novelty));
        let small = match self.poi.target {
            PoiTarget::Window(id) => ctx.p.desktop.window(id).is_some_and(|w| w.rect.width() * w.rect.height() <= 420 * 320),
            PoiTarget::Spot(_) => false,
        };
        let (what, name) = match self.poi.kind {
            PoiKind::NewWindow if small => ("found a small new thing", "pick_up_trinket"),
            PoiKind::NewWindow | PoiKind::Unvisited if ctx.senses.can_sit() => ("mapping a new window", "read_map"),
            PoiKind::NewWindow | PoiKind::Unvisited => ("mapping a new window", "map_stroll"),
            PoiKind::ActiveWindow => {
                // Squatting on the title bar of whatever the user is working on.
                ctx.mind.adjust(|n| &mut n.mischief, -0.35);
                ctx.mind.excite(0.1);
                if ctx.senses.can_sit() { ("squatting on your window", "lean_back") } else { ("squatting on your window", "stare_at_user") }
            }
            PoiKind::MovingWindow => {
                let eye = ctx.p.knight.body().center();
                let rect = match self.poi.target {
                    PoiTarget::Window(id) => ctx.p.desktop.window(id).map(|w| w.rect),
                    PoiTarget::Spot(_) => None,
                };
                ("eyeing a moving window", rect.map_or("look_down", |r| View::of(r, r.to_f().center(), eye).act()))
            }
            PoiKind::HighLedge => ("looking down from up high", "look_down"),
        };
        self.what = what;
        let act = ActWait::named(ctx, name);
        self.phase = ExplorePhase::Inspect { act, left: 0.5 };
    }

    pub fn reason(&self) -> String {
        match self.phase {
            ExplorePhase::Travel(t) => format!("explore {} ({:?})", self.poi.kind.label(), t.style),
            ExplorePhase::Inspect { .. } => format!("explore {}: {}", self.poi.kind.label(), self.what),
        }
    }
}

// --- Rest ---------------------------------------------------------------------------

/// Tired or settling in: walk to the nearest seat (bench or perch) and sit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rest {
    travel: Option<Travel>,
    beat: Beat,
}

impl Rest {
    pub fn start(ctx: &mut Ctx) -> Self {
        let travel = if ctx.senses.can_sit() {
            None
        } else {
            let near = ctx.p.knight.pos;
            find_seat(ctx, near, 900.0).map(|(s, style)| Travel::exact(ctx, s, style, SEAT_TOL))
        };
        Self { travel, beat: Beat::new(0.2) }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        if let Some(t) = &mut self.travel {
            match t.tick(ctx) {
                TravelStep::Moving => return Status::Running,
                TravelStep::Arrived => self.travel = None,
                TravelStep::Failed => {
                    // Could not get onto the seat: sit right here if possible, else give up.
                    self.travel = None;
                    if !ctx.senses.can_sit() {
                        return Status::Failed;
                    }
                }
            }
        }
        if ctx.mind.tiredness() < 0.2 && !self.beat.busy() {
            return Status::Done;
        }
        if self.beat.tick(ctx, [0.5, 1.5]) {
            if !ctx.senses.can_sit() {
                // Nowhere to sit here (no seat found, or the seat moved / closed).
                let here = ctx.p.knight.pos;
                ctx.interest.block(PoiTarget::Spot(here), ctx.now + FAILED_TARGET_SECS);
                return Status::Failed;
            }
            let act = ctx.seated_act();
            self.beat.play(ctx, act, [0.5, 1.5]);
        }
        Status::Running
    }

    pub fn reason(&self) -> String {
        if self.travel.is_some() { "rest: walking to a seat".into() } else { "rest: sitting".into() }
    }
}

// --- Nap ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum NapPhase {
    /// Walk to the nearest seat first: sleeping clips are the seated pose.
    ToSeat(Travel),
    /// The "nap" act; ends on the "woken" signal (a brain reflex) or by itself.
    Seated(Option<ActWait>),
    /// No seat: lie face-down ("bow_down", tag deep_sleep), repeated while sleepy.
    Deep(Option<ActWait>),
}

/// Sleep: seated nap when a bench / perch is around, deep sleep on the spot otherwise.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nap {
    phase: NapPhase,
    woken: bool,
}

impl Nap {
    /// Seats farther than this are not worth the walk when sleepy (px).
    const SEAT_RANGE: f32 = 1600.0;

    pub fn start(ctx: &mut Ctx) -> Self {
        let phase = if ctx.senses.can_sit() {
            NapPhase::Seated(ActWait::named(ctx, "nap"))
        } else {
            let k = ctx.p.knight;
            match find_seat(ctx, k.pos, Self::SEAT_RANGE) {
                Some((s, style)) => NapPhase::ToSeat(Travel::exact(ctx, s, style, SEAT_TOL)),
                None => NapPhase::Deep(ActWait::named(ctx, "bow_down")),
            }
        };
        Self { phase, woken: false }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.woken |= ctx.senses.woken;
        match &mut self.phase {
            NapPhase::ToSeat(t) => {
                if self.woken {
                    ctx.nav.stop();
                    return Status::Done;
                }
                match t.tick(ctx) {
                    TravelStep::Moving => {}
                    TravelStep::Arrived => self.phase = NapPhase::Seated(ActWait::named(ctx, "nap")),
                    TravelStep::Failed => self.phase = NapPhase::Deep(ActWait::named(ctx, "bow_down")),
                }
                Status::Running
            }
            NapPhase::Seated(act) => {
                ctx.nav.stop();
                if act.as_mut().is_none_or(|w| w.tick(ctx)) { Status::Done } else { Status::Running }
            }
            NapPhase::Deep(act) => {
                ctx.nav.stop();
                if !act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    return Status::Running;
                }
                let sleepy = ctx.mind.tiredness() > 0.3 || ctx.senses.user_away;
                if self.woken || !sleepy {
                    return Status::Done;
                }
                *act = ActWait::named(ctx, "bow_down");
                if act.is_some() { Status::Running } else { Status::Done }
            }
        }
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        let why = if ctx.senses.user_away { "user away" } else { "tired" };
        match self.phase {
            NapPhase::ToSeat(_) => format!("nap ({why}): walking to a seat"),
            NapPhase::Seated(_) => format!("nap ({why}): asleep on a seat"),
            NapPhase::Deep(_) => format!("nap ({why}): deep sleep, no seat"),
        }
    }
}

// --- KnockedOut / Recover -----------------------------------------------------------

/// Flat on the floor after a huge fall ("bow_down", tag knocked_out).
#[derive(Clone, Copy, Debug)]
pub(crate) struct KnockedOut {
    act: Option<ActWait>,
}

impl KnockedOut {
    pub fn start(ctx: &mut Ctx) -> Self {
        ctx.mind.hurt(0.4);
        Self { act: ActWait::named(ctx, "bow_down") }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        ctx.nav.stop();
        if self.act.as_mut().is_none_or(|w| w.tick(ctx)) { Status::Done } else { Status::Running }
    }
}

/// After being hurt: `hurt_idle` for a bit, then `focus` to heal.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Recover {
    sore: Option<ActWait>,
    focus: Option<ActWait>,
}

impl Recover {
    pub fn start(ctx: &mut Ctx) -> Self {
        Self { sore: ActWait::named(ctx, "hurt_idle"), focus: ActWait::named(ctx, "focus") }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        ctx.nav.stop();
        if let Some(w) = &mut self.sore {
            if w.tick(ctx) {
                self.sore = None;
            }
            return Status::Running;
        }
        if let Some(w) = &mut self.focus {
            if w.tick(ctx) {
                self.focus = None;
                ctx.mind.emotions.hurt *= 0.3; // healed
            }
            return Status::Running;
        }
        Status::Done
    }

    pub fn reason(&self) -> String {
        if self.sore.is_some() { "recover: hurt".into() } else { "recover: focusing to heal".into() }
    }
}

// --- Flee ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum FleePhase {
    /// Wait to land, then the "startled" act.
    Startle(Option<ActWait>),
    Run(Travel),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Flee {
    from: Vec2,
    phase: FleePhase,
}

impl Flee {
    pub fn start(_ctx: &mut Ctx, from: Vec2) -> Self {
        Self { from, phase: FleePhase::Startle(None) }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        match &mut self.phase {
            FleePhase::Startle(act) => {
                if !ctx.grounded() {
                    return Status::Running;
                }
                if act.is_none() {
                    *act = ActWait::named(ctx, "startled");
                }
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    let k = ctx.p.knight;
                    let away = match Side::of(k.pos.x - self.from.x) {
                        Some(s) => s.sign(),
                        None if ctx.rng.chance(0.5) => 1.0,
                        None => -1.0,
                    };
                    let b = ctx.p.terrain.bounds;
                    let x = (k.pos.x + away * 380.0).clamp(b.left + 40.0, (b.right - 40.0).max(b.left + 41.0));
                    let to = Vec2::new(x, k.pos.y);
                    self.phase = FleePhase::Run(Travel::to(ctx, to, TravelStyle::Hurry));
                }
                Status::Running
            }
            FleePhase::Run(t) => match t.tick(ctx) {
                TravelStep::Moving => Status::Running,
                TravelStep::Arrived | TravelStep::Failed => Status::Done,
            },
        }
    }
}

// --- Hide ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hide {
    travel: Option<Travel>,
    beat: Beat,
}

impl Hide {
    pub fn start(ctx: &mut Ctx) -> Self {
        let k = ctx.p.knight;
        let desk = ctx.p.desktop;
        let mon = desk.monitor_at(k.pos.x as i32, (k.pos.y - 1.0) as i32).or(desk.primary());
        let travel = mon.map(|m| {
            let left = Vec2::new(m.work.left as f32 + 30.0, m.work.bottom as f32);
            let right = Vec2::new(m.work.right as f32 - 30.0, m.work.bottom as f32);
            let to = if k.pos.distance(left) <= k.pos.distance(right) { left } else { right };
            let style = if ctx.mind.emotions.startle > 0.3 { TravelStyle::Hurry } else { TravelStyle::Normal };
            Travel::to(ctx, to, style)
        });
        Self { travel, beat: Beat::new(1.0) }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        if !ctx.senses.fullscreen {
            ctx.nav.stop();
            return Status::Done;
        }
        if let Some(t) = &mut self.travel {
            if t.tick(ctx) == TravelStep::Moving {
                return Status::Running;
            }
            self.travel = None;
        }
        if self.beat.tick(ctx, [6.0, 12.0]) {
            let act = if ctx.mind.tiredness() > 0.5 {
                ActWait::named(ctx, "bow_down")
            } else if ctx.senses.can_sit() {
                ctx.seated_act()
            } else {
                None
            };
            self.beat.play(ctx, act, [6.0, 12.0]);
        }
        ctx.out.waiting = false; // keeping out of the way is the point
        Status::Running
    }
}

// --- Ride ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ride {
    act: Option<ActWait>,
}

impl Ride {
    pub fn start(ctx: &mut Ctx) -> Self {
        ctx.mind.excite(0.2);
        Self { act: None }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        if ctx.senses.since_carried(ctx.now) > 1.5 {
            return Status::Done;
        }
        ctx.nav.stop();
        if self.act.as_mut().is_none_or(|w| w.tick(ctx)) {
            self.act = ActWait::named(ctx, "windswept");
        }
        Status::Running
    }
}

// --- Dispatch -----------------------------------------------------------------------

/// The running behaviour with its state.
#[derive(Clone, Debug)]
pub(crate) enum Active {
    Idle(Idle),
    Companion(Box<Companion>),
    Watch(WatchSpot),
    Explore(Explore),
    Rest(Rest),
    Nap(Nap),
    Battle(Box<Battle>),
    Flee(Flee),
    Hide(Hide),
    Ride(Ride),
    KnockedOut(KnockedOut),
    Recover(Recover),
    Dead(Dead),
    ShadeHunt(Box<ShadeHunt>),
    Loot(Loot),
}

impl Active {
    pub fn kind(&self) -> BehaviourKind {
        match self {
            Active::Idle(_) => BehaviourKind::Idle,
            Active::Companion(_) => BehaviourKind::Companion,
            Active::Watch(_) => BehaviourKind::Watch,
            Active::Explore(_) => BehaviourKind::Explore,
            Active::Rest(_) => BehaviourKind::Rest,
            Active::Nap(_) => BehaviourKind::Nap,
            Active::Battle(_) => BehaviourKind::Battle,
            Active::Flee(_) => BehaviourKind::Flee,
            Active::Hide(_) => BehaviourKind::Hide,
            Active::Ride(_) => BehaviourKind::Ride,
            Active::KnockedOut(_) => BehaviourKind::KnockedOut,
            Active::Recover(_) => BehaviourKind::Recover,
            Active::Dead(_) => BehaviourKind::Dead,
            Active::ShadeHunt(_) => BehaviourKind::ShadeHunt,
            Active::Loot(_) => BehaviourKind::Loot,
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        match self {
            Active::Idle(b) => b.tick(ctx),
            Active::Companion(b) => b.tick(ctx),
            Active::Watch(b) => b.tick(ctx),
            Active::Explore(b) => b.tick(ctx),
            Active::Rest(b) => b.tick(ctx),
            Active::Nap(b) => b.tick(ctx),
            Active::Battle(b) => b.tick(ctx),
            Active::Flee(b) => b.tick(ctx),
            Active::Hide(b) => b.tick(ctx),
            Active::Ride(b) => b.tick(ctx),
            Active::KnockedOut(b) => b.tick(ctx),
            Active::Recover(b) => b.tick(ctx),
            Active::Dead(b) => b.tick(ctx),
            Active::ShadeHunt(b) => b.tick(ctx),
            Active::Loot(b) => b.tick(ctx),
        }
    }

    /// Why the knight is doing what he does, for the debug overlay.
    pub fn reason(&self, ctx: &Ctx) -> String {
        match self {
            Active::Idle(b) => b.reason(ctx),
            Active::Companion(b) => b.reason(ctx),
            Active::Watch(b) => b.reason(ctx),
            Active::Explore(b) => b.reason(),
            Active::Rest(b) => b.reason(),
            Active::Nap(b) => b.reason(ctx),
            Active::Battle(b) => format!("battle: {}", b.detail()),
            Active::Flee(_) => "flee: startled".into(),
            Active::Hide(_) => "hide: fullscreen app in front".into(),
            Active::Ride(_) => "ride: window moving".into(),
            Active::KnockedOut(_) => "knocked out: huge fall".into(),
            Active::Recover(b) => b.reason(),
            Active::Dead(b) => b.reason(),
            Active::ShadeHunt(b) => b.reason(),
            Active::Loot(b) => b.reason(),
        }
    }
}
