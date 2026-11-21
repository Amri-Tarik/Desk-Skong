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
use super::places::{DiscoveryEvent, Visit, favourite_bench};
use super::repertoire::{ActInfo, Repertoire};
use super::rng::Rng;
use super::spots::{self, View};
use super::watch::WatchSpot;
use super::death::Dead;
use super::shade_hunt::{Loot, ShadeHunt};
use super::stakes::StakesMind;
use super::{Intent, Motion};

/// Priority tier of a behaviour. The best eligible behaviour of the highest non-empty
/// tier wins, and a higher tier preempts a lower one at once (no commitment wait);
/// within a tier the usual utility / commitment / hysteresis rules apply.
/// Ordered lowest to highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Pastimes: keeping company, watching, exploring, visiting, sitting, fidgets.
    Ambient,
    /// A genuinely critical need (exhausted, badly hurt).
    Needs,
    /// Something worth dropping everything for: loot, the Shade, a fight, an NPC.
    Engagement,
    /// Safety and fate: fleeing, hiding, knocked out, dying.
    Urgent,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Ambient => "ambient",
            Tier::Needs => "needs",
            Tier::Engagement => "engagement",
            Tier::Urgent => "urgent",
        }
    }
}

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
    /// The window under him moved (he fell off it / it slid away): shake it off, windswept.
    Ride,
    /// Knocked flat by a huge fall.
    KnockedOut,
    /// Hurt: nurse it, then focus to heal.
    Recover,
    /// A visitor (NPC) showed up: hurry over and talk.
    Greet,
    /// A teaching visitor (Sly) runs a lesson: watch, practise the drills, take the notes.
    Train,
    /// Died: death act, gone, respawn at a bench / safe ledge.
    Dead,
    /// Show off around the cursor (hops, loops, dash-bys): mischief without a fight.
    Pester,
    /// Go fight the Shade to get the Geo back.
    ShadeHunt,
    /// Walk over loose Geo to pick it up.
    Loot,
    /// Use a place found in a window (bench, geo deposit).
    Visit,
    /// Walk over to a shiny on the ground and pick it up.
    Shiny,
    /// Go and look at text the user highlighted (opt-in; only its rectangle is known).
    Highlight,
    /// No route to a prize: plant a dream gate by it and warp there.
    DreamGate,
    /// Fight the enemies that wandered in (or spar with a visitor who challenged him).
    Fight,
    /// A gauntlet's ceremonies: ready himself at the announcement, cheer at the end.
    Gauntlet,
    /// A visitor's calm scene after the talk: sit with them a while (maps, the view, a song).
    Company,
    /// Something to do on his own (`ai::pastime`): the training dummy, a lumafly, the view
    /// from a high window edge, the stars at night.
    Pastime,
    /// A visitor scene: Hornet's race, watching the Troupe Master's visit, lingering by a
    /// scarlet flame (see `ai::scene`).
    Scene,
    /// Shaken about while held: he lands and reels, dazed (see `ai::handling`).
    Dizzy,
}

