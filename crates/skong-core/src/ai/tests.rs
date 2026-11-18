//! Scripted percept sequences → expected behaviour choices. Navigation is faked (routes
//! "arrive" after a fixed time) and acts are emulated with fixed durations, so these
//! tests exercise only the brain.
use std::path::Path;

use super::*;
use crate::acts::ActLibrary;
use crate::anim::ClipLibrary;
use crate::assets::SpriteBank;
use crate::controller::{Abilities, Character, CharacterParams, MoveEvent, MoveState, Support};
use crate::desktop::{DesktopSnapshot, Monitor, WindowId, WindowInfo};
use crate::geom::RectI;
use crate::terrain::{LedgeKind, Terrain, TerrainOptions};

const DT: f32 = 1.0 / 30.0;
const FLOOR: f32 = 1040.0;

fn catalog() -> ActCatalog {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&data.join("hk")).expect("bank");
    let clips = ClipLibrary::load(&data.join("hk/animations/Knight.json"), &bank).expect("clips");
    let acts = ActLibrary::load(&data.join("acts.json"), &clips).expect("acts");
    ActCatalog::from_library(&acts)
}

/// Navigator stand-in: every route takes `trip` seconds.
#[derive(Default)]
struct FakeNav {
    status: NavStatus,
    elapsed: f32,
    trip: f32,
    gos: Vec<(NavGoal, TravelStyle)>,
}

impl Navigator for FakeNav {
    fn estimate(&mut self, from: &Character, goal: NavGoal, _style: TravelStyle) -> Option<f32> {
        Some(match goal {
            NavGoal::Point(p) => from.pos.distance(p) / 400.0,
            NavGoal::OnWindow(_) => 1.5,
        })
    }
    fn go(&mut self, _from: &Character, goal: NavGoal, style: TravelStyle) -> bool {
        self.gos.push((goal, style));
        self.status = NavStatus::Moving;
        self.elapsed = 0.0;
        true
    }
    fn stop(&mut self) {
        self.status = NavStatus::Idle;
    }
    fn status(&self) -> NavStatus {
        self.status
    }
}

struct PlayingAct {
    act: RunningAct,
    left: f32,
}

/// Desktop + knight + user + act player, driven by a brain.
struct Sim {
    brain: Brain,
    nav: FakeNav,
    catalog: ActCatalog,
    desk: DesktopSnapshot,
    terrain: Terrain,
    knight: Character,
    benches: Vec<Vec2>,
    cursor: Option<Cursor>,
    user: UserActivity,
    pokes: Vec<Poke>,
    events: Vec<MoveEvent>,
    act: Option<PlayingAct>,
    started: Vec<String>,
    intents: Vec<Intent>,
    switches: Vec<Switch>,
    t: f32,
}

fn window(id: WindowId, rect: RectI, process: &str) -> WindowInfo {
    WindowInfo {
        id,
        rect,
        title: format!("{process} window"),
        class: "Test".into(),
        process: process.into(),
        pid: id as u32,
        maximized: false,
        fullscreen: false,
        foreground: false,
        topmost: false,
    }
}

fn floor_support() -> Support {
    Support { owner: None, kind: LedgeKind::Ground, y: FLOOR, x0: 0.0, x1: 1920.0 }
}

