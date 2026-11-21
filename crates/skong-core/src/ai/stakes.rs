//! The brain's side of the battle stakes (see [`crate::stakes`]): deciding that a lost
//! fight was lethal, the death → respawn sequence ([`super::death::Dead`]), hunting the
//! Shade and picking up Geo ([`super::shade_hunt`]), and spells in fights.
//!
//! The app feeds [`StakesSense`] each tick ([`Brain::sense_stakes`]) and reacts to the
//! [`StakeEvent`]s in [`Intent::stakes`](super::Intent::stakes).
use serde::Deserialize;

use crate::controller::Side;
use crate::geom::Vec2;
use crate::stakes::{DeathKind, Spell, SpellBook, SpellConfig, SpellCues, SpellSituation};

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
    /// He couldn't reach the coin at `at`: let it fade.
    AbandonCoin { at: Vec2 },
    /// A combat move chosen for a reason (see [`super::combat`]); spells and nail arts are
    /// `Cast`, and the pogo bounce itself is the controller's `MoveEvent::Pogo`.
    Technique(Technique),
}

/// Combat moves the brain picks by situation (besides spells and nail arts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Technique {
    /// Desolate Dive (`dark`: Descending Dark) onto a foe below him.
    Dive { dark: bool },
    /// A shadow dash through a lunging foe.
    Dodge,
    /// A crystal dash at a far foe on his ledge to open the fight.
    CrystalOpener,
    /// Kicked off a wall to get at a flyer above.
    WallLeap,
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
    /// Seconds after a respawn before he goes after the Shade (he gets his bearings).
    pub shade_delay: f32,
    /// Seconds before trying again when there's no way to the Shade (no route, no Dream
    /// Gate spot): never minutes — it comes first.
    pub shade_retry: f32,
    /// Seconds after the Shade killed him before he goes for it again (counted from the
    /// death; no throwing himself at it in a tight loop).
    pub shade_breather: f32,
    /// Hits the knight can take from the Shade before dying again.
    pub masks: u32,
    /// Re-plan (go under it again) when the Shade stays out of slash reach this long (s);
    /// cursor battles give up after it.
    pub unreachable_secs: f32,
    /// Walk over to resting Geo within this distance (px).
    pub loot_range: f32,
    /// Geo dropped by a beaten cursor: [min, max].
    pub victory_geo: [u32; 2],
    pub spells: SpellConfig,
    /// Organic Geo: shattered windows, showers.
    pub sources: crate::stakes::GeoSourceConfig,
    /// Loose coins vanish after this long (s); unreachable ones fade away.
    pub coin_lifetime: f32,
}

