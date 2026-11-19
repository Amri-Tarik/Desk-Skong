//! Train: a teaching visitor (Sly) talks, then runs a lesson. The knight watches, practises
//! each drill through attacks / acts / direct inputs, answers the feedback, bows out with
//! the closing line, and goes back to normal life. Typing ends the lesson early.
use super::*;

const SLY: u32 = 11;

fn sly() -> Visitor {
    Visitor {
        id: SLY,
        name: "sly".into(),
        pos: Vec2::new(960.0 + 2.6 * 64.0, FLOOR),
        state: VisitorState::Waiting,
        greet_react: Some("look_up".into()),
        excitement: 0.95,
        stand_off: 2.6 * 64.0,
        ..Default::default()
    }
}

fn cue(serial: u32, drill: &str, stage: LessonStage) -> Option<LessonCue> {
    Some(LessonCue { serial, drill: drill.into(), stage })
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

/// Greet Sly and have the (scripted) conversation; returns him ready to teach.
fn talked(sim: &mut Sim) -> Visitor {
    let mut v = sly();
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(until(sim, 8.0, |_, i| i.greet == Some(SLY)), "{}", sim.last_reason());
    v.state = VisitorState::Talking;
    for line in 0..3 {
        v.line = Some(line);
        sim.brain.see_visitors(vec![v.clone()]);
        sim.run(2.5);
        assert_eq!(sim.brain.behaviour(), BehaviourKind::Greet);
    }
    v.line = None;
    v.greeted = true;
    v.state = VisitorState::Teaching;
    v.lesson = cue(1, "slash", LessonStage::Instruct);
    sim.brain.see_visitors(vec![v.clone()]);
    v
}

/// The knight's turn at drill `serial`: step until he reports it. Returns `clean`.
fn practise(sim: &mut Sim, v: &mut Visitor, serial: u32, drill: &str) -> bool {
    v.lesson = cue(serial, drill, LessonStage::Practice);
    v.react = None;
    sim.brain.see_visitors(vec![v.clone()]);
    let mut report = None;
    assert!(
        until(sim, 15.0, |_, i| {
            report = i.lesson;
            report.is_some()
        }),
        "drill {serial} {drill}: {}",
        sim.last_reason()
    );
    let r = report.unwrap();
    assert_eq!(r.visitor, SLY);
    match r.signal {
        LessonSignal::Drilled { serial: s, clean } => {
            assert_eq!(s, serial);
            clean
        }
        LessonSignal::Stop => panic!("drill {serial}: stopped"),
    }
}

/// Feedback for drill `serial` with the knight's answer.
fn feedback(sim: &mut Sim, v: &mut Visitor, serial: u32, drill: &str, react: &str) {
    v.lesson = cue(serial, drill, LessonStage::Feedback);
    v.react = Some(react.into());
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(3.5);
    v.react = None;
}

#[test]
fn sly_talks_then_trains_the_knight_then_leaves() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let mut v = talked(&mut sim);
    assert!(sim.run_until(1.0, BehaviourKind::Train), "{}", sim.last_reason());
    // Watching the instruction: faces the teacher, stays put.
    sim.run(1.5);
    assert!(sim.last_reason().starts_with("train with sly: watching"), "{}", sim.last_reason());
    assert_eq!(sim.intents.last().and_then(|i| i.face), Some(Side::Right), "faces Sly");
    assert!(sim.intents.iter().rev().take(30).all(|i| i.attack.is_none()), "no drilling during the instruction");

    // Drill 1: three forward slashes into the air, away from Sly, never a hit.
    let n = sim.intents.len();
    assert!(practise(&mut sim, &mut v, 1, "slash"), "clean");
    let slashes: Vec<Attack> = sim.intents[n..].iter().filter_map(|i| i.attack).collect();
    assert_eq!(slashes.len(), 3, "{slashes:?}");
    assert!(slashes.iter().all(|a| a.dir == AttackDir::Forward && a.side == Side::Left && !a.hit), "{slashes:?}");
    let a = sim.started.len();
    feedback(&mut sim, &mut v, 1, "slash", "look_down");
    assert_eq!(sim.started_since(a), ["look_down"], "answers the correction");

    // Drill 2: up slashes.
    let n = sim.intents.len();
    assert!(practise(&mut sim, &mut v, 2, "up_slash"));
    assert_eq!(sim.intents[n..].iter().filter(|i| i.attack.is_some_and(|a| a.dir == AttackDir::Up)).count(), 2);
    feedback(&mut sim, &mut v, 2, "up_slash", "look_up");

    // Drill 3 & 4: nail arts by their acts.
    let a = sim.started.len();
    assert!(practise(&mut sim, &mut v, 3, "great_slash"));
    assert_eq!(sim.started_since(a), ["great_slash"]);
    feedback(&mut sim, &mut v, 3, "great_slash", "look_up");
    let a = sim.started.len();
    assert!(practise(&mut sim, &mut v, 4, "cyclone_slash"));
    assert_eq!(sim.started_since(a).first().map(String::as_str), Some("cyclone_slash"));
    feedback(&mut sim, &mut v, 4, "cyclone_slash", "look_down");

    // Drill 5 & 6: moves through direct inputs (the stand-in physics never leaves the
    // ground, so they don't come off — but they are tried, and reported).
    let n = sim.intents.len();
    let clean = practise(&mut sim, &mut v, 5, "dash");
    let dashes = sim.intents[n..].iter().filter(|i| matches!(i.motion, Motion::Direct(m) if m.dash_pressed)).count();
    assert!(dashes >= 1 && !clean, "{dashes} dashes");
    feedback(&mut sim, &mut v, 5, "dash", "look_down");
    let n = sim.intents.len();
    let clean = practise(&mut sim, &mut v, 6, "down_slash");
    let jumps = sim.intents[n..].iter().filter(|i| matches!(i.motion, Motion::Direct(m) if m.jump_pressed)).count();
    assert!(jumps >= 1 && !clean, "{jumps} jumps");
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Train);

    // Closing line: the knight salutes.
    let a = sim.started.len();
    v.lesson = cue(6, "down_slash", LessonStage::Closing);
    v.react = Some("challenge".into());
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(2.0);
    assert_eq!(sim.started_since(a), ["challenge"]);

    // He bows and leaves; the knight goes back to normal life.
    v.state = VisitorState::Waiting;
    v.lesson = None;
    v.react = None;
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_while(8.0, |s| s.brain.behaviour() == BehaviourKind::Train), "lesson over");
    v.state = VisitorState::Leaving;
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(1.0);
    sim.brain.see_visitors(vec![]);
    let kinds = sim.run(10.0);
    assert!(!kinds.iter().any(|k| matches!(k, BehaviourKind::Train | BehaviourKind::Greet)), "{kinds:?}");
    assert!(sim.intents.iter().all(|i| i.lesson.is_none_or(|l| l.signal != LessonSignal::Stop)), "never stopped");
    assert!(sim.brain.since_fight().is_infinite(), "drills are not fights");
}

