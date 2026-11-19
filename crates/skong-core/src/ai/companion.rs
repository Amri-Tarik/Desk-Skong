//! Cursor companion: the default when the user is around and nothing else is interesting.
//!
//! The knight keeps within a comfortable radius of the cursor's projection onto terrain
//! (the ledge under it), standing a little to the side rather than right under it. He
//! strolls after a drifting cursor and hurries when it is far, faces it, looks up / down
//! at it, stares at the user when the cursor rests near him, and settles down on a nearby
//! seat when the cursor stays parked. At `bother_level` 0 he keeps a polite distance and
//! never hurries; while the user types fast he stays put.
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Beat, Ctx, SEAT_TOL, Status, Travel, TravelStep, find_seat};
use super::interest::PoiTarget;
use super::spots;

/// Minimum seconds between two pacing strolls.
const PACE_GAP: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Doing {
    Following,
    Settling,
    /// Nothing to do near a resting cursor: stroll over to its other side.
    Pacing,
    Attending,
    Staying,
}

#[derive(Clone, Debug)]
pub(crate) struct Companion {
    travel: Option<Travel>,
    /// Where the current travel goes.
    goal: Vec2,
    goal_set_at: f32,
    /// Seconds the cursor has been out of comfort range.
    drift: f32,
    /// Don't follow again before this time (after a failed trip).
    rest_until: f32,
    beat: Beat,
    last_stare: f32,
    last_pace: f32,
    last_gaze: f32,
    doing: Doing,
    /// Last act's reason.
    why: &'static str,
}

impl Companion {
    pub fn start(ctx: &mut Ctx) -> Self {
        let gap = ctx.range(ctx.cfg.companion.gap);
        Self {
            travel: None,
            goal: ctx.p.knight.pos,
            goal_set_at: f32::NEG_INFINITY,
            drift: 0.0,
            rest_until: 0.0,
            beat: Beat::new(gap),
            last_stare: f32::NEG_INFINITY,
            last_pace: f32::NEG_INFINITY,
            last_gaze: f32::NEG_INFINITY,
            doing: Doing::Attending,
            why: "",
        }
    }

