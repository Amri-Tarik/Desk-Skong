//! Behaviours: small state machines that turn the brain's choice into per-tick intent.
//! Selection (utilities, commitment) lives in [`super::Brain`]; this module only knows
//! how to *do* each behaviour once chosen.
use crate::acts::ActId;
use crate::controller::Side;
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::battle::Battle;
use super::config::BrainConfig;
use super::interest::{InterestMap, Poi, PoiKind, PoiTarget};
use super::navigator::{NavStatus, Navigator};
use super::needs::Mind;
use super::percept::{Percepts, Senses};
use super::repertoire::Repertoire;
use super::rng::Rng;
use super::{Intent, Motion};

/// What the knight is up to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BehaviourKind {
    /// Stand around, fidget with mood-picked acts.
    Idle,
    /// Amble to a random nearby spot.
    Wander,
    /// Travel to a point of interest and inspect it.
    Explore,
    /// Sleep (the "nap" act) until rested or woken.
    Nap,
    /// Go near the cursor and watch it.
    Watch,
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
}

impl BehaviourKind {
    pub const ALL: [BehaviourKind; 10] = [
        BehaviourKind::Idle,
        BehaviourKind::Wander,
        BehaviourKind::Explore,
        BehaviourKind::Nap,
        BehaviourKind::Watch,
        BehaviourKind::Battle,
        BehaviourKind::Flee,
        BehaviourKind::Hide,
        BehaviourKind::Ride,
        BehaviourKind::KnockedOut,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BehaviourKind::Idle => "idle",
            BehaviourKind::Wander => "wander",
            BehaviourKind::Explore => "explore",
            BehaviourKind::Nap => "nap",
            BehaviourKind::Watch => "watch",
            BehaviourKind::Battle => "battle",
            BehaviourKind::Flee => "flee",
            BehaviourKind::Hide => "hide",
            BehaviourKind::Ride => "ride",
            BehaviourKind::KnockedOut => "knocked out",
        }
    }

    /// Exclusive behaviours end themselves; only urgent challengers may interrupt.
    pub fn exclusive(self) -> bool {
        matches!(self, BehaviourKind::Nap | BehaviourKind::Battle | BehaviourKind::Flee | BehaviourKind::Hide | BehaviourKind::KnockedOut)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Running,
    Done,
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
        let ok = self.rep.catalog.get(id).is_some_and(|a| a.startable(self.grounded(), self.senses.at_bench.is_some()));
        if self.p.act.is_none() && ok {
            self.out.act = Some(id);
            self.rep.used(id, self.now);
        }
    }

    /// Pick an act by mood (optionally preferring a tag).
    pub fn pick_act(&mut self, prefer: Option<&str>) -> Option<ActId> {
        let situation = (self.grounded(), self.senses.at_bench.is_some());
        self.rep.pick(self.mind, self.senses.user_away, situation, prefer, self.now, self.rng)
    }

    pub fn face_cursor(&mut self) {
        if let Some(c) = self.senses.cursor {
            self.out.face = Side::of(c.offset.x);
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TravelStep {
    Moving,
    Arrived,
    Failed,
}

/// Navigation towards one goal with a timeout.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Travel {
    pub goal: NavGoal,
    pub style: TravelStyle,
    started: bool,
    elapsed: f32,
    timeout: f32,
}

impl Travel {
    pub fn new(goal: NavGoal, style: TravelStyle, timeout: f32) -> Self {
        Self { goal, style, started: false, elapsed: 0.0, timeout }
    }

    /// Change destination; the next tick re-issues `go`.
    pub fn retarget(&mut self, goal: NavGoal) {
        self.goal = goal;
        self.started = false;
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> TravelStep {
        if !ctx.free_to_move() {
            return TravelStep::Moving;
        }
        if !self.started {
            if !ctx.nav.go(ctx.p.knight, self.goal, self.style) {
                return TravelStep::Failed;
            }
            self.started = true;
        }
        self.elapsed += ctx.dt();
        if self.elapsed > self.timeout {
            ctx.nav.stop();
            return TravelStep::Failed;
        }
        match ctx.nav.status() {
            NavStatus::Arrived => TravelStep::Arrived,
            NavStatus::Failed | NavStatus::Idle => TravelStep::Failed,
            NavStatus::Moving => {
                ctx.out.motion = Motion::Navigate { goal: self.goal, style: self.style };
                TravelStep::Moving
            }
        }
    }
}

/// Travel timeout from a straight-line guess (planner etas are better but optional).
fn timeout_for(ctx: &Ctx, to: Vec2, style: TravelStyle) -> f32 {
    let speed = ctx.p.knight.params.gait_speed_px(style.gait()).max(1.0);
    4.0 + 3.0 * ctx.p.knight.pos.distance(to) / speed
}

// --- Idle ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct Idle {
    left: f32,
    next_act_in: f32,
    act: Option<ActWait>,
}

impl Idle {
    /// Placeholder that finishes at once, so the first tick chooses freely.
    pub fn initial() -> Self {
        Self { left: 0.0, next_act_in: 1.0, act: None }
    }

    pub fn start(ctx: &mut Ctx) -> Self {
        let lazy = 1.0 + ctx.mind.tiredness();
        Self { left: ctx.rng.range(3.0, 8.0) * lazy, next_act_in: ctx.rng.range(0.5, 2.5), act: None }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.left -= ctx.dt();
        if let Some(w) = &mut self.act {
            if w.tick(ctx) {
                self.act = None;
            }
            return Status::Running;
        }
        self.next_act_in -= ctx.dt();
        if self.next_act_in <= 0.0 {
            self.next_act_in = ctx.rng.range(3.0, 6.0);
            if ctx.rng.chance(0.55)
                && let Some(id) = ctx.pick_act(None)
            {
                self.act = Some(ActWait::new(id));
                return Status::Running;
            }
        }
        if ctx.senses.cursor.is_some_and(|c| c.dist() < 400.0) && ctx.p.act.is_none() {
            ctx.face_cursor();
        }
        if self.left <= 0.0 { Status::Done } else { Status::Running }
    }
}

// --- Wander -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct Wander {
    travel: Travel,
}

