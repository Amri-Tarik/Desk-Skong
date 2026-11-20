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

#[test]
fn dream_nails_a_visitor_only_after_the_talk_and_at_most_once() {
    let (mut some, mut none) = (0, 0);
    for seed in 0..40 {
        let mut sim = calm(BrainConfig::default());
        sim.brain = Brain::new(BrainConfig { personality: Some(Personality::default()), ..BrainConfig::default() }, sim.catalog.clone(), seed);
        sim.run(1.0);
        let mut v = visitor(7, 960.0 + STAND_OFF_UNITS * 64.0);
        sim.brain.see_visitors(vec![v.clone()]);
        assert!(until(&mut sim, 6.0, |_, i| i.greet == Some(7)), "seed {seed}: {}", sim.last_reason());
        // Talking: only conversation reactions.
        v.state = VisitorState::Talking;
        for line in 0..3 {
            v.line = Some(line);
            sim.brain.see_visitors(vec![v.clone()]);
            sim.run(3.0);
            assert_eq!(sim.brain.behaviour(), BehaviourKind::Greet, "seed {seed}: conversation held");
        }
        assert!(!sim.started.iter().any(|a| a == "dream_nail"), "seed {seed}: no dream nail mid-conversation");
        assert!(sim.intents.iter().all(|i| i.dream_visitor.is_none()));
        // Over: lingering.
        v.state = VisitorState::Waiting;
        v.line = None;
        v.greeted = true;
        sim.brain.see_visitors(vec![v.clone()]);
        let n = sim.started.len();
        sim.run(8.0);
        let dn = sim.started_since(n).iter().filter(|a| *a == "dream_nail").count();
        let aimed = sim.intents.iter().filter(|i| i.dream_visitor == Some(7)).count();
        assert!(dn <= 1 && aimed <= 1, "seed {seed}: at most once per visit ({dn}, {aimed})");
        assert_eq!(dn, aimed, "seed {seed}: the dream nail is aimed at the visitor");
        if dn == 1 { some += 1 } else { none += 1 }
    }
    assert!(some > 0 && none > 0, "sometimes, not always: {some} of 40");
}

/// A small window (top at y 450) above a wider one (top at y 700); the visitor stands on
/// the small one at x 500.
fn perched_visitor(sim: &mut Sim) -> Visitor {
    sim.open(window(1, RectI::new(200, 700, 900, 1000), "editor.exe"));
    sim.open(window(2, RectI::new(300, 450, 700, 600), "notes.exe"));
    sim.run(1.0);
    Visitor { pos: Vec2::new(500.0, 450.0), ..visitor(6, 500.0) }
}

#[test]
fn a_visitor_he_cannot_get_beside_is_talked_to_from_a_ledge_below() {
    // Live: "gave up on visitor 2 (no route or no answer)" — Elderbug in a busy window,
    // right after the knight got fairly close. A talk needs no contact.
    let mut sim = calm(BrainConfig::default());
    let v = perched_visitor(&mut sim);
    let off = STAND_OFF_UNITS * 64.0;
    for x in [v.pos.x - off, v.pos.x + off] {
        // No way up onto the small window: routes there end on the floor.
        sim.nav.ends.push((Vec2::new(x, 450.0), None, Vec2::new(x, FLOOR)));
    }
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());
    assert!(until(&mut sim, 8.0, |_, i| i.greet == Some(6)), "talks from close by: {} at {:?}", sim.last_reason(), sim.knight.pos);
    assert_eq!(sim.knight.pos.y, 700.0, "from the wider window just below");
    assert!((sim.knight.pos.x - v.pos.x).abs() <= crate::ai::greet::TALK_DX_UNITS * 64.0);
    assert!(sim.nav.gos.iter().all(|(g, _)| !matches!(g, NavGoal::Point(p) if p.y == 450.0)), "never set off for the unreachable spot: {:?}", sim.nav.gos);
    assert_eq!(sim.intents.last().and_then(|i| i.face), Some(Side::Left), "faces the visitor");
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Greet);
}

#[test]
fn a_visitor_he_can_get_beside_is_still_approached_normally() {
    let mut sim = calm(BrainConfig::default());
    let v = perched_visitor(&mut sim);
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());
    sim.step();
    let (goal, _) = *sim.nav.gos.last().expect("travels");
    let NavGoal::Point(p) = goal else { panic!("point goal") };
    assert_eq!(p.y, 450.0, "right beside them on their window: {p}");
    assert!(until(&mut sim, 8.0, |_, i| i.greet == Some(6)), "{}", sim.last_reason());
    assert_eq!(sim.knight.pos.y, 450.0);
}

#[test]
fn a_visitor_out_of_talking_range_everywhere_is_given_up_on() {
    // (No Dream Gate here: that would take him up to them.)
    let mut cfg = BrainConfig::default();
    cfg.dream_gate.enabled = false;
    let mut sim = calm(cfg);
    sim.open(window(2, RectI::new(300, 300, 700, 500), "notes.exe"));
    sim.run(1.0);
    let v = Visitor { pos: Vec2::new(500.0, 300.0), ..visitor(7, 500.0) };
    let off = STAND_OFF_UNITS * 64.0;
    for x in [v.pos.x - off, v.pos.x + off] {
        sim.nav.ends.push((Vec2::new(x, 300.0), None, Vec2::new(x, FLOOR)));
    }
    sim.brain.see_visitors(vec![v]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());
    // The floor is 740 px below: too far to talk. He tries the walk, it fails, he lets it be.
    assert!(sim.run_while(15.0, |s| s.brain.behaviour() == BehaviourKind::Greet), "{}", sim.last_reason());
    assert!(!sim.intents.iter().any(|i| i.greet == Some(7)), "no talk from out of range");
}
