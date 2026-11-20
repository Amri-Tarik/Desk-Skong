//! Visitors and the "Greet" behaviour: when a Hollow Knight NPC shows up, the knight gets
//! excited, hurries over (planner, [`TravelStyle::Hurry`]), stands beside them facing them,
//! asks the app to start the conversation ([`super::Intent::greet`]), and reacts to each
//! line with the act the line suggests (look up, challenge, open a map...).
//!
//! The app owns the visitors (`crate::npc`) and reports them each tick through
//! [`super::Brain::see_visitors`].
use std::collections::{BTreeMap, BTreeSet};

use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};

/// Where a visitor is in its visit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisitorState {
    /// Walking in / stepping out of a portal / landing.
    #[default]
    Arriving,
    /// Standing around (a conversation can start).
    Waiting,
    /// Talking with the knight.
    Talking,
    /// Teaching the knight after the talk (see [`Visitor::lesson`]).
    Teaching,
    /// Sparring with the knight after the talk (he is also one of the brain's foes).
    Sparring,
    /// A calm scene after the talk: keep them company (see [`Visitor::company`]).
    Company,
    /// Racing the knight after the talk (Hornet; the race itself is a [`super::SceneCue`]).
    Racing,
    /// Picked up by the user (carried or falling): nobody to greet until they're down.
    Held,
    Leaving,
}

/// A visible NPC, as the brain perceives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visitor {
    pub id: u32,
    /// Definition name ("cornifer", "zote"...).
    pub name: String,
    /// Feet (or seat) point.
    pub pos: Vec2,
    pub state: VisitorState,
    /// Line being said (while talking).
    pub line: Option<usize>,
    /// Knight act(s) answering the current line; `|` separates alternatives.
    pub react: Option<String>,
    /// Knight act on arriving next to them.
    pub greet_react: Option<String>,
    /// Already had their conversation.
    pub greeted: bool,
    /// How thrilled the knight is to see them (0..1).
    pub excitement: f32,
    /// Horizontal distance to stand from their feet point (px); 0 = default.
    pub stand_off: f32,
    /// While teaching: the drill and what the knight should be doing.
    pub lesson: Option<super::train::LessonCue>,
    /// While keeping company: the knight's part and where the scene is.
    pub company: Option<super::company::CompanyCue>,
}

/// Chance (at curiosity 0.5) that the knight dream-nails a visitor after their talk.
pub const DREAM_NAIL_CHANCE: f32 = 0.25;
/// Seconds the knight waits beside a visitor for the conversation to start.
const START_PATIENCE: f32 = 6.0;
/// Horizontal distance kept from the visitor's feet point, in game units.
pub const STAND_OFF_UNITS: f32 = 1.7;
/// A talk (or a dream nail) needs no contact: when no route gets him right beside the
/// visitor, anywhere he can stand this close (game units, feet to feet) will do.
pub const TALK_DX_UNITS: f32 = 6.0;
pub const TALK_DY_UNITS: f32 = 5.0;
/// Talk spots whose routes are planned per look at most.
const TALK_PLANS: usize = 5;

/// What the brain remembers about visitors between ticks.
#[derive(Clone, Debug, Default)]
pub(crate) struct VisitorMemory {
    seen: BTreeSet<u32>,
    /// Visitors the knight couldn't reach: ignored until the given brain time.
    snubbed: BTreeMap<u32, f64>,
    /// Someone new arrived since the last tick (wakes a napping knight).
    pub knock: bool,
}

impl VisitorMemory {
    /// Remember the ids; returns the visitors seen for the first time.
    pub fn arrivals<'v>(&mut self, visitors: &'v [Visitor]) -> Vec<&'v Visitor> {
        let new: Vec<&Visitor> = visitors.iter().filter(|v| !self.seen.contains(&v.id)).collect();
        for v in &new {
            self.seen.insert(v.id);
        }
        self.knock |= !new.is_empty();
        new
    }

    pub fn snub(&mut self, id: u32, until: f64) {
        // Forget long-expired snubs.
        self.snubbed.retain(|_, t| *t > until - 600.0);
        self.snubbed.insert(id, until);
    }

    /// Worth greeting again (he warped over to them).
    pub fn unsnub(&mut self, id: u32) {
        self.snubbed.remove(&id);
    }

    /// The visitor worth greeting now: not yet greeted, not leaving, not given up on;
    /// the one already talking first, else the nearest.
    pub fn target<'v>(&self, visitors: &'v [Visitor], knight: Vec2, now: f64) -> Option<&'v Visitor> {
        visitors
            .iter()
            .filter(|v| !v.greeted && !matches!(v.state, VisitorState::Leaving | VisitorState::Held) && !self.snubbed.get(&v.id).is_some_and(|t| *t > now))
            .min_by(|a, b| {
                let key = |v: &Visitor| (v.state != VisitorState::Talking, v.pos.distance(knight));
                let (ka, kb) = (key(a), key(b));
                ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            })
    }
}

