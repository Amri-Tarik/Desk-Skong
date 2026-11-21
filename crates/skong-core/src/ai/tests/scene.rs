//! Scenes: Hornet's race (ready, the full kit on "Go!", a reaction to the outcome), the
//! Troupe Master's friendly visit (gaze and answers), and the scarlet flame (a pastime he
//! lingers at, below fights; his interest wanes, and he comes back).
use super::*;

const FINISH: Vec2 = Vec2::new(1500.0, FLOOR);

fn race(go: bool, over: bool, won: Option<bool>) -> Option<SceneCue> {
    Some(SceneCue::Race { serial: 4, finish: FINISH, go, over, won })
}

#[test]
fn he_readies_for_the_race_runs_on_go_and_reacts_to_the_result() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(race(false, false, None));
    assert!(sim.run_until(1.0, BehaviourKind::Scene), "{}", sim.last_reason());
    sim.run(1.0);
    assert!(sim.last_reason().contains("ready"), "{}", sim.last_reason());
    assert!(matches!(sim.intents.last().unwrap().motion, Motion::Hold));
    assert_eq!(sim.intents.last().and_then(|i| i.face), Some(Side::Right), "faces the finish");
    // Go! The whole kit: a Hurry route to the barb.
    sim.brain.see_scene(race(true, false, None));
    sim.run(0.5);
    assert_eq!(sim.nav.gos.last(), Some(&(NavGoal::Point(FINISH), TravelStyle::Hurry)), "{:?}", sim.nav.gos);
    sim.run(2.5);
    assert!(sim.knight.pos.distance(FINISH) < 2.0, "there: {:?}", sim.knight.pos);
    assert!(sim.last_reason().contains("waiting for the result"), "{}", sim.last_reason());
    // He won: a victory.
    let a = sim.started.len();
    sim.brain.see_scene(race(false, true, Some(true)));
    sim.run(3.0);
    assert!(sim.started_since(a).iter().any(|s| s == "victory" || s == "collect_happy"), "{:?}", sim.started_since(a));
    // Then life goes on (the decided race isn't picked again while it lingers).
    sim.run(3.0);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Scene);
}

#[test]
fn a_lost_race_gets_a_look_down_and_a_called_off_one_nothing() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(race(true, false, None));
    assert!(sim.run_until(1.0, BehaviourKind::Scene));
    sim.run(1.0);
    let a = sim.started.len();
    sim.brain.see_scene(race(false, true, Some(false)));
    sim.run(3.0);
    assert_eq!(sim.started_since(a).first().map(String::as_str), Some("look_down"));
    // Called off: no reaction, back to normal.
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(race(true, false, None));
    assert!(sim.run_until(1.0, BehaviourKind::Scene));
    let a = sim.started.len();
    sim.brain.see_scene(race(false, true, None));
    sim.run(2.0);
    assert!(sim.started_since(a).is_empty(), "{:?}", sim.started_since(a));
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Scene);
}

#[test]
fn the_shade_comes_before_a_race() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(race(true, false, None));
    assert!(sim.run_until(1.0, BehaviourKind::Scene));
    sim.brain.sense_stakes(StakesSense { shade: Some(ShadeSense { pos: Vec2::new(300.0, FLOOR - 60.0), vulnerable: true, fighting: false, geo: 80 }), ..Default::default() });
    assert!(sim.run_until(1.0, BehaviourKind::ShadeHunt), "{}", sim.last_reason());
}

fn watch(beat: u32, x: f32, react: Option<&str>) -> Option<SceneCue> {
    Some(SceneCue::Watch { serial: 9, at: Vec2::new(x, FLOOR - 90.0), beat, react: react.map(str::to_string) })
}

#[test]
fn he_watches_the_troupe_master_and_answers_his_bow() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(watch(1, 1200.0, Some("startled|look_up")));
    assert!(sim.run_until(1.0, BehaviourKind::Scene));
    sim.run(2.0);
    assert!(sim.started.iter().any(|s| s == "startled"), "{:?}", sim.started);
    let a = sim.started.len();
    sim.brain.see_scene(watch(2, 1200.0, Some("challenge")));
    sim.run(2.0);
    assert_eq!(sim.started_since(a), ["challenge"], "a bow for his bow");
    // He teleports to the other side: the gaze follows, the knight stays put.
    let at = sim.knight.pos;
    sim.brain.see_scene(watch(3, 600.0, Some("glance|look_up")));
    sim.run(2.0);
    assert_eq!(sim.intents.last().and_then(|i| i.face), Some(Side::Left), "follows him with his gaze");
    assert!(sim.knight.pos.distance(at) < 1.0, "watches, doesn't move");
    assert!(sim.intents.iter().all(|i| i.attack.is_none() && i.strikes.is_empty()), "never a fight");
    // Gone: back to normal life.
    sim.brain.see_scene(None);
    sim.run(1.0);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Scene);
}

const FLAME: Vec2 = Vec2::new(1400.0, FLOOR);

#[test]
fn the_flame_draws_him_over_he_lingers_and_his_interest_wanes_then_returns() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(Some(SceneCue::Flame { at: FLAME }));
    assert!(sim.run_until(2.0, BehaviourKind::Scene), "{}", sim.last_reason());
    sim.run(3.0);
    assert!(sim.last_reason().contains("lingering by the scarlet flame"), "{}", sim.last_reason());
    assert!((sim.knight.pos.x - FLAME.x).abs() < 3.0 * 64.0, "beside it: {:?}", sim.knight.pos);
    assert!(sim.intents.iter().all(|i| i.attack.is_none()), "never strikes it");
    // He stays a while, wondering (looks, paces), then his interest wanes ...
    assert!(sim.run_while(100.0, |s| s.brain.behaviour() == BehaviourKind::Scene), "his interest wanes");
    let left_at = sim.t;
    assert!(left_at > 40.0, "lingered a good while: {left_at}");
    // ... and it draws him back later.
    assert!(sim.run_until(90.0, BehaviourKind::Scene), "comes back to it");
    assert!(sim.t - left_at >= 55.0, "not straight away: {}", sim.t - left_at);
}

#[test]
fn the_flame_is_a_pastime_fights_come_first_and_he_returns_after() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.see_scene(Some(SceneCue::Flame { at: FLAME }));
    assert!(sim.run_until(5.0, BehaviourKind::Scene));
    sim.run(4.0);
    // An enemy turns up right by him: the fight comes first.
    let k = sim.knight.pos;
    let foe = Foe { id: 1, kind: FoeKind::Enemy, pos: k + Vec2::new(60.0, -40.0), feet: k + Vec2::new(60.0, 0.0), half: Vec2::new(30.0, 40.0), vulnerable: true, ..Default::default() };
    sim.brain.see_foes(vec![foe], 0);
    assert!(sim.run_until(1.0, BehaviourKind::Fight), "{}", sim.last_reason());
    // It's gone: back to the flame.
    sim.brain.see_foes(vec![], 0);
    assert!(sim.run_until(15.0, BehaviourKind::Scene), "back at the flame: {}", sim.last_reason());
}
