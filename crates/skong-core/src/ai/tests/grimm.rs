//! The troupe master, fought for real: the brain and the real controller against the Grimm
//! sim (`crate::enemy::grimm`). He strikes the lantern, answers the bow, uses his kit
//! against the routine (dodges, jumps the spikes, steps off the pillars), wins most shows
//! and loses some; a loss is a real death (a Shade). The Shade still comes first.
use std::path::Path;

use super::*;
use crate::enemy::grimm::{GrimmDirector, GrimmEnv, GrimmEvent, GrimmSet, GrimmWorld, HitBy, Summon};
use crate::geom::RectF;
use crate::stakes::DeathKind;

const PPU: f32 = 64.0;
const AREA: RectF = RectF::new(0.0, 0.0, 1920.0, FLOOR);

fn troupe() -> GrimmSet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    GrimmSet::load(&d.join("grimm.json"), &d.join("hk/animations"), &bank).expect("grimm")
}

/// The brain sim + the director (the app's side: foes, blows, hits, the lantern).
struct Show {
    sim: Sim,
    set: GrimmSet,
    d: GrimmDirector,
    hits: u32,
    events: Vec<GrimmEvent>,
    died: bool,
    took: u32,
    shade: Option<ShadeSense>,
    lantern_at: Option<Vec2>,
    hurt_by: Vec<HitBy>,
    /// What was going on at each hit (balance report).
    notes: Vec<String>,
    fullscreen: bool,
}

impl Show {
    fn new(sim: Sim, seed: u32) -> Self {
        let set = troupe();
        let d = GrimmDirector::new(&set.cfg, seed);
        let mut sim = sim;
        sim.physics = true;
        Self { sim, set, d, hits: 0, events: vec![], died: false, took: 0, shade: None, lantern_at: None, hurt_by: vec![], notes: vec![], fullscreen: false }
    }

    fn step(&mut self) {
        let foes = self.d.foes(&self.set, PPU);
        self.sim.brain.see_foes(foes, std::mem::take(&mut self.hits));
        self.sim.brain.sense_stakes(StakesSense { shade: self.shade, ..Default::default() });
        let intent = self.sim.step();
        let x = self.sim.knight.pos.x;
        for s in &intent.strikes {
            match s.kind {
                FoeKind::Boss => {
                    self.d.strike(s.damage, &self.set);
                }
                FoeKind::Lantern => self.events.extend(self.d.strike_lantern(&self.set, x, PPU)),
                _ => {}
            }
        }
        if intent.stakes.iter().any(|e| matches!(e, StakeEvent::Died { kind: DeathKind::Fight, .. })) {
            self.died = true;
        }
        let down = self.sim.brain.behaviour() == BehaviourKind::Dead || self.died;
        let w = GrimmWorld { achievements: 20, user_idle_secs: 2.0, knight_at: self.sim.knight.pos, knight_down: down, fullscreen: self.fullscreen, ready: true, ..Default::default() };
        let knight = (!down).then(|| self.sim.knight.body());
        let env = GrimmEnv { terrain: &self.sim.terrain, knight, ppu: PPU, area: AREA };
        let step = self.d.tick(DT, &w, &env, &self.set);
        if step.place_lantern {
            let at = self.lantern_at.unwrap_or(Vec2::new(1400.0, FLOOR));
            self.events.extend(self.d.lantern_placed(Some((at, None)), &env));
        }
        self.hits += step.hits;
        self.took += step.hits;
        self.hurt_by.extend(step.hit_by);
        if let Some(by) = step.hit_by {
            let act = self.sim.act.as_ref().map(|a| self.sim.act_name(a.act.id)).unwrap_or_default();
            let g = self.d.grimm().map(|g| (g.pos - self.sim.knight.pos) / PPU);
            self.notes.push(format!("{:5.1}s {by:?} act '{act}' {:?} grimm at {g:?} | {}", self.sim.t, self.sim.knight.state, self.sim.last_reason()));
        }
        self.events.extend(step.events);
    }

    fn run_while(&mut self, secs: f32, mut cond: impl FnMut(&Show) -> bool) -> bool {
        for _ in 0..(secs / DT).round() as usize {
            self.step();
            if !cond(self) {
                return true;
            }
        }
        false
    }

    fn won(&self) -> bool {
        self.events.iter().any(|e| matches!(e, GrimmEvent::Defeated { .. }))
    }

    fn over(&self) -> bool {
        self.won() || self.died
    }
}

