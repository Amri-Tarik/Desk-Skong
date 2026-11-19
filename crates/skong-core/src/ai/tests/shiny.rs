//! Shinies: one turns up on the ground → the knight spots it, walks over, faces it,
//! picks it up (`pick_up_trinket`) and it is gone. Not while asleep, typing-busy or
//! fullscreen; Geo comes first.
use super::*;

fn shiny(id: u64, x: f32) -> KnownShiny {
    KnownShiny { id, pos: Vec2::new(x, FLOOR), span: (0.0, 1920.0), reach: 20.0 }
}

#[test]
fn spots_a_shiny_walks_over_picks_it_up_and_it_is_gone() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.shinies.push(shiny(1, 1300.0));
    assert!(sim.run_until(1.0, BehaviourKind::Shiny), "noticed at once: {}", sim.last_reason());
    assert!(sim.last_reason().contains("glinting"), "{}", sim.last_reason());
    assert!(sim.run_while(10.0, |s| !s.shinies.is_empty()), "picked up: {} / acts {:?}", sim.last_reason(), sim.started);
    assert_eq!(sim.picked, vec![1]);
    assert_eq!(sim.started.last().map(String::as_str), Some("pick_up_trinket"));
    assert_eq!(sim.knight.pos, Vec2::new(1280.0, FLOOR), "beside it, on the side he came from");
    let picking: Vec<&Intent> = sim.intents.iter().filter(|i| i.reason.contains("picking it up")).collect();
    assert!(!picking.is_empty() && picking.iter().all(|i| i.face.is_none_or(|f| f == Side::Right)), "faces it");
    assert!(picking.iter().any(|i| i.face == Some(Side::Right)));
    // Done once the act is over: on to something else.
    assert!(sim.run_while(6.0, |s| s.brain.behaviour() == BehaviourKind::Shiny), "{}", sim.last_reason());
    assert_eq!(sim.started.iter().filter(|a| *a == "pick_up_trinket").count(), 1);
}

#[test]
fn a_shiny_beats_keeping_company_but_not_a_visitor_or_geo() {
    // Company: a cursor resting near him.
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(150.0, -120.0));
    sim.run(2.0);
    sim.shinies.push(shiny(1, 600.0));
    assert!(sim.run_until(1.0, BehaviourKind::Shiny), "{}", sim.last_reason());

    // Geo appears while he walks over: Geo first, then back for the shiny.
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(1500.0, FLOOR)], ..Default::default() });
    assert!(sim.run_until(1.0, BehaviourKind::Loot), "{}", sim.last_reason());
    assert!(sim.picked.is_empty());
    sim.brain.sense_stakes(StakesSense::default());
    assert!(sim.run_until(3.0, BehaviourKind::Shiny), "back to the shiny: {}", sim.last_reason());
    assert!(sim.run_while(10.0, |s| !s.shinies.is_empty()));
}

#[test]
fn not_while_asleep() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_while(20.0, |s| s.act.as_ref().is_none_or(|a| s.act_name(a.act.id) != "nap")), "asleep: {}", sim.last_reason());
    sim.shinies.push(shiny(1, 700.0));
    let kinds = sim.run(30.0);
    assert!(!kinds.contains(&BehaviourKind::Shiny), "a shiny never wakes him");
    assert!(sim.picked.is_empty());
}

#[test]
fn not_while_the_user_types_hard() {
    let mut sim = calm(BrainConfig::default());
    sim.user.keys_per_sec = 4.0;
    sim.run(1.0);
    sim.shinies.push(shiny(1, 1300.0));
    let kinds = sim.run(20.0);
    assert!(!kinds.contains(&BehaviourKind::Shiny));
    // The typing stops: now he goes for it.
    sim.user.keys_per_sec = 0.0;
    assert!(sim.run_until(5.0, BehaviourKind::Shiny), "{}", sim.last_reason());
}

#[test]
fn not_while_something_is_fullscreen() {
    let mut sim = calm(BrainConfig { hide_for_fullscreen: true, ..Default::default() });
    sim.run(1.0);
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "player.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    sim.shinies.push(shiny(1, 1300.0));
    let kinds = sim.run(15.0);
    assert!(!kinds.contains(&BehaviourKind::Shiny));
    assert!(sim.picked.is_empty());
}

#[test]
fn an_unreachable_shiny_is_left_alone() {
    // No walking at it (with a Dream Gate ready he warps there instead: tests/dream_gate.rs).
    let mut cfg = BrainConfig::default();
    cfg.dream_gate.enabled = false;
    let mut sim = calm(cfg);
    // Every route to its pick-up spot ends somewhere else (it lies on a ledge he can't get onto).
    for st in [TravelStyle::Stroll, TravelStyle::Normal, TravelStyle::Hurry] {
        sim.nav.ends.push((Vec2::new(1280.0, FLOOR), Some(st), Vec2::new(1100.0, FLOOR)));
    }
    sim.run(1.0);
    sim.shinies.push(shiny(1, 1300.0));
    let kinds = sim.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::Shiny), "{}", sim.last_reason());
}

#[test]
fn a_shiny_that_goes_away_is_given_up_quietly() {
    let mut sim = calm(BrainConfig::default());
    sim.nav.trip = 6.0;
    sim.run(1.0);
    sim.shinies.push(shiny(1, 1700.0));
    assert!(sim.run_until(1.0, BehaviourKind::Shiny));
    sim.run(1.5);
    sim.shinies.clear(); // its window closed / it faded
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::Shiny), "{}", sim.last_reason());
    assert!(!sim.started.iter().any(|a| a == "pick_up_trinket"));
}

#[test]
fn a_visitor_comes_before_a_shiny() {
    let mut sim = calm(BrainConfig::default());
    sim.nav.trip = 6.0;
    sim.run(1.0);
    sim.shinies.push(shiny(1, 1700.0));
    assert!(sim.run_until(1.0, BehaviourKind::Shiny));
    sim.run(0.5);
    let v = Visitor { id: 5, name: "cornifer".into(), pos: Vec2::new(400.0, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() };
    sim.brain.see_visitors(vec![v]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());
    assert!(sim.picked.is_empty());
}
