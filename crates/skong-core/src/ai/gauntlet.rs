//! The knight's part in a gauntlet (see [`crate::enemy::gauntlet`]): the app runs the waves;
//! the brain sees where it is at through [`Brain::see_gauntlet`]. At the announcement he
//! readies himself (the "!" is the app's emote on the switch, then his `challenge` bow) and
//! stands ready; the waves themselves are ordinary fights (`Fight`); between waves he
//! gathers the Geo; when the last wave falls he cheers (`victory`) and then collects the
//! reward like any other Geo.
use crate::enemy::{GauntletCue, GauntletStage};

use super::behaviours::{ActWait, Ctx, Status};
use super::{Active, Brain, Motion, Percepts};

impl Brain {
    /// The gauntlet on screen, if any (call before [`Brain::tick`]).
    pub fn see_gauntlet(&mut self, cue: Option<GauntletCue>) {
        if let Some(c) = cue
            && self.senses.gauntlet.is_none_or(|o| o.serial != c.serial)
        {
            self.mind.excite(0.4);
        }
        self.senses.gauntlet = cue;
    }

    /// He backed off from a fight (knocked about, or it dragged on) and leaves enemies alone
    /// for now: a gauntlet should end.
    pub fn fight_backed_off(&self) -> bool {
        self.now < self.stakes.fight.ready_at
    }

    /// Utility of the gauntlet's ceremonies (the announcement, the closing cheer); 0 during
    /// the waves and breathers (fights and Geo then), and once a ceremony was done. One
    /// under way keeps its score until its bow / cheer is over (they're short): the Geo
    /// waits for it.
    pub(super) fn gauntlet_utility(&self, _p: &Percepts) -> f32 {
        let Some(c) = self.senses.gauntlet else { return 0.0 };
        let ceremony = matches!(c.stage, GauntletStage::Announce | GauntletStage::Reward);
        let acting = matches!(&self.active, Active::Gauntlet(g) if g.is(c) && g.acting());
        let done = self.gauntlet_done == Some((c.serial, c.stage)) && !acting;
        if !ceremony || done || self.senses.fullscreen || self.stakes.pending_death.is_some() {
            return 0.0;
        }
        0.99
    }
}

/// The announcement (`challenge`, then standing ready) or the closing cheer (`victory`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ceremony {
    serial: u32,
    stage: GauntletStage,
    wave: (usize, usize),
    act: Option<ActWait>,
    /// The act was asked for (or there was none to ask for).
    acted: bool,
}

impl Ceremony {
    pub fn start(ctx: &mut Ctx, cue: GauntletCue) -> Self {
        ctx.nav.stop();
        ctx.mind.adjust(|n| &mut n.boredom, -0.3);
        Self { serial: cue.serial, stage: cue.stage, wave: (cue.wave, cue.waves), act: None, acted: false }
    }

    /// The ceremony for this cue (same gauntlet, same stage)?
    pub fn is(&self, cue: GauntletCue) -> bool {
        (cue.serial, cue.stage) == (self.serial, self.stage)
    }

    /// The bow / the cheer is still to come or under way.
    pub fn acting(&self) -> bool {
        !self.acted || self.act.is_some()
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        let Some(cue) = ctx.senses.gauntlet else { return Status::Done };
        if ctx.senses.fullscreen || (cue.serial, cue.stage) != (self.serial, self.stage) {
            return Status::Done;
        }
        self.wave = (cue.wave, cue.waves);
        ctx.out.motion = Motion::Hold;
        if !self.acted && ctx.grounded() {
            self.acted = true;
            let names: &[&str] = match self.stage {
                GauntletStage::Reward => &["victory", "collect_happy"],
                _ => &["challenge"],
            };
            self.act = names.iter().find_map(|n| ActWait::named(ctx, n));
        }
        if let Some(w) = self.act.as_mut()
            && w.tick(ctx)
        {
            self.act = None;
            if self.stage == GauntletStage::Reward {
                return Status::Done;
            }
        }
        if self.acted && self.act.is_none() && self.stage == GauntletStage::Reward {
            return Status::Done;
        }
        // Announcement: stand ready until the first wave comes.
        Status::Running
    }

    pub fn reason(&self) -> String {
        match self.stage {
            GauntletStage::Reward => format!("gauntlet: all {} waves cleared, victory", self.wave.1),
            _ => format!("gauntlet: readying for wave {}/{}", self.wave.0, self.wave.1),
        }
    }
}
