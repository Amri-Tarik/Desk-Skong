//! Fights: an enemy wanders in → the knight goes for it, slashes it to death (the real
//! enemy sim), its Geo bursts and he gathers it. Fights come before Geo and cursor play;
//! never while asleep, typing hard or with something fullscreen. Bumps hurt (he backs off
//! sore, or dies when fights are lethal); a sparring partner is fought without killing.
use std::path::Path;

use super::*;
use crate::enemy::{Enemy, EnemyEnv, EnemyPlacement, EnemySet, EntryKind};
use crate::geom::RectF;

const PPU: f32 = 64.0;

fn roster() -> EnemySet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    EnemySet::load(&d.join("enemies.json"), &d.join("hk/animations"), &bank).expect("enemies")
}

/// The brain sim plus live enemies (the app's side: foes, strikes, bumps, Geo bursts).
struct Arena {
    sim: Sim,
    set: EnemySet,
    enemies: Vec<Enemy>,
    next_id: u32,
    /// Bumps not yet reported to the brain.
    hits: u32,
    /// Enemies killed (id) and where their Geo burst.
    killed: Vec<(u32, Vec2)>,
    /// Loose Geo on the floor (from bursts).
    coins: Vec<Vec2>,
    strikes: Vec<FoeStrike>,
    /// Extra foes reported as they are (a sparring partner).
    fixed: Vec<Foe>,
}

impl Arena {
    fn new(sim: Sim) -> Self {
        Self { sim, set: roster(), enemies: vec![], next_id: 1, hits: 0, killed: vec![], coins: vec![], strikes: vec![], fixed: vec![] }
    }

    /// An enemy standing (fading in) on the floor at `x`.
    fn add(&mut self, name: &str, x: f32) -> u32 {
        let def = self.set.id(name).expect(name);
        let id = self.next_id;
        self.next_id += 1;
        let at = Vec2::new(x, FLOOR);
        let mut e = Enemy::new(id, def, &self.set, EnemyPlacement { at, owner: None, entry: EntryKind::Edge { from: at } }, id * 7, PPU);
        e.attach(&EnemyEnv { terrain: &self.sim.terrain, knight: None, ppu: PPU, area: RectF::new(0.0, 0.0, 1920.0, FLOOR) });
        self.enemies.push(e);
        id
    }

    fn step(&mut self) -> Intent {
        let mut foes: Vec<Foe> = self.enemies.iter().filter_map(|e| e.foe(&self.set, PPU)).collect();
        foes.extend(self.fixed.iter().copied());
        self.sim.brain.see_foes(foes, std::mem::take(&mut self.hits));
        self.sim.brain.sense_stakes(StakesSense { coins: self.coins.clone(), ..Default::default() });
        let intent = self.sim.step();
        // Direct input walks him (the brain sim only moves him along routes).
        if let Motion::Direct(i) = intent.motion
            && self.sim.act.is_none()
        {
            self.sim.knight.pos.x = (self.sim.knight.pos.x + i.x * 300.0 * DT).clamp(0.0, 1920.0);
        }
        for s in &intent.strikes {
            self.strikes.push(*s);
            if let Some(e) = self.enemies.iter_mut().find(|e| e.id == s.id && s.kind == FoeKind::Enemy)
                && e.strike(s.damage, s.from, &self.set, PPU)
            {
                // The app bursts its Geo: a few coins land around it.
                let at = e.pos;
                self.killed.push((e.id, at));
                self.coins.extend([at + Vec2::new(-40.0, 0.0), at + Vec2::new(30.0, 0.0), at + Vec2::new(70.0, 0.0)]);
            }
        }
        let body = self.sim.knight.body();
        let env = EnemyEnv { terrain: &self.sim.terrain, knight: Some(body), ppu: PPU, area: RectF::new(0.0, 0.0, 1920.0, FLOOR) };
        for e in &mut self.enemies {
            self.hits += u32::from(e.tick(DT, &self.set, &env).hit_knight);
        }
        self.enemies.retain(|e| e.phase() != crate::enemy::EnemyPhase::Gone);
        // Coins under him get picked up.
        let k = self.sim.knight.pos;
        self.coins.retain(|c| c.distance(k) > 12.0);
        intent
    }

