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
    /// Controller events so far (with `physics`).
    moves: Vec<MoveEvent>,
}

impl Arena {
    fn new(sim: Sim) -> Self {
        Self { sim, set: roster(), enemies: vec![], next_id: 1, hits: 0, killed: vec![], coins: vec![], strikes: vec![], fixed: vec![], moves: vec![] }
    }

    /// The real controller moves him (jumps, pogos, dives, dashes).
    fn physical(sim: Sim) -> Self {
        let mut a = Self::new(sim);
        a.sim.physics = true;
        a
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
        // His moves this step (with `physics`: the real controller ran).
        self.moves.extend(self.sim.events.iter().copied());
        // Direct input walks him (the brain sim only moves him along routes).
        if let Motion::Direct(i) = intent.motion
            && self.sim.act.is_none()
            && !self.sim.physics
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
    Foe { id: 9, kind, pos: Vec2::new(x, FLOOR - 70.0), feet: Vec2::new(x, FLOOR), half: Vec2::new(40.0, 70.0), flying: false, vulnerable: true, attacking: true, ..Default::default() }
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
fn bumps_while_the_user_holds_him_do_not_land_on_release() {
    let mut cfg = BrainConfig::default();
    cfg.fight.lethal = true;
    let mut a = Arena::new(calm(cfg));
    a.run(0.5);
    a.fixed.push(dummy(FoeKind::Enemy, 1060.0));
    assert!(a.run_until(1.0, BehaviourKind::Fight));
    // Picked up mid-fight; the foe keeps bumping the dangling knight for a while.
    a.sim.pokes.push(Poke::Grab);
    for _ in 0..(5.0 / DT) as usize {
        a.hits += 1;
        a.step();
    }
    a.sim.pokes.push(Poke::Release { vel: Vec2::ZERO });
    let kinds = a.run(2.0);
    assert!(!kinds.contains(&BehaviourKind::Dead), "a backlog of bumps killed him on release: {}", a.sim.last_reason());
    assert_eq!(a.sim.brain.stakes.fight.hits, 0);
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

#[test]
fn no_swing_before_the_spar_begins() {
    // Hornet / Zote still saying their challenge (`waiting`, not yet struck-able): he squares
    // up facing them, but starts no attack, casts nothing and lands nothing until it's said.
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(0.5);
    a.fixed.push(Foe { waiting: true, vulnerable: false, attacking: false, ..dummy(FoeKind::Sparring, 1080.0) });
    assert!(a.run_until(1.0, BehaviourKind::Fight), "{}", a.sim.last_reason());
    let n = a.sim.intents.len();
    a.run(5.0);
    let early = &a.sim.intents[n..];
    assert!(early.iter().all(|i| i.attack.is_none()), "no swing while she's still talking");
    assert!(early.iter().all(|i| !i.stakes.iter().any(|e| matches!(e, StakeEvent::Cast { .. }))), "no spells either");
    assert!(a.strikes.is_empty(), "nothing struck: {:?}", a.strikes);
    assert_eq!(a.sim.brain.behaviour(), BehaviourKind::Fight, "still squared up: {}", a.sim.last_reason());
    assert!(early.iter().rev().take(30).all(|i| i.face.is_none_or(|f| f == Side::Right)), "facing her");
    // Her line is over, she's on guard: now the blows come.
    a.fixed[0] = dummy(FoeKind::Sparring, 1080.0);
    a.run(3.0);
    assert!(a.strikes.iter().any(|s| s.kind == FoeKind::Sparring), "then he fights");
}

// --- The combat kit: pogo, spells by situation, dive, dodge ------------------------------

use crate::ai::Technique;
use crate::stakes::Spell;

fn techniques(a: &Arena) -> Vec<Technique> {
    a.sim.intents.iter().flat_map(|i| i.stakes.iter()).filter_map(|e| if let StakeEvent::Technique(t) = e { Some(*t) } else { None }).collect()
}

/// Spells and nail arts cast, with the time (s since the start).
fn casts(a: &Arena) -> Vec<(f32, Spell)> {
    a.sim
        .intents
        .iter()
        .enumerate()
        .flat_map(|(n, i)| i.stakes.iter().map(move |e| (n as f32 * DT, *e)))
        .filter_map(|(t, e)| if let StakeEvent::Cast { spell, .. } = e { Some((t, spell)) } else { None })
        .collect()
}

fn pogos(a: &Arena) -> usize {
    a.moves.iter().filter(|m| **m == MoveEvent::Pogo).count()
}

/// The knight in mid-air at `at` (feet), coming down.
fn airborne_at(a: &mut Arena, at: Vec2) {
    let k = &mut a.sim.knight;
    k.pos = at;
    k.vel = Vec2::new(0.0, 100.0);
    k.support = None;
    k.state = MoveState::Airborne;
}

#[test]
fn he_pogos_a_ground_enemy_when_above_it() {
    let mut a = Arena::physical(calm(BrainConfig::default()));
    a.run(0.5);
    let id = a.add("crawlid", 1000.0);
    a.run(0.3);
    // Coming down right on top of it.
    let x = a.enemies[0].pos.x;
    airborne_at(&mut a, Vec2::new(x, FLOOR - 260.0));
    assert!(a.run_while(2.0, |a| pogos(a) == 0), "bounces off it: {} / {:?}", a.sim.last_reason(), a.moves);
    assert!(a.strikes.iter().any(|s| s.id == id), "the down slash hits it");
    assert!(a.sim.intents.iter().any(|i| i.attack.is_some_and(|at| at.dir == AttackDir::Down && at.hit)), "a down slash");
    assert!(a.sim.knight.vel.y < 0.0, "going up again");
    assert!(a.sim.knight.can_double_jump(), "double jump back");
    // It doesn't survive the next bounce (2 hp).
    assert!(a.run_while(3.0, |a| a.killed.is_empty()), "pogoed to death: {}", a.sim.last_reason());
}

#[test]
fn he_jumps_up_to_pogo_a_foe_beside_him_and_chains_the_bounces() {
    let mut cfg = BrainConfig::default();
    cfg.stakes.spells.chance = 0.0; // plain moves only
    let mut a = Arena::physical(calm(cfg));
    a.run(0.5);
    // A sturdy one that stays put (so the chain has something to bounce on).
    a.fixed.push(dummy(FoeKind::Enemy, 1150.0));
    assert!(a.run_while(6.0, |a| pogos(a) < 2), "chains pogos: {} / {:?}", a.sim.last_reason(), a.moves);
    assert!(a.moves.contains(&MoveEvent::Jumped), "jumped at it first");
    assert!(a.strikes.iter().filter(|s| s.id == 9).count() >= 2, "every bounce is a hit");
}

#[test]
fn he_screams_at_a_flyer_above() {
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(0.5);
    let k = a.sim.body_center();
    a.fixed.push(Foe { id: 21, pos: k + Vec2::new(30.0, -230.0), feet: k + Vec2::new(30.0, -185.0), half: Vec2::new(45.0, 45.0), flying: true, vulnerable: true, ..Default::default() });
    assert!(a.run_while(3.0, |a| !casts(a).iter().any(|c| c.1 == Spell::Scream)), "howling wraiths: {:?} / {}", casts(&a), a.sim.last_reason());
    assert!(a.strikes.iter().any(|s| s.id == 21 && s.damage == Spell::Scream.damage()), "the wraiths hit it");
}

#[test]
fn he_dives_on_an_enemy_below() {
    let mut sim = calm(BrainConfig::default());
    sim.open(window(1, RectI::new(700, 600, 1300, 900), "editor.exe"));
    let mut a = Arena::physical(sim);
    // Standing on the window top, right above the floor where a crawlid crawls.
    a.sim.place(Vec2::new(1000.0, 600.0));
    a.run(0.3);
    let id = a.add("crawlid", 1040.0);
    assert!(a.run_while(4.0, |a| !a.moves.contains(&MoveEvent::DiveLanded)), "dives: {} / {:?}", a.sim.last_reason(), techniques(&a));
    assert!(techniques(&a).iter().any(|t| matches!(t, Technique::Dive { .. })), "{:?}", techniques(&a));
    a.run(0.2);
    assert!(a.strikes.iter().any(|s| s.id == id && s.damage >= 2), "the slam hits it: {:?}", a.strikes);
}

#[test]
fn he_dodges_a_lunge_with_a_shadow_dash_and_it_cannot_touch_him() {
    let mut a = Arena::physical(calm(BrainConfig::default()));
    a.run(0.5);
    // A husk mid-lunge, coming at him from the right (it can't be struck right now).
    let k = a.sim.knight.pos;
    let husk = Foe { id: 31, pos: k + Vec2::new(150.0, -80.0), feet: k + Vec2::new(150.0, 0.0), half: Vec2::new(40.0, 80.0), attacking: true, lunging: true, ..Default::default() };
    a.fixed.push(husk);
    assert!(a.run_while(1.0, |a| !techniques(a).contains(&Technique::Dodge)), "dodges: {}", a.sim.last_reason());
    a.run(0.05);
    assert!(a.moves.contains(&MoveEvent::DashStarted { shadow: true }), "a shadow dash: {:?}", a.moves);
    // Its bump while he passes through doesn't count.
    let hurt = a.sim.brain.mind().emotions.hurt;
    a.hits += 1;
    a.run(0.1);
    assert!(a.sim.brain.mind().emotions.hurt <= hurt + 1e-4, "untouched");
    a.run(0.2);
    assert!(a.sim.knight.pos.x > k.x + 150.0, "came out the other side: {}", a.sim.knight.pos.x - k.x);
}

/// A long fight against waves of foes (the real controller moving him).
fn long_fight(secs: f32) -> Arena {
    let mut a = Arena::physical(calm(BrainConfig::default()));
    a.run(0.5);
    let mut wave = 0;
    for _ in 0..(secs / DT) as usize {
        if a.enemies.iter().all(|e| !e.alive()) {
            wave += 1;
            let x = if wave % 2 == 0 { 400.0 } else { 1500.0 };
            a.add(["husk", "vengefly", "crawlid"][wave % 3], x);
            a.add(["crawlid", "husk", "vengefly"][wave % 3], x + 140.0);
        }
        a.step();
    }
    a
}

#[test]
fn spells_and_nail_arts_each_show_up_in_a_longer_fight() {
    let a = long_fight(120.0);
    let spells: Vec<Spell> = casts(&a).iter().map(|c| c.1).collect();
    assert!(spells.iter().any(|s| matches!(s, Spell::Fireball | Spell::Scream)), "a spell: {spells:?}");
    assert!(spells.iter().any(|s| matches!(s, Spell::GreatSlash | Spell::Cyclone | Spell::DashSlash)), "a nail art: {spells:?}");
    assert!(pogos(&a) > 0, "pogos");
    assert!(a.killed.len() >= 4, "and it gets things done: {} killed", a.killed.len());
}

#[test]
fn nothing_spammy_cooldowns_hold() {
    use crate::ai::combat::{DIVE_COOLDOWN, DODGE_COOLDOWN, OPENER_COOLDOWN, POGO_CHAIN, POGO_SWING_CD};
    let a = long_fight(90.0);
    let cfg = crate::stakes::SpellConfig::default();
    // Specials keep their gap (thorns answer hits, focus closes a fight: not choices).
    let specials: Vec<f32> = casts(&a).iter().filter(|c| !matches!(c.1, Spell::Thorns | Spell::Focus)).map(|c| c.0).collect();
    assert!(specials.windows(2).all(|w| w[1] - w[0] >= cfg.min_gap - 1e-3), "specials too close: {specials:?}");
    // Dives, dodges and openers keep theirs.
    let when = |pred: &dyn Fn(&Technique) -> bool| -> Vec<f32> {
        let used = |i: &Intent| i.stakes.iter().any(|e| matches!(e, StakeEvent::Technique(t) if pred(t)));
        a.sim.intents.iter().enumerate().filter(|(_, i)| used(i)).map(|(n, _)| n as f32 * DT).collect()
    };
    for (name, times, gap) in [
        ("dive", when(&|t| matches!(t, Technique::Dive { .. })), DIVE_COOLDOWN),
        ("dodge", when(&|t| *t == Technique::Dodge), DODGE_COOLDOWN),
        ("opener", when(&|t| *t == Technique::CrystalOpener), OPENER_COOLDOWN),
    ] {
        assert!(times.windows(2).all(|w| w[1] - w[0] >= gap - 1e-3), "{name}s too close: {times:?}");
    }
    // Pogo runs end after a few bounces: never more than the chain without touching ground.
    let mut run = 0;
    for m in &a.moves {
        match m {
            MoveEvent::Pogo => run += 1,
            MoveEvent::Landed { .. } | MoveEvent::DiveLanded => run = 0,
            _ => {}
        }
        assert!(run <= POGO_CHAIN, "pogo chain too long");
    }
    // Swings keep a cooldown.
    let swings: Vec<usize> = a.sim.intents.iter().enumerate().filter(|(_, i)| i.attack.is_some()).map(|(n, _)| n).collect();
    assert!(swings.windows(2).all(|w| (w[1] - w[0]) as f32 * DT >= POGO_SWING_CD - 1e-3), "slash spam");
}

#[test]
fn he_pogos_off_a_low_cursor_in_a_battle() {
    let mut sim = calm(BrainConfig::default());
    sim.physics = true;
    sim.brain.set_fight_mode(true);
    // Down by his legs: awkward for a forward slash, perfect to bounce on.
    sim.park_cursor(Vec2::new(70.0, 25.0));
    assert!(sim.run_until(2.0, BehaviourKind::Battle), "{}", sim.last_reason());
    let mut seen = Vec::new();
    let bounced = sim.run_while(10.0, |s| {
        seen.extend(s.events.iter().copied());
        !s.events.contains(&MoveEvent::Pogo)
    });
    assert!(bounced, "pogos off the cursor: {} / {seen:?}", sim.last_reason());
}

#[test]
fn hurt_after_a_fight_he_focuses_to_heal() {
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(0.5);
    a.fixed.push(dummy(FoeKind::Enemy, 1060.0));
    assert!(a.run_until(1.0, BehaviourKind::Fight));
    a.hits += 2;
    a.run(2.0);
    let hurt = a.sim.brain.mind().emotions.hurt;
    assert!(hurt > 0.15, "bumped: {hurt}");
    // It's gone (beaten, or it left): he focuses before moving on.
    a.fixed.clear();
    assert!(a.run_while(3.0, |a| !a.sim.started.iter().any(|s| s == "focus_heal")), "focuses: {} / {:?}", a.sim.last_reason(), a.sim.started);
    a.run(4.0);
    assert!(a.sim.brain.mind().emotions.hurt < hurt, "feels better");
}


#[test]
fn a_fight_under_way_keeps_its_place_when_he_gets_sore() {
    // Live: a stutter with several foes. A bump took him past the soreness line; the fight
    // lost its tier and Geo (or Recover) cut in, then the fight took over again as he
    // healed — fight, loot, fight, loot, every second or so. The fight ends itself.
    let mut a = Arena::physical(calm(BrainConfig::default()));
    a.run(0.5);
    a.coins = vec![Vec2::new(200.0, FLOOR), Vec2::new(240.0, FLOOR)];
    a.add("husk", 1500.0);
    a.add("crawlid", 1700.0);
    assert!(a.run_until(2.0, BehaviourKind::Fight), "{}", a.sim.last_reason());
    a.sim.brain.mind_mut().emotions.hurt = 0.7;
    let mut kinds = Vec::new();
    a.run_while(6.0, |a| {
        kinds.push(a.sim.brain.behaviour());
        a.enemies.iter().any(|e| e.alive())
    });
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Fight), "sore, he fights on: {} {:?} killed {:?}", a.sim.last_reason(), a.sim.switches, a.killed);
}

#[test]
fn a_foe_in_the_way_becomes_the_target() {
    // Walking past one foe to reach another got him bumped and stopped at every step.
    let mut a = Arena::new(calm(BrainConfig::default()));
    a.run(0.5);
    a.fixed.push(dummy(FoeKind::Enemy, 1600.0));
    assert!(a.run_until(1.0, BehaviourKind::Fight), "{}", a.sim.last_reason());
    assert!(a.sim.last_reason().contains("#9 ") || a.sim.last_reason().ends_with("#9"), "{}", a.sim.last_reason());
    // Another one now right in front of him, between him and it.
    let k = a.sim.knight.pos.x;
    a.fixed.push(Foe { id: 78, ..dummy(FoeKind::Enemy, k + 150.0) });
    a.run(0.2);
    assert!(a.sim.last_reason().contains("#78"), "the one in the way first: {}", a.sim.last_reason());
}

#[test]
fn he_trails_a_foe_walking_away_without_stuttering() {
    // Live: behind a crawler walking away he started and stopped every frame (the stop
    // line of his approach was also its start line), creeping along at its pace. Now he
    // keeps pace with it on the ground instead of standing until it is out of reach again.
    let mut a = Arena::physical(calm(BrainConfig::default()));
    a.run(0.5);
    a.fixed.push(Foe { vulnerable: false, attacking: false, ..dummy(FoeKind::Enemy, 1100.0) });
    assert!(a.run_until(1.0, BehaviourKind::Fight), "{}", a.sim.last_reason());
    a.run(1.0);
    let start = a.sim.knight.pos.x;
    let (mut ground, mut standing) = (0, 0);
    for _ in 0..(4.0 / DT) as usize {
        a.fixed[0].pos.x += 60.0 * DT;
        a.fixed[0].feet.x += 60.0 * DT;
        let i = a.step();
        if let Motion::Direct(input) = i.motion
            && a.sim.knight.is_grounded()
        {
            ground += 1;
            if input.x == 0.0 {
                standing += 1;
            }
        }
    }
    let gap = a.fixed[0].pos.x - a.sim.knight.pos.x;
    assert!(a.sim.knight.pos.x - start > 150.0, "he follows it: {start} -> {}", a.sim.knight.pos.x);
    assert!(gap < 4.0 * PPU, "and keeps up (gap {gap})");
    assert!(ground > 10, "walking it down on the ground: {ground} ticks");
    assert!(standing * 5 <= ground, "keeps pace instead of standing until it is out of reach: stood {standing} of {ground} ground ticks");
}