/// One whole show from a direct summon: (won, hits he took, seconds).
fn bout(seed: u32) -> (bool, u32, f32, Show) {
    let mut s = Show::new(calm(BrainConfig::default()), seed);
    s.sim.run(0.3);
    s.d.request(Summon::Direct);
    let mut t = 0.0;
    s.run_while(300.0, |s| {
        t += DT;
        !s.over()
    });
    (s.won(), s.took, t, s)
}

fn techniques(s: &Show) -> Vec<Technique> {
    s.sim.intents.iter().flat_map(|i| i.stakes.iter()).filter_map(|e| if let StakeEvent::Technique(t) = e { Some(*t) } else { None }).collect()
}

#[test]
fn he_strikes_the_scarlet_flame_and_answers_the_troupe_masters_bow() {
    let mut s = Show::new(calm(BrainConfig::default()), 3);
    s.sim.run(0.5);
    s.lantern_at = Some(Vec2::new(1350.0, FLOOR));
    s.d.request(Summon::Lantern);
    assert!(s.run_while(3.0, |s| s.d.lantern().is_none()), "the flame lights");
    // Curious, he goes over and strikes it.
    assert!(s.run_while(15.0, |s| s.d.grimm().is_none()), "strikes it: {}", s.sim.last_reason());
    assert!(s.events.contains(&GrimmEvent::Summoned { via: Summon::Lantern }), "{:?}", s.events);
    assert!(s.sim.intents.iter().any(|i| i.reason.contains("scarlet flame")), "a reason for it");
    // The bow: he answers with his own, and no blows while Grimm is introducing himself.
    let before = s.sim.intents.len();
    assert!(s.run_while(5.0, |s| !s.sim.started.iter().any(|a| a == "challenge")), "answers the bow: {:?}", s.sim.started);
    s.run_while(8.0, |s| s.d.grimm().is_some_and(|g| g.stage() == crate::enemy::grimm::GrimmStage::Intro));
    assert!(s.sim.intents[before..].iter().all(|i| i.strikes.is_empty()), "no cheap shots during the bow");
    assert!(s.sim.last_reason().contains("troupe master"), "{}", s.sim.last_reason());
}

#[test]
fn he_beats_the_troupe_master_with_his_whole_kit_and_wins_the_geo() {
    let (won, took, t, s) = bout(3);
    assert!(won, "seed 3 is a win: {took} hits taken, {t:.0} s");
    assert!((30.0..120.0).contains(&t), "a real fight: {t:.0} s");
    let tech = techniques(&s);
    assert!(tech.contains(&Technique::Dodge), "shadow dashes through his dashes / bats: {tech:?}");
    let blows = s.sim.intents.iter().filter(|i| i.strikes.iter().any(|st| st.kind == FoeKind::Boss)).count();
    assert!(blows >= 10, "lots of blows: {blows}");
    assert!(s.events.iter().any(|e| matches!(e, GrimmEvent::Phase { phase: 3 })), "saw all three acts: {:?}", s.events);
    let geo = s.events.iter().find_map(|e| if let GrimmEvent::Defeated { geo } = e { Some(*geo) } else { None }).expect("defeated");
    assert!(geo >= 100, "a big burst");
    assert!(!s.died && s.sim.brain.behaviour() != BehaviourKind::Dead);
}

#[test]
fn he_wins_most_shows_but_not_all() {
    // Seeded bouts on the plain floor: hard, but winnable (the balance guard).
    let results: Vec<bool> = (1..=12).map(|seed| bout(seed).0).collect();
    let wins = results.iter().filter(|w| **w).count();
    assert!((8..12).contains(&wins), "{wins} / 12 won: {results:?}");
}

#[test]
fn falling_to_him_is_a_real_death_and_leaves_a_shade() {
    let mut cfg = BrainConfig::default();
    cfg.fight.boss_masks = 1;
    let mut s = Show::new(calm(cfg), 5);
    s.sim.run(0.3);
    s.d.request(Summon::Direct);
    assert!(s.run_while(90.0, |s| !s.died), "one mask: he falls ({})", s.sim.last_reason());
    let died = s.sim.intents.iter().flat_map(|i| i.stakes.iter()).find_map(|e| if let StakeEvent::Died { kind, .. } = e { Some(*kind) } else { None });
    assert_eq!(died, Some(DeathKind::Fight));
    assert!(DeathKind::Fight.leaves_shade(), "a Shade with his Geo");
    // Grimm bows, says something kind, and leaves.
    s.run_while(6.0, |s| s.d.grimm().is_some());
    assert!(s.events.contains(&GrimmEvent::KnightFell), "{:?}", s.events);
    assert!(s.d.grimm().is_none(), "gone");
}