impl Wander {
    pub fn start(ctx: &mut Ctx) -> Self {
        let k = ctx.p.knight;
        let near_ledges: Vec<(usize, f32)> = ctx
            .p
            .terrain
            .ledges
            .iter()
            .enumerate()
            .filter(|(_, l)| (l.x0 + l.x1) * 0.5 - k.pos.x < 700.0 && k.pos.x - (l.x0 + l.x1) * 0.5 < 700.0 && l.x1 - l.x0 > 60.0)
            .map(|(i, _)| (i, 1.0))
            .collect();
        let target = match (ctx.rng.chance(0.3), ctx.rng.pick_weighted(&near_ledges)) {
            (true, Some(i)) => {
                let l = ctx.p.terrain.ledges[i];
                Vec2::new(ctx.rng.range(l.x0 + 20.0, (l.x1 - 20.0).max(l.x0 + 21.0)), l.y)
            }
            _ => {
                let (lo, hi) = k.support.map_or((k.pos.x - 400.0, k.pos.x + 400.0), |s| (s.x0 + 25.0, s.x1 - 25.0));
                Vec2::new(ctx.rng.range(lo, hi.max(lo + 1.0)), k.pos.y)
            }
        };
        let style = ctx.mind.travel_style();
        Self { travel: Travel::new(NavGoal::Point(target), style, timeout_for(ctx, target, style)) }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        match self.travel.tick(ctx) {
            TravelStep::Moving => Status::Running,
            TravelStep::Arrived | TravelStep::Failed => Status::Done,
        }
    }
}

// --- Explore ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum ExplorePhase {
    Travel(Travel),
    Inspect { act: Option<ActWait>, left: f32 },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Explore {
    pub poi: Poi,
    phase: ExplorePhase,
}

impl Explore {
    pub fn start(ctx: &mut Ctx, poi: Poi) -> Self {
        let style = ctx.mind.travel_style();
        let to = poi.target.position(ctx.p.terrain).unwrap_or(ctx.p.knight.pos);
        Self { poi, phase: ExplorePhase::Travel(Travel::new(poi.target.goal(), style, timeout_for(ctx, to, style))) }
    }