#[derive(Clone, Debug)]
enum Phase {
    Approach(Travel),
    /// Beside them: say hello (an act), ask for the conversation.
    Arrive { waited: f32, act: Option<ActWait>, hello_done: bool },
    Converse { line: Option<usize>, act: Option<ActWait> },
}

#[derive(Clone, Debug)]
pub(crate) struct Greet {
    pub id: u32,
    name: String,
    phase: Phase,
    /// Gave up (no route, nobody answered): the brain snubs this visitor for a while.
    pub failed: bool,
    /// The after-conversation dream nail was considered (at most once per visit).
    dream_decided: bool,
    /// No route beside them: heading for this spot within talking range instead.
    talk_spot: Option<Vec2>,
}

impl Greet {
    pub fn start(ctx: &mut Ctx, v: &Visitor) -> Self {
        ctx.mind.excite(0.3 * v.excitement);
        ctx.mind.adjust(|n| &mut n.boredom, -0.2);
        ctx.mind.adjust(|n| &mut n.loneliness, -0.3);
        let goal = stand_point(ctx, v);
        let mut g = Self { id: v.id, name: v.name.clone(), phase: Phase::Arrive { waited: 0.0, act: None, hello_done: false }, failed: false, dream_decided: false, talk_spot: None };
        if beside(ctx, v, goal) {
            return g;
        }
        let k = ctx.p.knight;
        if ctx.nav.reaches(k, goal, TravelStyle::Hurry, 40.0) {
            g.phase = Phase::Approach(Travel::to(ctx, goal, TravelStyle::Hurry));
        } else if !g.talk_from_afar(ctx, v) {
            // Nowhere within talking range either: try the walk anyway (the planner may
            // know better once he moves); a failure gives them up for a while.
            g.phase = Phase::Approach(Travel::to(ctx, goal, TravelStyle::Hurry));
        }
        g
    }

    /// No route right beside them: talk from where he stands if that's in range, else head
    /// for the best reachable spot in range. `false`: no such spot.
    fn talk_from_afar(&mut self, ctx: &mut Ctx, v: &Visitor) -> bool {
        if in_talk_range(ctx, v) {
            log::debug!("brain: can't get beside {}; talking from here", self.name);
            ctx.nav.stop();
            self.phase = Phase::Arrive { waited: 0.0, act: None, hello_done: false };
            return true;
        }
        match talk_spot(ctx, v) {
            Some(spot) => {
                log::debug!("brain: can't get beside {}; talking from ({:.0},{:.0})", self.name, spot.x, spot.y);
                self.talk_spot = Some(spot);
                self.phase = Phase::Approach(Travel::to(ctx, spot, TravelStyle::Hurry));
                true
            }
            None => false,
        }
    }

