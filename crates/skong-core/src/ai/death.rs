//! Death and respawn: land, play the death act (the app spawns the Shade and drops the
//! mask piece), stay gone for a moment, ask the app to put him back at the nearest bench or
//! a safe ledge, then wake up there — sore and a little sad.
use crate::geom::Vec2;
use crate::stakes::{DeathKind, respawn_point};

use super::behaviours::{ActWait, Ctx, Status};
use super::stakes::StakeEvent;

#[derive(Clone, Copy, Debug)]
pub(crate) enum DeadPhase {
    /// Waiting to land (deaths are grounded choreography).
    Falling { waited: f32 },
    Dying(Option<ActWait>),
    /// Gone (the app hides him) until the respawn.
    Gone { left: f32 },
    /// Respawn requested at `at`: wait to be there, then wake up.
    Rising { at: Vec2, waited: f32, act: Option<ActWait> },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Dead {
    pub kind: DeathKind,
    pub cracked: bool,
    pub phase: DeadPhase,
}

impl Dead {
    pub fn start(ctx: &mut Ctx, kind: DeathKind, cracked: bool) -> Self {
        ctx.nav.stop();
        ctx.mind.emotions.excitement = 0.0;
        ctx.stakes.dying = true;
        Self { kind, cracked, phase: DeadPhase::Falling { waited: 0.0 } }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        ctx.nav.stop();
        match &mut self.phase {
            DeadPhase::Falling { waited } => {
                *waited += ctx.dt();
                if ctx.grounded() || *waited > 4.0 {
                    let at = ctx.p.knight.pos;
                    ctx.out.stakes.push(StakeEvent::Died { at, kind: self.kind, cracked: self.cracked });
                    self.phase = DeadPhase::Dying(ActWait::named(ctx, self.kind.act_name()));
                }
            }
            DeadPhase::Dying(act) => {
                if act.as_mut().is_none_or(|w| w.tick(ctx)) {
                    self.phase = DeadPhase::Gone { left: ctx.cfg.stakes.dead_secs };
                }
            }
            DeadPhase::Gone { left } => {
                *left -= ctx.dt();
                if *left <= 0.0 {
                    let at = respawn_point(ctx.p.terrain, ctx.p.benches, ctx.p.knight.pos);
                    ctx.out.stakes.push(StakeEvent::Respawn { at });
                    self.phase = DeadPhase::Rising { at, waited: 0.0, act: None };
                }
            }
            DeadPhase::Rising { at, waited, act } => {
                *waited += ctx.dt();
                if act.is_none() {
                    let there = ctx.p.knight.pos.distance(*at) < 40.0 && ctx.grounded();
                    if !there && *waited < 2.5 {
                        return Status::Running;
                    }
                    let name = if ctx.senses.can_sit() { "respawn_wake" } else { "hazard_respawn" };
                    *act = ActWait::named(ctx, name);
                    if act.is_none() {
                        return self.finish(ctx);
                    }
                }
                if act.as_mut().is_some_and(|w| w.tick(ctx)) {
                    return self.finish(ctx);
                }
            }
        }
        Status::Running
    }

    /// Back on his feet: sore, a little sad; a few seconds to get his bearings, then the
    /// Shade (a breather set when the Shade itself killed him still holds).
    fn finish(&mut self, ctx: &mut Ctx) -> Status {
        ctx.mind.hurt(0.45);
        ctx.mind.adjust(|n| &mut n.loneliness, 0.25);
        ctx.mind.adjust(|n| &mut n.energy, -0.15);
        ctx.mind.adjust(|n| &mut n.mischief, -0.6);
        ctx.stakes.shade_ready_at = ctx.stakes.shade_ready_at.max(ctx.now + ctx.cfg.stakes.shade_delay as f64);
        // Somewhere new: look for a way to it afresh.
        ctx.stakes.shade_way = true;
        ctx.stakes.dying = false;
        Status::Done
    }

    pub fn reason(&self) -> String {
        match self.phase {
            DeadPhase::Falling { .. } => "dead: falling".into(),
            DeadPhase::Dying(_) => format!("dead: {}", self.kind.act_name().replace('_', " ")),
            DeadPhase::Gone { .. } => "dead: gone".into(),
            DeadPhase::Rising { .. } => "dead: respawning".into(),
        }
    }
}
