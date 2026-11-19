//! Watching screen activity: go to a vantage point (the edge above the changing region,
//! a ledge in front of it, or the one below), then watch for as long as it stays active —
//! gaze / look acts back to back with short pauses, sitting down for long-lived spots —
//! until curiosity is satisfied (satiation fills over a 4–15 s budget scaled by interest).
use super::behaviours::{ActWait, Beat, Ctx, Status, Travel, TravelStep, style_for};
use super::spots::{Vantage, View};

/// Satiation at which the knight has seen enough (satiation decays a little every tick, so
/// it never sits at exactly 1).
const SATISFIED: f32 = 0.98;
/// Arrival tolerance at a vantage point (px): any spot on that ledge near it will do.
pub(crate) const VANTAGE_TOL: f32 = 30.0;

#[derive(Clone, Copy, Debug)]
enum Phase {
    Travel(Travel),
    Watching,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WatchSpot {
    pub spot: u32,
    phase: Phase,
    view: View,
    /// Seconds spent watching.
    watched: f32,
    /// Watching time that satisfies curiosity about this spot.
    budget: f32,
    beat: Beat,
    seated: bool,
}

impl WatchSpot {
    /// `interest` 0..1 scales how long he'll watch.
    pub fn start(ctx: &mut Ctx, spot: u32, vantage: Vantage, interest: f32) -> Self {
        let w = ctx.cfg.watch;
        let budget = w.min_secs + (w.max_secs - w.min_secs) * interest.clamp(0.0, 1.0);
        let k = ctx.p.knight;
        let here = (vantage.point.x - k.pos.x).abs() < 30.0 && (vantage.point.y - k.pos.y).abs() < 4.0 && k.is_grounded();
        let phase = if here {
            Phase::Watching
        } else {
            let style = style_for(ctx, vantage.point);
            Phase::Travel(Travel::exact(ctx, vantage.point, style, VANTAGE_TOL))
        };
        Self { spot, phase, view: vantage.view, watched: 0.0, budget, beat: Beat::new(0.0), seated: false }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let Some(spot) = ctx.spots.get(self.spot).cloned() else { return Status::Done };
        if !spot.lively() && spot.quiet_for > ctx.cfg.watch.quiet_secs {
            return Status::Done;
        }
        if let Phase::Travel(t) = &mut self.phase {
            match t.tick(ctx) {
                TravelStep::Moving => return Status::Running,
                TravelStep::Arrived => self.phase = Phase::Watching,
                TravelStep::Failed => return Status::Failed,
            }
        }

        self.watched += ctx.dt();
        ctx.spots.satiate(self.spot, ctx.dt() / self.budget.max(0.5));
        ctx.mind.adjust(|n| &mut n.boredom, -0.03 * ctx.dt());
        let satisfied = spot.satiation >= SATISFIED;
        if satisfied && !self.beat.busy() {
            return Status::Done;
        }

        let eye = ctx.p.knight.body().center();
        self.view = View::of(spot.rect, spot.center, eye);
        if !self.beat.busy() {
            ctx.face_toward(spot.center.x);
        }
        ctx.out.waiting = false; // eyes on the spot, even between acts
        let w = ctx.cfg.watch;
        if self.beat.tick(ctx, w.gap) {
            let long_lived = spot.age >= 8.0 || self.watched >= w.sit_after;
            let act = if long_lived && !self.seated && ctx.senses.can_sit() {
                self.seated = true;
                ctx.seated_act()
            } else {
                ActWait::named(ctx, self.view.act())
            };
            self.beat.play(ctx, act, w.gap);
        }
        Status::Running
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        let Some(s) = ctx.spots.get(self.spot) else { return "watch activity (gone)".into() };
        let what = match self.phase {
            Phase::Travel(t) => format!("going ({:?})", t.style),
            Phase::Watching => format!("{} {:.0}/{:.0}s", self.beat.doing_name(ctx).unwrap_or(self.view.act()), self.watched, self.budget),
        };
        format!("watch activity @ ({:.0},{:.0}) {:.1} [{what}]", s.center.x, s.center.y, s.intensity)
    }
}