impl Sim {
    fn new(cfg: BrainConfig) -> Self {
        let cfg = BrainConfig { personality: Some(cfg.personality.unwrap_or_default()), ..cfg };
        let catalog = catalog();
        let desk = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows: vec![],
        };
        let terrain = Terrain::from_desktop(&desk, &TerrainOptions::default());
        let mut knight = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(960.0, FLOOR));
        knight.state = MoveState::Grounded;
        knight.support = Some(floor_support());
        Self {
            brain: Brain::new(cfg, catalog.clone(), 42),
            nav: FakeNav { trip: 2.0, ..Default::default() },
            catalog,
            desk,
            terrain,
            knight,
            benches: vec![],
            cursor: None,
            user: UserActivity { idle_secs: 5.0, keys_per_sec: 0.0 },
            pokes: vec![],
            events: vec![],
            act: None,
            started: vec![],
            intents: vec![],
            switches: vec![],
            t: 0.0,
        }
    }

    fn desktop_changed(&mut self) {
        self.desk.generation += 1;
        self.terrain = Terrain::from_desktop(&self.desk, &TerrainOptions::default());
    }

    fn open(&mut self, w: WindowInfo) {
        self.desk.windows.insert(0, w);
        self.desktop_changed();
    }

    fn close(&mut self, id: WindowId) {
        self.desk.windows.retain(|w| w.id != id);
        self.desktop_changed();
    }

    fn act_name(&self, id: ActId) -> String {
        self.catalog.get(id).map(|a| a.name.clone()).unwrap_or_default()
    }

    fn step(&mut self) -> Intent {
        let p = Percepts {
            dt: DT,
            knight: &self.knight,
            terrain: &self.terrain,
            desktop: &self.desk,
            cursor: self.cursor,
            user: self.user,
            fullscreen_front: false,
            pokes: &self.pokes,
            benches: &self.benches,
            perch: false,
            act: self.act.as_ref().map(|a| a.act),
            move_events: &self.events,
        };
        let intent = self.brain.tick(&p, &mut self.nav);
        self.pokes.clear();
        self.events.clear();
        self.t += DT;
        self.user.idle_secs += DT;

        // Act player emulation.
        if intent.cancel_act && self.act.as_ref().is_some_and(|a| a.act.interruptible) {
            self.act = None;
        }
        if intent.signal == Some("woken")
            && let Some(a) = &mut self.act
            && self.catalog.get(a.act.id).is_some_and(|i| i.name == "nap")
        {
            a.left = 1.0;
            a.act.interruptible = false; // "Wake" step
        }
        if let Some(id) = intent.act
            && self.act.is_none()
        {
            let name = self.act_name(id);
            let left = if name == "nap" { 600.0 } else { 1.5 };
            self.started.push(name);
            self.act = Some(PlayingAct { act: RunningAct { id, interruptible: true }, left });
        }
        if let Some(a) = &mut self.act {
            a.left -= DT;
            if a.left <= 0.0 {
                self.act = None;
            }
        }

        // Every route arrives after `trip` seconds.
        if self.nav.status == NavStatus::Moving {
            self.nav.elapsed += DT;
            if self.nav.elapsed >= self.nav.trip {
                self.nav.status = NavStatus::Arrived;
                if let Some((NavGoal::Point(p), _)) = self.nav.gos.last() {
                    self.knight.pos.x = p.x; // walked there (stays on the same floor)
                }
            }
        }

        if let Some(s) = self.brain.last_switch()
            && self.switches.last() != Some(&s)
        {
            self.switches.push(s);
        }
        self.intents.push(intent.clone());
        intent
    }

    fn run(&mut self, secs: f32) -> Vec<BehaviourKind> {
        let n = (secs / DT).round() as usize;
        (0..n).map(|_| self.step().behaviour.unwrap_or(BehaviourKind::Idle)).collect()
    }

    fn run_until(&mut self, secs: f32, kind: BehaviourKind) -> bool {
        self.run_while(secs, |s| s.brain.behaviour() != kind)
    }

    fn run_while(&mut self, secs: f32, mut cond: impl FnMut(&Sim) -> bool) -> bool {
        let n = (secs / DT).round() as usize;
        for _ in 0..n {
            self.step();
            if !cond(self) {
                return true;
            }
        }
        false
    }

    fn body_center(&self) -> Vec2 {
        self.knight.body().center()
    }

    fn park_cursor(&mut self, offset: Vec2) {
        self.cursor = Some(Cursor { pos: self.body_center() + offset, ..Default::default() });
    }
}

