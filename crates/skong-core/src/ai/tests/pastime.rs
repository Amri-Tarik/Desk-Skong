//! Pastimes: the training dummy, chasing a lumafly, taking in the view from a high window
//! edge, stargazing at night. Ambient: never ahead of the Shade, fights, visitors, Geo,
//! shinies or deposits. Plus the long check that plain companion time went down.
use super::*;
use crate::geom::RectF;

fn dummy_at(x: f32) -> KnownDummy {
    KnownDummy { id: 1, pos: Vec2::new(x, FLOOR), span: (0.0, 1920.0), body: RectF::new(x - 30.0, FLOOR - 150.0, x + 30.0, FLOOR - 70.0) }
}

/// Keeping the user company (cursor resting beside him), a little bored.
fn in_company() -> Sim {
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(150.0, -120.0));
    assert!(sim.run_until(3.0, BehaviourKind::Companion), "{}", sim.last_reason());
    sim
}

fn hits(sim: &Sim) -> Vec<crate::pastime::DummyHit> {
    sim.pastime_events.iter().filter_map(|e| if let PastimeEvent::DummyHit { hit, .. } = e { Some(*hit) } else { None }).collect()
}

// --- the training dummy ---------------------------------------------------------------

#[test]
fn restless_in_company_he_practises_on_the_dummy_and_comes_back() {
    let mut sim = in_company();
    sim.dummy = Some(dummy_at(1400.0));
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "took it up: {}", sim.last_reason());
    assert!(sim.last_reason().contains("training dummy"), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("more fun than standing around"), "the reason says why: {}", sim.last_reason());
    let n = sim.intents.len();
    assert!(sim.run_while(90.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "ends: {}", sim.last_reason());
    // He stood beside it, on its side he came from, and practised.
    let drills: Vec<&Intent> = sim.intents[n..].iter().filter(|i| i.reason.contains("training dummy:") && !i.reason.contains("breath")).collect();
    assert!(!drills.is_empty());
    assert!((sim.knight.pos.x - 1335.0).abs() < 80.0, "beside it: {:?}", sim.knight.pos);
    let attacks = sim.intents[n..].iter().filter(|i| i.attack.is_some_and(|a| a.side == Side::Right)).count();
    let h = hits(&sim);
    assert!(attacks >= 2 || h.iter().any(|h| h.heavy), "swung at it: {attacks} slashes, {h:?}");
    assert!(h.len() >= 3, "blows landed: {h:?}");
    assert!(h.iter().all(|h| !h.from_right), "from his side (left of it)");
    // Several different drills.
    let kinds: std::collections::BTreeSet<&str> = drills.iter().map(|i| i.reason.split("training dummy: ").nth(1).unwrap_or("").split(" (").next().unwrap_or("")).collect();
    assert!(kinds.len() >= 2, "varied drills: {kinds:?}");
    assert_eq!(sim.switches.last().map(|s| s.reason), Some(SwitchReason::Finished), "ends cleanly");
    // Practice doesn't count as a fight (Sly's lessons, achievements).
    assert!(sim.brain.since_fight() > 10.0);
    // Then a while back with the user before practising again.
    let kinds = sim.run(40.0);
    assert!(!kinds.contains(&BehaviourKind::Pastime) || sim.brain.pastime() != Some(PastimeKind::Dummy), "a cooldown");
}

#[test]
fn a_dummy_beats_following_the_cursor_right_away() {
    // Just started keeping company (no restless stretch), content, the cursor moving about
    // near him: a dummy turning up is still more fun.
    for seed in 0..4 {
        let mut sim = calm(BrainConfig::default());
        sim.brain = Brain::new(BrainConfig { personality: Some(Personality::default()), ..Default::default() }, sim.catalog.clone(), 3 + seed);
        sim.brain.mind_mut().needs = Needs { energy: 0.85, boredom: 0.1, loneliness: 0.6, mischief: 0.1 };
        sim.park_cursor(Vec2::new(150.0, -120.0));
        assert!(sim.run_until(3.0, BehaviourKind::Companion), "{}", sim.last_reason());
        sim.dummy = Some(dummy_at(1400.0));
        let mut took_up = false;
        for i in 0..(5.0 / DT) as usize {
            // The user wiggles the mouse around near him.
            let wiggle = Vec2::new(150.0 + 60.0 * ((i / 30) % 3) as f32, -120.0);
            sim.cursor = Some(Cursor { pos: sim.body_center() + wiggle, ..Default::default() });
            sim.user.idle_secs = 0.0;
            sim.step();
            if sim.brain.behaviour() == BehaviourKind::Pastime {
                took_up = true;
                break;
            }
        }
        assert!(took_up, "seed {seed}: {}", sim.last_reason());
        assert!(sim.last_reason().contains("training dummy"), "{}", sim.last_reason());
        // Committed: no dithering back to company while he goes over and practises.
        let kinds = sim.run(15.0);
        assert!(kinds.iter().all(|k| *k == BehaviourKind::Pastime), "seed {seed}: {:?}", sim.switches.last());
        assert!(!hits(&sim).is_empty(), "practised: {}", sim.last_reason());
    }
}