    pub fn style(&self) -> Option<TravelStyle> {
        match self.phase {
            ExplorePhase::Travel(t) => Some(t.style),
            ExplorePhase::Inspect { .. } => None,
        }
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
                        Status::Done
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

    fn arrive(&mut self, ctx: &mut Ctx) {
        let novelty = self.poi.novelty;
        ctx.interest.visit(self.poi.target, ctx.now, ctx.p.desktop);
        ctx.mind.adjust(|n| &mut n.boredom, -(0.15 + 0.45 * novelty));
        let prefer = match self.poi.kind {
            PoiKind::ActiveWindow => {
                // Squatting on the title bar of whatever the user is working on.
                ctx.mind.adjust(|n| &mut n.mischief, -0.35);
                ctx.mind.excite(0.1);
                "playful"
            }
            PoiKind::HighLedge => "bold",
            PoiKind::Corner => "calm",
            PoiKind::Bench => "bench",
            PoiKind::NewWindow | PoiKind::MovingWindow | PoiKind::Unvisited => "curious",
        };
        let act = if self.poi.kind == PoiKind::Bench {
            // `at_bench` lags a tick behind arrival; ActWait retries until it holds.
            ctx.rep.pick(ctx.mind, ctx.senses.user_away, (true, true), Some(prefer), ctx.now, ctx.rng)
        } else {
            ctx.pick_act(Some(prefer))
        }
        .map(ActWait::new);
        self.phase = ExplorePhase::Inspect { act, left: 1.0 };
    }
}

// --- Nap ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum NapPhase {
    /// Walk to the nearest bench first: sitting clips are the bench pose.
    ToBench(Travel),
    /// The "nap" act on a bench; ends on the "woken" signal (a brain reflex) or by itself.
    Bench(Option<ActWait>),
    /// No bench: lie face-down ("bow_down", tag deep_sleep), repeated while sleepy.
    Deep(Option<ActWait>),
}

/// Sleep: bench nap when a bench is around, deep sleep on the spot otherwise.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nap {
    phase: NapPhase,
    woken: bool,
}

impl Nap {
    /// Benches farther than this are not worth the walk when sleepy (px).
    const BENCH_RANGE: f32 = 1600.0;