    /// They got to talk (the conversation started and he didn't give up).
    pub fn chatted(&self) -> bool {
        matches!(self.phase, Phase::Converse { .. }) && !self.failed
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let Some(v) = ctx.senses.visitors.iter().find(|v| v.id == self.id).cloned() else {
            ctx.nav.stop();
            return Status::Done;
        };
        let talking = v.state == VisitorState::Talking;
        // Picked up by the user: this greeting is over (once set down they're greeted
        // afresh: he comes over again, or talks from nearby).
        if matches!(v.state, VisitorState::Leaving | VisitorState::Held) || (v.greeted && !matches!(self.phase, Phase::Converse { .. })) {
            ctx.nav.stop();
            return Status::Done;
        }
        if talking && !matches!(self.phase, Phase::Converse { .. }) {
            ctx.nav.stop();
            self.phase = Phase::Converse { line: None, act: None };
        }
        match &mut self.phase {
            Phase::Approach(t) => {
                let goal = stand_point(ctx, &v);
                let there = match self.talk_spot {
                    // Heading for a talk spot: there, or anywhere in range on the way down.
                    Some(_) => ctx.grounded() && in_talk_range(ctx, &v) && ctx.p.knight.vel.y.abs() < 1.0,
                    None => beside(ctx, &v, goal),
                };
                if there {
                    ctx.nav.stop();
                    self.phase = Phase::Arrive { waited: 0.0, act: None, hello_done: false };
                    return Status::Running;
                }
                let goal = self.talk_spot.unwrap_or(goal);
                if let NavGoal::Point(p) = t.goal
                    && p.distance(goal) > 48.0
                {
                    t.retarget(NavGoal::Point(goal));
                }
                let mut step = t.tick(ctx);
                if step == TravelStep::Arrived && !in_talk_range(ctx, &v) {
                    // The route ended somewhere else (under their window, say).
                    ctx.nav.stop();
                    step = TravelStep::Failed;
                }
                match step {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Arrived => {
                        self.phase = Phase::Arrive { waited: 0.0, act: None, hello_done: false };
                        Status::Running
                    }
                    TravelStep::Failed => {
                        // Couldn't get beside them (live: Elderbug in a busy window, given up
                        // right after he got fairly close): a talk needs no contact.
                        if self.talk_spot.is_none() && self.talk_from_afar(ctx, &v) {
                            return Status::Running;
                        }
                        self.failed = true;
                        Status::Done
                    }
                }
            }
            Phase::Arrive { waited, act, hello_done } => {
                ctx.nav.stop();
                ctx.face_toward(v.pos.x);
                *waited += ctx.dt();
                if !*hello_done {
                    *hello_done = true;
                    *act = v.greet_react.as_deref().and_then(|r| startable(ctx, r));
                }
                if let Some(w) = act
                    && w.tick(ctx)
                {
                    *act = None;
                }
                if act.is_none() && v.state == VisitorState::Waiting {
                    ctx.out.greet = Some(v.id);
                }
                if *waited > START_PATIENCE + if act.is_some() { 4.0 } else { 0.0 } {
                    self.failed = true;
                    return Status::Done;
                }
                ctx.out.waiting = false;
                Status::Running
            }
            Phase::Converse { line, act } => {
                ctx.nav.stop();
                if act.is_none() {
                    ctx.face_toward(v.pos.x);
                }
                if talking && v.line != *line {
                    *line = v.line;
                    if let Some(r) = v.react.as_deref() {
                        *act = startable(ctx, r);
                    }
                }
                if let Some(w) = act
                    && w.tick(ctx)
                {
                    *act = None;
                }
                ctx.out.waiting = false;
                if !talking && act.is_none() {
                    if v.greeted && !self.dream_decided && !matches!(v.state, VisitorState::Teaching | VisitorState::Sparring | VisitorState::Company) {
                        // Afterwards, sometimes: dream-nail them to hear what they really think.
                        self.dream_decided = true;
                        let chance = DREAM_NAIL_CHANCE * (0.5 + ctx.mind.personality.curiosity);
                        if ctx.rng.chance(chance)
                            && let Some(w) = ActWait::named(ctx, "dream_nail")
                        {
                            ctx.face_toward(v.pos.x);
                            *act = Some(w);
                            ctx.out.dream_visitor = Some(v.id);
                            return Status::Running;
                        }
                    }
                    if v.greeted {
                        ctx.mind.adjust(|n| &mut n.loneliness, -0.4);
                        ctx.mind.excite(0.15);
                        return Status::Done;
                    }
                    // Between "arrived" and the first line: keep asking.
                    if v.state == VisitorState::Waiting {
                        ctx.out.greet = Some(v.id);
                    }
                }
                Status::Running
            }
        }
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        match &self.phase {
            Phase::Approach(_) => format!("greet {}: hurrying over", self.name),
            Phase::Arrive { act: Some(w), .. } => format!("greet {}: hello ({})", self.name, act_name(ctx, w)),
            Phase::Arrive { .. } => format!("greet {}: saying hello", self.name),
            Phase::Converse { act: Some(w), line } => format!("greet {}: line {} -> {}", self.name, line.map_or(0, |l| l + 1), act_name(ctx, w)),
            Phase::Converse { line, .. } => format!("greet {}: listening (line {})", self.name, line.map_or(0, |l| l + 1)),
        }
    }
}

fn act_name<'c>(ctx: &'c Ctx, w: &ActWait) -> &'c str {
    ctx.rep.catalog.get(w.id).map_or("?", |a| a.name.as_str())
}

