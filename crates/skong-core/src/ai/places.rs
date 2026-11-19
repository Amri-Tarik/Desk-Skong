//! Known places: things the knight found inside windows (benches, hot springs, geo
//! deposits; see [`crate::discovery`]) and the **Visit** behaviour that uses them.
//!
//! The app tells the brain what is on screen every tick ([`Brain::set_places`]); a place
//! flagged `fresh` was just revealed, and the knight reacts to it at once (looks at it,
//! walks over, tries it out). Otherwise places are chosen by need: found seats and
//! springs when tired (favourites first), deposits when playful.
use std::collections::{BTreeMap, BTreeSet};

use crate::controller::Side;
use crate::desktop::WindowId;
use crate::geom::Vec2;
use crate::nav::{NavGoal, TravelStyle};

use super::behaviours::{ActWait, Beat, Ctx, SEAT_TOL, Status, Travel, TravelStep, style_for};
use super::interest::PoiTarget;
use super::needs::Mind;
use super::navigator::Navigator;
use super::percept::Percepts;
use super::{Attack, AttackDir, Brain};

pub type PlaceId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlaceKind {
    Bench,
    Spring,
    Deposit,
}

impl PlaceKind {
    pub fn label(self) -> &'static str {
        match self {
            PlaceKind::Bench => "bench",
            PlaceKind::Spring => "hot spring",
            PlaceKind::Deposit => "geo deposit",
        }
    }

    /// Seconds before a used place is wanted again.
    fn cooldown(self) -> f32 {
        match self {
            PlaceKind::Bench => 120.0,
            PlaceKind::Spring => 900.0,
            PlaceKind::Deposit => 90.0,
        }
    }
}

/// A found thing on screen right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnownPlace {
    pub id: PlaceId,
    pub kind: PlaceKind,
    /// Feet position to use it from (on the bench / in the spring / beside the deposit).
    pub at: Vec2,
    /// The prop's bottom center.
    pub center: Vec2,
    /// 0 = never used, → 1 = a favourite.
    pub favour: f32,
    /// Just revealed: worth an immediate look.
    pub fresh: bool,
}

/// Discovery traffic from the brain to the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// Arrived on a never-visited window: roll / reveal its room.
    Explored(WindowId),
    /// Sat on / bathed in this place.
    Used(PlaceId),
    /// Slashed this deposit (the attack is in the same intent).
    Struck(PlaceId),
}

/// The best place to visit right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlaceChoice {
    pub place: KnownPlace,
    pub score: f32,
    pub fresh: bool,
}

/// Places on screen plus what the knight did with them (lives in the interest map).
#[derive(Clone, Debug, Default)]
pub struct PlaceMemory {
    places: Vec<KnownPlace>,
    used: BTreeMap<PlaceId, f32>,
    noticed: BTreeSet<PlaceId>,
    /// The visit in progress: place and whether it was just found.
    current: Option<(PlaceId, bool)>,
    pub(crate) best: Option<PlaceChoice>,
}

impl PlaceMemory {
    pub fn all(&self) -> &[KnownPlace] {
        &self.places
    }

    pub fn get(&self, id: PlaceId) -> Option<KnownPlace> {
        self.places.iter().find(|p| p.id == id).copied()
    }

    fn cooling(&self, p: &KnownPlace, now: f32) -> bool {
        self.used.get(&p.id).is_some_and(|t| now - t < p.kind.cooldown())
    }

    fn needs_notice(&self, p: &KnownPlace) -> bool {
        p.fresh && !self.noticed.contains(&p.id)
    }
}

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much the knight wants this place right now (before travel).
pub(crate) fn want(p: &KnownPlace, m: &Mind) -> f32 {
    let tired = m.tiredness();
    let calm = 1.0 - 0.6 * m.arousal();
    match p.kind {
        // Beats sitting on any old ledge when tired (Rest tops out at 0.75 here).
        PlaceKind::Bench => (0.1 + 0.85 * smooth(0.3, 0.7, tired) + 0.12 * p.favour) * calm,
        // Restores faster than a bench: slightly preferred, and earlier.
        PlaceKind::Spring => (0.12 + 0.9 * smooth(0.25, 0.65, tired) + 0.12 * p.favour) * calm,
        PlaceKind::Deposit if m.needs.energy < 0.3 => 0.0,
        PlaceKind::Deposit => 0.7 * m.needs.mischief + 0.35 * m.emotions.excitement + 0.2 * m.needs.boredom * m.personality.playfulness + 0.05 * p.favour,
    }
}

