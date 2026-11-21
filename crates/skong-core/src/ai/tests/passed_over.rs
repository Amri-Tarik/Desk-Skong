//! The passed-over list (diagnostics): what the knight could be doing instead, and why he
//! isn't — the scoring's verdicts on found places, and the reason a scored one lost.
use super::*;

fn describe(sim: &Sim) -> String {
    let mut s = String::new();
    sim.brain.describe_passed_over(&mut s);
    s
}

fn verdict(sim: &Sim, what: Subject) -> Option<Why> {
    sim.brain.passed_over().iter().find(|c| c.what == what).map(|c| c.why)
}

#[test]
fn an_unreachable_deposit_is_listed_with_its_reason_while_he_idles() {
    let mut sim = calm(BrainConfig::default());
    // A rock up on a ledge whose route only gets him to the floor below it.
    let at = Vec2::new(1200.0, 600.0);
    sim.unmountable(at, None);
    sim.brain.set_places(&[KnownPlace { center: at, ..place(12, PlaceKind::Deposit, at, 0.0, false) }]);
    sim.run(3.0);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Visit, "{}", sim.last_reason());
    let rock = Subject::Place { kind: PlaceKind::Deposit, id: 12 };
    assert_eq!(verdict(&sim, rock), Some(Why::Misses), "{}", describe(&sim));
    assert!(describe(&sim).contains("deposit #12: the route ends off it"), "{}", describe(&sim));
    let (n, _) = sim.brain.idling_past().expect("idling with an activity on screen");
    assert_eq!(n, 1);
}

#[test]
fn a_mined_deposit_is_listed_as_cooling_and_a_lost_one_says_why() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.mischief = 0.8;
    let rock = KnownPlace { center: Vec2::new(1250.0, FLOOR), ..place(4, PlaceKind::Deposit, Vec2::new(1200.0, FLOOR), 0.0, false) };
    sim.brain.set_places(&[rock]);
    assert!(sim.run_until(3.0, BehaviourKind::Visit), "{}", sim.last_reason());
    // The rock never breaks: six swings, then the visit is over and the rock cools down.
    assert!(sim.run_while(12.0, |s| s.brain.behaviour() == BehaviourKind::Visit), "{}", sim.last_reason());
    sim.run(0.5);
    let what = Subject::Place { kind: PlaceKind::Deposit, id: 4 };
    assert!(matches!(verdict(&sim, what), Some(Why::Cooling { .. })), "{}", describe(&sim));
    assert!(describe(&sim).contains("deposit #4: cooling"), "{}", describe(&sim));
    // A cooling place isn't an activity he's expected to take up.
    assert!(sim.brain.idling_past().is_none(), "{}", describe(&sim));

    // A bench when he isn't tired: scored and route-planned, but it loses to idling.
    let mut sim = calm(BrainConfig::default());
    sim.brain.set_places(&[place(7, PlaceKind::Bench, Vec2::new(1300.0, FLOOR), 0.0, false)]);
    sim.run(3.0);
    let bench = Subject::Place { kind: PlaceKind::Bench, id: 7 };
    match verdict(&sim, bench) {
        Some(Why::Lost { tier: Tier::Ambient, to, .. }) => assert!(!matches!(to, Lose::Tier { .. }), "{to:?}"),
        other => panic!("{other:?}: {}", describe(&sim)),
    }
    assert!(describe(&sim).contains("bench #7: 0."), "{}", describe(&sim));
}