/// First act among `a|b|c` that can start here.
pub(crate) fn startable(ctx: &Ctx, alternatives: &str) -> Option<ActWait> {
    let (grounded, sit) = (ctx.grounded(), ctx.senses.can_sit());
    alternatives
        .split('|')
        .map(str::trim)
        .filter_map(|n| ctx.rep.catalog.id(n))
        .find(|id| ctx.rep.catalog.get(*id).is_some_and(|a| a.startable(grounded, sit)))
        .map(ActWait::new)
}

/// Where to stand: beside the visitor on the knight's side of them, on their ledge (the
/// other side when the ledge ends too soon).
pub(crate) fn stand_point(ctx: &Ctx, v: &Visitor) -> Vec2 {
    let k = ctx.p.knight;
    let off = stand_off(ctx, v);
    let side = if k.pos.x >= v.pos.x { 1.0 } else { -1.0 };
    let ledge = ctx.p.terrain.ledges.iter().filter(|l| (l.y - v.pos.y).abs() < 2.0 && l.x0 - 1.0 <= v.pos.x && v.pos.x <= l.x1 + 1.0).max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)));
    let margin = 12.0;
    let x = match ledge {
        Some(l) => {
            let fits = |s: f32| (l.x0 + margin..=l.x1 - margin).contains(&(v.pos.x + s * off));
            let s = if fits(side) || !fits(-side) { side } else { -side };
            (v.pos.x + s * off).clamp(l.x0 + margin, (l.x1 - margin).max(l.x0 + margin))
        }
        None => v.pos.x + side * off,
    };
    Vec2::new(x, v.pos.y)
}

/// Standing within talking range of the visitor (any ledge; no line of sight needed).
pub(crate) fn in_talk_range(ctx: &Ctx, v: &Visitor) -> bool {
    let k = ctx.p.knight;
    let ppu = k.params.px_per_unit;
    k.is_grounded() && (k.pos.x - v.pos.x).abs() <= TALK_DX_UNITS * ppu && (k.pos.y - v.pos.y).abs() <= TALK_DY_UNITS * ppu
}

/// The best spot within talking range that a route gets him onto: on a ledge near the
/// visitor's, the same level first, then the smallest height gap, then the nearest to them.
fn talk_spot(ctx: &mut Ctx, v: &Visitor) -> Option<Vec2> {
    let k = ctx.p.knight;
    let ppu = k.params.px_per_unit;
    let (dx_max, dy_max) = (TALK_DX_UNITS * ppu, TALK_DY_UNITS * ppu);
    let off = stand_off(ctx, v);
    let half_w = 0.5 * k.size.x + 4.0;
    let mut spots: Vec<Vec2> = ctx
        .p
        .terrain
        .ledges
        .iter()
        .filter(|l| (l.y - v.pos.y).abs() <= dy_max && l.x1 - l.x0 >= 2.0 * half_w)
        .filter_map(|l| {
            // As close to beside them as the ledge allows (not right under / over them).
            let (lo, hi) = (l.x0 + half_w, l.x1 - half_w);
            let side = if k.pos.x >= v.pos.x { 1.0 } else { -1.0 };
            let want = [v.pos.x + side * off, v.pos.x - side * off];
            let x = want.iter().map(|w| w.clamp(lo, hi)).min_by(|a, b| (a - want[0]).abs().total_cmp(&(b - want[0]).abs()))?;
            ((x - v.pos.x).abs() <= dx_max).then_some(Vec2::new(x, l.y))
        })
        .collect();
    let level = |p: &Vec2| ((p.y - v.pos.y).abs() > 4.0, (p.y - v.pos.y).abs(), (p.x - v.pos.x).abs());
    spots.sort_by(|a, b| {
        let (la, lb) = (level(a), level(b));
        la.0.cmp(&lb.0).then(la.1.total_cmp(&lb.1)).then(la.2.total_cmp(&lb.2))
    });
    spots.into_iter().take(TALK_PLANS).find(|s| ctx.nav.reaches(k, *s, TravelStyle::Hurry, 40.0))
}

fn stand_off(ctx: &Ctx, v: &Visitor) -> f32 {
    if v.stand_off > 0.0 { v.stand_off } else { STAND_OFF_UNITS * ctx.p.knight.params.px_per_unit }
}

pub(crate) fn beside(ctx: &Ctx, v: &Visitor, goal: Vec2) -> bool {
    let k = ctx.p.knight;
    let off = stand_off(ctx, v);
    let dx = (k.pos.x - v.pos.x).abs();
    k.is_grounded() && (k.pos.y - goal.y).abs() < 12.0 && dx <= off + 40.0 && dx >= off * 0.4
}