impl Brain {
    /// What has been found on screen (call before [`Brain::tick`], every tick or on change).
    pub fn set_places(&mut self, places: &[KnownPlace]) {
        let mem = &mut self.interest.places;
        mem.places.clear();
        mem.places.extend_from_slice(places);
    }

    /// Places the brain currently knows about.
    pub fn places(&self) -> &[KnownPlace] {
        self.interest.places.all()
    }

    /// The place being visited, if any.
    pub fn visiting(&self) -> Option<KnownPlace> {
        match &self.active {
            super::behaviours::Active::Visit(v) => Some(v.place),
            _ => None,
        }
    }

    /// Rank places: fresh ones first (urgent), else by need minus travel.
    pub(crate) fn score_places(&mut self, p: &Percepts, nav: &mut dyn Navigator) {
        let knight = p.knight;
        let style = self.mind.travel_style();
        // The place being visited (until that visit is done) is exempt from its cooldown,
        // and a just-found one is tried out before anything else is considered.
        let current = self.interest.places.current.filter(|_| matches!(self.active, super::behaviours::Active::Visit(_)));
        let visiting = current.map(|c| c.0);
        let trying = current.filter(|c| c.1).map(|c| c.0);
        let mem = &self.interest.places;
        let mut ranked: Vec<(KnownPlace, f32, bool)> = mem
            .places
            .iter()
            .filter(|pl| !self.interest.is_blocked(PoiTarget::Spot(pl.at)))
            .filter_map(|pl| {
                let fresh = mem.needs_notice(pl) && pl.at.distance(knight.pos) < 1600.0;
                if !fresh && Some(pl.id) != visiting && mem.cooling(pl, self.now) {
                    return None;
                }
                let w = match fresh {
                    true => 1.1,
                    false if Some(pl.id) == trying => want(pl, &self.mind).max(0.95),
                    false => want(pl, &self.mind),
                };
                (w > 0.05).then_some((*pl, w, fresh))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.interest.places.best = ranked
            .into_iter()
            .take(2)
            .filter_map(|(place, w, fresh)| {
                let here = place.at.distance(knight.pos) < 4.0 && knight.is_grounded();
                // Only places he can actually get onto (not just near).
                let eta = if here {
                    0.0
                } else {
                    let r = nav.reach(knight, NavGoal::Point(place.at), style)?;
                    if r.end.is_some() && !r.lands_on(place.at, SEAT_TOL + 2.0) {
                        return None;
                    }
                    r.eta
                };
                let travel = if fresh { 0.0 } else { self.cfg.travel_cost_per_sec * eta };
                Some(PlaceChoice { place, score: w - travel, fresh })
            })
            .max_by(|a, b| a.score.total_cmp(&b.score));
    }

    /// Utility of the Visit behaviour.
    pub(crate) fn visit_utility(&self) -> f32 {
        let s = &self.senses;
        if s.fullscreen || s.user_away && self.mind.tiredness() > 0.7 {
            return 0.0; // hiding, or too sleepy: nap (which prefers favourite benches) instead
        }
        self.interest.places.best.map_or(0.0, |c| c.score.clamp(0.0, if c.fresh { 1.1 } else { 0.95 }))
    }
}

// --- Visit --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Just revealed: look at it a moment.
    Notice { act: Option<ActWait>, left: f32 },
    Travel(Travel),
    /// Seated acts on a found bench.
    Sit { beat: Beat, acts: u32 },
    /// The "bathe" act in a spring.
    Bathe(Option<ActWait>),
    /// A few slashes at a deposit.
    Strike { left: u32, gap: f32 },
}

/// Go to a known place and use it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Visit {
    pub place: KnownPlace,
    phase: Phase,
    fresh: bool,
    /// Seconds the place has been missing (a flickering ledge; gone for good after a while).
    missing: f32,
}

