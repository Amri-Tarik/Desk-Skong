//! Passed-over activities: what the knight could be doing instead of what he does, and why
//! he isn't (diagnostics only; nothing here changes a decision).
//!
//! Every think, after the switch decision, the brain lists the things on screen he might
//! take up — found places, shinies, visitors, the training dummy, lumaflies, loose Geo —
//! and the activity behaviours that scored but lost, each with a short verdict: the
//! scoring's own outcome (blocked, cooling, no route, outranked...) or, for the one its
//! behaviour would go for, why that behaviour didn't win (a gate that zeroed it, a higher
//! tier, the hysteresis margin, the commitment period). The list is rebuilt into a reused
//! buffer of plain `Copy` entries (no allocation once warm); text is only made when asked
//! ([`Brain::describe_passed_over`]), and a Debug log line only when the list changes in
//! kind (not on every countdown second).
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};

use crate::nav::TravelStyle;

use super::behaviours::{Active, BehaviourKind, Tier};
use super::percept::Percepts;
use super::places::{PlaceId, PlaceKind};
use super::{Brain, VisitorState};

/// What was passed over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Place { kind: PlaceKind, id: PlaceId },
    Shiny(u64),
    Visitor(u32),
    Dummy(u64),
    Lumafly(u64),
    /// Loose Geo on the ground (coins in total).
    Geo { coins: u16 },
    /// An activity behaviour that isn't about one thing on screen (watch, explore, the
    /// highlight, a scene...), listed when it scored but lost.
    Behaviour(BehaviourKind),
}

/// Why a behaviour that scored lost the contest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lose {
    /// A higher tier holds (the winner or the running behaviour).
    Tier { to: BehaviourKind, tier: Tier },
    /// Another behaviour scored higher in the same tier.
    Outscored { to: BehaviourKind, u: f32 },
    /// It beats the running behaviour, but not by the hysteresis margin.
    Hysteresis { to: BehaviourKind, u: f32 },
    /// The running behaviour is still within its commitment period (s left).
    Commit { to: BehaviourKind, left: f32 },
    /// The running behaviour ends itself (only urgent ones may interrupt it).
    Exclusive { to: BehaviourKind },
}

/// What stops a behaviour (its utility is 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gate {
    Fullscreen,
    UserBusy,
    UserAwaySleepy,
    Asleep,
    Napping,
    Airborne,
    /// It just failed: not picked again until then (brain time).
    Failed { until: f64 },
    /// Calming down after dithering (pastimes are quiet).
    Calming,
    /// Not up to it (tired / sore).
    NotUpTo,
    /// Loose Geo: resting after a fruitless gathering until then.
    LootResting { until: f64 },
    /// None of the reasons known here (scored 0).
    Zero,
}

/// The verdict on one candidate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    /// Its behaviour scored `u` at `tier` and lost.
    Lost { u: f32, tier: Tier, to: Lose },
    /// Its behaviour scored 0.
    Gated(Gate),
    /// Failed to get there recently (the shared failure map).
    Blocked,
    /// Used / done a little while ago: not wanted again until then (brain time).
    Cooling { until: f64 },
    /// Wanted too little right now (a bench when not tired, a deposit when spent...).
    NotWanted,
    /// Another one of its kind is better (only the best two are route-planned).
    Outranked,
    /// No route with this style.
    NoRoute(TravelStyle),
    /// The route ends somewhere else (not on / beside it).
    Misses,
    /// He set out for it and gave up: left alone until then (brain time).
    GaveUp { until: f64 },
    /// A visitor he couldn't reach: ignored until then (brain time).
    Snubbed { until: f64 },
    /// Loose Geo out of his range.
    OutOfRange,
    /// A free-form verdict.
    Note(&'static str),
    /// Scored and route-planned (the scoring's choice). Resolved into `Lost` / `Gated`
    /// when the list is built.
    Planned,
}

/// One passed-over activity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub what: Subject,
    pub why: Why,
}

