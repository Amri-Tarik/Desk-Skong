//! Placeholder brain until the mood AI lands: stroll / run around the current platform,
//! hop now and then, rest, and perform random acts.
use std::collections::HashMap;

use skong_core::acts::{ActId, ActLibrary, Requirement};
use skong_core::controller::{Character, Gait, MoveInput};

enum Plan {
    Rest { left: f32 },
    Walk { target_x: f32, gait: Gait, left: f32, hop_in: Option<f32>, hop_hold: f32 },
}

pub struct Decision {
    pub input: MoveInput,
    pub act: Option<ActId>,
}

pub struct Wander {
    plan: Plan,
    rng: u32,
    cooldowns: HashMap<ActId, f32>,
}

impl Wander {
    pub fn new(seed: u32) -> Self {
        Self { plan: Plan::Rest { left: 2.0 }, rng: seed | 1, cooldowns: HashMap::new() }
    }

    /// Called every tick. `act_running` freezes planning while an act plays.
    pub fn decide(&mut self, ch: &Character, acts: &ActLibrary, act_running: bool, dt: f32) -> Decision {
        for cd in self.cooldowns.values_mut() {
            *cd -= dt;
        }
        let mut d = Decision { input: MoveInput::default(), act: None };
        if act_running {
            return d;
        }
        match &mut self.plan {
            Plan::Rest { left } => {
                *left -= dt;
                if *left <= 0.0 && ch.is_grounded() {
                    let roll = self.rand();
                    if roll < 0.45 {
                        self.plan = self.walk_plan(ch);
                    } else if roll < 0.75 {
                        d.act = self.pick_act(ch, acts);
                        self.plan = Plan::Rest { left: 1.0 + 2.0 * self.rand() };
                    } else {
                        self.plan = Plan::Rest { left: 1.0 + 3.0 * self.rand() };
                    }
                }
            }
            Plan::Walk { target_x, gait, left, hop_in, hop_hold } => {
                *left -= dt;
                let dx = *target_x - ch.pos.x;
                d.input.gait = *gait;
                d.input.x = dx.signum();
                if let Some(h) = hop_in {
                    *h -= dt;
                    if *h <= 0.0 {
                        *hop_in = None;
                        *hop_hold = 0.15 + 0.3 * xorshift(&mut self.rng);
                        d.input.jump_pressed = true;
                    }
                }
                if *hop_hold > 0.0 {
                    *hop_hold -= dt;
                    d.input.jump_held = true;
                }
                if dx.abs() < 6.0 || *left <= 0.0 {
                    d.input.x = 0.0;
                    self.plan = Plan::Rest { left: 1.5 + 4.0 * self.rand() };
                }
            }
        }
        d
    }

    fn walk_plan(&mut self, ch: &Character) -> Plan {
        let (lo, hi) = ch.support.map_or((ch.pos.x - 400.0, ch.pos.x + 400.0), |s| (s.x0 + 20.0, s.x1 - 20.0));
        let target_x = lo + (hi - lo).max(1.0) * self.rand();
        let g = self.rand();
        let gait = if g < 0.55 {
            Gait::Walk
        } else if g < 0.9 {
            Gait::Run
        } else {
            Gait::Sprint
        };
        let speed = ch.params.gait_speed_px(gait).max(1.0);
        let eta = (target_x - ch.pos.x).abs() / speed;
        let hop_in = (self.rand() < 0.2).then(|| eta * self.rand());
        Plan::Walk { target_x, gait, left: eta * 1.5 + 1.0, hop_in, hop_hold: 0.0 }
    }

    fn pick_act(&mut self, ch: &Character, acts: &ActLibrary) -> Option<ActId> {
        let candidates: Vec<(ActId, f32)> = acts
            .iter()
            .filter(|(id, a)| {
                a.weight > 0.0
                    && !a.has_tag("travel")
                    && !a.has_tag("intro")
                    && (a.requires == Requirement::Any || ch.is_grounded())
                    && self.cooldowns.get(id).is_none_or(|c| *c <= 0.0)
            })
            .map(|(id, a)| (id, a.weight))
            .collect();
        let total: f32 = candidates.iter().map(|c| c.1).sum();
        let mut r = self.rand() * total;
        for (id, w) in candidates {
            r -= w;
            if r <= 0.0 {
                self.cooldowns.insert(id, acts.get(id).cooldown);
                return Some(id);
            }
        }
        None
    }

    fn rand(&mut self) -> f32 {
        xorshift(&mut self.rng)
    }
}

fn xorshift(s: &mut u32) -> f32 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    (x >> 8) as f32 / (1u32 << 24) as f32
}
