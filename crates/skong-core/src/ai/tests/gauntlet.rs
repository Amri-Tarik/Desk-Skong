//! Gauntlets with the brain: the knight readies himself at the announcement ("!" is the
//! app's emote on the switch, then `challenge`), fights each wave as it comes (the real
//! enemy sim, placed by `plan_arrival`), and cheers (`victory`) when the last one falls,
//! then gathers the reward. Typing hard or a fullscreen app ends it gracefully; it never
//! starts while a visitor or another fight is on.
use std::path::Path;

use super::*;
use crate::enemy::{
    AbortReason, Enemy, EnemyEnv, EnemyPhase, EnemyPlacement, EnemySet, EntryKind, GauntletConfig, GauntletDirector, GauntletEvent, GauntletStage, GauntletWorld, plan_arrival,
};
use crate::geom::RectF;
use crate::portal::Footing;

const PPU: f32 = 64.0;

fn roster() -> EnemySet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    EnemySet::load(&d.join("enemies.json"), &d.join("hk/animations"), &bank).expect("enemies")
}

/// The brain sim plus the app's side of enemies and gauntlets.
struct Gym {
    sim: Sim,
    set: EnemySet,
    g: GauntletDirector,
    rng: crate::ai::rng::Rng,
    enemies: Vec<Enemy>,
    next_id: u32,
    hits: u32,
    coins: Vec<Vec2>,
    /// The completion burst's coins.
    reward: Vec<Vec2>,
    events: Vec<GauntletEvent>,
    /// The app's view: a fullscreen app / a visitor.
    fullscreen: bool,
    visitor: bool,
    /// Behaviour each tick, and the gauntlet stage the brain saw.
    log: Vec<(BehaviourKind, Option<GauntletStage>)>,
}

impl Gym {
    fn new(sim: Sim) -> Self {
        let set = roster();
        let g = GauntletDirector::new(set.gauntlet.clone(), &set).expect("gauntlet");
        Self { sim, set, g, rng: crate::ai::rng::Rng::new(5), enemies: vec![], next_id: 1, hits: 0, coins: vec![], reward: vec![], events: vec![], fullscreen: false, visitor: false, log: vec![] }
    }

    fn with(sim: Sim, cfg: GauntletConfig) -> Self {
        let mut gym = Self::new(sim);
        gym.g = GauntletDirector::new(cfg, &gym.set).expect("gauntlet");
        gym
    }

    /// A single enemy (not part of a gauntlet) on the floor at `x`.
    fn add(&mut self, name: &str, x: f32) -> u32 {
        let def = self.set.id(name).expect(name);
        let id = self.next_id;
        self.next_id += 1;
        let at = Vec2::new(x, FLOOR);
        self.enemies.push(Enemy::new(id, def, &self.set, EnemyPlacement { at, owner: None, entry: EntryKind::Edge { from: at } }, id * 7, PPU));
        id
    }

    fn area() -> RectF {
        RectF::new(0.0, 0.0, 1920.0, FLOOR)
    }

