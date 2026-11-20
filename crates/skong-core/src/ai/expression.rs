//! Expressions: the silent knight's small emote bubbles ("…", an anger mark, "!", "?",
//! "z", a heart, a sweat drop), decided here from brain state and handed to the app as
//! [`super::Intent::emote`]. The app only draws them (`skong/src/emotes.rs`).
//!
//! Every bubble has a reason and is rare on purpose:
//! * **Transitions, not states.** Switches into a behaviour (to a fight: anger; to the
//!   Shade: "!"), a behaviour that ended (a nice chat: heart; gave up on a route: "?"), or a
//!   need crossing a threshold (boredom, energy, hurt), with hysteresis: a need must fall
//!   back past a lower mark before it can show again.
//! * **Per-expression cooldowns** ([`Expression::cooldown`]), plus a global gap between
//!   soft (non-reaction) bubbles.
//! * **At most one per tick**, the most important first ([`Expression`] order).
use super::behaviours::{Active, Status};
use super::{BehaviourKind, Brain, FoeKind, Intent, StakeEvent, Switch};

/// One emote bubble, most important first (when several are due on one tick, the first
/// wins and the rest are dropped, not queued).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Expression {
    /// "!": a startle, or spotting the Shade, the scarlet flame, a visitor, a gauntlet.
    Alert,
    /// An anger mark: enemies, or a cursor worth fighting (as he goes for them); clicked
    /// once too often, woken by a click, thrown around (`ai::handling`).
    Angry,
    /// A sweat drop: badly hurt.
    Sweat,
    /// A bare "?": he couldn't get to what he wanted (no route, gave up), or reeling after
    /// a shake.
    Lost,
    /// A small heart: a pleasant chat with a visitor just ended, or (rarely) a gentle carry.
    Fond,
    /// A Geo coin in a thought cloud: going for loot.
    Greedy,
    /// "?" in a thought cloud: something caught his curiosity.
    Curious,
    /// A single "z": drowsy, or settling down for a nap.
    Drowsy,
    /// "…" in a thought cloud: bored, with nothing to do.
    Bored,
}

impl Expression {
    pub const ALL: [Expression; 9] = [
        Expression::Alert,
        Expression::Angry,
        Expression::Sweat,
        Expression::Lost,
        Expression::Fond,
        Expression::Greedy,
        Expression::Curious,
        Expression::Drowsy,
        Expression::Bored,
    ];

    /// Minimum seconds between two of the same.
    pub fn cooldown(self) -> f32 {
        match self {
            Expression::Alert => 5.0,
            Expression::Angry => 30.0,
            Expression::Sweat => 90.0,
            Expression::Lost => 45.0,
            Expression::Fond => 60.0,
            Expression::Greedy => 40.0,
            Expression::Curious => 40.0,
            Expression::Drowsy => 180.0,
            Expression::Bored => 180.0,
        }
    }

    /// Reactions (a jolt) skip the global gap between soft bubbles.
    pub fn reaction(self) -> bool {
        matches!(self, Expression::Alert | Expression::Angry)
    }
}

/// Minimum seconds between any two soft (non-reaction) bubbles.
pub const SOFT_GAP: f32 = 6.0;

/// Boredom at or above this, held for [`BORED_HOLD`] s with nothing to do: "…".
pub const BORED_ON: f32 = 0.8;
/// ... and it must fall below this before "…" can show again.
pub const BORED_OFF: f32 = 0.6;
pub const BORED_HOLD: f32 = 20.0;
/// Energy at or below this: "z" (awake only); re-armed above [`DROWSY_OFF`].
pub const DROWSY_ON: f32 = 0.28;
pub const DROWSY_OFF: f32 = 0.5;
/// Hurt at or above this: a sweat drop; re-armed below [`SWEAT_OFF`].
pub const SWEAT_ON: f32 = 0.65;
pub const SWEAT_OFF: f32 = 0.3;

/// A threshold with hysteresis: fires once on reaching `on`, re-arms below `off`.
#[derive(Clone, Copy, Debug)]
struct Latch {
    armed: bool,
}

impl Default for Latch {
    fn default() -> Self {
        Self { armed: true }
    }
}

impl Latch {
    /// Re-arm when the value has fallen back; `true` while it is due (armed and high).
    fn due(&mut self, high: bool, low: bool) -> bool {
        if low {
            self.armed = true;
        }
        self.armed && high
    }
}

/// How a behaviour ended (recorded as it ends, read on the same tick).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ending {
    pub kind: BehaviourKind,
    /// Gave up: no route, unreachable, nobody answered.
    pub failed: bool,
    /// A visitor conversation that ran its course.
    pub chatted: bool,
}