impl Visit {
    const STRIKES: u32 = 3;

    pub fn start(ctx: &mut Ctx, choice: PlaceChoice) -> Self {
        let place = choice.place;
        ctx.interest.places.noticed.insert(place.id);
        ctx.interest.places.current = Some((place.id, choice.fresh));
        let phase = if choice.fresh {
            ctx.mind.excite(0.25);
            ctx.mind.adjust(|n| &mut n.boredom, -0.15);
            let eye = ctx.p.knight.body().center();
            let dy = place.center.y - eye.y;
            let act = if dy < -100.0 {
                ActWait::named(ctx, "look_up")
            } else if dy > 60.0 {
                ActWait::named(ctx, "look_down")
            } else {
                None
            };
            Phase::Notice { act, left: 0.8 }
        } else {
            Self::travel(ctx, &place)
        };
        Self { place, phase, fresh: choice.fresh, missing: 0.0 }
    }

    fn travel(ctx: &mut Ctx, place: &KnownPlace) -> Phase {
        let style = match style_for(ctx, place.at) {
            TravelStyle::Stroll if ctx.mind.needs.energy > 0.4 && place.kind == PlaceKind::Deposit => TravelStyle::Normal,
            s => s,
        };
        Phase::Travel(Travel::exact(ctx, place.at, style, SEAT_TOL + 2.0))
    }

    fn arrive(&mut self, ctx: &mut Ctx) {
        let p = self.place;
        ctx.face_toward(p.center.x);
        self.phase = match p.kind {
            PlaceKind::Bench => Phase::Sit { beat: Beat::new(0.2), acts: 0 },
            PlaceKind::Spring => Phase::Bathe(ActWait::named(ctx, "bathe")),
            PlaceKind::Deposit => Phase::Strike { left: Self::STRIKES, gap: 0.3 },
        };
        if p.kind != PlaceKind::Deposit {
            ctx.out.discovery = Some(DiscoveryEvent::Used(p.id));
        }
        ctx.interest.places.used.insert(p.id, ctx.now);
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let s = self.step(ctx);
        if s != Status::Running {
            ctx.interest.places.current = None;
        }
        s
    }