impl Candidate {
    /// Something actually on screen (not a behaviour-level entry).
    pub fn is_thing(&self) -> bool {
        !matches!(self.what, Subject::Behaviour(_))
    }

    /// Counts as an activity he might be expected to take up: a thing on screen that is not
    /// merely resting on a timer, and not a bench (a seat for when he's tired).
    pub fn available(&self) -> bool {
        self.is_thing() && !matches!(self.what, Subject::Place { kind: PlaceKind::Bench, .. }) && !matches!(self.why, Why::Cooling { .. } | Why::Gated(Gate::LootResting { .. }))
    }

    /// What kind of verdict this is (ignoring scores and countdowns), for change detection.
    fn key<H: Hasher>(&self, h: &mut H) {
        self.what.hash(h);
        std::mem::discriminant(&self.why).hash(h);
        match self.why {
            Why::Lost { tier, to, .. } => {
                tier.hash(h);
                std::mem::discriminant(&to).hash(h);
                match to {
                    Lose::Tier { to, .. } | Lose::Outscored { to, .. } | Lose::Hysteresis { to, .. } | Lose::Commit { to, .. } | Lose::Exclusive { to } => to.hash(h),
                }
            }
            Why::Gated(g) => std::mem::discriminant(&g).hash(h),
            Why::NoRoute(style) => (style as u8).hash(h),
            Why::Note(n) => n.hash(h),
            _ => {}
        }
    }
}

/// The running behaviour is a fallback: idling, keeping company, or an ambient pastime
/// (sitting, watching, exploring) — the moments "why isn't he doing X" is asked about.
pub(crate) fn is_fallback(kind: BehaviourKind) -> bool {
    matches!(kind, BehaviourKind::Idle | BehaviourKind::Companion | BehaviourKind::Rest | BehaviourKind::Watch | BehaviourKind::Explore)
}

/// The last think's decision, for explaining losses.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Decision {
    pub best: BehaviourKind,
    pub best_u: f32,
    pub best_tier: Tier,
    /// The running behaviour before the decision, its utility and tier.
    pub cur: BehaviourKind,
    pub cur_u: f32,
    pub cur_tier: Tier,
    pub since_switch: f32,
}

/// The passed-over list and its bookkeeping (lives in the brain).
#[derive(Clone, Debug, Default)]
pub(crate) struct PassedOver {
    pub list: Vec<Candidate>,
    /// The running behaviour is a fallback (see [`is_fallback`]) in the Ambient tier.
    pub fallback: bool,
    /// Signature of the list (kinds of verdicts) and the one last logged.
    pub signature: u64,
    logged: u64,
}

fn secs(until: f64, now: f64) -> f32 {
    ((until - now) as f32).max(0.0)
}

impl Brain {
    /// What he passed over at the last think (rebuilt every think).
    pub fn passed_over(&self) -> &[Candidate] {
        &self.passed.list
    }

    /// Idling, keeping company or on an ambient pastime while activities are on screen:
    /// how many (see [`Candidate::available`]) and the list's signature (changes when the
    /// kind of a verdict changes, not with scores or countdowns). `None` otherwise.
    pub fn idling_past(&self) -> Option<(usize, u64)> {
        if !self.passed.fallback {
            return None;
        }
        let n = self.passed.list.iter().filter(|c| c.available()).count();
        (n > 0).then_some((n, self.passed.signature))
    }

    /// The passed-over list as one line, e.g. `deposit #12: cooling 7 s; dummy #3: 0.78
    /// ambient, lost to companion 0.71 (hysteresis)`. Empty when there is nothing.
    pub fn describe_passed_over(&self, out: &mut String) {
        for (i, c) in self.passed.list.iter().enumerate() {
            if i > 0 {
                out.push_str("; ");
            }
            self.describe(c, out);
        }
    }