/// What the knight is up to this tick, as far as expressions care.
#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub now: f64,
    pub dt: f32,
    pub kind: BehaviourKind,
    /// The last behaviour switch (new ones are told apart by their time).
    pub switch: Option<Switch>,
    pub boredom: f32,
    pub energy: f32,
    pub hurt: f32,
    pub asleep: bool,
    /// Nothing worth showing a bubble for right now (fullscreen app, held, dead, hiding).
    pub muted: bool,
    /// Foes he would fight for real (enemies, the boss) are about.
    pub enemies: bool,
    /// The scarlet flame (Grimm's lantern) is on screen.
    pub lantern: bool,
    /// A gauntlet is being announced.
    pub gauntlet_announce: bool,
    /// He let a coin go (couldn't reach it).
    pub abandoned_coin: bool,
    /// A reaction to how the user handles him (`ai::handling`).
    pub handled: Option<Expression>,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            now: 0.0,
            dt: 0.0,
            kind: BehaviourKind::Idle,
            switch: None,
            boredom: 0.0,
            energy: 1.0,
            hurt: 0.0,
            asleep: false,
            muted: false,
            enemies: false,
            lantern: false,
            gauntlet_announce: false,
            abandoned_coin: false,
            handled: None,
        }
    }
}

/// The expression tracker (cooldowns, latches, what was already seen).
#[derive(Clone, Debug)]
pub(crate) struct Expressions {
    /// When each expression last showed.
    last: [f64; Expression::ALL.len()],
    /// When the last soft bubble showed.
    last_soft: f64,
    seen_switch: Option<f64>,
    endings: Vec<Ending>,
    bored_for: f32,
    bored: Latch,
    drowsy: Latch,
    sweat: Latch,
    lantern_seen: bool,
}

impl Default for Expressions {
    fn default() -> Self {
        Self {
            last: [f64::NEG_INFINITY; Expression::ALL.len()],
            last_soft: f64::NEG_INFINITY,
            seen_switch: None,
            endings: Vec::new(),
            bored_for: 0.0,
            bored: Latch::default(),
            drowsy: Latch::default(),
            sweat: Latch::default(),
            lantern_seen: false,
        }
    }
}

impl Expressions {
    /// A behaviour ended (call as it ends; read at the next [`Self::observe`]).
    pub fn ended(&mut self, e: Ending) {
        if self.endings.len() < 4 {
            self.endings.push(e);
        }
    }

    /// At most one expression for this tick.
    pub fn observe(&mut self, o: &Observation) -> Option<Expression> {
        let mut due: Vec<Expression> = Vec::new();
        let endings = std::mem::take(&mut self.endings);

        // A new behaviour switch.
        if let Some(sw) = o.switch
            && self.seen_switch != Some(sw.at)
        {
            self.seen_switch = Some(sw.at);
            if let Some(e) = on_switch(sw, o) {
                due.push(e);
            }
        }
        // Behaviours that just ended.
        for e in &endings {
            if e.failed
                && !matches!(e.kind, BehaviourKind::Dead | BehaviourKind::KnockedOut | BehaviourKind::Flee | BehaviourKind::Hide | BehaviourKind::Ride | BehaviourKind::Idle | BehaviourKind::Dizzy)
            {
                due.push(Expression::Lost);
            } else if e.chatted && e.kind == BehaviourKind::Greet {
                due.push(Expression::Fond);
            }
        }
        if o.abandoned_coin {
            due.push(Expression::Lost);
        }
        if let Some(e) = o.handled {
            due.push(e);
        }
        // The scarlet flame appears.
        if o.lantern && !self.lantern_seen {
            due.push(Expression::Alert);
        }
        self.lantern_seen = o.lantern;

        // Needs crossing thresholds (hysteresis).
        let idle = matches!(o.kind, BehaviourKind::Idle | BehaviourKind::Companion | BehaviourKind::Rest) && !o.asleep;
        self.bored_for = if idle && o.boredom >= BORED_ON { self.bored_for + o.dt } else { 0.0 };
        let bored = self.bored.due(self.bored_for >= BORED_HOLD, o.boredom < BORED_OFF);
        if bored {
            due.push(Expression::Bored);
        }
        let drowsy = self.drowsy.due(o.energy <= DROWSY_ON && !o.asleep, o.energy >= DROWSY_OFF);
        if drowsy {
            due.push(Expression::Drowsy);
        }
        let sweat = self.sweat.due(o.hurt >= SWEAT_ON && o.kind != BehaviourKind::Dead, o.hurt < SWEAT_OFF);
        if sweat {
            due.push(Expression::Sweat);
        }

        if o.muted {
            return None;
        }
        due.sort();
        let pick = due.into_iter().find(|&e| (o.now - self.last[e as usize]) as f32 >= e.cooldown() && (e.reaction() || (o.now - self.last_soft) as f32 >= SOFT_GAP))?;
        self.last[pick as usize] = o.now;
        if !pick.reaction() {
            self.last_soft = o.now;
        }
        // A latched need is used up once it shows (until it falls back past its low mark).
        match pick {
            Expression::Bored => self.bored.armed = false,
            Expression::Drowsy => self.drowsy.armed = false,
            Expression::Sweat => self.sweat.armed = false,
            _ => {}
        }
        Some(pick)
    }
}