    fn step(&mut self) -> Intent {
        let foes: Vec<Foe> = self.enemies.iter().filter_map(|e| e.foe(&self.set, PPU)).collect();
        self.sim.brain.see_foes(foes, std::mem::take(&mut self.hits));
        let cue = self.g.cue();
        self.sim.brain.see_gauntlet(cue);
        self.sim.brain.sense_stakes(StakesSense { coins: self.coins.clone(), ..Default::default() });
        let intent = self.sim.step();
        if let Motion::Direct(i) = intent.motion
            && self.sim.act.is_none()
        {
            self.sim.knight.pos.x = (self.sim.knight.pos.x + i.x * 300.0 * DT).clamp(0.0, 1920.0);
        }
        // His blows; kills burst Geo and count for the gauntlet (or towards a natural one).
        for s in &intent.strikes {
            if let Some(e) = self.enemies.iter_mut().find(|e| e.id == s.id && s.kind == FoeKind::Enemy)
                && e.strike(s.damage, s.from, &self.set, PPU)
            {
                // Its Geo lands on the floor around it.
                let at = Vec2::new(e.pos.x, FLOOR);
                let id = e.id;
                self.coins.extend([at + Vec2::new(-30.0, 0.0), at + Vec2::new(30.0, 0.0)]);
                self.g.on_kill(id, &mut self.rng);
            }
        }
        // The gauntlet (the app's `Enemies::tick_gauntlet`).
        let behaviour = self.sim.brain.behaviour();
        let alive: Vec<u32> = self.enemies.iter().filter(|e| e.alive() && !e.dismissed()).map(|e| e.id).collect();
        let w = GauntletWorld {
            keys_per_sec: self.sim.user.keys_per_sec,
            user_idle_secs: self.sim.user.idle_secs,
            away_secs: 90.0,
            fullscreen: self.fullscreen,
            visitor_present: self.visitor,
            others_alive: alive.iter().filter(|&&id| !self.g.is_member(id)).count(),
            knight_busy: false,
            knight_fighting: matches!(behaviour, BehaviourKind::Fight | BehaviourKind::Train),
            knight_down: behaviour == BehaviourKind::Dead,
            knight_hurt: self.sim.brain.mind().emotions.hurt > self.g.cfg.hurt || self.sim.brain.fight_backed_off(),
            shade: false,
            knight_at: self.sim.knight.pos,
            ready: true,
        };
        let step = self.g.tick(DT, &w, &|id| alive.contains(&id), &mut self.rng);
        for e in self.enemies.iter_mut().filter(|e| step.dismiss.contains(&e.id)) {
            e.dismiss();
        }
        if let Some(def) = step.spawn {
            let floor = Footing { x0: 0.0, x1: 1920.0, y: FLOOR, owner: None };
            let placed = plan_arrival(&self.set.defs[def], &[floor], self.sim.knight.pos, &self.sim.desk.monitors, PPU, &mut self.rng, &mut |_| true);
            let id = placed.map(|at| {
                let id = self.next_id;
                self.next_id += 1;
                let mut e = Enemy::new(id, def, &self.set, at, id * 7, PPU);
                e.hold(self.g.hold_secs());
                self.enemies.push(e);
                id
            });
            self.g.spawned(id);
        }
        if let Some(geo) = step.reward {
            // The reward burst lands around him.
            let k = self.sim.knight.pos;
            self.reward = (0..(geo / 10)).map(|i| k + Vec2::new(-90.0 + 40.0 * i as f32, 0.0)).collect();
            self.coins.extend(self.reward.iter().copied());
        }
        self.events.extend(step.events);
        let env = EnemyEnv { terrain: &self.sim.terrain, knight: Some(self.sim.knight.body()), ppu: PPU, area: Self::area() };
        for e in &mut self.enemies {
            self.hits += u32::from(e.tick(DT, &self.set, &env).hit_knight);
        }
        self.enemies.retain(|e| e.phase() != EnemyPhase::Gone);
        let k = self.sim.knight.pos;
        self.coins.retain(|c| c.distance(k) > 12.0);
        self.log.push((intent.behaviour.unwrap_or(BehaviourKind::Idle), cue.map(|c| c.stage)));
        intent
    }

    fn run_while(&mut self, secs: f32, mut cond: impl FnMut(&Gym) -> bool) -> bool {
        for _ in 0..(secs / DT).round() as usize {
            self.step();
            if !cond(self) {
                return true;
            }
        }
        false
    }

    fn run(&mut self, secs: f32) {
        self.run_while(secs, |_| true);
    }

    fn names(&self) -> Vec<&'static str> {
        self.events.iter().map(|e| e.name()).collect()
    }
}