#[test]
fn a_lumafly_drifting_by_beats_plain_company_right_away() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.boredom = 0.1;
    sim.park_cursor(Vec2::new(150.0, -120.0));
    assert!(sim.run_until(3.0, BehaviourKind::Companion), "{}", sim.last_reason());
    sim.flies = vec![KnownLumafly { id: 3, pos: Vec2::new(1300.0, FLOOR - 120.0), leaving: false }];
    assert!(sim.run_until(3.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("lumafly"), "{}", sim.last_reason());
}

#[test]
fn a_coin_or_a_visitor_still_come_before_a_fresh_dummy() {
    // Geo on the floor and a dummy at once: Geo first.
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(150.0, -120.0));
    assert!(sim.run_until(3.0, BehaviourKind::Companion), "{}", sim.last_reason());
    sim.dummy = Some(dummy_at(1400.0));
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(500.0, FLOOR)], ..Default::default() });
    assert!(sim.run_until(1.0, BehaviourKind::Loot), "{}", sim.last_reason());
    // Practising: a visitor cuts in.
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(150.0, -120.0));
    sim.dummy = Some(dummy_at(1400.0));
    assert!(sim.run_until(5.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    let v = Visitor { id: 5, name: "cornifer".into(), pos: Vec2::new(500.0, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() };
    sim.brain.see_visitors(vec![v]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "{}", sim.last_reason());
}

#[test]
fn the_summoned_flame_and_a_dummy_take_turns_without_dithering() {
    // Both at once: the flame the user summoned first, the dummy after (not starved).
    let mut sim = in_company();
    sim.dummy = Some(dummy_at(1400.0));
    sim.brain.see_scene(Some(SceneCue::Flame { at: Vec2::new(500.0, FLOOR) }));
    assert!(sim.run_until(3.0, BehaviourKind::Scene), "{}", sim.last_reason());
    let kinds = sim.run(20.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Scene), "no dithering: {:?}", sim.switches.last());
    sim.brain.see_scene(None);
    assert!(sim.run_until(5.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    // Practising when a flame turns up: he finishes first (no flip-flop).
    sim.brain.see_scene(Some(SceneCue::Flame { at: Vec2::new(500.0, FLOOR) }));
    let kinds = sim.run(5.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Pastime), "{:?}", sim.switches.last());
}

#[test]
fn with_real_physics_the_drills_jump_pogo_and_end_back_by_the_dummy() {
    let mut pogos = 0;
    for seed in 0..4 {
        let mut sim = in_company();
        sim.brain = Brain::new(BrainConfig { personality: Some(Personality::default()), ..Default::default() }, sim.catalog.clone(), 7 + seed);
        sim.brain.mind_mut().needs = Needs { energy: 0.85, boredom: 0.8, loneliness: 0.2, mischief: 0.3 };
        sim.physics = true;
        sim.dummy = Some(dummy_at(1400.0));
        assert!(sim.run_until(60.0, BehaviourKind::Pastime), "{}", sim.last_reason());
        assert!(sim.run_while(120.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "ends: {}", sim.last_reason());
        assert!(sim.knight.is_grounded() && sim.knight.pos.y == FLOOR);
        assert!((sim.knight.pos.x - 1400.0).abs() < 200.0, "still by the dummy: {:?}", sim.knight.pos);
        pogos += sim.intents.iter().filter(|i| matches!(i.motion, Motion::Direct(m) if m.pogo)).count();
    }
    assert!(pogos > 0, "bounced off its head at least once");
}

#[test]
fn the_dummy_gone_mid_session_ends_it() {
    let mut sim = in_company();
    sim.dummy = Some(dummy_at(1400.0));
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    sim.run(4.0);
    sim.dummy = None;
    assert!(sim.run_while(2.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "{}", sim.last_reason());
}

#[test]
fn geo_a_visitor_and_the_shade_all_preempt_a_pastime() {
    // Geo.
    let mut sim = in_company();
    sim.dummy = Some(dummy_at(1400.0));
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    sim.run(3.0);
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(300.0, FLOOR)], ..Default::default() });
    assert!(sim.run_until(1.0, BehaviourKind::Loot), "Geo first: {}", sim.last_reason());
    assert_eq!(sim.switches.last().map(|s| s.reason), Some(SwitchReason::Preempted));
    // While coins lie about, no pastime starts.
    let kinds = sim.run(10.0);
    assert!(!kinds.contains(&BehaviourKind::Pastime), "{:?}", sim.last_reason());

    // A visitor.
    let mut sim = in_company();
    sim.dummy = Some(dummy_at(1400.0));
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    sim.run(3.0);
    let v = Visitor { id: 5, name: "cornifer".into(), pos: Vec2::new(500.0, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() };
    sim.brain.see_visitors(vec![v]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet), "visitors first: {}", sim.last_reason());

    // The Shade.
    let mut sim = in_company();
    sim.dummy = Some(dummy_at(1400.0));
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    sim.run(3.0);
    sim.brain.sense_stakes(StakesSense { shade: Some(ShadeSense { pos: Vec2::new(300.0, FLOOR - 40.0), vulnerable: true, fighting: false, geo: 30 }), ..Default::default() });
    assert!(sim.run_until(2.0, BehaviourKind::ShadeHunt), "the Shade first: {}", sim.last_reason());
}

#[test]
fn not_while_the_user_types_hard_or_something_is_fullscreen() {
    let mut sim = in_company();
    sim.user.keys_per_sec = 4.0;
    sim.dummy = Some(dummy_at(1400.0));
    sim.brain.mind_mut().needs.boredom = 0.9;
    let kinds = sim.run(60.0);
    assert!(!kinds.contains(&BehaviourKind::Pastime), "{}", sim.last_reason());
    sim.user.keys_per_sec = 0.0;
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    // Typing starts mid-practice: he stops.
    sim.user.keys_per_sec = 4.0;
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "{}", sim.last_reason());

    let mut sim = calm(BrainConfig { hide_for_fullscreen: true, ..Default::default() });
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "player.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    sim.dummy = Some(dummy_at(1400.0));
    let kinds = sim.run(90.0);
    assert!(!kinds.contains(&BehaviourKind::Pastime));
}

