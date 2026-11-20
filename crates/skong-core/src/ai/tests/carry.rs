//! Carried items (`ai::carry`): the map and the lantern only change his look. With one in
//! his hands he keeps doing his usual thing (following the cursor); anything needing his
//! hands puts it away; there's no patrol act any more.
use super::*;

/// Keeping the user company, cursor resting beside him.
fn in_company() -> Sim {
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(60.0, -20.0));
    assert!(sim.run_until(3.0, BehaviourKind::Companion), "{}", sim.last_reason());
    sim
}

/// The map comes out (as when he or a behaviour asks for `take_out_map`).
fn take_out_map(sim: &mut Sim) {
    let id = sim.catalog.id("take_out_map").expect("act");
    sim.started.push("take_out_map".into());
    sim.act = Some(PlayingAct { act: RunningAct { id, interruptible: false }, left: 0.5 });
    sim.run(1.0);
    assert_eq!(sim.intents.last().and_then(|i| i.carry), Some(Carry::Map), "{}", sim.last_reason());
}

fn carry(sim: &Sim) -> Option<Carry> {
    sim.intents.last().and_then(|i| i.carry)
}

#[test]
fn with_the_map_out_he_still_follows_the_cursor() {
    let mut sim = in_company();
    take_out_map(&mut sim);
    // The cursor drifts off along the floor: he strolls after it, map in hand.
    let to = sim.knight.pos + Vec2::new(600.0, -40.0);
    sim.cursor = Some(Cursor { pos: to, ..Default::default() });
    let n = sim.intents.len();
    let ok = sim.run_while(3.0, |s| !matches!(s.last_motion, Motion::Navigate { .. }));
    assert!(ok, "follows: {}", sim.last_reason());
    let i = sim.intents.last().expect("intent");
    assert_eq!(i.behaviour, Some(BehaviourKind::Companion));
    assert!(matches!(i.motion, Motion::Navigate { style: TravelStyle::Stroll, goal: NavGoal::Point(g) } if g.x > sim.knight.pos.x), "{:?}", i.motion);
    assert_eq!(i.carry, Some(Carry::Map), "map still out while walking: {}", i.reason);
    assert!(i.reason.contains("following cursor") && i.reason.contains("map out"), "{}", i.reason);
    // No patrol, no put-away on the way.
    assert!(!sim.started_since(0).iter().any(|a| a == "put_map_away"), "{:?}", sim.started);
    sim.run(3.0);
    assert!(sim.knight.pos.x > to.x - 500.0, "got there: {:?}", sim.knight.pos);
    assert!(sim.intents[n..].iter().all(|i| i.carry == Some(Carry::Map)), "kept the map out");
}

#[test]
fn glances_wait_while_he_reads_but_the_map_goes_back_after_a_while() {
    let mut sim = in_company();
    // Cursor above him: he'd look up; with the map out he reads instead.
    sim.park_cursor(Vec2::new(40.0, -300.0));
    take_out_map(&mut sim);
    let n = sim.started.len();
    sim.run(10.0);
    assert!(!sim.started_since(n).iter().any(|a| a == "look_up"), "{:?}", sim.started_since(n));
    assert_eq!(carry(&sim), Some(Carry::Map));
    // 15-40 s later it's folded away (gracefully) and he carries on.
    let gone = sim.run_while(40.0, |s| carry(s).is_some());
    assert!(gone, "put away: {}", sim.last_reason());
    assert!(sim.started_since(n).iter().any(|a| a == "put_map_away"), "{:?}", sim.started_since(n));
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Companion);
}

#[test]
fn a_fight_makes_him_put_the_map_away() {
    let mut sim = in_company();
    take_out_map(&mut sim);
    let n = sim.started.len();
    let foe = Foe { id: 9, kind: FoeKind::Enemy, pos: Vec2::new(1300.0, FLOOR - 70.0), feet: Vec2::new(1300.0, FLOOR), half: Vec2::new(40.0, 70.0), vulnerable: true, ..Default::default() };
    let mut fought = false;
    for _ in 0..(3.0 / DT) as usize {
        sim.brain.see_foes(vec![foe], 0);
        sim.step();
        fought |= sim.brain.behaviour() == BehaviourKind::Fight;
        if fought && carry(&sim).is_none() {
            break;
        }
    }
    assert!(fought, "{}", sim.last_reason());
    assert_eq!(carry(&sim), None, "{}", sim.last_reason());
    assert!(sim.started_since(n).iter().any(|a| a == "put_map_away"), "folded it first: {:?}", sim.started_since(n));
}

