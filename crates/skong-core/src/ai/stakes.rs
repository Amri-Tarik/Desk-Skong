//! The brain's side of the battle stakes (see [`crate::stakes`]): deciding that a lost
//! fight was lethal, the death → respawn sequence ([`super::death::Dead`]), hunting the
//! Shade and picking up Geo ([`super::shade_hunt`]), and spells in fights.
//!
//! The app feeds [`StakesSense`] each tick ([`Brain::sense_stakes`]) and reacts to the
//! [`StakeEvent`]s in [`Intent::stakes`](super::Intent::stakes).
use serde::Deserialize;

use crate::controller::Side;
use crate::geom::Vec2;
use crate::stakes::{DeathKind, Spell, SpellBook, SpellConfig, SpellSituation};

use super::behaviours::{ActWait, BehaviourKind, Ctx};
use super::battle::BattleScore;
use super::{Active, Brain, Percepts};

/// What the brain tells the app about the stakes this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StakeEvent {
    /// The knight dies at `at` (feet) and plays the death act. Fight deaths leave a Shade
    /// with his Geo; `cracked` = beaten badly (the cracked mask piece).
    Died { at: Vec2, kind: DeathKind, cracked: bool },
    /// Put the knight back at `at` (feet: a bench's sitting spot or a safe ledge).
    Respawn { at: Vec2 },
    /// A spell / nail art was chosen (its act was requested).
    Cast { spell: Spell, side: Side },
    /// A blow that connects with the Shade.
    StrikeShade { damage: u32, from: Vec2 },
    /// A cursor battle was won: loot bursts out of the beaten cursor at `at`.
    Victory { at: Vec2 },
}

/// The Shade as the app sees it (only while it exists and is alive).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadeSense {
    /// Body centre (screen px).
    pub pos: Vec2,
    /// Can be struck right now.
    pub vulnerable: bool,
    /// Mid-fight with the knight.
    pub fighting: bool,
    /// Geo it carries.
    pub geo: u32,
}

/// Stakes percepts, refreshed by the app every tick.
#[derive(Clone, Debug, PartialEq)]
pub struct StakesSense {
    pub shade: Option<ShadeSense>,
    /// Resting Geo coins (bottom centre, screen px).
    pub coins: Vec<Vec2>,
    /// Shade slashes that hit the knight since the last tick.
    pub shade_hits: u32,
    /// Render scale (px per game unit / 64): spell reach scales with it.
    pub scale: f32,
}

impl Default for StakesSense {
    fn default() -> Self {
        Self { shade: None, coins: Vec::new(), shade_hits: 0, scale: 1.0 }
    }
}

/// When a lost cursor battle kills the knight.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Lethality {
    /// Losing only knocks him flat (`bow_down`).
    Never,
    /// Losing badly (the cursor won by `death_margin` or more) kills him.
    #[default]
    Badly,
    /// Every lost battle kills him.
    Always,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct StakesConfig {
    pub lethal: Lethality,
    /// Cursor score − knight score at which a loss counts as "badly".
    pub death_margin: u32,
    /// Seconds between the death act and the respawn.
    pub dead_secs: f32,
    /// Seconds after a respawn before he dares go after the Shade.
    pub shade_delay: f32,
    /// Seconds before trying again after giving up on the Shade.
    pub shade_retry: f32,
    /// Hits the knight can take from the Shade before dying again.
    pub masks: u32,
    /// Give up a Shade fight after this long (s).
    pub shade_fight_secs: f32,
    /// Walk over to resting Geo within this distance (px).
    pub loot_range: f32,
    /// Geo dropped by a beaten cursor: [min, max].
    pub victory_geo: [u32; 2],
    pub spells: SpellConfig,
}

impl Default for StakesConfig {
    fn default() -> Self {
        Self {
            lethal: Lethality::Badly,
            death_margin: 2,
            dead_secs: 1.5,
            shade_delay: 90.0,
            shade_retry: 180.0,
            masks: 5,
            shade_fight_secs: 60.0,
            loot_range: 700.0,
            victory_geo: [8, 30],
            spells: SpellConfig::default(),
        }
    }
}

/// Brain-side stakes state.
#[derive(Clone, Debug)]
pub(crate) struct StakesMind {
    pub sense: StakesSense,
    pub spells: SpellBook,
    /// A death to play out (set by a lethal loss, the Shade or the app).
    pub pending_death: Option<(DeathKind, bool)>,
    pub shade_ready_at: f32,
    pub loot_ready_at: f32,
    /// Debug: go after the Shade right away.
    pub force_hunt: bool,
}

impl StakesMind {
    pub fn new(cfg: &StakesConfig) -> Self {
        Self { sense: StakesSense::default(), spells: SpellBook::new(&cfg.spells), pending_death: None, shade_ready_at: 0.0, loot_ready_at: 0.0, force_hunt: false }
    }
}