#[test]
fn typing_ends_the_lesson_politely() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let mut v = talked(&mut sim);
    assert!(sim.run_until(1.0, BehaviourKind::Train), "{}", sim.last_reason());
    v.lesson = cue(1, "slash", LessonStage::Practice);
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(0.2);
    // The user starts typing: tell Sly, and stop.
    sim.user.keys_per_sec = 4.0;
    let mut stop = None;
    assert!(until(&mut sim, 1.0, |_, i| {
        stop = i.lesson;
        stop.is_some()
    }));
    assert_eq!(stop, Some(LessonReport { visitor: SLY, signal: LessonSignal::Stop }));
    sim.run(0.5);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Train);
    // While the user types, the (still teaching) visitor isn't picked up again.
    assert!(!sim.run(3.0).contains(&BehaviourKind::Train));
    assert_eq!(sim.intents.iter().filter(|i| i.lesson.is_some()).count(), 1, "told once");
}

#[test]
fn a_teacher_far_away_is_walked_back_to_between_drills() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let mut v = talked(&mut sim);
    assert!(sim.run_until(1.0, BehaviourKind::Train));
    // A dash drill carried him off along the ledge.
    sim.place(Vec2::new(500.0, FLOOR));
    v.lesson = cue(1, "slash", LessonStage::Feedback);
    sim.brain.see_visitors(vec![v.clone()]);
    let gos = sim.nav.gos.len();
    sim.run(4.0);
    assert!(sim.nav.gos.len() > gos, "walks back: {}", sim.last_reason());
    let (goal, _) = *sim.nav.gos.last().unwrap();
    let NavGoal::Point(p) = goal else { panic!("point") };
    assert!((v.pos.x - p.x).abs() <= 2.7 * 64.0, "to Sly's side: {p}");
}
