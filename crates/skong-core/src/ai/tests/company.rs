//! Company: after the talk a visitor invites the knight to stay a while (Cornifer's maps,
//! Quirrel's view, Myla's song). He stays beside them, plays his part when they settle in
//! (seated on a window edge when he can), wraps up when they part, and excuses himself
//! (`Intent::part`) for typing, enemies or a fullscreen app; the Shade cuts in, Geo waits.
use super::*;

const GUEST: u32 = 21;
/// The window top (`with_window`): y = 400, x 200..900.
const TOP: f32 = 400.0;

fn guest(name: &str, pos: Vec2, stand_off_units: f32) -> Visitor {
    Visitor {
        id: GUEST,
        name: name.into(),
        pos,
        state: VisitorState::Waiting,
        excitement: 0.9,
        stand_off: stand_off_units * 64.0,
        ..Default::default()
    }
}

fn cue(act: &str, stage: CompanyStage) -> Option<CompanyCue> {
    Some(CompanyCue { act: act.into(), stage })
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

/// Greet `v` and have a short (scripted) talk; returns them inviting the knight to stay.
fn invited(sim: &mut Sim, mut v: Visitor, act: &str) -> Visitor {
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(until(sim, 10.0, |_, i| i.greet == Some(GUEST)), "{}", sim.last_reason());
    v.state = VisitorState::Talking;
    for line in 0..2 {
        v.line = Some(line);
        sim.brain.see_visitors(vec![v.clone()]);
        sim.run(2.5);
    }
    v.line = None;
    v.greeted = true;
    v.state = VisitorState::Company;
    v.company = cue(act, CompanyStage::Invite);
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_until(2.0, BehaviourKind::Company), "{}", sim.last_reason());
    v
}

fn set_stage(sim: &mut Sim, v: &mut Visitor, stage: CompanyStage) {
    if let Some(c) = &mut v.company {
        c.stage = stage;
    }
    sim.brain.see_visitors(vec![v.clone()]);
}

/// On the window top, the knight at x = 500 and the visitor to his right.
fn on_the_window(name: &str, stand_off_units: f32) -> (Sim, Visitor) {
    on_the_window_with(BrainConfig::default(), name, stand_off_units)
}

fn on_the_window_with(cfg: BrainConfig, name: &str, stand_off_units: f32) -> (Sim, Visitor) {
    let mut sim = with_window(calm(cfg));
    sim.place(Vec2::new(500.0, TOP));
    sim.run(1.0);
    let v = guest(name, Vec2::new(500.0 + stand_off_units * 64.0, TOP), stand_off_units);
    (sim, v)
}

#[test]
fn cornifer_and_the_knight_sit_with_their_maps_then_part() {
    let (mut sim, v) = on_the_window("cornifer", 2.4);
    let mut v = invited(&mut sim, v, "company_maps|company_maps_stand");
    // The invitation: he stays beside Cornifer, facing him, nothing started yet.
    let n = sim.started.len();
    sim.run(2.0);
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Company);
    assert!(sim.last_reason().contains("staying beside"), "{}", sim.last_reason());
    assert_eq!(sim.intents.last().and_then(|i| i.face), Some(Side::Right), "faces Cornifer");
    assert!(sim.started_since(n).is_empty(), "{:?}", sim.started_since(n));

    // Together: he sits down beside him on the window edge with his map open.
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    assert_eq!(sim.started_since(n), ["company_maps"], "seated, map open");
    let seat_x = sim.knight.pos.x;
    assert!((seat_x - (v.pos.x - 2.4 * 64.0)).abs() < 40.0 && sim.knight.pos.y == TOP, "side by side: {}", sim.knight.pos);
    assert!(sim.last_reason().contains("company_maps"), "{}", sim.last_reason());

    // Geo turning up waits: visitors come before Geo.
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(300.0, FLOOR)], ..Default::default() });
    let kinds = sim.run(6.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Company), "stays: {}", sim.last_reason());
    assert!(sim.intents.iter().rev().take(150).all(|i| i.part.is_none()));
    assert_eq!(sim.knight.pos.x, seat_x, "still seated beside him");

    // Parting: the map closes, he gets up; the visitor lingers, then he moves on.
    set_stage(&mut sim, &mut v, CompanyStage::Parting);
    assert!(until(&mut sim, 1.0, |_, i| i.signal == Some(PARTED)), "tells his act to wrap up");
    assert!(sim.run_while(3.0, |s| s.act.is_some()), "act wrapped up");
    v.state = VisitorState::Waiting;
    v.company = None;
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::Company), "moves on");
    assert!(sim.intents.iter().all(|i| i.part.is_none()), "no goodbye needed: they parted");
    // Greeted: not greeted again.
    let kinds = sim.run(5.0);
    assert!(!kinds.contains(&BehaviourKind::Greet) && !kinds.contains(&BehaviourKind::Company));
}