    /// Bother-scaled (comfort radius, stand-beside offset).
    fn radii(ctx: &Ctx) -> (f32, f32) {
        let c = &ctx.cfg.companion;
        let b = ctx.cfg.bother_level.clamp(0.0, 1.0);
        (c.polite_comfort + (c.comfort - c.polite_comfort) * b, c.polite_beside + (c.beside - c.polite_beside) * b)
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let Some(c) = ctx.senses.cursor else { return Status::Done };
        let cfg = ctx.cfg.companion;
        let k = ctx.p.knight;
        let (comfort, beside) = Self::radii(ctx);
        let proj = spots::project(ctx.p.terrain, c.pos).unwrap_or(c.pos);
        let side = if k.pos.x >= proj.x { 1.0 } else { -1.0 };
        let mut want = Vec2::new(proj.x + side * beside, proj.y);
        // Stay on the cursor's monitor: never "beside" it across a monitor edge.
        if let Some(m) = ctx.p.desktop.monitor_at(c.pos.x as i32, c.pos.y as i32) {
            let r = m.rect.to_f();
            if want.x < r.left + 20.0 || want.x > r.right - 20.0 {
                want.x = (proj.x - side * beside).clamp(r.left + 20.0, (r.right - 20.0).max(r.left + 21.0));
            }
        }
        let off = want - k.pos;
        // Being on another level counts extra: a ledge 300 px up is a real trip.
        let dist = off.x.abs() + 1.5 * off.y.abs();

        // Typing fast: keep still and quiet, don't chase the caret's mouse.
        if ctx.senses.user_busy {
            if self.travel.take().is_some() {
                ctx.nav.stop();
            }
            self.doing = Doing::Staying;
            self.drift = 0.0;
            return self.keep_company(ctx, false);
        }

        if let Some(t) = &mut self.travel {
            let moved = self.goal.distance(want) > 150.0;
            if moved && ctx.now - self.goal_set_at > 0.8 && self.doing == Doing::Following {
                t.retarget(NavGoal::Point(want));
                self.goal = want;
                self.goal_set_at = ctx.now;
            }
            match t.tick(ctx) {
                TravelStep::Moving => return Status::Running,
                TravelStep::Arrived => {
                    self.travel = None;
                    if self.doing == Doing::Settling {
                        let act = ctx.seated_act();
                        self.why = "settling down near you";
                        self.beat.play(ctx, act, cfg.gap);
                    }
                }
                TravelStep::Failed => {
                    self.travel = None;
                    self.rest_until = ctx.now + 8.0;
                }
            }
            self.doing = Doing::Attending;
        }

        let reachable = !ctx.interest.is_blocked(PoiTarget::Spot(want));
        if dist > comfort && ctx.now >= self.rest_until && reachable {
            self.drift += ctx.dt();
            if self.drift >= cfg.drift_secs {
                let polite = ctx.cfg.bother_level <= 0.0;
                let style = if dist > cfg.hurry_beyond && !polite {
                    TravelStyle::Hurry
                } else if dist > 2.0 * comfort {
                    TravelStyle::Normal
                } else {
                    TravelStyle::Stroll
                };
                self.drift = 0.0;
                // Only follow if the route really ends near the cursor (not, say, under a
                // ledge this style cannot mount); otherwise keep company from here.
                let ends_near = ctx.nav.reach(k, NavGoal::Point(want), style).is_some_and(|r| {
                    r.end.is_none_or(|e| (e.x - want.x).abs() + 1.5 * (e.y - want.y).abs() <= comfort)
                });
                if !ends_near {
                    ctx.interest.block(PoiTarget::Spot(want), ctx.now + 20.0);
                    self.rest_until = ctx.now + 5.0;
                } else {
                    self.travel = Some(Travel::to(ctx, want, style));
                    self.goal = want;
                    self.goal_set_at = ctx.now;
                    self.doing = Doing::Following;
                    return Status::Running;
                }
            }
        } else {
            self.drift = 0.0;
        }
        self.doing = Doing::Attending;
        self.keep_company(ctx, true)
    }

    /// Standing near the user: face the cursor and react to it every few seconds.
    fn keep_company(&mut self, ctx: &mut Ctx, may_move: bool) -> Status {
        let Some(c) = ctx.senses.cursor else { return Status::Done };
        let cfg = ctx.cfg.companion;
        if !self.beat.busy() {
            ctx.face_cursor();
        }
        let tracking = c.speed() > 60.0;
        if !self.beat.tick(ctx, cfg.gap) {
            if tracking {
                ctx.out.waiting = false; // following the cursor with his eyes
            }
            return Status::Running;
        }

        let parked = c.still_for >= cfg.settle_secs;
        let (act, why): (Option<ActWait>, &'static str) = if parked && ctx.senses.can_sit() && ctx.may_sit() {
            (ctx.seated_act(), "settling down near you")
        } else if parked && may_move && ctx.may_sit() {
            let proj = spots::project(ctx.p.terrain, c.pos).unwrap_or(c.pos);
            match find_seat(ctx, proj, cfg.seat_range) {
                Some((seat, style)) => {
                    self.travel = Some(Travel::exact(ctx, seat, style, SEAT_TOL));
                    self.goal = seat;
                    self.goal_set_at = ctx.now;
                    self.doing = Doing::Settling;
                    return Status::Running;
                }
                None => self.contextual(ctx),
            }
        } else {
            self.contextual(ctx)
        };
        if act.is_none() && parked && may_move && self.pace(ctx) {
            return Status::Running;
        }
        self.why = why;
        self.beat.play(ctx, act, cfg.gap);
        Status::Running
    }