/// The bubble a behaviour switch calls for, if any.
fn on_switch(sw: Switch, o: &Observation) -> Option<Expression> {
    use BehaviourKind as K;
    let fighting = |k: K| matches!(k, K::Battle | K::Pester | K::Fight);
    match sw.to {
        K::Flee | K::Greet | K::ShadeHunt => Some(Expression::Alert),
        // A gauntlet's announcement: a brief "!" before he readies himself.
        K::Gauntlet if o.gauntlet_announce => Some(Expression::Alert),
        // A friendly spar or the lantern alone is no reason for anger.
        K::Fight if !o.enemies => o.lantern.then_some(Expression::Alert),
        K::Battle | K::Pester | K::Fight if !fighting(sw.from) => Some(Expression::Angry),
        K::Loot => Some(Expression::Greedy),
        // Reeling after a shake: which way is up?
        K::Dizzy => Some(Expression::Lost),
        K::Explore | K::Watch | K::Shiny | K::Highlight => Some(Expression::Curious),
        K::Nap if !o.asleep => Some(Expression::Drowsy),
        _ => None,
    }
}

impl Brain {
    /// Record how the active behaviour ended (from the tick loop, before re-thinking).
    pub(super) fn note_ending(&mut self, status: Status) {
        let (failed, chatted) = match &self.active {
            Active::Greet(g) => (g.failed || status == Status::Failed, g.chatted()),
            _ => (status == Status::Failed, false),
        };
        self.expr.ended(Ending { kind: self.active.kind(), failed, chatted });
    }

    /// This tick's expression (see the module docs).
    pub(super) fn express(&mut self, dt: f32, asleep: bool, out: &Intent) -> Option<Expression> {
        let s = &self.senses;
        let kind = self.active.kind();
        let o = Observation {
            now: self.now,
            dt,
            kind,
            switch: self.last_switch,
            boredom: self.mind.needs.boredom,
            energy: self.mind.needs.energy,
            hurt: self.mind.emotions.hurt,
            asleep,
            muted: s.fullscreen || s.held || matches!(kind, BehaviourKind::Dead | BehaviourKind::Hide),
            enemies: s.foes.iter().any(|f| matches!(f.kind, FoeKind::Enemy | FoeKind::Boss)),
            lantern: s.foes.iter().any(|f| f.kind == FoeKind::Lantern),
            gauntlet_announce: s.gauntlet.is_some_and(|c| c.stage == crate::enemy::GauntletStage::Announce),
            abandoned_coin: out.stakes.iter().any(|e| matches!(e, StakeEvent::AbandonCoin { .. })),
            handled: self.handling.take_emote(),
        };
        self.expr.observe(&o)
    }
}

/// A big Geo haul: this much picked up within [`HAUL_WINDOW`] s earns a sparkle (at most
/// once per [`HAUL_COOLDOWN`] s). The app counts pickups; the bubble is the same quiet
/// sparkle as an achievement's.
pub const HAUL_GEO: u32 = 40;
pub const HAUL_WINDOW: f32 = 12.0;
pub const HAUL_COOLDOWN: f32 = 120.0;

#[derive(Clone, Debug)]
pub struct GeoHaul {
    recent: Vec<(f64, u32)>,
    last: f64,
}

impl Default for GeoHaul {
    fn default() -> Self {
        Self { recent: Vec::new(), last: f64::NEG_INFINITY }
    }
}