// --- the lumafly ----------------------------------------------------------------------

#[test]
fn he_follows_a_lumafly_jumps_at_it_and_watches_it_float_away() {
    let mut sim = in_company();
    sim.physics = true;
    sim.brain.mind_mut().needs.boredom = 0.8;
    // Drifting a little above the floor, off to his right.
    sim.flies = vec![KnownLumafly { id: 3, pos: Vec2::new(1300.0, FLOOR - 120.0), leaving: false }];
    assert!(sim.run_until(60.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("lumafly"), "{}", sim.last_reason());
    // Follows it and jumps: the fly is caught (the Sim's app: a swipe within reach) and
    // floats off while he watches.
    let ok = sim.run_while(40.0, |s| !s.flies[0].leaving);
    assert!(ok, "caught it: {} at {:?}", sim.last_reason(), sim.knight.pos);
    assert!(sim.pastime_events.iter().any(|e| matches!(e, PastimeEvent::Swipe { fly: 3, .. })));
    assert!(sim.intents.iter().any(|i| i.reason.contains("jumping to catch")));
    assert!(sim.intents.iter().any(|i| matches!(i.motion, Motion::Direct(m) if m.jump_pressed)), "jumped");
    assert!(sim.run_until(1.0, BehaviourKind::Pastime) || sim.last_reason().contains("float away"), "{}", sim.last_reason());
    assert!(sim.run_while(8.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "ends: {}", sim.last_reason());
    assert!(sim.started.iter().any(|a| a == "look_up"), "watched it go");
}

#[test]
fn a_leaving_lumafly_is_not_chased() {
    let mut sim = in_company();
    sim.brain.mind_mut().needs.boredom = 0.9;
    sim.flies = vec![KnownLumafly { id: 3, pos: Vec2::new(1300.0, FLOOR - 120.0), leaving: true }];
    let kinds = sim.run(60.0);
    assert!(!sim.intents.iter().any(|i| i.reason.contains("lumafly")), "{:?}", kinds.last());
}

// --- the view and the stars -----------------------------------------------------------

#[test]
fn he_climbs_to_the_end_of_a_high_settled_window_edge_and_takes_in_the_view() {
    let mut sim = in_company();
    sim.open(window(1, RectI::new(700, 250, 1500, 900), "browser.exe"));
    // A fresh window: its edge hasn't settled; nothing yet.
    sim.run(10.0);
    assert!(!sim.intents.iter().any(|i| i.reason.contains("view")), "{}", sim.last_reason());
    sim.brain.mind_mut().needs.boredom = 0.7;
    assert!(sim.run_until(150.0, BehaviourKind::Pastime), "{}", sim.last_reason());
    assert!(sim.last_reason().contains("browser.exe's edge"), "{}", sim.last_reason());
    assert!(sim.run_while(20.0, |s| s.act.as_ref().is_none_or(|a| s.act_name(a.act.id) != "take_in_view")), "sat down: {} {:?}", sim.last_reason(), sim.started);
    // On the title bar, at one of its ends.
    assert_eq!(sim.knight.pos.y, 250.0);
    assert!((sim.knight.pos.x - 724.0).abs() < 8.0 || (sim.knight.pos.x - 1476.0).abs() < 8.0, "{:?}", sim.knight.pos);
    assert!(sim.last_reason().contains("taking in the view"), "{}", sim.last_reason());
    assert!(sim.run_while(40.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "ends: {}", sim.last_reason());
    assert_eq!(sim.switches.last().map(|s| s.reason), Some(SwitchReason::Finished));
}

#[test]
fn low_window_edges_have_no_view() {
    let mut sim = in_company();
    sim.open(window(1, RectI::new(700, 800, 1500, 1000), "low.exe"));
    sim.brain.mind_mut().needs.boredom = 0.9;
    sim.run(150.0);
    assert!(!sim.intents.iter().any(|i| i.reason.contains("view")), "{}", sim.last_reason());
}

#[test]
fn at_night_he_goes_up_high_and_looks_at_the_stars() {
    let mut sim = in_company();
    sim.open(window(1, RectI::new(700, 300, 1500, 900), "browser.exe"));
    sim.open(window(2, RectI::new(100, 600, 600, 1000), "notes.exe"));
    sim.hour = Some(23.5);
    sim.brain.mind_mut().needs.boredom = 0.8;
    let found = sim.run_while(200.0, |s| s.act.as_ref().is_none_or(|a| s.act_name(a.act.id) != "stargaze"));
    assert!(found, "stargazed: {} / {:?}", sim.last_reason(), sim.started);
    assert!(sim.last_reason().contains("stargazing from browser.exe"), "the highest open ledge: {}", sim.last_reason());
    assert_eq!(sim.knight.pos.y, 300.0);
    assert!(sim.run_while(40.0, |s| s.brain.behaviour() == BehaviourKind::Pastime), "ends: {}", sim.last_reason());
}

#[test]
fn no_stargazing_by_day() {
    let mut sim = in_company();
    sim.open(window(1, RectI::new(700, 300, 1500, 900), "browser.exe"));
    sim.hour = Some(14.0);
    sim.brain.mind_mut().needs.boredom = 0.9;
    sim.run(240.0);
    assert!(!sim.started.iter().any(|a| a == "stargaze"));
}

/// What a long run did.
#[derive(Default)]
struct Day {
    /// Share of time per behaviour.
    shares: BTreeMap<BehaviourKind, f32>,
    /// Pastimes started, by kind.
    started: BTreeMap<PastimeKind, u32>,
}

/// A desk with three windows, the user around (moving the cursor now and then near the
/// knight), for `secs`, with the app's pastime props (`props`: the dummy and lumaflies
/// turning up on their schedules) and the local hour.
fn day_at_the_desk(secs: f32, seed_shift: u32, props: bool, hour: Option<f32>) -> Day {
    use crate::pastime::{DummyRules, LumaflyRules, Sky, Yard};
    let mut sim = calm(BrainConfig::default());
    sim.brain = Brain::new(BrainConfig { personality: Some(Personality::default()), ..Default::default() }, sim.catalog.clone(), 42 + seed_shift);
    sim.hour = hour;
    sim.open(window(1, RectI::new(150, 420, 900, 1000), "editor.exe"));
    sim.open(window(2, RectI::new(1000, 250, 1750, 800), "browser.exe"));
    sim.open(window(3, RectI::new(600, 120, 1100, 380), "chat.exe"));
    let (drules, frules) = (DummyRules::default(), LumaflyRules::default());
    let mut yard = Yard::new(seed_shift as u64, &drules);
    let mut sky = Sky::new(seed_shift as u64, &frules);
    let area = RectF::new(0.0, 0.0, 1920.0, 1040.0);
    let mut day = Day::default();
    let mut last_pastime = None;
    let n = (secs / DT).round() as usize;
    let mut next_move = 0.0;
    let mut k = 0u32;
    for _ in 0..n {
        if props {
            // The dummy: on the floor somewhere away from him (the app places it on settled
            // ledges; the floor always is).
            if yard.due(DT, true, &drules) {
                let x = if sim.knight.pos.x > 960.0 { 400.0 } else { 1500.0 };
                yard.add(crate::shiny::Spot::Ground { pos: Vec2::new(x, FLOOR) }, &drules);
            }
            yard.take_landed();
            sim.dummy = yard.shown(&sim.desk, &sim.terrain, 40.0, &|_| true).map(|d| KnownDummy { id: d.id, pos: d.pos, span: d.span, body: RectF::new(d.pos.x - 30.0, d.pos.y - 150.0, d.pos.x + 30.0, d.pos.y - 70.0) });
            let bars: Vec<(f32, f32, f32)> = sim.terrain.ledges.iter().filter(|l| l.owner.is_some()).map(|l| (l.x0, l.x1, l.y)).collect();
            if sky.due(DT, true, &frules) {
                sky.spawn(area, &bars, sim.knight.pos, &frules, 1.0);
            }
            sky.step(DT, &bars, &frules, 1.0);
            for ev in std::mem::take(&mut sim.pastime_events) {
                match ev {
                    PastimeEvent::Swipe { fly, from } => {
                        sky.swipe(fly, from, &frules, 1.0);
                    }
                    PastimeEvent::DummyHit { hit, delay } => yard.strike(hit, delay),
                }
            }
            sim.flies = sky.shown(1.0).iter().map(|f| KnownLumafly { id: f.id, pos: f.pos, leaving: f.leaving }).collect();
        }
        if sim.t >= next_move {
            // The user nudges the mouse somewhere near the knight every 15-40 s.
            k += 1;
            let dx = [-260.0, 180.0, -90.0, 320.0, 60.0][k as usize % 5];
            let dy = [-160.0, -60.0, -220.0, -120.0, -40.0][k as usize % 5];
            let at = Vec2::new((sim.knight.pos.x + dx).clamp(40.0, 1880.0), (sim.knight.pos.y + dy).clamp(40.0, 1000.0));
            sim.cursor = Some(Cursor { pos: at, ..Default::default() });
            sim.user.idle_secs = 0.0;
            next_move = sim.t + 15.0 + (k % 6) as f32 * 5.0;
        }
        let b = sim.step().behaviour.unwrap_or(BehaviourKind::Idle);
        *day.shares.entry(b).or_default() += DT / secs;
        let now = sim.brain.pastime();
        if let Some(kind) = now
            && last_pastime != now
        {
            *day.started.entry(kind).or_default() += 1;
        }
        last_pastime = now;
    }
    day
}

#[test]
#[ignore = "measurement"]
fn measure_companion_share() {
    for (props, hour) in [(false, None), (true, Some(15.0)), (true, Some(23.0))] {
        for s in 0..3 {
            let day = day_at_the_desk(1800.0, s, props, hour);
            let mut v: Vec<_> = day.shares.into_iter().collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            println!(
                "props {props} hour {hour:?} seed {s}: {} | {:?}",
                v.iter().map(|(k, f)| format!("{} {:.0}%", k.label(), f * 100.0)).collect::<Vec<_>>().join(", "),
                day.started
            );
        }
    }
}

#[test]
fn half_an_hour_at_the_desk_spends_much_less_time_in_plain_company() {
    // Before the pastimes this scenario was ~91 % companion (seeds 0-2, measured).
    for (seed, hour) in [(0, Some(15.0)), (1, Some(23.0))] {
        let day = day_at_the_desk(1800.0, seed, true, hour);
        let share = |k| day.shares.get(&k).copied().unwrap_or(0.0);
        let company = share(BehaviourKind::Companion);
        assert!(company < 0.8, "companion {:.0}%", company * 100.0);
        assert!(company > 0.4, "still mostly with the user: {:.0}%", company * 100.0);
        assert!(share(BehaviourKind::Pastime) > 0.15, "{:?}", day.started);
        // Every pastime that can happen here did, and none hogs the time.
        for kind in [PastimeKind::Dummy, PastimeKind::Lumafly, PastimeKind::View] {
            let n = day.started.get(&kind).copied().unwrap_or(0);
            assert!((1..=12).contains(&n), "{kind:?}: {n} in half an hour");
        }
        if hour == Some(23.0) {
            assert!(day.started.contains_key(&PastimeKind::Stargaze));
        } else {
            assert!(!day.started.contains_key(&PastimeKind::Stargaze));
        }
    }
}