fn calm(cfg: BrainConfig) -> Sim {
    let mut sim = Sim::new(cfg);
    let m = sim.brain.mind_mut();
    m.needs = Needs { energy: 0.85, boredom: 0.5, loneliness: 0.2, mischief: 0.0 };
    m.emotions = Emotions::default();
    sim
}

#[test]
fn tired_with_idle_user_walks_to_bench_and_naps() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_until(3.0, BehaviourKind::Nap), "debug: {}", sim.intents.last().map_or("", |i| &i.debug));
    sim.step();
    assert_eq!(sim.nav.gos.last(), Some(&(NavGoal::Point(BENCH), TravelStyle::Stroll)), "walks to the bench first");
    assert!(!sim.started.iter().any(|a| a == "nap"), "no sitting before reaching the bench");
    sim.run(4.0);
    assert_eq!(sim.started.last().map(String::as_str), Some("nap"));
    // Stays asleep: nothing voluntary interrupts a nap.
    let kinds = sim.run(30.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Nap));
    assert!(sim.brain.mind().needs.energy > 0.1);
}

#[test]
fn new_window_is_explored_at_a_stroll_when_calm() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.open(window(10, RectI::new(300, 500, 900, 900), "notepad.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    let poi = sim.brain.exploring().expect("exploring");
    assert_eq!(poi.kind, PoiKind::NewWindow);
    assert_eq!(poi.target, PoiTarget::Window(10));
    sim.step();
    assert_eq!(sim.nav.gos.last(), Some(&(NavGoal::OnWindow(10), TravelStyle::Stroll)));
}

#[test]
fn new_window_is_rushed_to_when_excited() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.brain.mind_mut().emotions.excitement = 0.9;
    sim.open(window(10, RectI::new(300, 500, 900, 900), "notepad.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    sim.step();
    assert_eq!(sim.nav.gos.last(), Some(&(NavGoal::OnWindow(10), TravelStyle::Hurry)));
}

#[test]
fn explored_window_is_remembered() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.open(window(10, RectI::new(300, 500, 900, 900), "notepad.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    // Arrive, inspect, and move on.
    assert!(sim.run_while(10.0, |s| s.brain.behaviour() == BehaviourKind::Explore));
    let b0 = sim.brain.mind().needs.boredom;
    assert!(b0 < 0.5, "exploring satisfied boredom ({b0})");
    sim.run(20.0);
    let again = sim.nav.gos.iter().filter(|(g, _)| *g == NavGoal::OnWindow(10)).count();
    assert_eq!(again, 1, "visited window is not re-explored right away");
}

/// Mischievous knight, cursor parked right next to it.
fn parked_cursor_sim(cfg: BrainConfig) -> Sim {
    let mut sim = calm(cfg);
    sim.brain.mind_mut().needs.mischief = 0.95;
    sim.park_cursor(Vec2::new(50.0, -10.0));
    sim
}

#[test]
fn parked_cursor_with_mischief_starts_a_battle() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    assert!(sim.run_until(4.0, BehaviourKind::Battle), "debug: {}", sim.intents.last().map_or("", |i| &i.debug));
    sim.run(8.0);
    assert!(sim.started.iter().any(|a| a == "challenge"), "acts: {:?}", sim.started);
    let attacks: Vec<Attack> = sim.intents.iter().filter_map(|i| i.attack).collect();
    assert!(attacks.len() >= 5, "attacks: {attacks:?}");
    assert!(attacks.iter().all(|a| a.side == Side::Right && a.dir == AttackDir::Forward));
    assert!(sim.started.iter().any(|a| a == "victory"), "acts: {:?}", sim.started);
    assert!(sim.brain.mind().needs.mischief < 0.5, "winning satisfies mischief");
}

#[test]
fn cursor_above_gets_jumped_at_and_up_slashed() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    sim.park_cursor(Vec2::new(10.0, -80.0));
    assert!(sim.run_until(4.0, BehaviourKind::Battle));
    sim.run(4.0);
    assert!(sim.intents.iter().any(|i| matches!(i.motion, Motion::Direct(m) if m.jump_pressed)));
    assert!(sim.intents.iter().filter_map(|i| i.attack).any(|a| a.dir == AttackDir::Up));
}

#[test]
fn bother_level_zero_never_battles() {
    let mut sim = parked_cursor_sim(BrainConfig { bother_level: 0.0, ..Default::default() });
    let kinds = sim.run(120.0);
    assert!(!kinds.contains(&BehaviourKind::Battle));
    assert_eq!(sim.brain.mind().needs.mischief, 0.0);
}

#[test]
fn typing_user_is_never_battled() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    sim.user.keys_per_sec = 4.0;
    let kinds = sim.run(60.0);
    assert!(!kinds.contains(&BehaviourKind::Battle));
}