impl Brain {
    /// Stakes percepts for the next tick (call before [`Brain::tick`]).
    pub fn sense_stakes(&mut self, sense: StakesSense) {
        // Unclaimed hits only carry over while a Shade fight is on.
        let carried = if matches!(self.active, Active::ShadeHunt(_)) { self.stakes.sense.shade_hits } else { 0 };
        let hits = carried + sense.shade_hits;
        self.stakes.sense = StakesSense { shade_hits: hits, ..sense };
    }

    /// Kill the knight (debug command, hazards). He plays the death act once grounded.
    pub fn kill(&mut self, kind: DeathKind) {
        self.stakes.pending_death = Some((kind, false));
    }

    /// Debug: go after the Shade now (if there is one).
    pub fn hunt_shade(&mut self) {
        self.stakes.force_hunt = true;
    }

    /// Soul meter (0..1).
    pub fn soul(&self) -> f32 {
        self.stakes.spells.soul
    }

    /// Utilities of the stakes behaviours.
    pub(super) fn stakes_utilities(&self, p: &Percepts) -> [(BehaviourKind, f32); 3] {
        let st = &self.stakes;
        let m = &self.mind;
        let e = m.emotions;
        let s = &self.senses;
        let cfg = &self.cfg.stakes;
        let dead = if st.pending_death.is_some() || matches!(self.active, Active::Dead(_)) { 2.0 } else { 0.0 };

        let hunting = matches!(self.active, Active::ShadeHunt(_));
        let hunt = match st.sense.shade {
            Some(_) if st.force_hunt || hunting => 1.1,
            Some(sh) if self.now >= st.shade_ready_at && !s.fullscreen && e.hurt < 0.25 && e.startle < 0.3 && m.needs.energy > 0.35 => {
                let pers = m.personality;
                (0.3 + 0.35 * pers.boldness + 0.25 * e.excitement + 0.15 * (sh.geo as f32 / 200.0).min(1.0)).min(0.85)
            }
            _ => 0.0,
        };

        let k = p.knight.pos;
        let near_coin = st.sense.coins.iter().any(|c| c.distance(k) < cfg.loot_range);
        let loot = if near_coin && self.now >= st.loot_ready_at && !s.fullscreen && p.knight.is_grounded() { (0.6 + 0.2 * e.excitement).min(0.8) } else { 0.0 };
        [(BehaviourKind::Dead, dead), (BehaviourKind::ShadeHunt, hunt), (BehaviourKind::Loot, loot)]
    }
}

/// Did this loss kill the knight? Queues the death if so.
pub(crate) fn lethal_loss(ctx: &mut Ctx, score: BattleScore) -> bool {
    let cfg = ctx.cfg.stakes;
    let margin = score.cursor.saturating_sub(score.knight);
    let lethal = match cfg.lethal {
        Lethality::Never => false,
        Lethality::Badly => margin >= cfg.death_margin,
        Lethality::Always => true,
    };
    if lethal {
        ctx.stakes.pending_death = Some((DeathKind::Fight, score.knight == 0));
    }
    lethal
}

/// A won cursor battle: loot drops where the cursor was beaten.
pub(crate) fn victory(ctx: &mut Ctx) {
    if let Some(c) = ctx.senses.cursor {
        ctx.out.stakes.push(StakeEvent::Victory { at: c.pos });
    }
}

/// Maybe use a special against a target at `off` (target − body centre). On success the
/// act to play is returned (wrap it in a phase) and a `Cast` event is emitted.
pub(crate) fn try_spell(ctx: &mut Ctx, off: Option<Vec2>, just_hit: bool, fight_over: bool) -> Option<(Spell, ActWait)> {
    let situation = SpellSituation {
        target: off,
        grounded: ctx.grounded(),
        just_hit,
        fight_over,
        hurt: ctx.mind.emotions.hurt,
        rolls: [ctx.rng.f32(), ctx.rng.f32()],
        scale: ctx.stakes.sense.scale.max(0.1),
    };
    let cfg = ctx.cfg.stakes.spells;
    let wait = |ctx: &Ctx, spell: Spell| ActWait::named(ctx, spell.act_name());
    // Only commit (pay soul, start cooldowns) when the act exists.
    let mut probe = ctx.stakes.spells.clone();
    let spell = probe.choose(&situation, ctx.now, &cfg)?;
    let w = wait(ctx, spell)?;
    ctx.stakes.spells = probe;
    let side = off.and_then(|o| Side::of(o.x)).unwrap_or(ctx.p.knight.facing);
    ctx.out.face = Some(side);
    ctx.out.stakes.push(StakeEvent::Cast { spell, side });
    log::debug!("brain: casting {}", spell.label());
    Some((spell, w))
}

/// Knight body centre (spell reach and the Shade are measured from it).
pub(crate) fn body_centre(ctx: &Ctx) -> Vec2 {
    ctx.p.knight.body().center()
}