    pub fn start(ctx: &mut Ctx) -> Self {
        let k = ctx.p.knight;
        let bench = ctx
            .p
            .benches
            .iter()
            .copied()
            .filter(|b| b.distance(k.pos) <= Self::BENCH_RANGE)
            .min_by(|a, b| a.distance(k.pos).total_cmp(&b.distance(k.pos)));
        let phase = match (ctx.senses.at_bench, bench) {
            (Some(_), _) => NapPhase::Bench(ActWait::named(ctx, "nap")),
            (None, Some(b)) => {
                let style = if ctx.mind.tiredness() > 0.7 { TravelStyle::Stroll } else { ctx.mind.travel_style() };
                NapPhase::ToBench(Travel::new(NavGoal::Point(b), style, timeout_for(ctx, b, style)))
            }
            (None, None) => NapPhase::Deep(ActWait::named(ctx, "bow_down")),
        };
        Self { phase, woken: false }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.woken |= ctx.senses.woken;
        match &mut self.phase {
            NapPhase::ToBench(t) => {
                if self.woken {
                    ctx.nav.stop();
                    return Status::Done;
                }
                match t.tick(ctx) {
                    TravelStep::Moving => {}
                    TravelStep::Arrived => self.phase = NapPhase::Bench(ActWait::named(ctx, "nap")),
                    TravelStep::Failed => self.phase = NapPhase::Deep(ActWait::named(ctx, "bow_down")),
                }
                Status::Running
            }
            NapPhase::Bench(act) => {
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
}

// --- KnockedOut ---------------------------------------------------------------------

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

// --- Watch --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct Watch {
    left: f32,
    travel: Option<Travel>,
    goal_at: Vec2,
    looked: bool,
    act: Option<ActWait>,
}

impl Watch {
    const NEAR: f32 = 220.0;

    pub fn start(ctx: &mut Ctx) -> Self {
        Self { left: ctx.rng.range(6.0, 15.0), travel: None, goal_at: Vec2::ZERO, looked: false, act: None }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        self.left -= ctx.dt();
        let Some(c) = ctx.senses.cursor else { return Status::Done };
        if self.left <= 0.0 {
            return Status::Done;
        }
        if let Some(w) = &mut self.act {
            if w.tick(ctx) {
                self.act = None;
            }
            return Status::Running;
        }
        if c.offset.x.abs() > Self::NEAR {
            let goal = NavGoal::Point(c.pos);
            let style = ctx.mind.travel_style();
            match &mut self.travel {
                Some(t) if self.goal_at.distance(c.pos) > 150.0 => {
                    t.retarget(goal);
                    self.goal_at = c.pos;
                }
                Some(_) => {}
                None => {
                    self.travel = Some(Travel::new(goal, style, 20.0));
                    self.goal_at = c.pos;
                }
            }
            if let Some(t) = &mut self.travel
                && t.tick(ctx) != TravelStep::Moving
            {
                self.travel = None;
                if c.offset.x.abs() > 2.0 * Self::NEAR {
                    return Status::Done; // unreachable / too far: lose interest
                }
            }
            return Status::Running;
        }
        if self.travel.take().is_some() {
            ctx.nav.stop();
        }
        ctx.face_cursor();
        if !self.looked && ctx.p.act.is_none() {
            let name = if c.offset.y < -180.0 {
                Some("look_up")
            } else if c.offset.y > 180.0 {
                Some("look_down")
            } else if ctx.rng.chance(0.3 * ctx.mind.personality.sociability) {
                Some("stare_at_user")
            } else {
                None
            };
            self.looked = true;
            self.act = name.and_then(|n| ActWait::named(ctx, n));
        }
        Status::Running
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
                    self.phase = FleePhase::Run(Travel::new(NavGoal::Point(to), TravelStyle::Hurry, timeout_for(ctx, to, TravelStyle::Hurry)));
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
    act: Option<ActWait>,
    next_act_in: f32,
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
            Travel::new(NavGoal::Point(to), style, timeout_for(ctx, to, style))
        });
        Self { travel, act: None, next_act_in: 1.0 }
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
        if let Some(w) = &mut self.act {
            if w.tick(ctx) {
                self.act = None;
            }
            return Status::Running;
        }
        self.next_act_in -= ctx.dt();
        if self.next_act_in <= 0.0 {
            self.next_act_in = ctx.rng.range(8.0, 20.0);
            self.act = if ctx.mind.tiredness() > 0.5 { ActWait::named(ctx, "bow_down") } else { ctx.pick_act(Some("calm")).map(ActWait::new) };
        }
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
    Wander(Wander),
    Explore(Explore),
    Nap(Nap),
    Watch(Watch),
    Battle(Box<Battle>),
    Flee(Flee),
    Hide(Hide),
    Ride(Ride),
    KnockedOut(KnockedOut),
}

impl Active {
    pub fn kind(&self) -> BehaviourKind {
        match self {
            Active::Idle(_) => BehaviourKind::Idle,
            Active::Wander(_) => BehaviourKind::Wander,
            Active::Explore(_) => BehaviourKind::Explore,
            Active::Nap(_) => BehaviourKind::Nap,
            Active::Watch(_) => BehaviourKind::Watch,
            Active::Battle(_) => BehaviourKind::Battle,
            Active::Flee(_) => BehaviourKind::Flee,
            Active::Hide(_) => BehaviourKind::Hide,
            Active::Ride(_) => BehaviourKind::Ride,
            Active::KnockedOut(_) => BehaviourKind::KnockedOut,
        }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        match self {
            Active::Idle(b) => b.tick(ctx),
            Active::Wander(b) => b.tick(ctx),
            Active::Explore(b) => b.tick(ctx),
            Active::Nap(b) => b.tick(ctx),
            Active::Watch(b) => b.tick(ctx),
            Active::Battle(b) => b.tick(ctx),
            Active::Flee(b) => b.tick(ctx),
            Active::Hide(b) => b.tick(ctx),
            Active::Ride(b) => b.tick(ctx),
            Active::KnockedOut(b) => b.tick(ctx),
        }
    }

    /// Short state description for the debug overlay.
    pub fn detail(&self) -> String {
        match self {
            Active::Explore(e) => match e.style() {
                Some(s) => format!("{} ({s:?})", e.poi.kind.label()),
                None => format!("{} (inspect)", e.poi.kind.label()),
            },
            Active::Battle(b) => b.detail(),
            _ => String::new(),
        }
    }
}