#[test]
fn battle_is_abandoned_when_the_user_starts_typing() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    assert!(sim.run_until(4.0, BehaviourKind::Battle));
    sim.user.keys_per_sec = 4.0;
    assert!(sim.run_while(1.0, |s| s.brain.behaviour() == BehaviourKind::Battle));
    let kinds = sim.run(30.0);
    assert!(!kinds.contains(&BehaviourKind::Battle), "no rematch while typing");
}

#[test]
fn cursor_clicks_make_the_knight_recoil() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    sim.park_cursor(Vec2::new(50.0, -10.0));
    assert!(sim.run_until(4.0, BehaviourKind::Battle));
    for _ in 0..3 {
        // Wait for the fight phase (after the challenge act), then click.
        sim.run(2.0);
        sim.pokes.push(Poke::Click { pos: sim.body_center() });
    }
    sim.run(6.0);
    assert!(sim.started.iter().any(|a| a == "recoil" || a == "stunned"), "acts: {:?}", sim.started);
    assert!(sim.brain.mind().emotions.hurt > 0.0);
}

#[test]
fn fullscreen_app_sends_knight_into_hiding() {
    let mut sim = calm(BrainConfig::default());
    sim.knight.pos.x = 1500.0;
    sim.run(1.0);
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "game.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    assert!(sim.run_until(0.5, BehaviourKind::Hide));
    sim.step();
    assert_eq!(sim.nav.gos.last().map(|g| g.0), Some(NavGoal::Point(Vec2::new(1890.0, FLOOR))));
    let kinds = sim.run(20.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Hide));
    sim.close(20);
    assert!(sim.run_while(1.0, |s| s.brain.behaviour() == BehaviourKind::Hide));
}

#[test]
fn poke_during_nap_wakes_and_startles() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_until(3.0, BehaviourKind::Nap));
    sim.run(8.0);
    assert_eq!(sim.started.last().map(String::as_str), Some("nap"));
    assert!(sim.act.is_some(), "asleep");
    sim.pokes.push(Poke::Click { pos: sim.body_center() });
    let intent = sim.step();
    assert_eq!(intent.signal, Some("woken"));
    assert!(sim.brain.mind().emotions.startle > 0.5);
    assert!(sim.run_until(1.0, BehaviourKind::Flee));
    sim.run(4.0);
    assert!(sim.started.iter().any(|a| a == "startled"), "acts: {:?}", sim.started);
    assert!(sim.nav.gos.iter().any(|(_, s)| *s == TravelStyle::Hurry));
}

#[test]
fn user_returning_wakes_the_napping_knight() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_until(3.0, BehaviourKind::Nap));
    sim.run(8.0);
    sim.user.idle_secs = 0.0;
    let intent = sim.step();
    assert_eq!(intent.signal, Some("woken"));
    assert!(sim.brain.mind().emotions.excitement > 0.2);
}

