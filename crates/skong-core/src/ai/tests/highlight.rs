//! Highlighted text (opt-in): a settled selection → "what's that?" → he goes to a vantage
//! point near it and looks at it, once. Never above Geo or visitors; nothing when off.
use super::*;
use crate::ai::highlight::COOLDOWN_SECS;

/// Text selected in the window's body (the window's top at y = 400 is the vantage point).
const SELECTED: RectI = RectI::new(400, 500, 600, 520);

fn selecting() -> Sim {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.selection = Some(SELECTED);
    sim
}

#[test]
fn goes_over_and_looks_at_highlighted_text_once() {
    let mut sim = selecting();
    let n = sim.started.len();
    assert!(sim.run_until(2.0, BehaviourKind::Highlight), "noticed: {}", sim.last_reason());
    assert!(sim.last_reason().starts_with("highlight: what's that?"), "{}", sim.last_reason());
    assert!(sim.run_while(15.0, |s| s.brain.behaviour() == BehaviourKind::Highlight), "looked and moved on: {} {:?}", sim.last_reason(), sim.started_since(n));
    assert_eq!(sim.knight.pos.y, 400.0, "on the window's top edge, above the text");
    assert!((sim.knight.pos.x - 500.0).abs() <= 30.0, "{:?}", sim.knight.pos);
    assert!(sim.started_since(n).iter().any(|a| a == "look_down"), "looked down at it: {:?}", sim.started_since(n));
    // The same selection is not news any more.
    let kinds = sim.run(15.0);
    assert!(!kinds.contains(&BehaviourKind::Highlight), "looked at it twice");
}

#[test]
fn nothing_while_the_feature_is_off() {
    // Off: the app sends no selection at all.
    let mut sim = with_window(calm(BrainConfig::default()));
    let kinds = sim.run(20.0);
    assert!(!kinds.contains(&BehaviourKind::Highlight));
}

#[test]
fn switching_it_off_on_the_way_lets_it_go() {
    let mut sim = selecting();
    sim.nav.trip = 6.0;
    assert!(sim.run_until(2.0, BehaviourKind::Highlight));
    sim.run(1.0);
    sim.selection = None; // toggled off, or the selection was cleared
    assert!(sim.run_while(2.0, |s| s.brain.behaviour() == BehaviourKind::Highlight), "{}", sim.last_reason());
    assert!(!sim.started.iter().any(|a| a == "look_down"));
}

#[test]
fn waits_until_the_user_stops_dragging() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    // The selection grows every frame for 3 s (one long drag, the same selection).
    let n = (3.0 / DT) as i32;
    for i in 0..n {
        sim.selection = Some(RectI::new(400, 500, 410 + i * 2, 520));
        sim.step();
        assert_ne!(sim.brain.behaviour(), BehaviourKind::Highlight, "mid-drag");
    }
    assert!(sim.run_until(2.0, BehaviourKind::Highlight), "settled: {}", sim.last_reason());
}

#[test]
fn leaves_new_selections_alone_for_a_while_after_a_look() {
    let mut sim = selecting();
    assert!(sim.run_until(2.0, BehaviourKind::Highlight));
    assert!(sim.run_while(15.0, |s| s.brain.behaviour() == BehaviourKind::Highlight));
    sim.selection = Some(RectI::new(300, 700, 500, 720));
    let kinds = sim.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::Highlight), "cooling down: {}", sim.last_reason());
    sim.selection = None;
    sim.run(COOLDOWN_SECS);
    sim.selection = Some(RectI::new(300, 800, 500, 820));
    assert!(sim.run_until(3.0, BehaviourKind::Highlight), "after the cooldown: {}", sim.last_reason());
}

#[test]
fn a_selection_made_during_the_cooldown_is_old_news_by_its_end() {
    let mut sim = selecting();
    assert!(sim.run_until(2.0, BehaviourKind::Highlight));
    assert!(sim.run_while(15.0, |s| s.brain.behaviour() == BehaviourKind::Highlight));
    // Selected right after his look and left there: stale by the time he'd care again.
    sim.selection = Some(RectI::new(300, 700, 500, 720));
    let kinds = sim.run(COOLDOWN_SECS + 5.0);
    assert!(!kinds.contains(&BehaviourKind::Highlight), "{}", sim.last_reason());
}

#[test]
fn beats_keeping_company() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.park_cursor(Vec2::new(150.0, -120.0));
    sim.run(3.0);
    sim.selection = Some(SELECTED);
    assert!(sim.run_until(3.0, BehaviourKind::Highlight), "{}", sim.last_reason());
}

#[test]
fn geo_comes_first() {
    let mut sim = selecting();
    sim.nav.trip = 6.0;
    assert!(sim.run_until(2.0, BehaviourKind::Highlight));
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(1500.0, FLOOR)], ..Default::default() });
    assert!(sim.run_until(1.0, BehaviourKind::Loot), "{}", sim.last_reason());
    // With Geo around a new selection doesn't pull him away either.
    let kinds = sim.run(3.0);
    assert!(!kinds.contains(&BehaviourKind::Highlight), "{}", sim.last_reason());
    sim.brain.sense_stakes(StakesSense::default());
    assert!(sim.run_until(3.0, BehaviourKind::Highlight), "back to the text: {}", sim.last_reason());
}

#[test]
fn a_visitor_comes_first() {
    let mut sim = selecting();
    sim.nav.trip = 6.0;
    assert!(sim.run_until(2.0, BehaviourKind::Highlight));
    let v = Visitor { id: 5, name: "cornifer".into(), pos: Vec2::new(1500.0, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() };
    sim.brain.see_visitors(vec![v]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());
    assert!(!sim.started.iter().any(|a| a == "look_down"));
}

#[test]
fn not_while_asleep() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_while(20.0, |s| s.act.as_ref().is_none_or(|a| s.act_name(a.act.id) != "nap")), "asleep: {}", sim.last_reason());
    sim.selection = Some(RectI::new(1200, 900, 1400, 920));
    let kinds = sim.run(20.0);
    assert!(!kinds.contains(&BehaviourKind::Highlight), "highlighted text never wakes him");
}
