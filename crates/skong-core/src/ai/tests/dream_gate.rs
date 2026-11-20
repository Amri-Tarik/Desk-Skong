//! Dream Gate fallback travel: a prize no route reaches → he plants a gate by it, warps
//! there and carries on. Never for reachable prizes, at most once per cooldown, only onto
//! settled ledges, and a gate he is called away from fades.
use super::*;
use crate::dreamgate::GateEvent;

/// Routes to `at` end on the floor below it (out of reach for every style).
fn unreachable(sim: &mut Sim, at: Vec2) {
    sim.nav.ends.push((at, None, Vec2::new(at.x, FLOOR)));
}

fn coins(sim: &mut Sim, coins: &[Vec2]) {
    sim.brain.sense_stakes(StakesSense { coins: coins.to_vec(), ..Default::default() });
}

fn gates(sim: &Sim) -> Vec<GateEvent> {
    sim.intents.iter().filter_map(|i| i.gate).collect()
}

fn the_order(started: &[String], names: &[&str]) -> bool {
    let mut it = started.iter();
    names.iter().all(|n| it.any(|s| s == n))
}

#[test]
fn an_unreachable_coin_is_worth_a_dream_gate() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    // Geo on the title bar (y = 400), and no way up there.
    let coin = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, coin);
    coins(&mut sim, &[coin]);
    assert!(sim.run_until(6.0, BehaviourKind::DreamGate), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("no route to the geo"), "{}", sim.last_reason());
    assert!(sim.run_while(8.0, |s| s.knight.pos != coin), "warped onto the coin: {} at {:?}", sim.last_reason(), sim.knight.pos);
    assert_eq!(gates(&sim), vec![GateEvent::Set { at: coin }], "one gate, by the coin");
    assert!(the_order(&sim.started, &["dreamgate_set", "dreamgate_warp", "dreamgate_in"]), "acts {:?}", sim.started);
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::DreamGate), "{}", sim.last_reason());
    assert_eq!(sim.brain.dream_gates(), 1);
    assert!(!sim.intents.iter().flat_map(|i| &i.stakes).any(|e| matches!(e, StakeEvent::AbandonCoin { .. })), "the coin was kept for the gate");
}

#[test]
fn reachable_prizes_are_walked_to() {
    // A coin on the floor, and one on the title bar he can climb to.
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    coins(&mut sim, &[Vec2::new(1500.0, FLOOR), Vec2::new(500.0, 400.0)]);
    let kinds = sim.run(15.0);
    assert!(kinds.contains(&BehaviourKind::Loot), "{}", sim.last_reason());
    assert!(!kinds.contains(&BehaviourKind::DreamGate), "{}", sim.last_reason());
    assert!(gates(&sim).is_empty());

    // A deposit on the title bar, within reach.
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.brain.set_places(&[KnownPlace { center: Vec2::new(650.0, 400.0), ..place(4, PlaceKind::Deposit, Vec2::new(600.0, 400.0), 0.0, false) }]);
    let kinds = sim.run(10.0);
    assert!(kinds.contains(&BehaviourKind::Visit), "{}", sim.last_reason());
    assert!(!kinds.contains(&BehaviourKind::DreamGate) && gates(&sim).is_empty(), "{}", sim.last_reason());

    // Something reachable comes first: the floor coin before a warp to the unreachable one.
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    let high = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, high);
    coins(&mut sim, &[Vec2::new(1100.0, FLOOR), high]);
    assert!(sim.run_until(2.0, BehaviourKind::Loot), "{}", sim.last_reason());
    assert!(gates(&sim).is_empty(), "walks to the floor coin first");
}

#[test]
fn an_unreachable_deposit_is_mined_after_a_dream_gate() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    let at = Vec2::new(600.0, 400.0);
    unreachable(&mut sim, at);
    sim.brain.set_places(&[KnownPlace { center: Vec2::new(650.0, 400.0), ..place(4, PlaceKind::Deposit, at, 0.0, false) }]);
    assert!(sim.run_until(3.0, BehaviourKind::DreamGate), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("geo deposit"), "{}", sim.last_reason());
    assert!(sim.run_until(10.0, BehaviourKind::Visit), "{}", sim.last_reason());
    assert_eq!(sim.knight.pos, at, "beside the rock");
    assert_eq!(gates(&sim), vec![GateEvent::Set { at }]);
    assert!(sim.run_while(5.0, |s| !s.intents.last().is_some_and(|i| i.discovery == Some(DiscoveryEvent::Struck(4)))), "mines it: {}", sim.last_reason());
}

#[test]
fn an_unreachable_shiny_is_picked_up_after_a_dream_gate() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    // On the title bar; he'd pick it up from its right (he comes from x = 960).
    let stand = Vec2::new(520.0, 400.0);
    unreachable(&mut sim, stand);
    sim.shinies.push(KnownShiny { id: 1, pos: Vec2::new(500.0, 400.0), span: (200.0, 900.0), reach: 20.0 });
    assert!(sim.run_until(3.0, BehaviourKind::DreamGate), "{}", sim.last_reason());
    assert!(sim.run_while(15.0, |s| s.picked.is_empty()), "picked up: {} / acts {:?}", sim.last_reason(), sim.started);
    assert_eq!(gates(&sim), vec![GateEvent::Set { at: stand }]);
}