    fn step(&mut self, ctx: &mut Ctx) -> Status {
        // Follow the place (its window may move); gone = window closed / deposit broken.
        match ctx.interest.places.get(self.place.id) {
            Some(p) => {
                self.place = p;
                self.missing = 0.0;
            }
            None if self.missing < 2.0 && !matches!(self.phase, Phase::Strike { .. }) => self.missing += ctx.dt(),
            None => {
                if matches!(self.phase, Phase::Travel(_)) {
                    ctx.nav.stop();
                }
                return Status::Done;
            }
        }
        let place = self.place;
        match &mut self.phase {
            Phase::Notice { act, left } => {
                ctx.nav.stop();
                ctx.face_toward(place.center.x);
                *left -= ctx.dt();
                if act.as_mut().is_none_or(|w| w.tick(ctx)) && *left <= 0.0 {
                    if place.at.distance(ctx.p.knight.pos) < 4.0 && ctx.grounded() {
                        self.arrive(ctx);
                    } else {
                        self.phase = Self::travel(ctx, &place);
                    }
                }
                Status::Running
            }
            Phase::Travel(t) => {
                if let NavGoal::Point(g) = t.goal
                    && g.distance(place.at) > 6.0
                {
                    t.retarget(NavGoal::Point(place.at));
                }
                match t.tick(ctx) {
                    TravelStep::Moving => Status::Running,
                    TravelStep::Failed => Status::Failed,
                    TravelStep::Arrived => {
                        self.arrive(ctx);
                        Status::Running
                    }
                }
            }
            Phase::Sit { beat, acts } => {
                if !beat.tick(ctx, [0.5, 1.5]) {
                    return Status::Running;
                }
                let rested = ctx.mind.tiredness() < 0.25;
                if !ctx.senses.can_sit() || *acts >= 3 || (*acts >= 1 && rested) {
                    return Status::Done;
                }
                *acts += 1;
                let act = ctx.seated_act();
                beat.play(ctx, act, [0.5, 1.5]);
                Status::Running
            }
            Phase::Bathe(act) => {
                ctx.nav.stop();
                let bathing = act.is_some_and(|w| ctx.p.act.is_some_and(|a| a.id == w.id));
                if bathing {
                    // On top of the seated rate: a hot spring restores faster than a bench.
                    let dt = ctx.dt();
                    ctx.mind.adjust(|n| &mut n.energy, 1.2 * ctx.cfg.needs.energy_restore * dt / 60.0);
                    ctx.mind.adjust(|n| &mut n.boredom, -0.02 * dt);
                    let e = &mut ctx.mind.emotions;
                    let calm = 0.5f32.powf(dt / 3.0);
                    e.annoyance *= calm;
                    e.startle *= calm;
                    e.hurt *= 0.5f32.powf(dt / 20.0);
                }
                if act.as_mut().is_none_or(|w| w.tick(ctx)) { Status::Done } else { Status::Running }
            }
            Phase::Strike { left, gap } => {
                ctx.nav.stop();
                *gap -= ctx.dt();
                if ctx.p.act.is_some() || *gap > 0.0 || !ctx.grounded() {
                    return Status::Running;
                }
                if *left == 0 {
                    return Status::Done;
                }
                *left -= 1;
                *gap = 0.55;
                let k = ctx.p.knight;
                let side = Side::of(place.center.x - k.pos.x).unwrap_or(k.facing);
                ctx.out.attack = Some(Attack { dir: AttackDir::Forward, side, hit: true });
                ctx.out.discovery = Some(DiscoveryEvent::Struck(place.id));
                ctx.mind.adjust(|n| &mut n.mischief, -0.12);
                ctx.mind.adjust(|n| &mut n.boredom, -0.05);
                ctx.mind.excite(0.08);
                Status::Running
            }
        }
    }

    pub fn reason(&self) -> String {
        let what = self.place.kind.label();
        match self.phase {
            Phase::Notice { .. } => format!("visit: found a {what}!"),
            Phase::Travel(t) => format!("visit {what}: walking over ({:?}){}", t.style, if self.fresh { ", just found" } else { "" }),
            Phase::Sit { .. } => format!("visit {what}: sitting on a found bench"),
            Phase::Bathe(_) => format!("visit {what}: bathing"),
            Phase::Strike { .. } => format!("visit {what}: mining geo"),
        }
    }
}

/// A favourite found bench within `range` he can get onto (naps go there first).
pub(crate) fn favourite_bench(ctx: &mut Ctx, range: f32) -> Option<(Vec2, TravelStyle)> {
    let k = ctx.p.knight;
    let mut favs: Vec<KnownPlace> = ctx
        .interest
        .places
        .all()
        .iter()
        .filter(|p| p.kind == PlaceKind::Bench && p.favour >= 0.3 && p.at.distance(k.pos) <= range && !ctx.interest.is_blocked(PoiTarget::Spot(p.at)))
        .copied()
        .collect();
    favs.sort_by(|a, b| (a.at.distance(k.pos) - 600.0 * a.favour).total_cmp(&(b.at.distance(k.pos) - 600.0 * b.favour)));
    favs.into_iter().take(2).find_map(|p| {
        [TravelStyle::Stroll, TravelStyle::Normal].into_iter().find(|&st| ctx.nav.reaches(k, p.at, st, SEAT_TOL)).map(|st| (p.at, st))
    })
}
