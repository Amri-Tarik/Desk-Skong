//! Reachable things on screen are not hidden by unreachable ones: candidates (found places,
//! shinies, lumaflies) are route-checked in want order until a reachable one turns up,
//! within a per-rescore plan budget, and only a failure at a thing itself keeps it from
//! being tried (not one at any goal in the same 64 px cell).
use super::*;
use crate::ai::interest::CANDIDATE_PLANS;

/// Calm, with no Dream Gate (it would warp to the unreachable ones and muddle the count).
fn no_gate() -> Sim {
    let mut cfg = BrainConfig::default();
    cfg.dream_gate.enabled = false;
    calm(cfg)
}

fn verdict(sim: &Sim, what: Subject) -> Option<Why> {
    sim.brain.passed_over().iter().find(|c| c.what == what).map(|c| c.why)
}

fn plans_to(sim: &Sim, at: Vec2) -> usize {
    sim.nav.planned.iter().filter(|p| p.distance(at) < 1.0).count()
}

#[test]
fn two_unreachable_jars_dont_hide_a_deposit_he_can_walk_to() {
    // Live: Visit scored 0 while he idled beside a reachable deposit: only the two most
    // wanted places got a route check, and both (grubs up where no route goes) failed it.
    let mut sim = no_gate();
    let jars = [Vec2::new(400.0, FLOOR), Vec2::new(500.0, FLOOR)];
    sim.nav.refuse.extend(jars);
    let rock = KnownPlace { center: Vec2::new(1550.0, FLOOR), ..place(3, PlaceKind::Deposit, Vec2::new(1500.0, FLOOR), 0.0, false) };
    sim.brain.set_places(&[place(1, PlaceKind::Grub, jars[0], 0.0, false), place(2, PlaceKind::Grub, jars[1], 0.0, false), rock]);
    assert!(sim.run_until(3.0, BehaviourKind::Visit), "{}", sim.last_reason());
    assert_eq!(sim.brain.visiting().map(|p| p.id), Some(3), "the deposit he can get to");
    for id in [1, 2] {
        let jar = Subject::Place { kind: PlaceKind::Grub, id };
        assert!(matches!(verdict(&sim, jar), Some(Why::NoRoute(_))), "{:?}", verdict(&sim, jar));
    }
}

#[test]
fn unreachable_places_are_planned_once_in_a_while_within_the_budget() {
    // Eight jars no route reaches: each rescore plans at most the budget, and one found
    // unreachable is left alone a few seconds, so the next rescores try the others instead
    // of re-planning the same favourites 4 times a second.
    let mut sim = no_gate();
    let jars: Vec<Vec2> = (0..8).map(|i| Vec2::new(150.0 + 100.0 * i as f32, FLOOR)).collect();
    sim.nav.refuse.extend(jars.iter().copied());
    let places: Vec<KnownPlace> = jars.iter().enumerate().map(|(i, &at)| place(i as PlaceId + 1, PlaceKind::Grub, at, 0.1 * i as f32, false)).collect();
    sim.brain.set_places(&places);
    sim.step();
    let first: usize = jars.iter().map(|&j| plans_to(&sim, j)).sum();
    assert_eq!(first, CANDIDATE_PLANS, "one rescore spends the budget");
    sim.run(2.0);
    for (i, &j) in jars.iter().enumerate() {
        assert_eq!(plans_to(&sim, j), 1, "jar {} planned once in 2 s: {:?}", i + 1, jars.iter().map(|&j| plans_to(&sim, j)).collect::<Vec<_>>());
    }
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Visit);
    // A while later they are looked at again (a window may have moved).
    sim.run(4.0);
    assert!(jars.iter().all(|&j| plans_to(&sim, j) >= 2), "{:?}", jars.iter().map(|&j| plans_to(&sim, j)).collect::<Vec<_>>());
}