#[test]
fn he_readies_himself_fights_the_waves_in_order_and_cheers_for_the_reward() {
    let mut gym = Gym::new(calm(BrainConfig::default()));
    // A line-up he wins (some end with him too sore: see the balance test below).
    gym.rng = crate::ai::rng::Rng::new(1);
    gym.run(1.0);
    gym.g.request();
    gym.run(0.2);
    assert_eq!(gym.names(), ["gauntlet_started"]);
    // The announcement: he stops and readies himself (challenge), nobody arrives yet.
    assert!(gym.run_while(1.0, |g| g.sim.brain.behaviour() != BehaviourKind::Gauntlet), "{}", gym.sim.last_reason());
    assert!(gym.sim.last_reason().contains("gauntlet: readying for wave 1/3"), "{}", gym.sim.last_reason());
    assert!(gym.run_while(3.0, |g| !g.sim.started.iter().any(|a| a == "challenge")), "the challenge bow: {:?}", gym.sim.started);
    assert!(gym.enemies.is_empty());
    // The waves: each fought as it comes, the next only after a clear.
    let mut wave_open_with_foes = false;
    let mut seen = gym.events.len();
    assert!(
        gym.run_while(400.0, |g| {
            let new = std::mem::replace(&mut seen, g.events.len()) < g.events.len();
            if new
                && let Some(&GauntletEvent::WaveStarted { wave, .. }) = g.events.last()
                && wave > 1
            {
                // A new wave never starts with the last one still about.
                wave_open_with_foes |= g.enemies.iter().any(|e| e.alive());
            }
            !g.names().contains(&"gauntlet_completed") && !g.names().contains(&"gauntlet_aborted")
        }),
        "{:?} / {}",
        gym.names(),
        gym.sim.last_reason()
    );
    assert!(!wave_open_with_foes);
    assert_eq!(
        gym.names(),
        [
            "gauntlet_started",
            "gauntlet_wave_started",
            "gauntlet_wave_cleared",
            "gauntlet_wave_started",
            "gauntlet_wave_cleared",
            "gauntlet_wave_started",
            "gauntlet_wave_cleared",
            "gauntlet_completed"
        ],
        "{} / {:?} / hurt {:.2}",
        gym.sim.last_reason(),
        gym.events.last(),
        gym.sim.brain.mind().emotions.hurt
    );
    assert!(gym.log.iter().any(|(b, s)| *b == BehaviourKind::Fight && *s == Some(GauntletStage::Wave)), "he fights the waves");
    assert!(!gym.log.iter().any(|(b, s)| *b == BehaviourKind::Gauntlet && *s == Some(GauntletStage::Wave)), "no ceremony mid-wave");
    // The reward: his cheer, then he gathers it.
    assert!(gym.run_while(4.0, |g| !g.sim.started.iter().any(|a| a == "victory")), "victory: {:?} / {}", gym.sim.started, gym.sim.last_reason());
    // The cheer plays out (the reward's Geo waits for it), then he goes for the Geo.
    let cheering = |g: &Gym| g.sim.act.as_ref().is_some_and(|a| g.sim.act_name(a.act.id) == "victory");
    assert!(cheering(&gym), "{:?}", gym.sim.started);
    let mut left = f32::INFINITY;
    while cheering(&gym) {
        left = gym.sim.act.as_ref().map_or(0.0, |a| a.left);
        assert_eq!(gym.sim.brain.behaviour(), BehaviourKind::Gauntlet, "left the cheer {left:.2} s early: {}", gym.sim.last_reason());
        gym.step();
    }
    assert!(left <= DT + 1e-3, "the cheer was cut {left:.2} s early: {}", gym.sim.last_reason());
    assert!(!gym.reward.is_empty(), "the reward burst");
    assert!(gym.run_while(30.0, |g| g.coins.iter().any(|c| g.reward.contains(c))), "gathers the reward: {:?} left, {}", gym.coins, gym.sim.last_reason());
    assert!(gym.g.cue().is_none());
    assert_eq!(gym.sim.started.iter().filter(|a| *a == "victory").count(), 1, "one cheer");
}

#[test]
fn the_announcement_bow_is_finished_before_any_geo() {
    let mut gym = Gym::new(calm(BrainConfig::default()));
    gym.run(1.0);
    gym.g.request();
    assert!(gym.run_while(3.0, |g| !g.sim.started.iter().any(|a| a == "challenge")), "the challenge bow: {:?}", gym.sim.started);
    // Geo lying about (an earlier fight's): it waits for the bow.
    let k = gym.sim.knight.pos;
    gym.coins.extend([k + Vec2::new(160.0, 0.0), k + Vec2::new(-220.0, 0.0)]);
    let bowing = |g: &Gym| g.sim.act.as_ref().is_some_and(|a| g.sim.act_name(a.act.id) == "challenge");
    assert!(bowing(&gym), "{:?}", gym.sim.started);
    let mut left = f32::INFINITY;
    while bowing(&gym) {
        left = gym.sim.act.as_ref().map_or(0.0, |a| a.left);
        assert_eq!(gym.sim.brain.behaviour(), BehaviourKind::Gauntlet, "left the bow {left:.2} s early: {}", gym.sim.last_reason());
        gym.step();
    }
    assert!(left <= DT + 1e-3, "the bow was cut {left:.2} s early: {}", gym.sim.last_reason());
}