#[test]
fn window_closing_underfoot_makes_it_flee() {
    let mut sim = calm(BrainConfig::default());
    sim.open(window(7, RectI::new(600, 600, 1300, 1000), "explorer.exe"));
    sim.knight.pos = Vec2::new(960.0, 600.0);
    sim.knight.support = Some(Support { owner: Some(7), kind: LedgeKind::OneWay, y: 600.0, x0: 600.0, x1: 1300.0 });
    sim.run(1.0);
    sim.close(7);
    sim.knight.support = None;
    sim.knight.state = MoveState::Airborne;
    sim.events.push(MoveEvent::LostSupport);
    assert!(sim.run_until(0.5, BehaviourKind::Flee));
    // Falls, lands, gets startled, runs.
    sim.run(0.3);
    sim.knight.pos.y = FLOOR;
    sim.knight.state = MoveState::Grounded;
    sim.knight.support = Some(floor_support());
    sim.run(3.0);
    assert!(sim.started.iter().any(|a| a == "startled"));
    let run = sim.nav.gos.last().copied().expect("ran away");
    assert_eq!(run.1, TravelStyle::Hurry);
}

#[test]
fn riding_a_moving_window_is_fun() {
    let mut sim = calm(BrainConfig::default());
    sim.open(window(7, RectI::new(600, 600, 1300, 1000), "explorer.exe"));
    sim.knight.pos = Vec2::new(960.0, 600.0);
    sim.knight.support = Some(Support { owner: Some(7), kind: LedgeKind::OneWay, y: 600.0, x0: 600.0, x1: 1300.0 });
    let mut rode = false;
    for _ in 0..(4.0 / DT) as i32 {
        if let Some(w) = sim.desk.windows.iter_mut().find(|w| w.id == 7) {
            w.rect.left += 2;
            w.rect.right += 2;
        }
        sim.desktop_changed();
        sim.events.push(MoveEvent::CarriedBy { owner: 7 });
        sim.step();
        rode |= sim.brain.behaviour() == BehaviourKind::Ride;
    }
    assert!(rode);
    assert!(sim.started.iter().any(|a| a == "windswept"), "acts: {:?}", sim.started);
}

#[test]
fn commitment_prevents_dithering() {
    let switches = dithering_switches(BrainConfig::default());
    let mut prev_at = f32::NEG_INFINITY;
    for s in &switches {
        if s.reason == SwitchReason::Outscored {
            assert!(s.at - prev_at >= BrainConfig::default().min_commit - 1e-3, "switched after {:.2}s: {s:?}", s.at - prev_at);
        }
        prev_at = s.at;
    }
    let outscored = |v: &[Switch]| v.iter().filter(|s| s.reason == SwitchReason::Outscored).count();
    // Same stimulus without commitment or hysteresis really does flip-flop.
    let loose = dithering_switches(BrainConfig { min_commit: 0.0, hysteresis: 0.0, ..Default::default() });
    assert!(outscored(&loose) >= 4 * outscored(&switches).max(1), "loose {} vs committed {}", outscored(&loose), outscored(&switches));
}

/// Excitement flips every 0.5 s so that Watch and Idle utilities keep crossing (by about
/// ±0.1); returns the switches made over a minute.
fn dithering_switches(cfg: BrainConfig) -> Vec<Switch> {
    let mut sim = calm(cfg);
    sim.cursor = Some(Cursor { pos: Vec2::new(200.0, 300.0), ..Default::default() });
    let n = (60.0 / DT) as usize;
    for i in 0..n {
        let m = sim.brain.mind_mut();
        m.needs.loneliness = 0.67;
        m.emotions.excitement = if (i / 15) % 2 == 0 { 0.8 } else { 0.0 };
        sim.step();
    }
    sim.switches
}

#[test]
fn same_seed_same_percepts_same_intents() {
    let script = |sim: &mut Sim| {
        sim.run(2.0);
        sim.open(window(10, RectI::new(300, 500, 900, 900), "notepad.exe"));
        sim.run(5.0);
        sim.park_cursor(Vec2::new(60.0, -10.0));
        sim.run(10.0);
        sim.intents.iter().map(|i| i.debug.clone()).collect::<Vec<_>>()
    };
    let a = script(&mut calm(BrainConfig::default()));
    let b = script(&mut calm(BrainConfig::default()));
    assert_eq!(a, b);
}