    fn run_while(&mut self, secs: f32, mut cond: impl FnMut(&Arena) -> bool) -> bool {
        for _ in 0..(secs / DT).round() as usize {
            self.step();
            if !cond(self) {
                return true;
            }
        }
        false
    }

    fn run_until(&mut self, secs: f32, kind: BehaviourKind) -> bool {
        self.run_while(secs, |a| a.sim.brain.behaviour() != kind)
    }

    fn run(&mut self, secs: f32) -> Vec<BehaviourKind> {
        (0..(secs / DT).round() as usize).map(|_| self.step().behaviour.unwrap_or(BehaviourKind::Idle)).collect()
    }
}

#[test]
fn an_enemy_wanders_in_he_fights_it_it_dies_and_he_gathers_its_geo() {
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(1.0);
    let id = a.add("crawlid", 1400.0);
    assert!(a.run_until(1.0, BehaviourKind::Fight), "goes for it at once: {}", a.sim.last_reason());
    assert!(a.sim.last_reason().contains("fight"), "{}", a.sim.last_reason());
    assert!(a.run_while(25.0, |a| a.killed.is_empty()), "kills it: {} / strikes {:?}", a.sim.last_reason(), a.strikes);
    assert_eq!(a.killed[0].0, id);
    assert!(a.strikes.iter().all(|s| s.id == id && s.kind == FoeKind::Enemy && s.damage >= 1));
    let blows = a.sim.intents.iter().any(|i| i.attack.is_some_and(|at| at.hit) || i.stakes.iter().any(|e| matches!(e, StakeEvent::Cast { .. })));
    assert!(blows, "nail blows or spells");
    // The Geo burst: he gathers it.
    assert!(a.run_until(3.0, BehaviourKind::Loot), "then the Geo: {}", a.sim.last_reason());
    assert!(a.run_while(20.0, |a| !a.coins.is_empty()), "picks it all up: {}", a.sim.last_reason());
    assert!(a.sim.brain.since_fight() < 30.0, "it counts as a fight (Sly's visits)");
}

#[test]
fn he_fights_several_one_after_the_other() {
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(1.0);
    a.add("crawlid", 1300.0);
    a.add("husk", 500.0);
    assert!(a.run_until(1.0, BehaviourKind::Fight));
    assert!(a.run_while(60.0, |a| a.killed.len() < 2), "beats both: {} killed, {}", a.killed.len(), a.sim.last_reason());
    let ids: Vec<u32> = a.killed.iter().map(|k| k.0).collect();
    assert!(ids.contains(&1) && ids.contains(&2));
}

#[test]
fn fights_come_before_geo_and_cursor_play() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.set_fight_mode(true);
    sim.park_cursor(Vec2::new(120.0, -60.0));
    let mut a = Arena::new(sim);
    a.coins = vec![Vec2::new(300.0, FLOOR), Vec2::new(320.0, FLOOR)];
    a.run(0.5);
    assert!(matches!(a.sim.brain.behaviour(), BehaviourKind::Loot | BehaviourKind::Battle), "{}", a.sim.last_reason());
    a.add("crawlid", 1500.0);
    assert!(a.run_until(1.0, BehaviourKind::Fight), "the enemy first: {}", a.sim.last_reason());
    let mut kinds = Vec::new();
    assert!(
        a.run_while(25.0, |a| {
            kinds.push(a.sim.brain.behaviour());
            a.killed.is_empty()
        }),
        "{}",
        a.sim.last_reason()
    );
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Fight), "nothing else while it lives: {kinds:?}");
    assert!(a.run_until(3.0, BehaviourKind::Loot), "then the Geo: {}", a.sim.last_reason());
}

#[test]
fn not_while_asleep() {
    let mut a = Arena::new(sleepy_with_bench());
    assert!(a.sim.run_while(20.0, |s| s.act.as_ref().is_none_or(|x| s.act_name(x.act.id) != "nap")), "asleep: {}", a.sim.last_reason());
    // An enemy off at the far side that never comes near: no fight.
    a.fixed.push(dummy(FoeKind::Enemy, 1850.0));
    let kinds = a.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::Fight), "{}", a.sim.last_reason());
    // A bump wakes him, and then he fights.
    a.hits += 1;
    assert!(a.run_until(5.0, BehaviourKind::Fight), "{}", a.sim.last_reason());
}