    /// Nothing fits and he may not sit again yet: move instead of standing around:
    /// stroll to the cursor's other side (if a stroll really gets there).
    fn pace(&mut self, ctx: &mut Ctx) -> bool {
        let Some(c) = ctx.senses.cursor else { return false };
        if ctx.now - self.last_pace < PACE_GAP {
            return false;
        }
        self.last_pace = ctx.now;
        let k = ctx.p.knight;
        let (_, beside) = Self::radii(ctx);
        let proj = spots::project(ctx.p.terrain, c.pos).unwrap_or(c.pos);
        let side = if k.pos.x >= proj.x { -1.0 } else { 1.0 };
        let to = Vec2::new(proj.x + side * beside.max(80.0), proj.y);
        let same_monitor = ctx.p.desktop.monitor_at(to.x as i32, (to.y - 1.0) as i32).map(|m| m.rect)
            == ctx.p.desktop.monitor_at(c.pos.x as i32, c.pos.y as i32).map(|m| m.rect);
        if !same_monitor || ctx.interest.is_blocked(PoiTarget::Spot(to)) || !ctx.nav.reaches(k, to, TravelStyle::Stroll, 40.0) {
            return false;
        }
        self.travel = Some(Travel::to(ctx, to, TravelStyle::Stroll));
        self.goal = to;
        self.goal_set_at = ctx.now;
        self.doing = Doing::Pacing;
        true
    }

    /// The act that fits where the cursor is.
    fn contextual(&mut self, ctx: &mut Ctx) -> (Option<ActWait>, &'static str) {
        let Some(c) = ctx.senses.cursor else { return (None, "") };
        let cfg = ctx.cfg.companion;
        let back = ctx.senses.returned_at.is_some_and(|t| ctx.now - t < 15.0 && self.last_stare < t);
        if back {
            self.last_stare = ctx.now;
            return (ActWait::named(ctx, "stare_at_user"), "you're back");
        }
        if c.offset.y < -180.0 && c.offset.x.abs() < 500.0 {
            return (ActWait::named(ctx, "look_up"), "cursor above");
        }
        if c.offset.y > 140.0 && c.offset.x.abs() < 500.0 {
            return (ActWait::named(ctx, "look_down"), "cursor below");
        }
        if c.still_for > 3.0 && c.dist() < 400.0 && ctx.now - self.last_stare >= cfg.stare_cooldown {
            self.last_stare = ctx.now;
            return (ActWait::named(ctx, "stare_at_user"), "cursor resting near me");
        }
        // The lantern comes out right next to you in the dark; the map only when you're
        // not right here.
        if (ctx.senses.dark || c.dist() > 250.0)
            && let Some(a) = ctx.wander_act()
        {
            let why = if ctx.rep.catalog.get(a.id).is_some_and(|i| i.name == "lantern_patrol") { "it's dark" } else { "checking his map" };
            return (Some(a), why);
        }
        if ctx.senses.can_sit() && c.still_for > 2.0 && ctx.may_sit() {
            return (ctx.seated_act(), "sitting with you");
        }
        if c.still_for >= cfg.settle_secs && c.dist() < 350.0 && ctx.now - self.last_gaze >= 10.0 {
            self.last_gaze = ctx.now;
            return (ActWait::named(ctx, "gaze_into_screen"), "looking at what you're pointing at");
        }
        if ctx.rng.chance(cfg.fidget_chance) {
            return (ctx.fidget(), "fidget");
        }
        (None, "")
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        let at = ctx.senses.cursor.map_or(Vec2::ZERO, |c| c.pos);
        match self.doing {
            Doing::Following => format!("companion: following cursor @ ({:.0},{:.0})", at.x, at.y),
            Doing::Settling => "companion: going to sit near you".into(),
            Doing::Pacing => "companion: strolling to your other side".into(),
            Doing::Staying => "companion: you're typing, staying put".into(),
            Doing::Attending => match self.beat.doing_name(ctx) {
                Some(n) => format!("companion: {n} ({})", self.why),
                None => "companion: keeping you company".into(),
            },
        }
    }
}