#[test]
fn a_failure_next_to_a_deposit_doesnt_hide_it_but_one_at_it_does() {
    // Live: a seat or some goal that failed in the same 64 px cell blocked the deposit
    // beside it for a minute.
    let at = Vec2::new(1200.0, FLOOR);
    let rock = KnownPlace { center: Vec2::new(1250.0, FLOOR), ..place(4, PlaceKind::Deposit, at, 0.0, false) };
    let mut sim = no_gate();
    sim.run(0.5);
    let near = at + Vec2::new(15.0, 0.0);
    assert_eq!(PoiTarget::Spot(near).key(), PoiTarget::Spot(at).key(), "same cell");
    sim.brain.interest.block(PoiTarget::Spot(near), sim.brain.now + 60.0);
    sim.brain.set_places(&[rock]);
    assert!(sim.run_until(3.0, BehaviourKind::Visit), "{}", sim.last_reason());

    // He failed to get to the rock itself: that is remembered.
    let mut sim = no_gate();
    sim.run(0.5);
    sim.brain.interest.block(PoiTarget::Spot(at), sim.brain.now + 60.0);
    sim.brain.set_places(&[rock]);
    let kinds = sim.run(3.0);
    assert!(!kinds.contains(&BehaviourKind::Visit), "{}", sim.last_reason());
    assert_eq!(verdict(&sim, Subject::Place { kind: PlaceKind::Deposit, id: 4 }), Some(Why::Blocked));
}

fn shiny(id: u64, x: f32) -> KnownShiny {
    KnownShiny { id, pos: Vec2::new(x, FLOOR), span: (0.0, 1920.0), reach: 20.0 }
}

#[test]
fn a_failure_next_to_a_shiny_doesnt_hide_it_but_one_at_its_spot_does() {
    // He comes from the left: he'd pick it up standing at 1280.
    let stand = Vec2::new(1280.0, FLOOR);
    let near = Vec2::new(1295.0, FLOOR);
    assert_eq!(PoiTarget::Spot(near).key(), PoiTarget::Spot(stand).key(), "same cell");
    let mut sim = no_gate();
    sim.run(0.5);
    sim.brain.interest.block(PoiTarget::Spot(near), sim.brain.now + 60.0);
    sim.shinies.push(shiny(1, 1300.0));
    assert!(sim.run_until(2.0, BehaviourKind::Shiny), "{}", sim.last_reason());

    let mut sim = no_gate();
    sim.run(0.5);
    sim.brain.interest.block(PoiTarget::Spot(stand), sim.brain.now + 60.0);
    sim.shinies.push(shiny(1, 1300.0));
    let kinds = sim.run(2.0);
    assert!(!kinds.contains(&BehaviourKind::Shiny), "{}", sim.last_reason());
    assert_eq!(verdict(&sim, Subject::Shiny(1)), Some(Why::Blocked));
}

#[test]
fn unreachable_near_shinies_dont_hide_a_reachable_one() {
    let mut sim = no_gate();
    sim.run(0.5);
    // Two near ones (from the left, picked up at x − 20) that no route reaches.
    sim.nav.refuse.extend([Vec2::new(1080.0, FLOOR), Vec2::new(1180.0, FLOOR)]);
    sim.shinies.extend([shiny(1, 1100.0), shiny(2, 1200.0), shiny(3, 1600.0)]);
    assert!(sim.run_until(2.0, BehaviourKind::Shiny), "{}", sim.last_reason());
    assert_eq!(sim.brain.shiny_target(), Some(3));
}

#[test]
fn an_unreachable_near_lumafly_doesnt_hide_a_reachable_one() {
    // Only the nearest untried fly was route-checked: one he couldn't get under kept every
    // other fly out of mind until he gave up on it (which needs setting out for it).
    let mut sim = no_gate();
    sim.brain.mind_mut().needs.boredom = 0.1;
    sim.park_cursor(Vec2::new(150.0, -120.0));
    assert!(sim.run_until(3.0, BehaviourKind::Companion), "{}", sim.last_reason());
    let near = KnownLumafly { id: 3, pos: Vec2::new(1100.0, FLOOR - 120.0), leaving: false };
    let far = KnownLumafly { id: 4, pos: Vec2::new(1500.0, FLOOR - 120.0), leaving: false };
    let under = crate::ai::spots::project(&sim.terrain, near.pos).expect("the floor under it");
    sim.nav.refuse.push(under);
    sim.flies = vec![near, far];
    assert!(sim.run_until(3.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("lumafly"), "{}", sim.last_reason());
    assert_eq!(sim.brain.pastime_target(), Some(far.id), "goes for the far fly");
}