#[test]
fn hurrying_or_leaving_the_ground_puts_the_map_away() {
    let mut sim = in_company();
    take_out_map(&mut sim);
    // Far away: he'd hurry; the map is folded first, then he goes.
    sim.cursor = Some(Cursor { pos: sim.knight.pos + Vec2::new(-1500.0, -40.0), ..Default::default() });
    let n = sim.started.len();
    assert!(sim.run_while(3.0, |s| carry(s).is_some()), "{}", sim.last_reason());
    assert!(sim.started_since(n).iter().any(|a| a == "put_map_away"), "{:?}", sim.started_since(n));
    assert!(sim.run_while(3.0, |s| !matches!(s.last_motion, Motion::Navigate { .. })), "then off he goes: {}", sim.last_reason());

    let mut sim = in_company();
    take_out_map(&mut sim);
    sim.knight.state = MoveState::Airborne;
    sim.knight.support = None;
    sim.step();
    assert_eq!(carry(&sim), None);
}

#[test]
fn no_patrol_on_a_small_ledge() {
    // Idle on a short window edge, a long while, now and then with the map out: he never
    // paces back and forth (no walking act at all), and the map does come out.
    let mut sim = calm(BrainConfig::default());
    sim.open(window(1, RectI::new(800, 600, 1000, 900), "small.exe"));
    sim.place(Vec2::new(900.0, 600.0));
    sim.brain.mind_mut().personality.curiosity = 1.0;
    let n = sim.intents.len();
    sim.run(400.0);
    assert!(sim.started.iter().any(|a| a == "take_out_map"), "{:?}", sim.started);
    let carried = sim.intents[n..].iter().filter(|i| i.carry == Some(Carry::Map)).count();
    assert!(carried > 0);
    // While carrying, he stayed put (idle: nothing to follow).
    let walked = sim.intents[n..].iter().filter(|i| i.carry == Some(Carry::Map) && !matches!(i.motion, Motion::Hold)).count();
    assert_eq!(walked, 0, "no patrol with the map out");
}

#[test]
fn with_the_lantern_setting_off_it_never_comes_out() {
    let mut sim = in_company();
    assert!(sim.brain.lantern(), "on by default");
    sim.brain.set_lantern(false);
    sim.dark = true;
    sim.run(30.0);
    assert!(sim.intents.iter().all(|i| i.carry != Some(Carry::Lantern)), "{}", sim.last_reason());
    // Turned back on: out again in the dark; turned off while out: put away.
    sim.brain.set_lantern(true);
    sim.run(4.0);
    assert_eq!(carry(&sim), Some(Carry::Lantern), "{}", sim.last_reason());
    sim.brain.set_lantern(false);
    sim.run(0.5);
    assert_eq!(carry(&sim), None);
}

#[test]
fn the_lantern_stays_out_while_he_follows_you_in_the_dark() {
    let mut sim = in_company();
    sim.dark = true;
    sim.run(4.0);
    assert_eq!(carry(&sim), Some(Carry::Lantern), "{}", sim.last_reason());
    sim.cursor = Some(Cursor { pos: sim.knight.pos + Vec2::new(600.0, -40.0), ..Default::default() });
    assert!(sim.run_while(3.0, |s| !matches!(s.last_motion, Motion::Navigate { .. })));
    assert_eq!(carry(&sim), Some(Carry::Lantern), "walking with it: {}", sim.last_reason());
    // A visitor: hands needed.
    let v = Visitor { id: 5, name: "cornifer".into(), pos: Vec2::new(300.0, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() };
    sim.brain.see_visitors(vec![v]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet));
    sim.step();
    assert_eq!(carry(&sim), None);
}