#[test]
fn enemies_are_fought_even_while_the_user_types() {
    // Strict priorities (the user's call): an enemy on screen is always dealt with. Typing
    // only holds back cursor games; he fights on his ledges, away from your cursor.
    let mut sim = calm(BrainConfig::default());
    sim.user.keys_per_sec = 4.0;
    let mut a = Arena::new(sim);
    a.run(1.0);
    a.add("crawlid", 1500.0);
    assert!(a.run_until(3.0, BehaviourKind::Fight), "fights while you type: {}", a.sim.last_reason());
    // And sees it through to the Geo (still typing).
    assert!(a.run_until(8.0, BehaviourKind::Loot), "then gathers the drop: {}", a.sim.last_reason());
}

#[test]
fn not_while_something_is_fullscreen() {
    let mut sim = calm(BrainConfig { hide_for_fullscreen: true, ..Default::default() });
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "player.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    let mut a = Arena::new(sim);
    a.run(1.0);
    a.add("crawlid", 1500.0);
    let kinds = a.run(5.0);
    assert!(!kinds.contains(&BehaviourKind::Fight));
}

/// A foe that never moves, standing next to the knight.
fn dummy(kind: FoeKind, x: f32) -> Foe {
    Foe { id: 9, kind, pos: Vec2::new(x, FLOOR - 70.0), feet: Vec2::new(x, FLOOR), half: Vec2::new(40.0, 70.0), flying: false, vulnerable: true, attacking: true }
}

#[test]
fn bumps_hurt_and_he_backs_off_sore_but_alive() {
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(0.5);
    a.fixed.push(dummy(FoeKind::Enemy, 1060.0));
    assert!(a.run_until(1.0, BehaviourKind::Fight));
    for _ in 0..4 {
        a.hits += 1;
        a.run(0.5);
    }
    assert!(a.sim.started.iter().any(|s| s == "recoil"), "recoils from bumps: {:?}", a.sim.started);
    assert_ne!(a.sim.brain.behaviour(), BehaviourKind::Fight, "out of masks: backs off");
    assert_ne!(a.sim.brain.behaviour(), BehaviourKind::Dead, "not lethal by default");
    assert!(a.sim.brain.mind().emotions.hurt > 0.3, "sore");
    let kinds = a.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::Fight), "leaves them alone for a while");
}

#[test]
fn lethal_fights_can_kill_him() {
    let mut cfg = BrainConfig::default();
    cfg.fight.lethal = true;
    let mut a = Arena::new(calm(cfg));
    a.run(0.5);
    a.fixed.push(dummy(FoeKind::Enemy, 1060.0));
    assert!(a.run_until(1.0, BehaviourKind::Fight));
    a.hits += 4;
    assert!(a.run_until(2.0, BehaviourKind::Dead), "{}", a.sim.last_reason());
}

#[test]
fn a_spar_opens_with_a_challenge_and_blows_land_without_killing() {
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(0.5);
    // Zote, on guard right beside him, after their talk.
    a.fixed.push(dummy(FoeKind::Sparring, 1080.0));
    assert!(a.run_until(1.0, BehaviourKind::Fight), "{}", a.sim.last_reason());
    assert!(a.sim.last_reason().contains("sparring"), "{}", a.sim.last_reason());
    a.run(3.0);
    assert!(a.sim.started.iter().any(|s| s == "challenge"), "answers the challenge: {:?}", a.sim.started);
    assert!(a.strikes.iter().any(|s| s.kind == FoeKind::Sparring && s.id == 9), "blows land on him");
    // He goes down (the app stops reporting him): the fight is over.
    a.fixed.clear();
    assert!(a.run_while(2.0, |a| a.sim.brain.behaviour() == BehaviourKind::Fight));
    assert!(a.killed.is_empty());
}
