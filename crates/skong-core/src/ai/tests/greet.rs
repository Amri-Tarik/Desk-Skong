//! Greet: a visitor shows up → the knight hurries over, says hello, asks for the
//! conversation, reacts to each line, and moves on once greeted.
use super::*;

fn visitor(id: u32, x: f32) -> Visitor {
    Visitor {
        id,
        name: "cornifer".into(),
        pos: Vec2::new(x, FLOOR),
        state: VisitorState::Waiting,
        greet_react: Some("look_up".into()),
        excitement: 1.0,
        ..Default::default()
    }
}

/// Step until `cond` holds (true) or `secs` pass (false).
fn until(sim: &mut Sim, secs: f32, mut cond: impl FnMut(&Sim, &Intent) -> bool) -> bool {
    for _ in 0..(secs / DT).round() as usize {
        let i = sim.step();
        if cond(sim, &i) {
            return true;
        }
    }
    false
}

#[test]
fn hurries_over_greets_and_follows_the_conversation() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let excitement = sim.brain.mind().emotions.excitement;
    let mut v = visitor(5, 1500.0);
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.brain.mind().emotions.excitement > excitement, "a visitor is exciting");
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());

    // Hurries to a spot beside the visitor, on his own side.
    sim.step();
    let (goal, style) = *sim.nav.gos.last().expect("travels");
    assert_eq!(style, TravelStyle::Hurry);
    let NavGoal::Point(p) = goal else { panic!("point goal") };
    assert!(p.x < 1500.0 && 1500.0 - p.x <= 2.0 * STAND_OFF_UNITS * 64.0, "stands left of the visitor: {p}");
    assert!(sim.last_reason().contains("hurrying"), "{}", sim.last_reason());

    // Arrives: says hello (look_up), then asks the app to start talking.
    let n = sim.started.len();
    assert!(until(&mut sim, 8.0, |_, i| i.greet == Some(5)), "{}", sim.last_reason());
    assert_eq!(sim.started_since(n).first().map(String::as_str), Some("look_up"));
    assert_eq!(sim.intents.last().and_then(|i| i.face), Some(Side::Right), "faces the visitor");

    // The visitor talks; each line's react is played.
    v.state = VisitorState::Talking;
    v.line = Some(0);
    v.react = Some("challenge".into());
    sim.brain.see_visitors(vec![v.clone()]);
    let n = sim.started.len();
    sim.run(3.0);
    assert_eq!(sim.started_since(n), ["challenge"]);
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Greet);

    v.line = Some(1);
    v.react = Some("read_map|look_down".into()); // read_map needs a seat: falls back
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(3.0);
    assert_eq!(sim.started_since(n), ["challenge", "look_down"]);
    assert!(sim.last_reason().starts_with("greet cornifer: "), "{}", sim.last_reason());

    // Conversation over: greeted visitors are not greeted again.
    v.state = VisitorState::Waiting;
    v.line = None;
    v.react = None;
    v.greeted = true;
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::Greet), "moves on");
    let kinds = sim.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::Greet));
}

#[test]
fn ignores_visitors_during_fullscreen_and_when_leaving() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let mut v = visitor(1, 1500.0);
    v.state = VisitorState::Leaving;
    sim.brain.see_visitors(vec![v]);
    assert!(!sim.run(3.0).contains(&BehaviourKind::Greet));

    let mut sim = calm(BrainConfig { hide_for_fullscreen: true, ..BrainConfig::default() });
    let mut game = window(9, RectI::new(0, 0, 1920, 1080), "game.exe");
    game.fullscreen = true;
    game.foreground = true;
    sim.open(game);
    sim.run(1.0);
    sim.brain.see_visitors(vec![visitor(2, 1500.0)]);
    assert!(!sim.run(3.0).contains(&BehaviourKind::Greet));
}

#[test]
fn gives_up_on_an_unreachable_visitor_for_a_while() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.wall = true; // pressing without moving: stuck on the way
    sim.brain.see_visitors(vec![visitor(3, 1500.0)]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet));
    assert!(sim.run_while(6.0, |s| s.brain.behaviour() == BehaviourKind::Greet), "gave up");
    sim.wall = false;
    let kinds = sim.run(20.0);
    assert!(!kinds.contains(&BehaviourKind::Greet), "snubbed for a while");
}

#[test]
fn a_visitor_already_beside_him_is_greeted_without_travel() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let gos = sim.nav.gos.len();
    sim.brain.see_visitors(vec![visitor(4, 960.0 + STAND_OFF_UNITS * 64.0)]);
    assert!(until(&mut sim, 6.0, |_, i| i.greet == Some(4)), "{}", sim.last_reason());
    assert_eq!(sim.nav.gos.len(), gos, "no trip needed");
}
