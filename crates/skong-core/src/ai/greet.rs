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
}

/// Chance (at curiosity 0.5) that the knight dream-nails a visitor after their talk.
pub const DREAM_NAIL_CHANCE: f32 = 0.25;
/// Seconds the knight waits beside a visitor for the conversation to start.
const START_PATIENCE: f32 = 6.0;
/// Horizontal distance kept from the visitor's feet point, in game units.
pub const STAND_OFF_UNITS: f32 = 1.7;

/// What the brain remembers about visitors between ticks.
#[derive(Clone, Debug, Default)]
pub(crate) struct VisitorMemory {
    seen: BTreeSet<u32>,
    /// Visitors the knight couldn't reach: ignored until the given brain time.
    snubbed: BTreeMap<u32, f32>,
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

    pub fn snub(&mut self, id: u32, until: f32) {
        // Forget long-expired snubs.
        self.snubbed.retain(|_, t| *t > until - 600.0);
        self.snubbed.insert(id, until);
    }

    /// The visitor worth greeting now: not yet greeted, not leaving, not given up on;
    /// the one already talking first, else the nearest.
    pub fn target<'v>(&self, visitors: &'v [Visitor], knight: Vec2, now: f32) -> Option<&'v Visitor> {
        visitors
            .iter()
            .filter(|v| !v.greeted && v.state != VisitorState::Leaving && !self.snubbed.get(&v.id).is_some_and(|t| *t > now))
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
}

impl Greet {
    pub fn start(ctx: &mut Ctx, v: &Visitor) -> Self {
        ctx.mind.excite(0.3 * v.excitement);
        ctx.mind.adjust(|n| &mut n.boredom, -0.2);
        ctx.mind.adjust(|n| &mut n.loneliness, -0.3);
        let goal = stand_point(ctx, v);
        let phase = if beside(ctx, v, goal) {
            Phase::Arrive { waited: 0.0, act: None, hello_done: false }
        } else {
            Phase::Approach(Travel::to(ctx, goal, TravelStyle::Hurry))
        };
        Self { id: v.id, name: v.name.clone(), phase, failed: false, dream_decided: false }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let Some(v) = ctx.senses.visitors.iter().find(|v| v.id == self.id).cloned() else {
            ctx.nav.stop();
            return Status::Done;
        };
        let talking = v.state == VisitorState::Talking;
        if v.state == VisitorState::Leaving || (v.greeted && !matches!(self.phase, Phase::Converse { .. })) {
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
                if beside(ctx, &v, goal) {
                    ctx.nav.stop();
                    self.phase = Phase::Arrive { waited: 0.0, act: None, hello_done: false };
                    return Status::Running;
                }
                if let NavGoal::Point(p) = t.goal
                    && p.distance(goal) > 48.0
                {
                    t.retarget(NavGoal::Point(goal));
                }
                match t.tick(ctx) {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Arrived => {
                        self.phase = Phase::Arrive { waited: 0.0, act: None, hello_done: false };
                        Status::Running
                    }
                    TravelStep::Failed => {
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
                    if v.greeted && !self.dream_decided && v.state != VisitorState::Teaching {
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

fn stand_off(ctx: &Ctx, v: &Visitor) -> f32 {
    if v.stand_off > 0.0 { v.stand_off } else { STAND_OFF_UNITS * ctx.p.knight.params.px_per_unit }
}

pub(crate) fn beside(ctx: &Ctx, v: &Visitor, goal: Vec2) -> bool {
    let k = ctx.p.knight;
    let off = stand_off(ctx, v);
    let dx = (k.pos.x - v.pos.x).abs();
    k.is_grounded() && (k.pos.y - goal.y).abs() < 12.0 && dx <= off + 40.0 && dx >= off * 0.4
}
