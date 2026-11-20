//! Scenes the knight takes part in, cued by the app through [`Brain::see_scene`]:
//!
//! * **Race** (Hornet's): he stands ready facing the finish; on "Go!" he hurries there with
//!   his whole kit (a `Hurry` route: sprint, dashes, crystal dash, wall jumps); once it's
//!   decided he reacts (`victory`, or a `look_down`). Engagement tier, like a spar.
//! * **Watch** (the Troupe Master's friendly visit): he stays put and follows the guest with
//!   his gaze, answering each beat of the performance with the act it suggests (a startle at
//!   the entrance, a bow for a bow). Engagement tier.
//! * **Flame** (a scarlet flame waiting to be clicked): curious, he walks over and lingers
//!   by it, looking at it and pacing a little (the app adds the occasional "?"), drawing the
//!   user's attention to it. A pastime: Geo, fights, visitors and the Shade all come first,
//!   and his interest wanes after a while (he comes back later while it burns).
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::greet::startable;
use super::{Brain, Motion};

/// What the app tells the brain about a scene under way.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneCue {
    /// A race to `finish` (feet point). Before `go`: get ready. `over`: decided or called
    /// off; `won`: whether he won (`None` when called off).
    Race { serial: u32, finish: Vec2, go: bool, over: bool, won: Option<bool> },
    /// A guest performing at `at` (body centre): watch. `react` (acts, `a|b`) answers beat
    /// number `beat` (played once per beat).
    Watch { serial: u32, at: Vec2, beat: u32, react: Option<String> },
    /// A scarlet flame burning at `at` (feet point), waiting for the user's click.
    Flame { at: Vec2 },
}

/// Utility of a race (ready, running or reacting): above a spar or a lesson.
const RACE_UTILITY: f32 = 0.985;
const WATCH_UTILITY: f32 = 0.96;
/// The flame: a pastime (ambient tier), but a strong one.
const FLAME_UTILITY: f32 = 0.8;
/// Seconds he lingers by the flame before his interest wanes ...
const FLAME_LINGER: [f32; 2] = [50.0, 80.0];
/// ... and before it draws him back.
pub(super) const FLAME_REST: f32 = 60.0;

impl Brain {
    /// The scene under way, if any (call before [`Brain::tick`]).
    pub fn see_scene(&mut self, cue: Option<SceneCue>) {
        let serial = |c: &SceneCue| match c {
            SceneCue::Race { serial, .. } | SceneCue::Watch { serial, .. } => Some(*serial),
            SceneCue::Flame { .. } => None,
        };
        if let Some(c) = &cue
            && serial(c).is_some()
            && self.senses.scene.as_ref().and_then(serial) != serial(c)
        {
            self.mind.excite(0.4);
        }
        self.senses.scene = cue;
    }

    pub(super) fn scene_utility(&self) -> f32 {
        let Some(cue) = &self.senses.scene else { return 0.0 };
        if self.senses.fullscreen || self.stakes.pending_death.is_some() {
            return 0.0;
        }
        match cue {
            SceneCue::Race { serial, over, won, .. } => {
                // Once decided: only the reaction is left (none when called off).
                if !over || (won.is_some() && self.scene_reacted != Some(*serial)) { RACE_UTILITY } else { 0.0 }
            }
            SceneCue::Watch { .. } => WATCH_UTILITY,
            SceneCue::Flame { .. } if self.now < self.flame_rest_until || self.senses.user_away => 0.0,
            SceneCue::Flame { .. } => FLAME_UTILITY,
        }
    }

    /// The flame is a pastime; races and performances are engagements.
    pub(super) fn scene_is_pastime(&self) -> bool {
        matches!(self.senses.scene, Some(SceneCue::Flame { .. }))
    }
}

#[derive(Clone, Debug)]
enum Phase {
    /// Race: standing ready.
    Ready,
    Racing { travel: Travel, tries: u32 },
    /// Race: there (or gave up), waiting for the outcome.
    Waiting,
    /// Race: the reaction to the outcome.
    React(Option<ActWait>),
    /// Watch: following the guest with his gaze.
    Watch { beat: u32, act: Option<ActWait> },
    /// Flame: on his way over.
    ToFlame(Travel),
    /// Flame: by it, looking, pacing now and then.
    Linger { left: f32, next: f32, act: Option<ActWait>, step: Option<Travel> },
}