impl Default for StakesConfig {
    fn default() -> Self {
        Self {
            lethal: Lethality::Badly,
            death_margin: 2,
            dead_secs: 1.5,
            shade_delay: 4.0,
            shade_retry: 5.0,
            shade_breather: 25.0,
            masks: 5,
            unreachable_secs: 4.0,
            loot_range: 8000.0,
            victory_geo: [8, 30],
            spells: SpellConfig::default(),
            sources: crate::stakes::GeoSourceConfig::default(),
            coin_lifetime: crate::stakes::geo::COIN_LIFETIME,
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
    pub shade_ready_at: f64,
    /// There's a way to the Shade (a route, or a Dream Gate spot by it) as far as he knows.
    /// Cleared when a hunt finds none; [`Brain::probe_shade`] looks again every few seconds.
    pub shade_way: bool,
    /// Next look for a way to the Shade (brain time).
    pub shade_probe_at: f64,
    pub loot_ready_at: f64,
    /// Debug: go after the Shade right away.
    pub force_hunt: bool,
    /// A death is playing out.
    pub dying: bool,
    /// Dream Gate state: cooldown and the unreachable prize worth a warp.
    pub gate: super::dream_gate::GateMind,
    /// Fights with enemies / sparring partners (see [`super::fight`]).
    pub fight: super::fight::FightMind,
}

impl StakesMind {
    pub fn new(cfg: &StakesConfig) -> Self {
        Self {
            sense: StakesSense::default(),
            spells: SpellBook::new(&cfg.spells),
            pending_death: None,
            shade_ready_at: 0.0,
            shade_way: true,
            shade_probe_at: 0.0,
            loot_ready_at: 0.0,
            force_hunt: false,
            dying: false,
            gate: Default::default(),
            fight: Default::default(),
        }
    }
}

impl Brain {
    /// Stakes percepts for the next tick (call before [`Brain::tick`]).
    pub fn sense_stakes(&mut self, sense: StakesSense) {
        // Unclaimed hits only carry over while a Shade fight is on.
        let carried = if matches!(self.active, Active::ShadeHunt(_)) { self.stakes.sense.shade_hits } else { 0 };
        let hits = carried + sense.shade_hits;
        // A fresh pile of Geo is exciting.
        if sense.coins.len() >= self.stakes.sense.coins.len() + 5 {
            self.mind.excite(0.35);
        }
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
        // Nothing interrupts dying (`dying` is cleared when the respawn is over).
        let dead = if st.pending_death.is_some() || (st.dying && matches!(self.active, Active::Dead(_))) { 2.0 } else { 0.0 };

        // The Shade is priority number one, whatever he's doing (lore: his Geo, his soul).
        // Only a fullscreen app holds it back, a few seconds after a respawn, a breather
        // after it beat him (so he doesn't throw himself at it in a loop), and no way there
        // at all (no route, no Dream Gate spot: looked for again every few seconds).
        let hunt = match st.sense.shade {
            Some(_) if st.force_hunt => 1.6,
            Some(_) if self.now >= st.shade_ready_at && st.shade_way && !s.fullscreen => 1.5,
            _ => 0.0,
        };

        let pile = if st.sense.coins.len() >= 8 { 0.1 } else { 0.0 };
        let cheering = matches!(&self.active, Active::Loot(l) if l.cheering());
        // Strict: loose Geo is always wanted (mid-jump included — a jump used to zero it and
        // let something else take over mid-gathering), just below fights and visitors. It is
        // only *taken up* on the ground, though: started in mid-air (live: during a dive),
        // it found no route from there, wrapped up with a breather, and preempted whatever
        // came next again 2 s later, over and over. Landing takes a moment; then it starts.
        let looting = matches!(&self.active, Active::Loot(_));
        let wanted = self.loose_geo_wanted(p) && (looting || p.knight.is_grounded());
        let loot = if wanted { (0.6 + 0.2 * e.excitement + pile).min(0.85) } else if cheering { 0.7 } else { 0.0 };
        [(BehaviourKind::Dead, dead), (BehaviourKind::ShadeHunt, hunt), (BehaviourKind::Loot, loot)]
    }

    /// No known way to the Shade: look again (a route, or a Dream Gate spot by it) every
    /// [`SHADE_PROBE_SECS`] (call with the other scorings, before the utilities).
    pub(super) fn probe_shade(&mut self, p: &Percepts, nav: &mut dyn super::Navigator) {
        let st = &mut self.stakes;
        if st.sense.shade.is_none() {
            st.shade_way = true;
            return;
        }
        if st.shade_way || self.now < st.shade_probe_at || !p.knight.is_grounded() || matches!(self.active, Active::ShadeHunt(_)) {
            return;
        }
        st.shade_probe_at = self.now + SHADE_PROBE_SECS as f64;
        let mut scratch = super::Intent::default();
        let mut ctx = self.ctx(p, nav, &mut scratch);
        let way = super::shade_hunt::ShadeHunt::way(&mut ctx).is_some();
        if way {
            log::debug!("brain: found a way to the shade again");
        }
        self.stakes.shade_way = way;
    }

    /// Loose Geo he'd go for now: a resting coin in range that he hasn't given up on for now
    /// (unreachable, kept for a Dream Gate — or Loot would start, find nothing and finish
    /// every tick), not right after a fruitless gathering, nothing fullscreen.
    pub(super) fn loose_geo_wanted(&self, p: &Percepts) -> bool {
        let st = &self.stakes;
        let k = p.knight.pos;
        let range = self.cfg.stakes.loot_range;
        self.now >= st.loot_ready_at && !self.senses.fullscreen && st.sense.coins.iter().any(|c| c.distance(k) < range && !self.interest.coin_skipped(*c))
    }
}

/// How often a way to an out-of-the-way Shade is looked for (s).
const SHADE_PROBE_SECS: f32 = 2.0;

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
    try_spell_cued(ctx, off, just_hit, fight_over, SpellCues::default())
}

/// [`try_spell`] with what the foe is up to: a moment that calls for one special (wraiths
/// at a flyer above, a great slash into a wind-up, a cyclone between two foes) gets it.
pub(crate) fn try_spell_cued(ctx: &mut Ctx, off: Option<Vec2>, just_hit: bool, fight_over: bool, cues: SpellCues) -> Option<(Spell, ActWait)> {
    let situation = SpellSituation {
        cues,
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