#[test]
fn the_shade_still_comes_first() {
    let mut s = Show::new(calm(BrainConfig::default()), 3);
    s.sim.run(0.3);
    s.d.request(Summon::Direct);
    assert!(s.run_while(12.0, |s| !s.sim.last_reason().contains("troupe master")), "fighting him: {}", s.sim.last_reason());
    assert_eq!(s.sim.brain.behaviour(), BehaviourKind::Fight);
    // His Shade turns up (from an earlier death): Urgent beats the boss.
    s.shade = Some(ShadeSense { pos: Vec2::new(400.0, FLOOR - 60.0), vulnerable: true, fighting: false, geo: 120 });
    assert!(s.run_while(2.0, |s| s.sim.brain.behaviour() != BehaviourKind::ShadeHunt), "{}", s.sim.last_reason());
}

#[test]
fn a_fullscreen_app_ends_the_show_without_harm() {
    let mut s = Show::new(calm(BrainConfig::default()), 3);
    s.sim.run(0.3);
    s.d.request(Summon::Direct);
    s.run_while(15.0, |s| s.d.grimm().is_none_or(|g| g.stage() != crate::enemy::grimm::GrimmStage::Fight));
    let mut w = window(30, RectI::new(0, 0, 1920, 1080), "player.exe");
    w.fullscreen = true;
    w.foreground = true;
    s.sim.open(w);
    s.fullscreen = true;
    let hurt = s.sim.brain.mind().emotions.hurt;
    s.run_while(6.0, |s| s.d.grimm().is_some());
    assert!(s.events.contains(&GrimmEvent::BowedOut { reason: crate::enemy::grimm::BowOutReason::Fullscreen }), "{:?}", s.events);
    assert!(s.d.grimm().is_none(), "gone");
    assert!(!s.died && s.sim.brain.mind().emotions.hurt <= hurt + 1e-3, "no penalty");
    assert_ne!(s.sim.brain.behaviour(), BehaviourKind::Fight);
}

#[test]
#[ignore = "debug trace: GRIMM_SEED=1 GRIMM_FROM=13 cargo test -p skong-core grimm_trace -- --ignored --nocapture"]
fn grimm_trace() {
    let seed: u32 = std::env::var("GRIMM_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let from: f32 = std::env::var("GRIMM_FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(13.0);
    let mut s = Show::new(calm(BrainConfig::default()), seed);
    s.sim.run(0.3);
    s.d.request(Summon::Direct);
    while s.sim.t < from + 2.0 && !s.over() {
        s.step();
        if s.sim.t >= from {
            let g = s.d.grimm();
            let i = s.sim.intents.last().cloned().unwrap_or_default();
            let input = match i.motion {
                Motion::Direct(m) => format!("x{:+.0} j{} d{}", m.x, u8::from(m.jump_pressed || m.jump_held), u8::from(m.dash_pressed)),
                m => format!("{m:?}").chars().take(20).collect(),
            };
            println!(
                "{:5.2} k {:?} {:?} | g {:?} {:?} rel {:?} vel {:?} | in {input} atk {:?} | {}",
                s.sim.t,
                s.sim.knight.state,
                (s.sim.knight.pos / PPU).round(),
                g.map(|g| g.current_move()),
                g.map(|g| (g.winding_up(), g.lunging())),
                g.map(|g| ((g.pos - s.sim.knight.pos) / PPU * 10.0).round() / 10.0),
                g.map(|g| g.foe(&s.set, PPU).map(|f| (f.vel / PPU).round())),
                i.attack.map(|a| a.dir),
                i.reason
            );
        }
    }
}

#[test]
#[ignore = "balance report: cargo test -p skong-core grimm_balance -- --ignored --nocapture"]
fn grimm_balance() {
    let mut wins = 0;
    let mut by: std::collections::BTreeMap<String, u32> = Default::default();
    for seed in 1..=20 {
        let (won, took, t, s) = bout(seed);
        wins += u32::from(won);
        let techniques: Vec<String> = s.sim.intents.iter().flat_map(|i| i.stakes.iter()).filter_map(|e| if let StakeEvent::Technique(t) = e { Some(format!("{t:?}")) } else { None }).collect();
        for h in &s.hurt_by {
            *by.entry(format!("{h:?}")).or_default() += 1;
        }
        println!("seed {seed:2}: {} after {t:5.1}s, {took} hits taken, hp left {:?}, techniques {techniques:?}", if won { "WON " } else { "lost" }, s.d.grimm().map(|g| g.hp));
        if std::env::var("GRIMM_NOTES").is_ok() {
            for n in &s.notes {
                println!("    {n}");
            }
        }
    }
    println!("hurt by {by:?}");
    println!("{wins} / 20 won");
}