#[test]
fn seeds_make_different_personalities() {
    assert_ne!(Personality::from_seed(1), Personality::from_seed(2));
    let p = Personality::from_seed(99);
    for v in [p.curiosity, p.playfulness, p.sociability, p.vigour, p.boldness, p.skittishness] {
        assert!((0.05..=0.95).contains(&v));
    }
}

#[test]
fn config_deserializes_partially() {
    let cfg: BrainConfig = serde_json::from_str(r#"{ "bother_level": 0.2, "battle": { "win_hits": 3 } }"#).expect("json");
    assert_eq!(cfg.bother_level, 0.2);
    assert_eq!(cfg.battle.win_hits, 3);
    assert_eq!(cfg.battle.lose_hits, BattleConfig::default().lose_hits);
}

#[test]
fn every_named_act_exists() {
    let cat = catalog();
    for name in ["nap", "challenge", "startled", "windswept", "look_up", "look_down", "stare_at_user", "recoil", "stunned", "bow_down", "victory"] {
        assert!(cat.id(name).is_some(), "missing act '{name}'");
    }
    for d in [AttackDir::Forward, AttackDir::Up, AttackDir::Down] {
        assert!(cat.id(d.act_name()).is_some());
    }
}

const BENCH: Vec2 = Vec2::new(1300.0, FLOOR);

/// Exhausted knight, user away, one bench on the floor.
fn sleepy_with_bench() -> Sim {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.energy = 0.1;
    sim.user.idle_secs = 900.0;
    sim.benches.push(BENCH);
    sim
}

#[test]
fn without_a_bench_it_sleeps_face_down() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.energy = 0.1;
    sim.user.idle_secs = 900.0;
    assert!(sim.run_until(3.0, BehaviourKind::Nap));
    sim.run(30.0);
    assert!(sim.nav.gos.is_empty(), "sleeps on the spot");
    let deep = sim.started.iter().filter(|a| *a == "bow_down").count();
    assert!(deep >= 2, "deep sleep repeats while the user is away: {:?}", sim.started);
    assert!(!sim.started.iter().any(|a| a == "nap"), "bench nap needs a bench");
}

#[test]
fn bench_acts_only_at_a_bench_and_bow_down_never_idle() {
    let bench_acts = ["rest", "lean_back", "nap", "read_map"];
    let mut sim = calm(BrainConfig::default());
    sim.run(180.0);
    assert!(!sim.started.is_empty());
    assert!(!sim.started.iter().any(|a| bench_acts.contains(&a.as_str()) || a == "bow_down"), "acts: {:?}", sim.started);

    // Standing on a bench: sitting acts become available.
    let mut sim = calm(BrainConfig::default());
    sim.benches.push(sim.knight.pos);
    sim.nav.trip = 1e9; // never leaves the bench
    sim.run(180.0);
    assert!(sim.started.iter().any(|a| bench_acts.contains(&a.as_str())), "acts: {:?}", sim.started);
}

#[test]
fn losing_a_battle_knocks_the_knight_out() {
    let mut sim = parked_cursor_sim(BrainConfig { battle: BattleConfig { win_hits: 99, ..Default::default() }, ..Default::default() });
    assert!(sim.run_until(4.0, BehaviourKind::Battle));
    sim.run(2.0);
    for _ in 0..3 {
        sim.pokes.push(Poke::Click { pos: sim.body_center() });
        sim.run(2.0);
    }
    sim.run(2.0);
    assert!(sim.started.iter().any(|a| a == "bow_down"), "acts: {:?}", sim.started);
    assert!(sim.brain.mind().emotions.hurt > 0.5);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Battle);
}

#[test]
fn huge_fall_knocks_the_knight_out() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.events.push(MoveEvent::Landed { hard: true });
    assert!(sim.run_until(0.5, BehaviourKind::KnockedOut));
    sim.run(1.0);
    assert_eq!(sim.started.last().map(String::as_str), Some("bow_down"));
}
