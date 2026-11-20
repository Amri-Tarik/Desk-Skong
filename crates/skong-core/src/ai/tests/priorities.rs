//! Strict priorities: the Shade, then fights and NPC talks, then Geo (coins, then
//! deposits, then shinies), then pastimes. Here: Geo against Geo.
use super::*;

#[test]
fn a_walk_to_a_deposit_gives_way_to_loose_coins() {
    // Live: a fight ended with its coins still bouncing (no resting Geo yet), so a walk to
    // a Geo deposit started; a fresh deposit outscored the coins once they landed, and the
    // walk never gave way. Loose coins come before deposits (they fade).
    let mut sim = calm(BrainConfig::default());
    sim.nav.trip = 1e9; // a long walk
    sim.brain.set_places(&[KnownPlace { fresh: true, ..place(2, PlaceKind::Deposit, Vec2::new(200.0, FLOOR), 0.5, true) }]);
    assert!(sim.run_until(5.0, BehaviourKind::Visit), "{}", sim.last_reason());
    sim.run(1.5);
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Visit, "{}", sim.last_reason());
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(1500.0, FLOOR), Vec2::new(1540.0, FLOOR)], ..Default::default() });
    assert!(sim.run_until(0.5, BehaviourKind::Loot), "coins first: {}", sim.last_reason());
}

#[test]
fn a_deposit_being_mined_is_finished_before_the_coins() {
    // Mining spills coins: he finishes the rock (a few swings), then gathers.
    let mut sim = calm(BrainConfig::default());
    let rock = KnownPlace { center: Vec2::new(1000.0, FLOOR), ..place(4, PlaceKind::Deposit, sim.knight.pos, 0.5, false) };
    sim.brain.set_places(&[rock]);
    assert!(sim.run_while(5.0, |s| !s.last_reason().contains("mining")), "{}", sim.last_reason());
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(1500.0, FLOOR)], ..Default::default() });
    sim.run(0.5);
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Visit, "keeps mining: {}", sim.last_reason());
}