#[test]
fn on_the_desktop_floor_he_reads_his_map_standing() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let v = guest("cornifer", Vec2::new(960.0 + 2.4 * 64.0, FLOOR), 2.4);
    let mut v = invited(&mut sim, v, "company_maps|company_maps_stand");
    let n = sim.started.len();
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    assert_eq!(sim.started_since(n), ["company_maps_stand"], "no seat on the floor");
}

#[test]
fn quirrel_and_the_knight_take_in_the_view() {
    let (mut sim, v) = on_the_window("quirrel", 1.6);
    let mut v = invited(&mut sim, v, "company_view");
    let n = sim.started.len();
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(3.0);
    assert_eq!(sim.started_since(n), ["company_view"]);
    assert!((sim.knight.pos.x - (v.pos.x - 1.6 * 64.0)).abs() < 40.0 && sim.knight.pos.y == TOP, "beside him on the ledge: {}", sim.knight.pos);
    set_stage(&mut sim, &mut v, CompanyStage::Parting);
    assert!(until(&mut sim, 1.0, |_, i| i.signal == Some(PARTED)));
}

#[test]
fn myla_sings_and_the_knight_listens_seated() {
    let (mut sim, v) = on_the_window("myla", 1.8);
    let mut v = invited(&mut sim, v, "company_listen|company_listen_stand");
    let n = sim.started.len();
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    assert_eq!(sim.started_since(n), ["company_listen"]);
    assert!(sim.last_reason().starts_with("company with myla"), "{}", sim.last_reason());
}

#[test]
fn typing_hard_ends_the_scene_politely() {
    let (mut sim, v) = on_the_window("cornifer", 2.4);
    let mut v = invited(&mut sim, v, "company_maps|company_maps_stand");
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    sim.user.keys_per_sec = 6.0;
    sim.user.idle_secs = 0.0;
    assert!(until(&mut sim, 2.0, |_, i| i.part == Some(GUEST)), "excuses himself: {}", sim.last_reason());
    sim.run(1.0);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Company);
    assert_eq!(sim.intents.iter().filter(|i| i.part.is_some()).count(), 1, "said once");
}

#[test]
fn an_enemy_ends_the_scene_and_he_fights() {
    let (mut sim, v) = on_the_window("quirrel", 1.6);
    let mut v = invited(&mut sim, v, "company_view");
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    let foe = Foe { id: 9, kind: FoeKind::Enemy, pos: Vec2::new(1300.0, FLOOR - 70.0), feet: Vec2::new(1300.0, FLOOR), half: Vec2::new(40.0, 70.0), vulnerable: true, ..Default::default() };
    sim.brain.see_foes(vec![foe], 0);
    assert!(until(&mut sim, 2.0, |_, i| i.part == Some(GUEST)), "{}", sim.last_reason());
    assert!(sim.run_until(3.0, BehaviourKind::Fight), "{}", sim.last_reason());
}

#[test]
fn a_fullscreen_app_ends_the_scene() {
    let (mut sim, v) = on_the_window_with(BrainConfig { hide_for_fullscreen: true, ..Default::default() }, "myla", 1.8);
    let mut v = invited(&mut sim, v, "company_listen|company_listen_stand");
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    let mut w = window(30, RectI::new(0, 0, 1920, 1080), "player.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    assert!(until(&mut sim, 3.0, |_, i| i.part == Some(GUEST)), "{}", sim.last_reason());
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::Company));
}

#[test]
fn the_shade_cuts_in_and_the_visitor_is_told() {
    let (mut sim, v) = on_the_window("cornifer", 2.4);
    let mut v = invited(&mut sim, v, "company_maps|company_maps_stand");
    set_stage(&mut sim, &mut v, CompanyStage::Together);
    sim.run(2.0);
    let shade = ShadeSense { pos: Vec2::new(1500.0, FLOOR - 60.0), vulnerable: true, fighting: false, geo: 120 };
    sim.brain.sense_stakes(StakesSense { shade: Some(shade), ..Default::default() });
    assert!(until(&mut sim, 3.0, |_, i| i.part == Some(GUEST)), "{}", sim.last_reason());
    assert!(sim.run_until(3.0, BehaviourKind::ShadeHunt), "{}", sim.last_reason());
}