#[test]
fn the_dream_gate_cooldown_holds() {
    let cfg = BrainConfig::default();
    let cooldown = cfg.dream_gate.cooldown;
    let mut sim = with_window(calm(cfg));
    sim.open(window(2, RectI::new(1000, 300, 1600, 800), "browser.exe"));
    sim.run(1.0);
    let first = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, first);
    coins(&mut sim, &[first]);
    assert!(sim.run_while(10.0, |s| s.knight.pos != first), "{}", sim.last_reason());
    let warped_at = sim.t;
    sim.run(3.0);

    // More Geo out of reach on the other window: not so soon.
    let second = Vec2::new(1300.0, 300.0);
    unreachable(&mut sim, second);
    coins(&mut sim, &[second]);
    let n = sim.intents.len();
    let kinds = sim.run(cooldown - 20.0);
    assert!(!kinds.contains(&BehaviourKind::DreamGate), "{}", sim.last_reason());
    assert!(sim.intents[n..].iter().all(|i| i.gate.is_none()));
    assert!(sim.intents[n..].iter().flat_map(|i| &i.stakes).any(|e| *e == StakeEvent::AbandonCoin { at: second }), "gives that coin up meanwhile");
    // Once the gate is ready again, it's worth another warp.
    assert!(sim.run_while(30.0, |s| s.knight.pos != second), "{}", sim.last_reason());
    assert!(sim.t - warped_at >= cooldown - 5.0, "{} s apart", sim.t - warped_at);
    sim.run(1.0);
    assert_eq!(sim.brain.dream_gates(), 2);
}

#[test]
fn no_gate_onto_a_ledge_that_has_not_settled() {
    let mut sim = calm(BrainConfig::default());
    sim.run(2.0);
    // A window that just opened (it may still be moving).
    sim.open(window(1, RectI::new(200, 400, 900, 900), "editor.exe"));
    // (Exploring the new window would walk him onto its top, right by the coin, which the
    // fake navigator lets him reach while calling the coin unreachable: keep him off it.)
    sim.brain.interest.block(crate::ai::PoiTarget::Window(1), sim.brain.now + 100.0);
    let coin = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, coin);
    coins(&mut sim, &[coin]);
    let kinds = sim.run(crate::lasting::SETTLED_SECS - 3.0);
    assert!(!kinds.contains(&BehaviourKind::DreamGate) && gates(&sim).is_empty(), "{}", sim.last_reason());
    assert!(sim.run_until(10.0, BehaviourKind::DreamGate), "once it has settled: {}", sim.last_reason());
}

#[test]
fn a_gate_he_is_called_away_from_fades() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    let coin = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, coin);
    coins(&mut sim, &[coin]);
    assert!(sim.run_while(6.0, |s| gates(s).is_empty()), "{}", sim.last_reason());
    // Shaken about while planting: he reels once set down, and the gate closes.
    sim.shake();
    assert!(sim.run_until(1.0, BehaviourKind::Dizzy), "{}", sim.last_reason());
    assert_eq!(gates(&sim), vec![GateEvent::Set { at: coin }, GateEvent::Close]);
    assert_eq!(sim.gate.at(), None);
    assert!(!sim.started.iter().any(|a| a == "dreamgate_warp"));
}

#[test]
fn disabled_means_no_dream_gates() {
    let mut cfg = BrainConfig::default();
    cfg.dream_gate.enabled = false;
    let mut sim = with_window(calm(cfg));
    sim.run(1.0);
    let coin = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, coin);
    coins(&mut sim, &[coin]);
    let kinds = sim.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::DreamGate) && gates(&sim).is_empty());
    assert!(sim.intents.iter().flat_map(|i| &i.stakes).any(|e| *e == StakeEvent::AbandonCoin { at: coin }), "the old give-up");
}


#[test]
fn a_shade_with_no_route_under_it_is_reached_by_dream_gate() {
    // Live: he respawned far from his Shade (often on the other monitor), found no route
    // and shelved it for 6–12 minutes. The Shade comes first: no route → a Dream Gate by
    // it (whatever the gate's cooldown), then the fight.
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.brain.stakes.gate.ready_at = sim.brain.now + 600.0; // a gate was just used for something else
    let under = Vec2::new(500.0, 400.0);
    unreachable(&mut sim, under);
    let shade = ShadeSense { pos: Vec2::new(500.0, 330.0), vulnerable: true, fighting: false, geo: 60 };
    sim.brain.sense_stakes(StakesSense { shade: Some(shade), ..Default::default() });
    assert!(sim.run_until(2.0, BehaviourKind::ShadeHunt), "{}", sim.last_reason());
    assert!(sim.run_while(8.0, |s| s.knight.pos != under), "warped under the shade: {} at {:?}", sim.last_reason(), sim.knight.pos);
    assert_eq!(gates(&sim), vec![GateEvent::Set { at: under }], "one gate, under the shade");
    assert!(the_order(&sim.started, &["dreamgate_set", "dreamgate_warp", "dreamgate_in"]), "acts {:?}", sim.started);
    assert!(sim.run_while(4.0, |s| !s.last_reason().contains("shade: fighting")), "then the fight: {}", sim.last_reason());
    assert_eq!(sim.brain.behaviour(), BehaviourKind::ShadeHunt, "one hunt throughout");
}