    fn describe(&self, c: &Candidate, s: &mut String) {
        let now = self.now;
        match c.what {
            Subject::Place { kind, id } => {
                let label = match kind {
                    PlaceKind::Bench => "bench",
                    PlaceKind::Deposit => "deposit",
                    PlaceKind::Grub => "grub",
                };
                let _ = write!(s, "{label} #{id}");
            }
            Subject::Shiny(id) => {
                let _ = write!(s, "shiny #{id}");
            }
            Subject::Visitor(id) => match self.senses.visitors.iter().find(|v| v.id == id) {
                Some(v) => {
                    let _ = write!(s, "visitor {} #{id}", v.name);
                }
                None => {
                    let _ = write!(s, "visitor #{id}");
                }
            },
            Subject::Dummy(id) => {
                let _ = write!(s, "dummy #{id}");
            }
            Subject::Lumafly(id) => {
                let _ = write!(s, "lumafly #{id}");
            }
            Subject::Geo { coins } => {
                let _ = write!(s, "loose geo ({coins})");
            }
            Subject::Behaviour(k) => s.push_str(k.label()),
        }
        s.push_str(": ");
        let _ = match c.why {
            Why::Lost { u, tier, to } => {
                let _ = write!(s, "{u:.2} {}, ", tier.label());
                match to {
                    Lose::Tier { to, tier } => write!(s, "{} holds a higher tier ({})", to.label(), tier.label()),
                    Lose::Outscored { to, u } => write!(s, "outscored by {} {u:.2}", to.label()),
                    Lose::Hysteresis { to, u } => write!(s, "not enough over {} {u:.2} (hysteresis {:.2})", to.label(), self.cfg.hysteresis),
                    Lose::Commit { to, left } => write!(s, "committed to {} for {left:.1} s more", to.label()),
                    Lose::Exclusive { to } => write!(s, "{} runs to its end", to.label()),
                }
            }
            Why::Gated(g) => match g {
                Gate::Fullscreen => write!(s, "a fullscreen app is up"),
                Gate::UserBusy => write!(s, "the user is typing"),
                Gate::UserAwaySleepy => write!(s, "user away and he's sleepy"),
                Gate::Asleep => write!(s, "asleep"),
                Gate::Napping => write!(s, "napping"),
                Gate::Airborne => write!(s, "in the air"),
                Gate::Failed { until } => write!(s, "its behaviour just failed (again in {:.0} s)", secs(until, now)),
                Gate::Calming => write!(s, "calming down after dithering"),
                Gate::NotUpTo => write!(s, "not up to it (tired / sore)"),
                Gate::LootResting { until } => write!(s, "resting after a fruitless gathering ({:.0} s)", secs(until, now)),
                Gate::Zero => write!(s, "scored 0"),
            },
            Why::Blocked => write!(s, "blocked (failed to get there recently)"),
            Why::Cooling { until } => write!(s, "cooling {:.0} s", secs(until, now)),
            Why::NotWanted => write!(s, "not wanted now"),
            Why::Outranked => write!(s, "a better one first"),
            Why::NoRoute(style) => write!(s, "no {style:?} route"),
            Why::Misses => write!(s, "the route ends off it"),
            Why::GaveUp { until } => write!(s, "gave up on it ({:.0} s)", secs(until, now)),
            Why::Snubbed { until } => write!(s, "snubbed {:.0} s (no route)", secs(until, now)),
            Why::OutOfRange => write!(s, "out of range"),
            Why::Note(n) => write!(s, "{n}"),
            Why::Planned => write!(s, "planned"),
        };
    }