#[test]
fn typing_hard_ends_it_and_the_enemies_wander_off() {
    let mut gym = Gym::new(calm(BrainConfig::default()));
    gym.g.request();
    assert!(gym.run_while(15.0, |g| !g.log.last().is_some_and(|(b, _)| *b == BehaviourKind::Fight)), "fights wave 1: {}", gym.sim.last_reason());
    gym.sim.user.keys_per_sec = 5.0;
    assert!(gym.run_while(4.0, |g| g.g.active()), "ends");
    assert!(matches!(gym.events.last(), Some(GauntletEvent::Aborted { reason: AbortReason::Typing, wave: 1, .. })), "{:?}", gym.events);
    // He lets them go: no foes left to fight; they wander off.
    assert!(gym.run_while(2.0, |g| g.sim.brain.behaviour() == BehaviourKind::Fight), "stops fighting: {}", gym.sim.last_reason());
    assert!(gym.run_while(40.0, |g| !g.enemies.is_empty()), "they wander off");
    assert!(!gym.names().contains(&"gauntlet_completed"));
    assert!(!gym.sim.started.iter().any(|a| a == "victory"));
}

#[test]
fn a_fullscreen_app_ends_it() {
    let mut gym = Gym::new(calm(BrainConfig::default()));
    gym.g.request();
    assert!(gym.run_while(15.0, |g| !g.enemies.iter().any(|e| e.alive())), "wave 1 is in");
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "player.exe");
    w.fullscreen = true;
    w.foreground = true;
    gym.sim.open(w);
    gym.fullscreen = true;
    assert!(gym.run_while(1.0, |g| g.g.active()));
    assert!(matches!(gym.events.last(), Some(GauntletEvent::Aborted { reason: AbortReason::Fullscreen, .. })), "{:?}", gym.events);
    assert!(gym.enemies.iter().all(|e| e.foe(&gym.set, PPU).is_none()), "no foes any more");
}

#[test]
fn never_while_another_fight_or_a_visit_is_on() {
    let mut gym = Gym::new(calm(BrainConfig::default()));
    gym.run(0.5);
    gym.add("crawlid", 1400.0);
    assert!(gym.run_while(2.0, |g| g.sim.brain.behaviour() != BehaviourKind::Fight));
    gym.g.request();
    let mut started_mid_fight = false;
    assert!(
        gym.run_while(40.0, |g| {
            started_mid_fight |= g.g.active() && g.enemies.iter().any(|e| e.alive() && !g.g.is_member(e.id));
            !g.g.active()
        }),
        "starts after the fight: {}",
        gym.sim.last_reason()
    );
    assert!(!started_mid_fight, "not while the other enemy lived");
    // A visitor about: none starts.
    let mut gym = Gym::new(calm(BrainConfig::default()));
    gym.visitor = true;
    gym.g.request();
    gym.run(5.0);
    assert!(gym.events.is_empty() && gym.enemies.is_empty());
    gym.visitor = false;
    assert!(gym.run_while(1.0, |g| !g.g.active()), "starts once the visitor is gone");
}

#[test]
fn badly_hurt_he_ends_it_without_losing_anything() {
    // A lot of bumps in one wave: he backs off sore; the gauntlet ends, nothing is lost.
    let mut gym = Gym::with(calm(BrainConfig::default()), GauntletConfig::default());
    gym.g.request();
    assert!(gym.run_while(15.0, |g| g.sim.brain.behaviour() != BehaviourKind::Fight), "{}", gym.sim.last_reason());
    let geo_before = gym.coins.len();
    gym.hits += 4;
    assert!(gym.run_while(3.0, |g| g.g.active()), "ends: {:?}", gym.events);
    assert!(matches!(gym.events.last(), Some(GauntletEvent::Aborted { reason: AbortReason::Hurt, .. })), "{:?}", gym.events);
    assert_ne!(gym.sim.brain.behaviour(), BehaviourKind::Dead);
    assert!(gym.coins.len() >= geo_before, "nothing lost");
}

#[test]
fn most_gauntlets_are_won_and_the_rest_end_with_him_too_hurt() {
    // Balance: the default waves are a fair challenge. He usually wins; now and then the
    // bumps add up and he is too sore to go on (it ends early, nothing lost).
    let mut won = 0;
    let mut out = vec![];
    for seed in 1..=12u32 {
        let mut gym = Gym::new(calm(BrainConfig::default()));
        gym.rng = crate::ai::rng::Rng::new(seed);
        gym.run(1.0);
        gym.g.request();
        gym.run_while(500.0, |g| !g.names().contains(&"gauntlet_completed") && !g.names().contains(&"gauntlet_aborted"));
        let last = gym.events.last().copied();
        match last {
            Some(GauntletEvent::Completed { .. }) => won += 1,
            Some(GauntletEvent::Aborted { reason: AbortReason::Hurt | AbortReason::KnightDown, .. }) => {}
            other => panic!("seed {seed}: {other:?}"),
        }
        out.push(last);
    }
    assert!(won >= 9, "won {won} of 12: {out:?}");
}