#[derive(Clone, Debug)]
pub(crate) struct Scene {
    serial: Option<u32>,
    phase: Phase,
    /// Race: he reacted to this race (the brain stops picking it).
    pub reacted: Option<u32>,
}

/// Where to stand by the flame: a little to one side, on its ledge.
fn by_flame(ctx: &Ctx, at: Vec2, side: f32) -> Vec2 {
    let u = ctx.p.knight.params.px_per_unit;
    let ledge = ctx.p.terrain.ledges.iter().find(|l| (l.y - at.y).abs() < 2.0 && l.x0 - 1.0 <= at.x && at.x <= l.x1 + 1.0);
    let x = at.x + side * 1.6 * u;
    let x = ledge.map_or(x, |l| x.clamp(l.x0 + 12.0, (l.x1 - 12.0).max(l.x0 + 12.0)));
    Vec2::new(x, at.y)
}

impl Scene {
    pub fn start(ctx: &mut Ctx) -> Self {
        ctx.nav.stop();
        ctx.mind.adjust(|n| &mut n.boredom, -0.3);
        let phase = match ctx.senses.scene.clone() {
            Some(SceneCue::Race { go: true, over: false, finish, .. }) => Phase::Racing { travel: Travel::to(ctx, finish, TravelStyle::Hurry), tries: 0 },
            Some(SceneCue::Race { over: true, .. }) => Phase::React(None),
            Some(SceneCue::Race { .. }) => Phase::Ready,
            Some(SceneCue::Watch { .. }) => Phase::Watch { beat: u32::MAX, act: None },
            Some(SceneCue::Flame { at }) => {
                let side = if ctx.p.knight.pos.x >= at.x { 1.0 } else { -1.0 };
                Phase::ToFlame(Travel::to(ctx, by_flame(ctx, at, side), TravelStyle::Normal))
            }
            None => Phase::Waiting,
        };
        let serial = match &ctx.senses.scene {
            Some(SceneCue::Race { serial, .. } | SceneCue::Watch { serial, .. }) => Some(*serial),
            _ => None,
        };
        Self { serial, phase, reacted: None }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let Some(cue) = ctx.senses.scene.clone() else {
            ctx.nav.stop();
            return Status::Done;
        };
        if ctx.senses.fullscreen {
            ctx.nav.stop();
            return Status::Done;
        }
        match cue {
            SceneCue::Race { serial, finish, go, over, won } => {
                if self.serial != Some(serial) {
                    return Status::Done;
                }
                self.race(ctx, finish, go, over, won)
            }
            SceneCue::Watch { serial, at, beat, react } => {
                if self.serial != Some(serial) {
                    return Status::Done;
                }
                self.watch(ctx, at, beat, react)
            }
            SceneCue::Flame { at } => self.flame(ctx, at),
        }
    }

    fn race(&mut self, ctx: &mut Ctx, finish: Vec2, go: bool, over: bool, won: Option<bool>) -> Status {
        if over && !matches!(self.phase, Phase::React(_)) {
            ctx.nav.stop();
            if won.is_none() {
                return Status::Done; // called off
            }
            self.phase = Phase::React(None);
        }
        match &mut self.phase {
            Phase::Ready => {
                ctx.out.motion = Motion::Hold;
                ctx.face_toward(finish.x);
                if go {
                    ctx.mind.excite(0.3);
                    self.phase = Phase::Racing { travel: Travel::to(ctx, finish, TravelStyle::Hurry), tries: 0 };
                }
                Status::Running
            }
            Phase::Racing { travel, tries } => match travel.tick(ctx) {
                TravelStep::Moving => Status::Running,
                TravelStep::Arrived => {
                    self.phase = Phase::Waiting;
                    Status::Running
                }
                TravelStep::Failed if *tries < 3 => {
                    // A missed jump or a moved window: plan again from here.
                    *tries += 1;
                    travel.retarget(NavGoal::Point(finish));
                    Status::Running
                }
                TravelStep::Failed => {
                    log::debug!("brain: no way to the finish; watching her win");
                    self.phase = Phase::Waiting;
                    Status::Running
                }
            },
            Phase::Waiting => {
                ctx.out.motion = Motion::Hold;
                Status::Running
            }
            Phase::React(act) => {
                ctx.out.motion = Motion::Hold;
                if act.is_none() && self.reacted.is_none() {
                    self.reacted = self.serial;
                    let names = if won == Some(true) { "victory|collect_happy" } else { "look_down" };
                    *act = startable(ctx, names);
                    if act.is_none() {
                        return Status::Done;
                    }
                }
                match act {
                    Some(w) => {
                        if w.tick(ctx) {
                            Status::Done
                        } else {
                            Status::Running
                        }
                    }
                    None => Status::Done,
                }
            }
            _ => Status::Done,
        }
    }