    /// Rebuild the passed-over list (end of every think).
    pub(crate) fn note_passed_over(&mut self, p: &Percepts, d: Option<Decision>) {
        let mut list = std::mem::take(&mut self.passed.list);
        list.clear();
        let active = self.active.kind();
        let now = self.now;

        // Found places: the scoring's verdicts; the chosen one is its behaviour's.
        let visiting = self.visiting().map(|v| v.id);
        let best_place = self.interest.places.best.map(|c| c.place.id);
        for &(kind, id, why) in self.interest.places.scan() {
            if Some(id) == visiting || self.interest.places.get(id).is_none() {
                continue;
            }
            let why = if why == Why::Planned && Some(id) != best_place { Why::Outranked } else { why };
            list.push(Candidate { what: Subject::Place { kind, id }, why: self.resolve(BehaviourKind::Visit, why, p, d) });
        }

        // Shinies.
        let current_shiny = self.shiny_target();
        let best_shiny = self.interest.shinies.best.map(|c| c.shiny.id);
        for &(id, why) in self.interest.shinies.scan() {
            if Some(id) == current_shiny || self.interest.shinies.get(id).is_none() {
                continue;
            }
            let why = if why == Why::Planned && Some(id) != best_shiny { Why::Outranked } else { why };
            list.push(Candidate { what: Subject::Shiny(id), why: self.resolve(BehaviourKind::Shiny, why, p, d) });
        }

        // Visitors not yet greeted.
        let greeting = match &self.active {
            Active::Greet(g) => Some(g.id),
            _ => None,
        };
        let target = self.visitors.target(&self.senses.visitors, p.knight.pos, now).map(|v| v.id);
        for v in &self.senses.visitors {
            if v.greeted || matches!(v.state, VisitorState::Leaving | VisitorState::Held) || Some(v.id) == greeting {
                continue;
            }
            let why = match self.visitors.snubbed_until(v.id) {
                Some(until) if until > now => Why::Snubbed { until },
                _ if Some(v.id) == target => self.resolve(BehaviourKind::Greet, Why::Planned, p, d),
                _ => Why::Outranked,
            };
            list.push(Candidate { what: Subject::Visitor(v.id), why });
        }

        // The dummy and lumaflies (unless a pastime is under way).
        if active != BehaviourKind::Pastime {
            let pm = &self.interest.pastimes;
            let best = pm.best.as_ref().map(|c| (c.kind, c.target));
            if let Some(dm) = pm.dummy() {
                let why = if let Some(until) = pm.cooling(super::pastime::PastimeKind::Dummy, now) {
                    Why::Cooling { until }
                } else if best == Some((super::pastime::PastimeKind::Dummy, dm.id)) {
                    Why::Planned
                } else {
                    pm.dummy_verdict(dm.id)
                };
                list.push(Candidate { what: Subject::Dummy(dm.id), why: self.resolve(BehaviourKind::Pastime, why, p, d) });
            }
            for (id, why) in pm.fly_verdicts(now) {
                let why = if why == Why::Planned && best != Some((super::pastime::PastimeKind::Lumafly, id)) { Why::Outranked } else { why };
                list.push(Candidate { what: Subject::Lumafly(id), why: self.resolve(BehaviourKind::Pastime, why, p, d) });
            }
        }

        // Loose Geo.
        let coins = &self.stakes.sense.coins;
        if !coins.is_empty() && active != BehaviourKind::Loot {
            let range = self.cfg.stakes.loot_range;
            let k = p.knight.pos;
            let why = if now < self.stakes.loot_ready_at {
                Why::Gated(Gate::LootResting { until: self.stakes.loot_ready_at })
            } else if !coins.iter().any(|c| c.distance(k) < range) {
                Why::OutOfRange
            } else if coins.iter().filter(|c| c.distance(k) < range).all(|c| self.interest.coin_skipped(*c)) {
                Why::Note("gave up on those in range for now (no route)")
            } else {
                self.resolve(BehaviourKind::Loot, Why::Planned, p, d)
            };
            list.push(Candidate { what: Subject::Geo { coins: coins.len().min(u16::MAX as usize) as u16 }, why });
        }

        // Activity behaviours not about one thing, that scored but lost.
        for &(k, u) in &self.utilities {
            let listed = matches!(k, BehaviourKind::Watch | BehaviourKind::Explore | BehaviourKind::Highlight | BehaviourKind::Scene | BehaviourKind::DreamGate | BehaviourKind::Fight);
            if listed && k != active && u > self.cfg.tiers.eligible {
                list.push(Candidate { what: Subject::Behaviour(k), why: self.resolve(k, Why::Planned, p, d) });
            }
        }

        let mut h = std::collections::hash_map::DefaultHasher::new();
        active.hash(&mut h);
        for c in &list {
            c.key(&mut h);
        }
        self.passed.signature = h.finish();
        self.passed.fallback = is_fallback(active) && self.tier_of(active, p) == Tier::Ambient;
        self.passed.list = list;
        if self.passed.fallback && self.passed.list.iter().any(|c| c.is_thing()) && self.passed.signature != self.passed.logged && log::log_enabled!(log::Level::Debug) {
            self.passed.logged = self.passed.signature;
            let mut s = String::new();
            self.describe_passed_over(&mut s);
            log::debug!("brain: {} while passing over: {s}", active.label());
        }
    }

