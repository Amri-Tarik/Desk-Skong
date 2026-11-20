//! Keeping a visitor company (calm scenes, see `crate::npc::company`): after the talk a
//! visitor may invite the knight to stay a while. He stays beside them, and when they
//! settle in he plays his part (sits with his map open beside Cornifer, sits back and takes
//! in the view with Quirrel, listens to Myla's song), then gets up as they part.
//!
//! Strict priorities: the Shade (urgent) always cuts in; enemies, a fullscreen app or the
//! user typing hard end the scene politely ([`Intent::part`]); Geo and pastimes wait (the
//! behaviour is an exclusive engagement, like a lesson).
use crate::nav::TravelStyle;

use super::FoeKind;
use super::behaviours::{ActWait, Ctx, Status, Travel, TravelStep};
use super::greet::{Visitor, VisitorState, beside, stand_point, startable};

/// Signal to the knight's scene act: the visitor is parting (put the map away, get up).
pub const PARTED: &str = "parted";

/// Where a visitor is in a calm scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompanyStage {
    /// Saying the invitation: stay beside them.
    #[default]
    Invite,
    /// Together: the knight plays his part.
    Together,
    /// The parting line: wrap up (close the map, get up).
    Parting,
}

/// What a visitor keeping company wants (part of [`Visitor`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompanyCue {
    /// The knight's part: act names, `a|b` = the first that can start here.
    pub act: String,
    pub stage: CompanyStage,
}

/// The visitor the knight keeps company with, if any.
pub(crate) fn host(visitors: &[Visitor]) -> Option<&Visitor> {
    visitors.iter().find(|v| v.state == VisitorState::Company)
}

#[derive(Clone, Debug)]
enum Phase {
    /// Beside them.
    Stay,
    /// Back beside them (he was nudged off, or the scene began a step away).
    Return(Travel),
}

#[derive(Clone, Debug)]
pub(crate) struct Company {
    pub id: u32,
    name: String,
    phase: Phase,
    /// His part (sitting with the map...), once started.
    act: Option<ActWait>,
    /// His part was tried (it may not have been possible here).
    tried: bool,
    /// The scene ended (finished or stopped): the brain sends no `part` when switching.
    pub ended: bool,
    return_ready_at: f32,
    /// Seconds spent together.
    together: f32,
}

impl Company {
    pub fn start(ctx: &mut Ctx, v: &Visitor) -> Self {
        ctx.mind.adjust(|n| &mut n.boredom, -0.2);
        ctx.mind.adjust(|n| &mut n.loneliness, -0.3);
        Self { id: v.id, name: v.name.clone(), phase: Phase::Stay, act: None, tried: false, ended: false, return_ready_at: 0.0, together: 0.0 }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let host = ctx.senses.visitors.iter().find(|v| v.id == self.id && v.state == VisitorState::Company).cloned();
        let Some(v) = host else {
            // They parted (or left): finish getting up, then carry on, warmed.
            self.ended = true;
            ctx.nav.stop();
            if self.wrap_up(ctx) {
                return Status::Running;
            }
            if self.together > 5.0 {
                ctx.mind.adjust(|n| &mut n.loneliness, -0.4);
                ctx.mind.adjust(|n| &mut n.boredom, -0.3);
            }
            return Status::Done;
        };
        let s = ctx.senses;
        let enemies = s.foes.iter().any(|f| f.kind != FoeKind::Sparring);
        if s.user_busy || s.fullscreen || enemies {
            // The screen is needed, or there's a fight to be had: excuse himself.
            self.stop(ctx);
            return Status::Done;
        }
        let Some(cue) = v.company.clone() else { return Status::Running };
        ctx.out.waiting = false;
        match &mut self.phase {
            Phase::Return(t) => match t.tick(ctx) {
                TravelStep::Moving => return Status::Running,
                TravelStep::Arrived => self.phase = Phase::Stay,
                TravelStep::Failed => {
                    self.return_ready_at = ctx.now + 6.0;
                    self.phase = Phase::Stay;
                }
            },
            Phase::Stay => {}
        }
        match cue.stage {
            CompanyStage::Invite => self.stay(ctx, &v),
            CompanyStage::Together => {
                self.together += ctx.dt();
                if self.act.is_none() && !self.tried {
                    let goal = stand_point(ctx, &v);
                    if !beside(ctx, &v, goal) {
                        self.stay(ctx, &v);
                        return Status::Running;
                    }
                    ctx.nav.stop();
                    ctx.face_toward(v.pos.x);
                    self.tried = true;
                    self.act = startable(ctx, &cue.act);
                }
                match &mut self.act {
                    Some(w) => {
                        ctx.nav.stop();
                        if w.tick(ctx) {
                            // Over early (it timed out, or was cut): just stay with them.
                            self.act = None;
                        }
                    }
                    None => self.stay(ctx, &v),
                }
            }
            CompanyStage::Parting => {
                ctx.nav.stop();
                if !self.wrap_up(ctx) {
                    ctx.face_toward(v.pos.x);
                }
            }
        }
        Status::Running
    }

    /// Stand beside them, facing them; walk back first if he drifted off.
    fn stay(&mut self, ctx: &mut Ctx, v: &Visitor) {
        let goal = stand_point(ctx, v);
        if !beside(ctx, v, goal) && ctx.grounded() && ctx.now >= self.return_ready_at && ctx.free_to_move() {
            self.phase = Phase::Return(Travel::to(ctx, goal, TravelStyle::Normal));
            return;
        }
        ctx.nav.stop();
        ctx.face_toward(v.pos.x);
    }

    /// Ask his part to wrap up (the map closes, he gets up); `true` while it still plays.
    fn wrap_up(&mut self, ctx: &mut Ctx) -> bool {
        let Some(w) = &mut self.act else { return false };
        if ctx.p.act.is_some_and(|a| a.id == w.id) {
            ctx.out.signal = Some(PARTED);
        }
        if w.tick(ctx) {
            self.act = None;
            return false;
        }
        true
    }

    fn stop(&mut self, ctx: &mut Ctx) {
        if !self.ended {
            self.ended = true;
            ctx.nav.stop();
            ctx.out.part = Some(self.id);
        }
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        match (&self.phase, &self.act) {
            (Phase::Return(_), _) => format!("company with {}: back beside them", self.name),
            (_, Some(w)) => format!("company with {}: {}", self.name, ctx.rep.catalog.get(w.id).map_or("?", |a| a.name.as_str())),
            _ => format!("company with {}: staying beside them", self.name),
        }
    }
}