impl GeoHaul {
    /// `geo` picked up at `now`: `true` when it completes a big haul.
    pub fn collect(&mut self, now: f64, geo: u32) -> bool {
        self.recent.retain(|(t, _)| ((now - t) as f32) < HAUL_WINDOW);
        self.recent.push((now, geo));
        let sum: u32 = self.recent.iter().map(|(_, g)| g).sum();
        if sum >= HAUL_GEO && (now - self.last) as f32 >= HAUL_COOLDOWN {
            self.last = now;
            self.recent.clear();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::SwitchReason;
    use super::*;

    const DT: f32 = 1.0 / 30.0;

    fn calm(now: f64) -> Observation {
        Observation { now, dt: DT, kind: BehaviourKind::Companion, boredom: 0.3, energy: 0.8, ..Default::default() }
    }

    fn switch(at: f64, from: BehaviourKind, to: BehaviourKind) -> Option<Switch> {
        Some(Switch { at, from, to, reason: SwitchReason::Outscored })
    }

    #[test]
    fn switches_map_to_their_bubbles() {
        use BehaviourKind as K;
        let cases = [
            (K::Idle, K::ShadeHunt, false, Some(Expression::Alert)),
            (K::Idle, K::Flee, false, Some(Expression::Alert)),
            (K::Companion, K::Battle, false, Some(Expression::Angry)),
            (K::Companion, K::Pester, false, Some(Expression::Angry)),
            (K::Idle, K::Fight, true, Some(Expression::Angry)),
            (K::Idle, K::Fight, false, None), // a friendly spar
            (K::Battle, K::Battle, false, None),
            (K::Fight, K::Battle, false, None), // already fighting
            (K::Idle, K::Loot, false, Some(Expression::Greedy)),
            (K::Idle, K::Explore, false, Some(Expression::Curious)),
            (K::Idle, K::Nap, false, Some(Expression::Drowsy)),
            (K::Idle, K::Rest, false, None),
        ];
        for (from, to, enemies, want) in cases {
            let mut x = Expressions::default();
            let got = x.observe(&Observation { switch: switch(10.0, from, to), enemies, ..calm(10.0) });
            assert_eq!(got, want, "{from:?} -> {to:?}");
        }
    }

    #[test]
    fn the_scarlet_flame_is_spotted_once() {
        let mut x = Expressions::default();
        assert_eq!(x.observe(&Observation { lantern: true, ..calm(1.0) }), Some(Expression::Alert));
        for i in 0..300 {
            assert_eq!(x.observe(&Observation { lantern: true, ..calm(1.0 + (i as f32 * DT) as f64) }), None);
        }
        // Going to strike it (a fight with only the lantern about): "!", never anger.
        let got = x.observe(&Observation { lantern: true, switch: switch(20.0, BehaviourKind::Idle, BehaviourKind::Fight), ..calm(20.0) });
        assert_eq!(got, Some(Expression::Alert));
    }

    #[test]
    fn a_nice_chat_earns_a_heart_and_a_failed_trip_a_question() {
        let mut x = Expressions::default();
        x.ended(Ending { kind: BehaviourKind::Greet, failed: false, chatted: true });
        assert_eq!(x.observe(&calm(1.0)), Some(Expression::Fond));
        x.ended(Ending { kind: BehaviourKind::Explore, failed: true, chatted: false });
        assert_eq!(x.observe(&calm(10.0)), Some(Expression::Lost));
        // A visitor who left before a word: nothing.
        x.ended(Ending { kind: BehaviourKind::Greet, failed: false, chatted: false });
        assert_eq!(x.observe(&calm(100.0)), None);
        // Failing to flee or idle isn't "can't reach".
        x.ended(Ending { kind: BehaviourKind::Flee, failed: true, chatted: false });
        assert_eq!(x.observe(&calm(200.0)), None);
        assert_eq!(x.observe(&Observation { abandoned_coin: true, ..calm(300.0) }), Some(Expression::Lost));
    }

    #[test]
    fn cooldowns_hold_back_repeats() {
        let mut x = Expressions::default();
        let mut shown = 0;
        // An enemy every 5 s for a minute: anger once per its cooldown.
        for i in 0..12 {
            let at = 1.0 + i as f64 * 5.0;
            let k = if i % 2 == 0 { BehaviourKind::Fight } else { BehaviourKind::Idle };
            let from = if i % 2 == 0 { BehaviourKind::Idle } else { BehaviourKind::Fight };
            if x.observe(&Observation { switch: switch(at, from, k), enemies: true, ..calm(at) }) == Some(Expression::Angry) {
                shown += 1;
            }
        }
        assert_eq!(shown, 2, "60 s / 30 s cooldown");
    }

    #[test]
    fn soft_bubbles_keep_a_gap_reactions_do_not() {
        let mut x = Expressions::default();
        assert_eq!(x.observe(&Observation { switch: switch(1.0, BehaviourKind::Idle, BehaviourKind::Explore), ..calm(1.0) }), Some(Expression::Curious));
        x.ended(Ending { kind: BehaviourKind::Explore, failed: true, chatted: false });
        assert_eq!(x.observe(&calm(2.0)), None, "too soon after the last soft bubble");
        assert_eq!(x.observe(&Observation { switch: switch(2.5, BehaviourKind::Idle, BehaviourKind::ShadeHunt), ..calm(2.5) }), Some(Expression::Alert));
    }

    #[test]
    fn the_most_important_bubble_wins_a_tick() {
        let mut x = Expressions::default();
        x.ended(Ending { kind: BehaviourKind::Greet, failed: false, chatted: true });
        let got = x.observe(&Observation { switch: switch(1.0, BehaviourKind::Greet, BehaviourKind::Fight), enemies: true, ..calm(1.0) });
        assert_eq!(got, Some(Expression::Angry));
    }

    /// Run `secs` of the same observation (needs fixed), counting each expression.
    fn hold(x: &mut Expressions, from: f64, secs: f32, o: Observation) -> Vec<Expression> {
        let n = (secs / DT) as usize;
        (0..n).filter_map(|i| x.observe(&Observation { now: from + (i as f32 * DT) as f64, ..o.clone() })).collect()
    }

    #[test]
    fn boredom_needs_time_and_hysteresis() {
        let mut x = Expressions::default();
        let bored = Observation { boredom: 0.85, kind: BehaviourKind::Idle, ..calm(0.0) };
        // Bored, but busy: nothing.
        assert!(hold(&mut x, 0.0, 60.0, Observation { kind: BehaviourKind::Explore, ..bored.clone() }).is_empty());
        // Idle and bored: once, after BORED_HOLD s.
        let got = hold(&mut x, 100.0, BORED_HOLD - 1.0, bored.clone());
        assert!(got.is_empty(), "not bored long enough yet");
        let got = hold(&mut x, 100.0 + (BORED_HOLD - 1.0) as f64, 10.0, bored.clone());
        assert_eq!(got, [Expression::Bored]);
        assert!(hold(&mut x, 130.0, 600.0, bored.clone()).is_empty(), "stays quiet until boredom falls back");
        // Dips a little (not past BORED_OFF): still latched.
        hold(&mut x, 800.0, 5.0, Observation { boredom: 0.65, ..bored.clone() });
        assert!(hold(&mut x, 805.0, 60.0, bored.clone()).is_empty());
        // Falls back properly, then bored again: shows again.
        hold(&mut x, 900.0, 5.0, Observation { boredom: 0.4, ..bored.clone() });
        assert_eq!(hold(&mut x, 905.0, 60.0, bored), [Expression::Bored]);
    }

    #[test]
    fn drowsy_and_sweat_latch_until_recovered() {
        let mut x = Expressions::default();
        let tired = Observation { energy: 0.2, ..calm(0.0) };
        assert_eq!(hold(&mut x, 0.0, 400.0, tired.clone()), [Expression::Drowsy]);
        assert!(hold(&mut x, 400.0, 10.0, Observation { asleep: true, energy: 0.6, ..tired.clone() }).is_empty());
        assert_eq!(hold(&mut x, 600.0, 10.0, tired), [Expression::Drowsy], "re-armed after a rest");

        let hurt = Observation { hurt: 0.8, ..calm(0.0) };
        assert_eq!(hold(&mut x, 1000.0, 300.0, hurt.clone()), [Expression::Sweat]);
        hold(&mut x, 1300.0, 5.0, Observation { hurt: 0.4, ..hurt.clone() });
        assert!(hold(&mut x, 1305.0, 30.0, hurt.clone()).is_empty(), "0.4 isn't recovered");
        hold(&mut x, 1400.0, 5.0, Observation { hurt: 0.1, ..hurt.clone() });
        assert_eq!(hold(&mut x, 1405.0, 30.0, hurt), [Expression::Sweat]);
    }

    #[test]
    fn muted_shows_nothing() {
        let mut x = Expressions::default();
        let o = Observation { muted: true, switch: switch(1.0, BehaviourKind::Idle, BehaviourKind::ShadeHunt), energy: 0.1, hurt: 0.9, ..calm(1.0) };
        assert!(hold(&mut x, 1.0, 30.0, o).is_empty());
    }

    #[test]
    fn a_big_haul_sparkles_rarely() {
        let mut h = GeoHaul::default();
        assert!(!h.collect(0.0, 10));
        assert!(!h.collect(1.0, 10));
        assert!(!h.collect(20.0, 25), "spread out: not a haul");
        assert!(h.collect(21.0, 20));
        assert!(!h.collect(30.0, 100), "cooldown");
        assert!(h.collect(30.0 + HAUL_COOLDOWN as f64, 45));
    }
}