    /// The verdict for a candidate whose scan said `why`: a planned one is judged by its
    /// behaviour's utility and the last decision.
    fn resolve(&self, kind: BehaviourKind, why: Why, p: &Percepts, d: Option<Decision>) -> Why {
        if why != Why::Planned {
            return why;
        }
        let u = self.utilities.iter().find(|t| t.0 == kind).map_or(0.0, |t| t.1);
        if u <= self.cfg.tiers.eligible {
            return Why::Gated(self.gate(kind, p));
        }
        let tier = self.tier_at(kind, u, p);
        let Some(d) = d else { return Why::Note("not yet judged") };
        if kind == self.active.kind() {
            return Why::Note("under way");
        }
        let to = if kind != d.best {
            if tier < d.best_tier { Lose::Tier { to: d.best, tier: d.best_tier } } else { Lose::Outscored { to: d.best, u: d.best_u } }
        } else if tier < d.cur_tier {
            Lose::Tier { to: d.cur, tier: d.cur_tier }
        } else if d.cur.exclusive() {
            Lose::Exclusive { to: d.cur }
        } else if d.since_switch < self.cfg.min_commit {
            Lose::Commit { to: d.cur, left: self.cfg.min_commit - d.since_switch }
        } else {
            Lose::Hysteresis { to: d.cur, u: d.cur_u }
        };
        Why::Lost { u, tier, to }
    }

    /// Why `kind` scored 0 (the first reason that applies; mirrors the utilities' gates).
    fn gate(&self, kind: BehaviourKind, p: &Percepts) -> Gate {
        let s = &self.senses;
        if let Some(&until) = self.cooldown.get(&kind)
            && self.now < until
        {
            return Gate::Failed { until };
        }
        let pastime = !matches!(kind.tier(), Tier::Engagement | Tier::Urgent);
        if self.now < self.calm_until && pastime {
            return Gate::Calming;
        }
        if s.fullscreen {
            return Gate::Fullscreen;
        }
        let napping = matches!(self.active, Active::Nap(_));
        let grounded = p.knight.is_grounded();
        match kind {
            BehaviourKind::Visit if s.user_away && self.mind.tiredness() > 0.7 => Gate::UserAwaySleepy,
            BehaviourKind::Shiny | BehaviourKind::Pastime | BehaviourKind::Highlight if self.asleep(p) => Gate::Asleep,
            BehaviourKind::Shiny | BehaviourKind::Pastime | BehaviourKind::Highlight if napping => Gate::Napping,
            BehaviourKind::Shiny if s.user_busy => Gate::UserBusy,
            BehaviourKind::Pastime if s.user_busy && self.interest.pastimes.best.as_ref().is_some_and(|c| c.kind.lively()) => Gate::UserBusy,
            BehaviourKind::Shiny | BehaviourKind::Pastime | BehaviourKind::Highlight if !grounded => Gate::Airborne,
            BehaviourKind::Loot if self.now < self.stakes.loot_ready_at => Gate::LootResting { until: self.stakes.loot_ready_at },
            BehaviourKind::Visit if self.interest.places.best.is_some_and(|c| super::places::want(&c.place, &self.mind) <= 0.0) => Gate::NotUpTo,
            _ => Gate::Zero,
        }
    }
}