impl BehaviourKind {
    pub const ALL: [BehaviourKind; 28] = [
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
        BehaviourKind::Greet,
        BehaviourKind::Dead,
        BehaviourKind::ShadeHunt,
        BehaviourKind::Loot,
        BehaviourKind::Visit,
        BehaviourKind::Pester,
        BehaviourKind::Train,
        BehaviourKind::Shiny,
        BehaviourKind::Highlight,
        BehaviourKind::DreamGate,
        BehaviourKind::Fight,
        BehaviourKind::Gauntlet,
        BehaviourKind::Company,
        BehaviourKind::Pastime,
        BehaviourKind::Scene,
        BehaviourKind::Dizzy,
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
            BehaviourKind::Greet => "greet",
            BehaviourKind::Dead => "dead",
            BehaviourKind::ShadeHunt => "shade hunt",
            BehaviourKind::Loot => "loot",
            BehaviourKind::Visit => "visit",
            BehaviourKind::Pester => "pester",
            BehaviourKind::Train => "train",
            BehaviourKind::Shiny => "shiny",
            BehaviourKind::Highlight => "highlight",
            BehaviourKind::DreamGate => "dream gate",
            BehaviourKind::Fight => "fight",
            BehaviourKind::Gauntlet => "gauntlet",
            BehaviourKind::Company => "company",
            BehaviourKind::Pastime => "pastime",
            BehaviourKind::Scene => "scene",
            BehaviourKind::Dizzy => "dizzy",
        }
    }

    /// Base priority tier. The brain refines it by context (see `Brain::tier_of`): Rest,
    /// Nap and Recover only reach [`Tier::Needs`] when the need is critical, and Loot /
    /// ShadeHunt step down while a critical need waits, unless the prize is right there.
    ///
    /// Extension point: a new engagement (e.g. the NPC `Greet`) is one line here.
    pub fn tier(self) -> Tier {
        match self {
            // Reeling after a shake is a reflex, like a knock-out: brief, and before anything.
            BehaviourKind::Flee | BehaviourKind::Hide | BehaviourKind::KnockedOut | BehaviourKind::Dead | BehaviourKind::Dizzy => Tier::Urgent,
            // Geo and visitors keep him busy: they beat anything cursor-related.
            // Fights rank with visitors, above Geo (by utility within the tier).
            BehaviourKind::ShadeHunt | BehaviourKind::Loot | BehaviourKind::Shiny | BehaviourKind::Greet | BehaviourKind::Train | BehaviourKind::Fight => Tier::Engagement,
            // Only ever for one of those prizes, out of walking reach.
            BehaviourKind::DreamGate => Tier::Engagement,
            // The gauntlet's announcement and closing cheer, between its fights.
            BehaviourKind::Gauntlet => Tier::Engagement,
            // A visitor's calm scene ranks with visitors, above Geo.
            BehaviourKind::Company => Tier::Engagement,
            // Races and guests' performances (the scarlet flame steps down: `Brain::tier_of`).
            BehaviourKind::Scene => Tier::Engagement,
            BehaviourKind::Nap | BehaviourKind::Rest | BehaviourKind::Recover => Tier::Needs,
            // The cursor is a boredom killer (battles, pestering): an ambient pastime, never
            // allowed to interrupt Geo or visitors, and dropped the moment those show up.
            BehaviourKind::Battle | BehaviourKind::Pester => Tier::Ambient,
            // Things to do on his own: below every prize, fight and visitor.
            BehaviourKind::Pastime => Tier::Ambient,
            BehaviourKind::Idle | BehaviourKind::Companion | BehaviourKind::Watch | BehaviourKind::Explore | BehaviourKind::Ride | BehaviourKind::Visit | BehaviourKind::Highlight => {
                Tier::Ambient
            }
        }
    }

    /// Exclusive behaviours end themselves; only urgent challengers may interrupt.
    pub fn exclusive(self) -> bool {
        matches!(
            self,
            BehaviourKind::Nap
                | BehaviourKind::Battle
                | BehaviourKind::Pester
                | BehaviourKind::Train
                | BehaviourKind::Flee
                | BehaviourKind::Hide
                | BehaviourKind::KnockedOut
                | BehaviourKind::Dizzy
                | BehaviourKind::Dead
                | BehaviourKind::ShadeHunt
                | BehaviourKind::Loot
                | BehaviourKind::DreamGate
                | BehaviourKind::Fight
                | BehaviourKind::Gauntlet
                | BehaviourKind::Company
                | BehaviourKind::Scene
        )
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
    pub now: f64,
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
                // Leave a pose gracefully (get up from a seat, put the map away): ask for its
                // short exit act and move once it's done, instead of snapping out of it.
                if let Some(exit) = self.rep.catalog.get(a.id).and_then(exit_act_for).and_then(|n| self.rep.catalog.id(n)) {
                    self.out.exit_act = Some(exit);
                    return false;
                }
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

    /// In a serene moment, rarely, the Unn slug's crawl (spaced out so it stays a treat).
    /// The map and the lantern are carried items, not acts (`ai::carry`): no patrol.
    pub fn wander_act(&mut self) -> Option<ActWait> {
        let since = |ctx: &Self, name: &str| ctx.act_id(name).map_or(0.0, |id| ctx.rep.since_used(id, ctx.now));
        let s = self.cfg.serene;
        let calm_nature = 1.5 - self.mind.personality.vigour; // placid knights slug more
        if self.serene() && since(self, SLUG_ACT) >= s.cooldown_secs && self.rng.chance(s.chance * calm_nature) {
            return ActWait::named(self, SLUG_ACT);
        }
        None
    }

    /// A serene moment: rested and calm, no input for a while, nobody visiting, no Geo or
    /// Shade about, nothing worth watching on screen, and a ledge long enough to crawl.
    pub fn serene(&self) -> bool {
        let s = &self.cfg.serene;
        let (k, senses) = (self.p.knight, self.senses);
        self.grounded()
            && self.p.user.idle_secs >= s.user_idle_secs
            && !senses.user_busy
            && !senses.fullscreen
            && !senses.held
            && senses.visitors.is_empty()
            && self.stakes.sense.coins.is_empty()
            && self.stakes.sense.shade.is_none()
            && self.p.activity.spots.iter().all(|a| a.intensity < self.cfg.watch.min_intensity)
            && self.mind.needs.energy >= s.min_energy
            && self.mind.arousal() <= s.max_arousal
            && self.mind.emotions.hurt <= s.max_arousal
            && k.support.is_some_and(|l| l.x1 - l.x0 >= s.min_ledge_widths * k.size.x)
    }

    /// Sitting down again is fine: tired, or the last seat was a while ago.
    pub fn may_sit(&self) -> bool {
        self.mind.tiredness() > 0.5 || self.rep.since_seated(self.now) >= self.cfg.boredom.seat_cooldown
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

    /// The act actually got going (it may also end without ever starting).
    pub fn started(&self) -> bool {
        self.started
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
            // A stroll walks, hops and climbs but never dashes: where only a dash gets
            // there, a tired knight runs instead of failing over and over.
            let mut planned = ctx.nav.go(ctx.p.knight, self.goal, self.style);
            if !planned && self.style == TravelStyle::Stroll && ctx.nav.go(ctx.p.knight, self.goal, TravelStyle::Normal) {
                self.style = TravelStyle::Normal;
                planned = true;
            }
            if !planned {
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
    ctx.interest.block(target, ctx.now + secs as f64);
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
    let (terrain, ages, now) = (ctx.p.terrain, &ctx.interest.ages, ctx.now);
    let settled = |l: &crate::terrain::Ledge| ages.settled(terrain, l, now);
    let candidates: Vec<Vec2> = spots::seats(terrain, k, near, ctx.p.benches, range, &settled)
        .into_iter()
        .filter(|s| !ctx.interest.is_blocked(PoiTarget::Spot(*s)) && !on_point(k, *s, SEAT_TOL))
        .take(SEAT_PLANS)
        .collect();
    let styles: &[TravelStyle] = if ctx.mind.needs.energy > 0.15 { &[TravelStyle::Stroll, TravelStyle::Normal] } else { &[TravelStyle::Stroll] };
    for s in candidates {
        if let Some(&style) = styles.iter().find(|&&st| ctx.nav.reaches(k, s, st, SEAT_TOL)) {
            return Some((s, style));
        }
        ctx.interest.block(PoiTarget::Spot(s), ctx.now + FAILED_TARGET_SECS as f64);
    }
    None
}

// --- Idle ---------------------------------------------------------------------------

/// Nobody around and nothing to look at: sit if possible, else the odd fidget (the map or
/// the lantern may be in his hands meanwhile: `ai::carry`).
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
            let wander = if ctx.grounded() { ctx.wander_act() } else { None };
            let act = if wander.is_some() {
                wander
            } else if ctx.senses.can_sit() && ctx.may_sit() {
                ctx.seated_act()
            } else if ctx.senses.support_owner.is_some() && ctx.rng.chance(0.06 * ctx.mind.personality.curiosity) {
                ActWait::named(ctx, "dream_nail") // what is this window dreaming about?
            } else {
                ctx.fidget()
            };
            self.beat.play(ctx, act, gap);
        }
        Status::Running
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        match self.beat.doing_name(ctx) {
            Some(SLUG_ACT) => "idle: feeling serene (slug crawl)".into(),
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
        if let (PoiKind::NewWindow | PoiKind::Unvisited, PoiTarget::Window(id)) = (self.poi.kind, self.poi.target) {
            ctx.out.discovery = Some(DiscoveryEvent::Explored(id)); // the app may reveal what lives there
        }
        ctx.mind.adjust(|n| &mut n.boredom, -(0.15 + 0.45 * novelty));
        let small = match self.poi.target {
            PoiTarget::Window(id) => ctx.p.desktop.window(id).is_some_and(|w| w.rect.width() * w.rect.height() <= 420 * 320),
            PoiTarget::Spot(_) => false,
        };
        // Curious about a big new window: dream-nail it to hear what it thinks.
        let dream = !small && matches!(self.poi.target, PoiTarget::Window(_)) && ctx.rng.chance(0.1 + 0.3 * ctx.mind.personality.curiosity);
        let (what, name) = match self.poi.kind {
            PoiKind::NewWindow if small => ("found a small new thing", "pick_up_trinket"),
            PoiKind::NewWindow | PoiKind::Unvisited if dream => ("dream-nailing a window", "dream_nail"),
            PoiKind::NewWindow | PoiKind::Unvisited if ctx.senses.can_sit() => ("mapping a new window", "read_map"),
            // Nowhere to sit: the map comes out and he carries on with it (`ai::carry`).
            PoiKind::NewWindow | PoiKind::Unvisited => ("mapping a new window", super::carry::TAKE_OUT_MAP),
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
                ctx.interest.block(PoiTarget::Spot(here), ctx.now + FAILED_TARGET_SECS as f64);
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
            // A favourite found bench is worth a longer walk.
            match favourite_bench(ctx, Self::SEAT_RANGE).or_else(|| find_seat(ctx, k.pos, Self::SEAT_RANGE)) {
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

/// Shaken about while held: once he lands, he reels ("dizzy": a recoil, then the swaying
/// Idle Hurt loop; the one-frame Stun clip looked frozen).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Dizzy {
    act: Option<ActWait>,
    airborne: f32,
}

impl Dizzy {
    /// Still falling after this long (s, a drop onto a far ledge): the daze wore off.
    const LAND_WITHIN: f32 = 4.0;

    pub fn start(_ctx: &mut Ctx) -> Self {
        Self { act: None, airborne: 0.0 }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        ctx.nav.stop();
        if !ctx.grounded() {
            self.airborne += ctx.dt();
            return if self.airborne > Self::LAND_WITHIN { Status::Done } else { Status::Running };
        }
        if self.act.is_none() {
            self.act = ActWait::named(ctx, "dizzy");
            if self.act.is_none() {
                return Status::Done;
            }
        }
        if self.act.as_mut().is_some_and(|w| w.tick(ctx)) { Status::Done } else { Status::Running }
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
        if ctx.senses.since_carried(ctx.now) > 3.0 {
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
    Pester(Box<super::pester::Pester>),
    Flee(Flee),
    Hide(Hide),
    Ride(Ride),
    KnockedOut(KnockedOut),
    Recover(Recover),
    Greet(Box<super::greet::Greet>),
    Train(Box<super::train::Train>),
    Dead(Dead),
    ShadeHunt(Box<ShadeHunt>),
    Loot(Loot),
    Visit(Visit),
    Shiny(super::shiny::PickShiny),
    Highlight(super::highlight::LookAtHighlight),
    DreamGate(super::dream_gate::DreamGate),
    Fight(Box<super::fight::Fight>),
    Gauntlet(super::gauntlet::Ceremony),
    Company(Box<super::company::Company>),
    Pastime(Box<super::pastime::Pastime>),
    Scene(Box<super::scene::Scene>),
    Dizzy(Dizzy),
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
            Active::Pester(_) => BehaviourKind::Pester,
            Active::Flee(_) => BehaviourKind::Flee,
            Active::Hide(_) => BehaviourKind::Hide,
            Active::Ride(_) => BehaviourKind::Ride,
            Active::KnockedOut(_) => BehaviourKind::KnockedOut,
            Active::Recover(_) => BehaviourKind::Recover,
            Active::Greet(_) => BehaviourKind::Greet,
            Active::Train(_) => BehaviourKind::Train,
            Active::Dead(_) => BehaviourKind::Dead,
            Active::ShadeHunt(_) => BehaviourKind::ShadeHunt,
            Active::Loot(_) => BehaviourKind::Loot,
            Active::Visit(_) => BehaviourKind::Visit,
            Active::Shiny(_) => BehaviourKind::Shiny,
            Active::Highlight(_) => BehaviourKind::Highlight,
            Active::DreamGate(_) => BehaviourKind::DreamGate,
            Active::Fight(_) => BehaviourKind::Fight,
            Active::Gauntlet(_) => BehaviourKind::Gauntlet,
            Active::Company(_) => BehaviourKind::Company,
            Active::Pastime(_) => BehaviourKind::Pastime,
            Active::Scene(_) => BehaviourKind::Scene,
            Active::Dizzy(_) => BehaviourKind::Dizzy,
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
            Active::Pester(b) => b.tick(ctx),
            Active::Flee(b) => b.tick(ctx),
            Active::Hide(b) => b.tick(ctx),
            Active::Ride(b) => b.tick(ctx),
            Active::KnockedOut(b) => b.tick(ctx),
            Active::Recover(b) => b.tick(ctx),
            Active::Greet(b) => b.tick(ctx),
            Active::Train(b) => b.tick(ctx),
            Active::Dead(b) => b.tick(ctx),
            Active::ShadeHunt(b) => b.tick(ctx),
            Active::Loot(b) => b.tick(ctx),
            Active::Visit(b) => b.tick(ctx),
            Active::Shiny(b) => b.tick(ctx),
            Active::Highlight(b) => b.tick(ctx),
            Active::DreamGate(b) => b.tick(ctx),
            Active::Fight(b) => b.tick(ctx),
            Active::Gauntlet(b) => b.tick(ctx),
            Active::Company(b) => b.tick(ctx),
            Active::Pastime(b) => b.tick(ctx),
            Active::Scene(b) => b.tick(ctx),
            Active::Dizzy(b) => b.tick(ctx),
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
            Active::Pester(b) => b.reason(),
            Active::Flee(_) => "flee: startled".into(),
            Active::Hide(_) => "hide: fullscreen app in front".into(),
            Active::Ride(_) => "ride: window moving".into(),
            Active::KnockedOut(_) => "knocked out: huge fall".into(),
            Active::Recover(b) => b.reason(),
            Active::Greet(b) => b.reason(ctx),
            Active::Train(b) => b.reason(),
            Active::Dead(b) => b.reason(),
            Active::ShadeHunt(b) => b.reason(),
            Active::Loot(b) => b.reason(),
            Active::Visit(b) => b.reason(),
            Active::Shiny(b) => b.reason(),
            Active::Highlight(b) => b.reason(ctx),
            Active::DreamGate(b) => b.reason(),
            Active::Fight(b) => b.reason(),
            Active::Gauntlet(b) => b.reason(),
            Active::Company(b) => b.reason(ctx),
            Active::Pastime(b) => b.reason(),
            Active::Scene(b) => b.reason(),
            Active::Dizzy(_) => "dizzy: shaken about".into(),
        }
    }
}

/// The Unn slug's serene crawl, and its way back (transform back to the knight).
pub(crate) const SLUG_ACT: &str = "slug_crawl";
pub(crate) const SLUG_EXIT: &str = "slug_up";

/// Short act that ends `act` gracefully when another behaviour takes over (getting up from
/// a seat, lowering the head, turning back from the screen, standing up from the slug)
/// instead of snapping to idle.
pub(crate) fn exit_act_for(act: &ActInfo) -> Option<&'static str> {
    match act.name.as_str() {
        SLUG_ACT => Some(SLUG_EXIT),
        "look_up" => Some("look_up_end"),
        "look_down" => Some("look_down_end"),
        "gaze_into_screen" | "stargaze" => Some("turn_from_screen"),
        super::carry::TAKE_OUT_MAP | "company_maps_stand" => Some(super::carry::PUT_MAP_AWAY),
        _ if act.requires == crate::acts::Requirement::Seat => Some("get_up"),
        _ => None,
    }
}