    fn watch(&mut self, ctx: &mut Ctx, at: Vec2, beat: u32, react: Option<String>) -> Status {
        ctx.nav.stop();
        ctx.out.motion = Motion::Hold;
        let Phase::Watch { beat: seen, act } = &mut self.phase else { return Status::Done };
        if *seen != beat {
            *seen = beat;
            if let Some(r) = react.as_deref() {
                *act = startable(ctx, r);
            }
        }
        // Following him with his gaze (turning to him first, then the answer).
        ctx.face_toward(at.x);
        if let Some(w) = act
            && w.tick(ctx)
        {
            *act = None;
        }
        Status::Running
    }

    fn flame(&mut self, ctx: &mut Ctx, at: Vec2) -> Status {
        match &mut self.phase {
            Phase::ToFlame(t) => match t.tick(ctx) {
                TravelStep::Moving => Status::Running,
                TravelStep::Arrived => {
                    let left = ctx.range(FLAME_LINGER);
                    self.phase = Phase::Linger { left, next: ctx.range([2.0, 4.0]), act: None, step: None };
                    Status::Running
                }
                TravelStep::Failed => Status::Failed,
            },
            Phase::Linger { left, next, act, step } => {
                *left -= ctx.dt();
                if *left <= 0.0 && act.is_none() && step.is_none() {
                    return Status::Done; // enough staring for now
                }
                if let Some(t) = step {
                    match t.tick(ctx) {
                        TravelStep::Moving => return Status::Running,
                        _ => *step = None,
                    }
                }
                if let Some(w) = act {
                    if w.tick(ctx) {
                        *act = None;
                    }
                    return Status::Running;
                }
                ctx.out.motion = Motion::Hold;
                ctx.face_toward(at.x);
                *next -= ctx.dt();
                if *next <= 0.0 {
                    *next = ctx.range([4.0, 8.0]);
                    if ctx.rng.chance(0.45) {
                        // Pace to the flame's other side.
                        let side = if ctx.p.knight.pos.x >= at.x { -1.0 } else { 1.0 };
                        let goal = by_flame(ctx, at, side);
                        *step = Some(Travel::to(ctx, goal, TravelStyle::Stroll));
                    } else {
                        *act = startable(ctx, "look_down|glance");
                    }
                }
                ctx.out.waiting = false;
                Status::Running
            }
            _ => {
                let side = if ctx.p.knight.pos.x >= at.x { 1.0 } else { -1.0 };
                self.phase = Phase::ToFlame(Travel::to(ctx, by_flame(ctx, at, side), TravelStyle::Normal));
                Status::Running
            }
        }
    }

    pub fn reason(&self) -> String {
        match &self.phase {
            Phase::Ready => "scene: race, ready for the start".into(),
            Phase::Racing { tries, .. } => format!("scene: racing to the needle (try {})", tries + 1),
            Phase::Waiting => "scene: race, waiting for the result".into(),
            Phase::React(_) => "scene: race decided, reacting".into(),
            Phase::Watch { .. } => "scene: watching the guest".into(),
            Phase::ToFlame(_) => "scene: curious about a scarlet flame, going over".into(),
            Phase::Linger { left, .. } => format!("scene: lingering by the scarlet flame ({left:.0} s)"),
        }
    }
}
