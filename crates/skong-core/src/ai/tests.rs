//! Scripted percept sequences → expected behaviour (and act) choices, each with its
//! reason. Navigation is faked (routes "arrive" after `trip` seconds and put the knight on
//! the goal's ledge), acts are emulated with durations close to the real ones, and a
//! "wall" switch makes the knight press without moving.
use std::path::Path;

use super::*;
use crate::acts::ActLibrary;
use crate::anim::ClipLibrary;
use crate::assets::SpriteBank;
use crate::controller::{Abilities, Character, CharacterParams, MoveEvent, MoveState, Support};
use crate::desktop::{DesktopSnapshot, Monitor, WindowId, WindowInfo};
use crate::geom::RectI;
use crate::terrain::{LedgeKind, Terrain, TerrainOptions};

mod greet;
mod shiny;
mod train;
mod slug;

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
    /// Routes to these goals (with this style, or any if `None`) end elsewhere, e.g. under
    /// a ledge the style cannot mount.
    ends: Vec<(Vec2, Option<TravelStyle>, Vec2)>,
    /// Hide those ends from `reach` (a planner that over-promises).
    lies: bool,
}

impl FakeNav {
    fn end_of(&self, goal: NavGoal, style: TravelStyle) -> Option<Vec2> {
        match goal {
            NavGoal::Point(p) => Some(
                self.ends.iter().find(|(g, s, _)| g.distance(p) < 1.0 && s.is_none_or(|s| s == style)).map_or(p, |e| e.2),
            ),
            NavGoal::OnWindow(_) => None,
        }
    }
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
    fn reach(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<Reach> {
        let eta = self.estimate(from, goal, style)?;
        let end = if self.lies { None } else { self.end_of(goal, style) };
        Some(Reach { eta, end })
    }
    fn stop(&mut self) {
        self.status = NavStatus::Idle;
    }
    fn status(&self) -> NavStatus {
        self.status
    }
}

/// Rough real durations (s) of the acts, for a believable time budget.
fn act_secs(name: &str) -> f32 {
    match name {
        "nap" => 600.0,
        "slug_crawl" => 20.0,
        "rest" => 12.0,
        "lean_back" => 10.0,
        "read_map" | "map_stroll" => 7.0,
        "bow_down" | "lantern_patrol" => 6.0,
        "gaze_into_screen" | "hurt_idle" | "windswept" => 4.5,
        "look_up" | "look_down" | "pick_up_trinket" => 3.0,
        "stare_at_user" | "focus" => 2.5,
        "slash" | "slash_alt" | "up_slash" | "down_slash" | "recoil" => 0.3,
        _ => 1.5,
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
    activity: Activity,
    cursor: Option<Cursor>,
    user: UserActivity,
    pokes: Vec<Poke>,
    events: Vec<MoveEvent>,
    act: Option<PlayingAct>,
    /// A wall in front: pressing a direction doesn't move the knight.
    wall: bool,
    /// Force the knight's horizontal velocity (oscillation tests).
    vel_override: Option<f32>,
    dark: bool,
    last_input: MoveInput,
    last_motion: Motion,
    started: Vec<String>,
    intents: Vec<Intent>,
    switches: Vec<Switch>,
    t: f32,
    /// Shinies on the ground (the app's side: one in his hand at the pick-up act is gone).
    shinies: Vec<KnownShiny>,
    /// Shinies picked up.
    picked: Vec<u64>,
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

fn spot(rect: RectI, intensity: f32, age: f32) -> ActivitySpot {
    ActivitySpot { rect, center: rect.to_f().center(), intensity, age_secs: age, owner: None }
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
            activity: Activity::default(),
            cursor: None,
            user: UserActivity { idle_secs: 5.0, keys_per_sec: 0.0 },
            pokes: vec![],
            events: vec![],
            act: None,
            wall: false,
            vel_override: None,
            dark: false,
            last_input: MoveInput::default(),
            last_motion: Motion::Hold,
            started: vec![],
            intents: vec![],
            switches: vec![],
            t: 0.0,
            shinies: vec![],
            picked: vec![],
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

    /// Put the knight at `p`, standing on the ledge there if there is one.
    fn place(&mut self, p: Vec2) {
        let ledge = self.terrain.ledges.iter().find(|l| (l.y - p.y).abs() < 2.0 && l.x0 <= p.x && p.x <= l.x1).copied();
        match ledge {
            Some(l) => {
                self.knight.pos = Vec2::new(p.x, l.y);
                self.knight.support = Some(Support::from(l));
                self.knight.state = MoveState::Grounded;
            }
            None => self.knight.pos.x = p.x,
        }
    }

    fn goal_point(&self, goal: NavGoal) -> Vec2 {
        match goal {
            NavGoal::Point(p) => p,
            NavGoal::OnWindow(id) => self.terrain.window_rect(id).map_or(self.knight.pos, |r| Vec2::new((r.left + r.right) * 0.5, r.top)),
        }
    }

    fn act_name(&self, id: ActId) -> String {
        self.catalog.get(id).map(|a| a.name.clone()).unwrap_or_default()
    }

    fn step(&mut self) -> Intent {
        // What the app would have applied last step.
        self.last_input = match self.last_motion {
            Motion::Hold => MoveInput::default(),
            Motion::Direct(i) => i,
            Motion::Navigate { goal, .. } => {
                let dx = self.goal_point(goal).x - self.knight.pos.x;
                MoveInput { x: if dx.abs() > 1.0 { dx.signum() } else { 0.0 }, ..Default::default() }
            }
        };
        if self.act.is_some() {
            self.last_input = MoveInput::default();
        }
        self.knight.vel.x = match self.vel_override {
            Some(v) => v,
            None if self.wall => 0.0,
            None => self.last_input.x * 150.0,
        };
        let perch = self.knight.support.is_some_and(|s| s.kind != LedgeKind::Ground);
        let p = Percepts {
            dt: DT,
            knight: &self.knight,
            terrain: &self.terrain,
            desktop: &self.desk,
            cursor: self.cursor,
            user: self.user,
            fullscreen_front: false,
            pokes: &self.pokes,
            act: self.act.as_ref().map(|a| a.act),
            benches: &self.benches,
            perch,
            move_events: &self.events,
            activity: &self.activity,
            last_input: self.last_input,
            local_hour: None,
            dark: self.dark,
        };
        self.brain.set_shinies(&self.shinies);
        let intent = self.brain.tick(&p, &mut self.nav);
        self.last_motion = intent.motion;
        self.pokes.clear();
        self.events.clear();
        self.t += DT;
        self.user.idle_secs += DT;

        // Act player emulation.
        if intent.cancel_act && self.act.as_ref().is_some_and(|a| a.act.interruptible) {
            self.act = None;
        }
        // Like the app: a graceful exit replaces the running act with its short exit act.
        if let Some(exit) = intent.exit_act
            && self.act.as_ref().is_some_and(|a| a.act.interruptible)
        {
            self.started.push(self.act_name(exit));
            self.act = Some(PlayingAct { act: RunningAct { id: exit, interruptible: false }, left: 0.4 });
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
            let left = act_secs(&name);
            self.started.push(name);
            self.act = Some(PlayingAct { act: RunningAct { id, interruptible: true }, left });
        }
        // The app: the shiny being picked up is gone once the act is under way.
        if let Some(DiscoveryEvent::PickUp(id)) = intent.discovery
            && self.act.as_ref().is_some_and(|a| self.catalog.get(a.act.id).is_some_and(|i| i.name == "pick_up_trinket"))
            && let Some(i) = self.shinies.iter().position(|s| s.id == id)
        {
            self.shinies.remove(i);
            self.picked.push(id);
        }
        if let Some(a) = &mut self.act {
            a.left -= DT;
            if a.left <= 0.0 {
                self.act = None;
            }
        }

        // Every route arrives after `trip` seconds (unless a wall is in the way).
        if self.nav.status == NavStatus::Moving && !self.wall {
            self.nav.elapsed += DT;
            if self.nav.elapsed >= self.nav.trip {
                self.nav.status = NavStatus::Arrived;
                if let Some(&(goal, style)) = self.nav.gos.last() {
                    let p = self.nav.end_of(goal, style).unwrap_or_else(|| self.goal_point(goal));
                    self.place(p);
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

    /// Step while `cond` holds; `true` if it stopped holding within `secs`.
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

    fn started_since(&self, n: usize) -> &[String] {
        &self.started[n.min(self.started.len())..]
    }

    fn last_reason(&self) -> &str {
        self.intents.last().map_or("", |i| i.reason.as_str())
    }
}

/// Calm, rested, a bit bored; no cursor.
fn calm(cfg: BrainConfig) -> Sim {
    let mut sim = Sim::new(cfg);
    let m = sim.brain.mind_mut();
    m.needs = Needs { energy: 0.85, boredom: 0.5, loneliness: 0.2, mischief: 0.0 };
    m.emotions = Emotions::default();
    sim
}

/// A window whose top (y = 400) is a perch.
fn with_window(mut sim: Sim) -> Sim {
    sim.open(window(1, RectI::new(200, 400, 900, 900), "editor.exe"));
    sim
}

// --- Activity watching ----------------------------------------------------------------

#[test]
fn activity_is_watched_from_its_windows_title_bar() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.activity.spots = vec![spot(RectI::new(400, 500, 600, 600), 0.8, 1.0)];
    assert!(sim.run_until(3.0, BehaviourKind::Watch), "{}", sim.last_reason());
    assert!(sim.last_reason().starts_with("watch activity @ (500,550) 0.8"), "{}", sim.last_reason());
    sim.step();
    assert_eq!(sim.nav.gos.last().map(|g| g.0), Some(NavGoal::Point(Vec2::new(500.0, 400.0))), "vantage = title bar above the spot");
    let n = sim.started.len();
    sim.run(6.0);
    assert!(sim.started_since(n).iter().any(|a| a == "look_down"), "acts: {:?}", sim.started_since(n));
}

#[test]
fn watching_lasts_while_interesting_then_moves_on() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.activity.spots = vec![spot(RectI::new(400, 500, 600, 600), 0.8, 1.0)];
    assert!(sim.run_until(3.0, BehaviourKind::Watch));
    let start = sim.t;
    assert!(sim.run_while(40.0, |s| s.brain.behaviour() == BehaviourKind::Watch), "never got bored of it");
    let watched = sim.t - start;
    let w = WatchConfig::default();
    assert!(watched >= w.min_secs && watched <= w.max_secs + 12.0 + 3.0, "watched {watched:.1}s");
    // Satisfied: does not go straight back to the same spot.
    let kinds = sim.run(20.0);
    assert!(!kinds.contains(&BehaviourKind::Watch), "re-watched a satisfied spot");
}

#[test]
fn watching_stops_when_the_spot_goes_quiet() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.activity.spots = vec![spot(RectI::new(400, 500, 600, 600), 0.8, 1.0)];
    assert!(sim.run_until(3.0, BehaviourKind::Watch));
    sim.run(3.0);
    sim.activity.spots.clear();
    assert!(sim.run_while(3.5, |s| s.brain.behaviour() == BehaviourKind::Watch));
}

#[test]
fn long_lived_activity_is_watched_sitting_down() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.run(1.0);
    sim.activity.spots = vec![spot(RectI::new(400, 500, 600, 600), 0.9, 20.0)];
    assert!(sim.run_until(3.0, BehaviourKind::Watch));
    let n = sim.started.len();
    sim.run(10.0);
    let acts = sim.started_since(n);
    assert!(acts.iter().any(|a| a == "rest" || a == "lean_back"), "acts: {acts:?}");
}

#[test]
fn activity_behind_him_is_gazed_at_and_activity_above_looked_up_at() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    // A big changing region right behind the knight (video in a window behind him).
    sim.activity.spots = vec![spot(RectI::new(800, 900, 1100, 1060), 0.8, 1.0)];
    assert!(sim.run_until(3.0, BehaviourKind::Watch));
    sim.run(4.0);
    assert!(sim.started.iter().any(|a| a == "gaze_into_screen"), "acts: {:?}", sim.started);

    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.activity.spots = vec![spot(RectI::new(850, 150, 1050, 300), 0.8, 1.0)];
    assert!(sim.run_until(3.0, BehaviourKind::Watch));
    sim.run(6.0);
    assert!(sim.started.iter().any(|a| a == "look_up"), "acts: {:?}", sim.started);
}

// --- Cursor companion -----------------------------------------------------------------

#[test]
fn companion_follows_the_cursor_and_stays_near() {
    let mut sim = calm(BrainConfig::default());
    sim.cursor = Some(Cursor { pos: Vec2::new(1000.0, 900.0), ..Default::default() });
    assert!(sim.run_until(1.0, BehaviourKind::Companion));
    sim.run(2.0);
    assert!(sim.nav.gos.is_empty(), "already close: no walking");
    // Cursor drifts a little: stroll after it.
    sim.cursor = Some(Cursor { pos: Vec2::new(1500.0, 900.0), ..Default::default() });
    sim.run(1.5);
    let (goal, style) = *sim.nav.gos.last().expect("follows");
    assert_eq!(style, TravelStyle::Stroll);
    assert!(sim.last_reason().starts_with("companion: following cursor"), "{}", sim.last_reason());
    let g = sim.goal_point(goal);
    assert!((g.x - 1500.0).abs() <= 160.0 && g.y == FLOOR, "goal {g:?}: beside the cursor's projection");
    sim.run(3.0);
    assert!((sim.knight.pos.x - 1500.0).abs() < 240.0);
    // Far away: hurry.
    sim.cursor = Some(Cursor { pos: Vec2::new(100.0, 900.0), ..Default::default() });
    sim.knight.pos.x = 1900.0;
    sim.run(1.5);
    assert_eq!(sim.nav.gos.last().map(|g| g.1), Some(TravelStyle::Hurry));
}

#[test]
fn polite_companion_keeps_its_distance_and_never_hurries() {
    let mut sim = calm(BrainConfig { bother_level: 0.0, ..Default::default() });
    sim.cursor = Some(Cursor { pos: Vec2::new(1300.0, 900.0), ..Default::default() });
    sim.run(3.0);
    assert!(sim.nav.gos.is_empty(), "340 px is close enough for a polite knight");
    sim.cursor = Some(Cursor { pos: Vec2::new(100.0, 900.0), ..Default::default() });
    sim.knight.pos.x = 1900.0;
    sim.run(2.0);
    let (goal, style) = *sim.nav.gos.last().expect("follows");
    assert_ne!(style, TravelStyle::Hurry);
    assert!(sim.goal_point(goal).x >= 100.0 + 200.0, "stands well beside the cursor");
}

#[test]
fn companion_stays_put_while_the_user_types() {
    let mut sim = calm(BrainConfig::default());
    sim.user.keys_per_sec = 5.0;
    sim.cursor = Some(Cursor { pos: Vec2::new(200.0, 900.0), ..Default::default() });
    sim.run(5.0);
    assert!(sim.nav.gos.is_empty());
    assert!(sim.last_reason().contains("typing"), "{}", sim.last_reason());
}

#[test]
fn companion_looks_up_at_a_cursor_above() {
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(60.0, -300.0));
    sim.run(5.0);
    assert!(sim.started.iter().any(|a| a == "look_up"), "acts: {:?}", sim.started);
    assert!(sim.nav.gos.is_empty(), "cursor above but close horizontally: no need to move");
}

#[test]
fn returning_user_is_greeted_with_a_stare() {
    let mut sim = calm(BrainConfig::default());
    sim.user.idle_secs = 900.0;
    sim.run(2.0);
    sim.user.idle_secs = 0.0;
    sim.park_cursor(Vec2::new(150.0, -40.0));
    sim.run(14.0);
    assert!(sim.started.iter().any(|a| a == "stare_at_user"), "acts: {:?}", sim.started);
}

#[test]
fn parked_cursor_makes_him_settle_on_a_nearby_perch() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.cursor = Some(Cursor { pos: Vec2::new(700.0, 300.0), ..Default::default() });
    sim.run(30.0);
    assert!(sim.knight.pos.y == 400.0, "went up onto the window top near the cursor: {:?}", sim.knight.pos);
    assert!(sim.started.iter().any(|a| a == "lean_back" || a == "rest"), "acts: {:?}", sim.started);
}

#[test]
fn night_brings_out_the_lantern() {
    let mut sim = calm(BrainConfig::default());
    sim.dark = true;
    sim.park_cursor(Vec2::new(100.0, -20.0));
    sim.run(120.0);
    assert!(sim.started.iter().any(|a| a == "lantern_patrol"), "acts: {:?}", sim.started);
}

// --- Exploring windows ------------------------------------------------------------------

#[test]
fn new_window_is_explored_at_a_stroll_when_calm() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.open(window(10, RectI::new(300, 500, 900, 900), "notepad.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    let poi = sim.brain.exploring().expect("exploring");
    assert_eq!(poi.kind, PoiKind::NewWindow);
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
fn a_new_window_gets_mapped_and_remembered() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.open(window(10, RectI::new(300, 500, 900, 900), "notepad.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    assert!(sim.run_while(15.0, |s| s.brain.behaviour() == BehaviourKind::Explore));
    assert!(sim.started.iter().any(|a| a == "read_map"), "perched on it, so maps it seated: {:?}", sim.started);
    sim.run(20.0);
    let again = sim.nav.gos.iter().filter(|(g, _)| *g == NavGoal::OnWindow(10)).count();
    assert_eq!(again, 1, "visited window is not re-explored right away");
}

#[test]
fn a_small_new_window_is_a_trinket() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.open(window(11, RectI::new(700, 700, 1000, 900), "toast.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    sim.run(6.0);
    assert!(sim.started.iter().any(|a| a == "pick_up_trinket"), "acts: {:?}", sim.started);
}

// --- Rest & sleep -----------------------------------------------------------------------

#[test]
fn tired_knight_finds_a_perch_and_sits() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.brain.mind_mut().needs.energy = 0.35;
    assert!(sim.run_until(3.0, BehaviourKind::Rest), "{}", sim.last_reason());
    sim.run(5.0);
    assert_eq!(sim.knight.pos.y, 400.0, "walked to the window top");
    assert!(sim.started.iter().any(|a| a == "rest"), "acts: {:?}", sim.started);
}

const BENCH: Vec2 = Vec2::new(1300.0, FLOOR);

/// Exhausted knight, user away, one bench on the floor.
fn sleepy_with_bench() -> Sim {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.energy = 0.05;
    sim.user.idle_secs = 900.0;
    sim.benches.push(BENCH);
    sim
}

#[test]
fn tired_with_idle_user_walks_to_bench_and_naps() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_until(3.0, BehaviourKind::Nap), "{}", sim.last_reason());
    sim.step();
    assert_eq!(sim.nav.gos.last(), Some(&(NavGoal::Point(BENCH), TravelStyle::Stroll)), "walks to the bench first");
    sim.run(4.0);
    assert_eq!(sim.started.last().map(String::as_str), Some("nap"));
    let kinds = sim.run(30.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Nap));
}

#[test]
fn without_a_seat_it_sleeps_face_down() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.energy = 0.05;
    sim.user.idle_secs = 900.0;
    assert!(sim.run_until(3.0, BehaviourKind::Nap));
    sim.run(30.0);
    assert!(sim.nav.gos.is_empty(), "sleeps on the spot");
    assert!(sim.started.iter().filter(|a| *a == "bow_down").count() >= 2, "{:?}", sim.started);
}

#[test]
fn poke_during_nap_wakes_and_startles() {
    let mut sim = sleepy_with_bench();
    assert!(sim.run_until(3.0, BehaviourKind::Nap));
    sim.run(8.0);
    assert_eq!(sim.started.last().map(String::as_str), Some("nap"));
    sim.pokes.push(Poke::Click { pos: sim.body_center() });
    let intent = sim.step();
    assert_eq!(intent.signal, Some("woken"));
    assert!(sim.brain.mind().emotions.startle > 0.5);
    assert!(sim.run_until(1.0, BehaviourKind::Flee));
    sim.run(4.0);
    assert!(sim.started.iter().any(|a| a == "startled"), "acts: {:?}", sim.started);
}

// --- Wall pushing -----------------------------------------------------------------------

#[test]
fn pushing_into_a_wall_is_noticed_and_the_goal_dropped() {
    let mut sim = calm(BrainConfig::default());
    sim.cursor = Some(Cursor { pos: Vec2::new(1000.0, 900.0), ..Default::default() });
    sim.run(2.0);
    sim.wall = true;
    sim.cursor = Some(Cursor { pos: Vec2::new(1700.0, 900.0), ..Default::default() });
    assert!(sim.run_while(3.0, |s| s.brain.stuck_count() == 0), "never noticed the wall");
    let i = sim.intents.last().expect("intent");
    assert!(matches!(i.motion, Motion::Hold), "stopped pushing");
    assert_eq!(i.face, Some(Side::Left), "turned away from the wall");
    let gos = sim.nav.gos.len();
    sim.run(5.0);
    assert_eq!(sim.nav.gos.len(), gos, "doesn't retry the blocked goal right away");
    let pushing = sim.intents.iter().rev().take((5.0 / DT) as usize).filter(|i| !matches!(i.motion, Motion::Hold)).count();
    assert_eq!(pushing, 0);
}

#[test]
fn sliding_down_a_wall_on_a_stroll_counts_as_stuck() {
    let mut sim = calm(BrainConfig::default());
    sim.cursor = Some(Cursor { pos: Vec2::new(1000.0, 900.0), ..Default::default() });
    sim.run(1.0);
    // Far enough to follow, close enough to stroll.
    sim.cursor = Some(Cursor { pos: Vec2::new(1600.0, 900.0), ..Default::default() });
    assert!(sim.run_while(2.0, |s| !matches!(s.last_motion, Motion::Navigate { .. })));
    sim.events.push(MoveEvent::WallSlideStart { side: Side::Right });
    sim.step();
    assert_eq!(sim.brain.stuck_count(), 1);
    assert!(matches!(sim.intents.last().map(|i| i.motion), Some(Motion::Hold)));
}

// --- Reactions ---------------------------------------------------------------------------

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
    assert!(sim.run_until(4.0, BehaviourKind::Battle), "{}", sim.last_reason());
    sim.run(8.0);
    assert!(sim.started.iter().any(|a| a == "challenge"), "acts: {:?}", sim.started);
    assert!(sim.intents.iter().filter(|i| i.attack.is_some()).count() >= 5);
    assert!(sim.started.iter().any(|a| a == "victory"), "acts: {:?}", sim.started);
}

#[test]
fn bother_level_zero_never_battles() {
    let mut sim = parked_cursor_sim(BrainConfig { bother_level: 0.0, ..Default::default() });
    let kinds = sim.run(120.0);
    assert!(!kinds.contains(&BehaviourKind::Battle));
}

#[test]
fn typing_user_is_never_battled() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    sim.user.keys_per_sec = 4.0;
    let kinds = sim.run(60.0);
    assert!(!kinds.contains(&BehaviourKind::Battle));
}

#[test]
fn losing_a_battle_knocks_out_then_heals_with_focus() {
    let mut sim = parked_cursor_sim(BrainConfig { battle: BattleConfig { win_hits: 99, ..Default::default() }, ..Default::default() });
    assert!(sim.run_until(4.0, BehaviourKind::Battle));
    sim.run(2.0);
    for _ in 0..3 {
        sim.pokes.push(Poke::Click { pos: sim.body_center() });
        sim.run(2.0);
    }
    sim.run(20.0);
    let s = &sim.started;
    let ko = s.iter().position(|a| a == "bow_down").expect("knocked out");
    assert!(s[ko..].iter().any(|a| a == "hurt_idle") && s[ko..].iter().any(|a| a == "focus"), "acts: {s:?}");
}

#[test]
fn an_unbroken_fall_is_fatal() {
    // Fall damage kills (a dive or down-dash lands softly and never reports a hard landing):
    // he dies, and leaves a Shade to go and fight.
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.events.push(MoveEvent::Landed { hard: true });
    assert!(sim.run_until(0.5, BehaviourKind::Dead), "{}", sim.last_reason());
}

#[test]
fn fullscreen_app_sends_knight_into_hiding_when_configured() {
    let mut sim = calm(BrainConfig { hide_for_fullscreen: true, ..Default::default() });
    sim.knight.pos.x = 1500.0;
    sim.run(1.0);
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "game.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    assert!(sim.run_until(0.5, BehaviourKind::Hide));
    sim.step();
    assert_eq!(sim.nav.gos.last().map(|g| g.0), Some(NavGoal::Point(Vec2::new(1890.0, FLOOR))));
    sim.close(20);
    assert!(sim.run_while(1.0, |s| s.brain.behaviour() == BehaviourKind::Hide));
}

#[test]
fn fullscreen_app_is_ignored_by_default() {
    // Games are handled by the app (the knight leaves their monitor); other fullscreen
    // apps are fine to play around.
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "chrome.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    assert!(!sim.run_until(2.0, BehaviourKind::Hide), "non-game fullscreen apps must not send him into hiding");
}

#[test]
fn window_closing_underfoot_makes_it_flee() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.place(Vec2::new(500.0, 400.0));
    sim.run(1.0);
    sim.close(1);
    sim.knight.support = None;
    sim.knight.state = MoveState::Airborne;
    sim.events.push(MoveEvent::LostSupport);
    assert!(sim.run_until(0.5, BehaviourKind::Flee));
    sim.run(0.3);
    sim.place(Vec2::new(500.0, FLOOR));
    sim.run(3.0);
    assert!(sim.started.iter().any(|a| a == "startled"));
    assert_eq!(sim.nav.gos.last().map(|g| g.1), Some(TravelStyle::Hurry));
}

#[test]
fn riding_a_moving_window_is_fun() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.place(Vec2::new(500.0, 400.0));
    let mut rode = false;
    for _ in 0..(4.0 / DT) as i32 {
        if let Some(w) = sim.desk.windows.iter_mut().find(|w| w.id == 1) {
            w.rect.left += 2;
            w.rect.right += 2;
        }
        sim.desktop_changed();
        sim.events.push(MoveEvent::PlatformMoved { owner: 1 });
        sim.step();
        rode |= sim.brain.behaviour() == BehaviourKind::Ride;
    }
    assert!(rode);
    assert!(sim.started.iter().any(|a| a == "windswept"), "acts: {:?}", sim.started);
}

// --- Balance & stability ----------------------------------------------------------------

/// Acts the knight may start with nobody doing anything special.
const QUIET_ACTS: [&str; 15] = ["dream_nail", "gaze_into_screen", "look_up", "look_down", "stare_at_user", "rest", "lean_back", "read_map", "map_stroll", "pick_up_trinket", "get_up", "look_up_end", "look_down_end", "turn_from_screen", "put_map_away"];

#[test]
fn quiet_five_minutes_are_mostly_purposeful() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.open(window(2, RectI::new(1100, 600, 1700, 1000), "browser.exe"));
    let spots = [Vec2::new(700.0, 300.0), Vec2::new(1400.0, 500.0), Vec2::new(400.0, 950.0), Vec2::new(1600.0, 950.0)];
    let mut waiting = 0usize;
    let mut ticks = 0usize;
    for i in 0..10 {
        sim.cursor = Some(Cursor { pos: spots[i % spots.len()], ..Default::default() });
        sim.user.idle_secs = 0.0;
        for _ in 0..(30.0 / DT) as usize {
            let intent = sim.step();
            ticks += 1;
            waiting += intent.waiting as usize;
        }
    }
    let ratio = waiting as f32 / ticks as f32;
    assert!(ratio < 0.3, "standing around {:.0}% of the time; acts {:?}", ratio * 100.0, sim.started);
    assert!(sim.started.iter().all(|a| QUIET_ACTS.contains(&a.as_str())), "out-of-context act: {:?}", sim.started);
    assert!(sim.started.iter().any(|a| a == "rest" || a == "lean_back"), "never sat: {:?}", sim.started);
    let fidgets = sim.intents.iter().filter(|i| i.reason.contains("(fidget)")).count();
    // (Graceful exits — getting up, putting the map away — keep the last fidget's reason for
    // their 0.4 s, hence a little headroom.)
    assert!((fidgets as f32) < 0.18 * ticks as f32, "fidgeting {:.0}% of the time", 100.0 * fidgets as f32 / ticks as f32);
}

#[test]
fn reasons_are_always_given() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.park_cursor(Vec2::new(300.0, -100.0));
    sim.run(20.0);
    sim.activity.spots = vec![spot(RectI::new(400, 500, 600, 600), 0.8, 1.0)];
    sim.run(20.0);
    assert!(sim.intents.iter().all(|i| !i.reason.is_empty() && i.debug.starts_with(&i.reason)));
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

/// Excitement flips every 0.5 s so that Rest (tired) and Companion utilities keep crossing.
fn dithering_switches(cfg: BrainConfig) -> Vec<Switch> {
    let mut sim = with_window(calm(cfg));
    sim.cursor = Some(Cursor { pos: Vec2::new(1000.0, 900.0), ..Default::default() });
    sim.nav.trip = 1e9; // never arrives: only the choice matters
    let n = (60.0 / DT) as usize;
    for i in 0..n {
        let m = sim.brain.mind_mut();
        m.needs.energy = 0.38;
        m.needs.loneliness = 0.3;
        m.emotions.excitement = if (i / 15) % 2 == 0 { 0.8 } else { 0.0 };
        sim.user.idle_secs = 0.0;
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
        sim.activity.spots = vec![spot(RectI::new(400, 600, 600, 700), 0.7, 1.0)];
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
    let cfg: BrainConfig = serde_json::from_str(r#"{ "bother_level": 0.2, "battle": { "win_hits": 3 }, "watch": { "max_secs": 20 } }"#).expect("json");
    assert_eq!(cfg.bother_level, 0.2);
    assert_eq!(cfg.battle.win_hits, 3);
    assert_eq!(cfg.watch.max_secs, 20.0);
    assert_eq!(cfg.watch.min_secs, WatchConfig::default().min_secs);
}

#[test]
fn every_named_act_exists_and_only_fidgets_are_random() {
    let cat = catalog();
    for name in [
        "nap", "rest", "lean_back", "read_map", "map_stroll", "challenge", "startled", "windswept", "look_up", "look_down", "gaze_into_screen",
        "stare_at_user", "pick_up_trinket", "lantern_patrol", "focus", "hurt_idle", "recoil", "stunned", "bow_down", "victory",
    ] {
        assert!(cat.id(name).is_some(), "missing act '{name}'");
    }
    for d in [AttackDir::Forward, AttackDir::Up, AttackDir::Down] {
        assert!(cat.id(d.act_name()).is_some());
    }
    for a in cat.iter().filter(|a| a.weight > 0.0) {
        assert!(a.has_tag("fidget"), "'{}' would be picked at random", a.name);
    }
}

// --- Ping-pong regressions -----------------------------------------------------------------

/// Seat height of the low content ledges (40 px above the floor: perchable, but a stroll
/// can't get up there).
const LOW_SEAT_Y: f32 = FLOOR - 40.0;

impl Sim {
    /// A content ledge (drawn line, table border) the knight could sit on.
    fn add_content_ledge(&mut self, x0: f32, x1: f32, y: f32) {
        self.terrain.ledges.push(crate::terrain::Ledge { x0, x1, y, kind: LedgeKind::Content, owner: None });
    }

    /// Navigation to `seat` ends on the floor below it (for `style`, or any style).
    fn unmountable(&mut self, seat: Vec2, style: Option<TravelStyle>) {
        self.nav.ends.push((seat, style, Vec2::new(seat.x, FLOOR)));
    }

    fn seat_trips(&self, seat: Vec2) -> usize {
        self.nav.gos.iter().filter(|(g, _)| *g == NavGoal::Point(seat)).count()
    }
}

fn tired_between_low_seats() -> (Sim, Vec2, Vec2) {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.energy = 0.35;
    sim.add_content_ledge(560.0, 700.0, LOW_SEAT_Y);
    sim.add_content_ledge(1220.0, 1360.0, LOW_SEAT_Y);
    // Seats are chosen at the ledge point closest to the knight (x = 960).
    (sim, Vec2::new(680.0, LOW_SEAT_Y), Vec2::new(1240.0, LOW_SEAT_Y))
}

#[test]
fn seat_a_stroll_cannot_mount_is_reached_at_a_run() {
    let (mut sim, left, right) = tired_between_low_seats();
    sim.unmountable(right, None);
    sim.unmountable(left, Some(TravelStyle::Stroll));
    assert!(sim.run_until(3.0, BehaviourKind::Rest), "{}", sim.last_reason());
    sim.run(4.0);
    assert_eq!(sim.nav.gos.first(), Some(&(NavGoal::Point(left), TravelStyle::Normal)), "runs (and jumps) up instead of strolling underneath");
    assert_eq!(sim.knight.pos, left);
    assert!(sim.started.iter().any(|a| a == "rest"), "acts: {:?}", sim.started);
    assert_eq!(sim.seat_trips(right), 0);
}

#[test]
fn unreachable_seats_on_both_sides_cause_no_ping_pong() {
    let (mut sim, left, right) = tired_between_low_seats();
    sim.unmountable(left, None);
    sim.unmountable(right, None);
    let x0 = sim.knight.pos.x;
    sim.run(60.0);
    assert_eq!(sim.seat_trips(left) + sim.seat_trips(right), 0, "never walks to a seat it can't get onto: {:?}", sim.nav.gos);
    let rests = sim.switches.iter().filter(|s| s.to == BehaviourKind::Rest).count();
    assert!(rests <= 6, "rest re-picked {rests} times in a minute");
    assert_eq!(sim.knight.pos.x, x0, "no pacing");
    assert_eq!(sim.brain.oscillations(), 0);
}

#[test]
fn arriving_without_getting_seated_blocks_the_seat_and_cools_rest_down() {
    // The planner over-promises: the route claims to reach the seats but ends below.
    let (mut sim, left, right) = tired_between_low_seats();
    sim.unmountable(left, None);
    sim.unmountable(right, None);
    sim.nav.lies = true;
    sim.nav.trip = 1.0;
    sim.run(30.0);
    assert!(sim.seat_trips(left) <= 1 && sim.seat_trips(right) <= 1, "retried a seat it failed to sit on: {:?}", sim.nav.gos);
    let fails: Vec<f32> = sim.switches.iter().filter(|s| s.from == BehaviourKind::Rest).map(|s| s.at).collect();
    let rest_starts: Vec<f32> = sim.switches.iter().filter(|s| s.to == BehaviourKind::Rest).map(|s| s.at).collect();
    for (i, w) in rest_starts.windows(2).enumerate() {
        assert!(w[1] - w[0] >= 10.0, "rest re-picked {:.1}s after it ended (#{i}); ends {fails:?}", w[1] - w[0]);
    }
    assert_eq!(sim.brain.oscillations(), 0);
}

#[test]
fn oscillation_guard_calms_a_dithering_knight() {
    let mut sim = calm(BrainConfig::default());
    sim.cursor = Some(Cursor { pos: Vec2::new(1800.0, 900.0), ..Default::default() });
    sim.nav.trip = 1e9; // never arrives
    assert!(sim.run_while(3.0, |s| !matches!(s.last_motion, Motion::Navigate { .. })), "follows the cursor");
    sim.run(2.0);
    assert_eq!(sim.brain.oscillations(), 0, "guard must not fire while travelling steadily");
    // Something makes him flip between two targets: left, right, left, right... without
    // ever arriving.
    for i in 0..8 {
        assert!(matches!(sim.last_motion, Motion::Navigate { .. }), "still travelling");
        sim.cursor = Some(Cursor { pos: Vec2::new(if i % 2 == 0 { 200.0 } else { 1800.0 }, 900.0), ..Default::default() });
        sim.vel_override = Some(if i % 2 == 0 { -150.0 } else { 150.0 });
        sim.run(0.7);
        if sim.brain.oscillations() > 0 {
            break;
        }
    }
    sim.vel_override = None;
    assert_eq!(sim.brain.oscillations(), 1, "3 reversals within 6 s");
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Idle);
    assert!(sim.last_reason().starts_with("calming down"), "{}", sim.last_reason());
    let gos = sim.nav.gos.len();
    let calm = sim.run(3.5);
    assert!(calm.iter().all(|k| *k == BehaviourKind::Idle));
    assert_eq!(sim.nav.gos.len(), gos, "stands still while calming down");
    sim.run(6.0);
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Companion, "back to normal afterwards");
}

// --- Found places (discoveries) ---------------------------------------------------------

fn place(id: PlaceId, kind: PlaceKind, at: Vec2, favour: f32, fresh: bool) -> KnownPlace {
    KnownPlace { id, kind, at, center: at, favour, fresh }
}

fn discoveries(sim: &Sim) -> Vec<DiscoveryEvent> {
    sim.intents.iter().filter_map(|i| i.discovery).collect()
}

#[test]
fn exploring_a_new_window_reveals_and_he_reacts_then_sits() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.energy = 0.6;
    sim.run(1.0);
    sim.open(window(10, RectI::new(300, 500, 900, 900), "paint.exe"));
    assert!(sim.run_until(3.0, BehaviourKind::Explore));
    assert!(sim.run_while(8.0, |s| !discoveries(s).contains(&DiscoveryEvent::Explored(10))), "arrival asks the app to reveal the room");
    // The app reveals a bench on the title bar (and lists it as a bench).
    let bench = place(5, PlaceKind::Bench, Vec2::new(420.0, 500.0), 0.0, true);
    sim.benches.push(bench.at);
    sim.brain.set_places(&[bench]);
    assert!(sim.run_until(1.0, BehaviourKind::Visit), "reacts at once: {}", sim.last_reason());
    assert!(sim.last_reason().contains("found a bench"), "{}", sim.last_reason());
    sim.brain.set_places(&[KnownPlace { fresh: false, ..bench }]);
    assert!(sim.run_while(8.0, |s| s.act.as_ref().is_none_or(|a| s.act_name(a.act.id) != "lean_back")), "acts: {:?} / {}", sim.started, sim.last_reason());
    assert_eq!(sim.knight.pos, bench.at, "on the bench");
    assert!(discoveries(&sim).contains(&DiscoveryEvent::Used(5)));
    // Then he gets up and moves on.
    assert!(sim.run_while(60.0, |s| s.brain.behaviour() == BehaviourKind::Visit));
}

#[test]
fn tired_knight_prefers_a_found_bench_to_a_perch() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.brain.mind_mut().needs.energy = 0.32;
    let bench = Vec2::new(1350.0, FLOOR);
    sim.benches.push(bench); // the app lists found benches as benches too
    sim.brain.set_places(&[place(9, PlaceKind::Bench, bench, 0.0, false)]);
    assert!(sim.run_until(3.0, BehaviourKind::Visit), "{}", sim.last_reason());
    sim.step();
    assert_eq!(sim.nav.gos.last().map(|g| g.0), Some(NavGoal::Point(bench)));
    sim.run(5.0);
    assert!(sim.started.iter().any(|a| a == "rest"), "acts: {:?}", sim.started);
    assert!(discoveries(&sim).contains(&DiscoveryEvent::Used(9)));
    assert_eq!(sim.knight.pos, bench);
}

#[test]
fn sleepy_knight_naps_on_his_favourite_bench() {
    let mut sim = sleepy_with_bench(); // plain bench at 1300
    let fav = Vec2::new(500.0, FLOOR);
    sim.benches.push(fav);
    sim.brain.set_places(&[place(3, PlaceKind::Bench, fav, 0.8, false)]);
    assert!(sim.run_until(3.0, BehaviourKind::Nap), "{}", sim.last_reason());
    sim.step();
    assert_eq!(sim.nav.gos.last().map(|g| g.0), Some(NavGoal::Point(fav)), "longer walk, but it's the Paint bench");
}

#[test]
fn playful_knight_mines_a_geo_deposit() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.mischief = 0.8;
    let rock = KnownPlace { center: Vec2::new(1250.0, FLOOR), ..place(4, PlaceKind::Deposit, Vec2::new(1200.0, FLOOR), 0.0, false) };
    sim.brain.set_places(&[rock]);
    assert!(sim.run_until(3.0, BehaviourKind::Visit), "{}", sim.last_reason());
    let struck = |sim: &Sim| sim.intents.iter().filter(|i| i.discovery == Some(DiscoveryEvent::Struck(4))).count();
    // A rock that doesn't break (swings not registering): he keeps at it, up to 6 swings.
    sim.run(6.0);
    assert_eq!(struck(&sim), 6, "keeps swinging until it breaks");
    // A fresh rock that breaks on the 3rd swing: he stops right there.
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.mischief = 0.8;
    sim.brain.set_places(&[rock]);
    assert!(sim.run_until(3.0, BehaviourKind::Visit), "{}", sim.last_reason());
    for _ in 0..(6.0 / DT) as usize {
        sim.step();
        if struck(&sim) == 3 {
            sim.brain.set_places(&[]);
        }
    }
    assert_eq!(struck(&sim), 3, "stops once it broke");
    let strikes: Vec<&Intent> = sim.intents.iter().filter(|i| i.discovery == Some(DiscoveryEvent::Struck(4))).collect();
    assert!(strikes.iter().all(|i| i.attack.is_some_and(|a| a.side == Side::Right && a.dir == AttackDir::Forward)));
    assert!(sim.brain.mind().needs.mischief < 0.5, "got it out of his system");
}

#[test]
fn rested_calm_knight_leaves_found_seats_alone() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.set_places(&[place(1, PlaceKind::Bench, Vec2::new(1300.0, FLOOR), 0.5, false)]);
    let kinds = sim.run(20.0);
    assert!(!kinds.contains(&BehaviourKind::Visit));
}

#[test]
fn a_geo_deposit_is_worth_it_even_when_calm() {
    // Geo is one of his top priorities: a known deposit beats lounging around, calm or not.
    let mut sim = calm(BrainConfig::default());
    sim.brain.set_places(&[place(1, PlaceKind::Bench, Vec2::new(1300.0, FLOOR), 0.5, false), place(2, PlaceKind::Deposit, Vec2::new(600.0, FLOOR), 0.5, false)]);
    assert!(sim.run_until(5.0, BehaviourKind::Visit), "{}", sim.last_reason());
    assert_eq!(sim.brain.visiting().map(|p| p.kind), Some(PlaceKind::Deposit));
    assert!(sim.last_reason().contains("deposit"), "{}", sim.last_reason());
}


// --- Sit loop / boredom regressions --------------------------------------------------------

fn seated_starts(sim: &Sim) -> usize {
    sim.started.iter().filter(|a| matches!(a.as_str(), "lean_back" | "rest")).count()
}

fn requests(sim: &Sim) -> Vec<(f32, WorldEvent)> {
    sim.intents.iter().enumerate().filter_map(|(i, it)| it.request.map(|r| ((i + 1) as f32 * DT, r))).collect()
}

/// Run `secs` keeping boredom pinned at the maximum (the live state of the bug).
fn run_bored(sim: &mut Sim, secs: f32) {
    for _ in 0..(secs / DT) as usize {
        sim.brain.mind_mut().needs.boredom = 1.0;
        sim.user.idle_secs = 1.0;
        sim.step();
    }
}

#[test]
fn bored_stiff_with_a_parked_cursor_goes_exploring_instead_of_sitting() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.open(window(2, RectI::new(1100, 600, 1700, 1000), "browser.exe"));
    sim.place(Vec2::new(500.0, 400.0)); // perched on window 1's title bar
    sim.cursor = Some(Cursor { pos: Vec2::new(560.0, 330.0), ..Default::default() });
    run_bored(&mut sim, 120.0);
    let explored = sim.switches.iter().any(|s| s.to == BehaviourKind::Explore);
    assert!(explored || !requests(&sim).is_empty(), "no exploring and no event request");
    assert!(seated_starts(&sim) <= 2, "sat {} times in 2 min: {:?}", seated_starts(&sim), sim.started);
}

#[test]
fn bored_with_nothing_around_asks_for_a_world_event_rate_limited() {
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(150.0, -40.0));
    run_bored(&mut sim, 200.0);
    let req = requests(&sim);
    assert!(!req.is_empty(), "never asked for anything");
    assert!(req.len() <= 2, "{req:?}");
    for w in req.windows(2) {
        assert!(w[1].0 - w[0].0 >= BoredomConfig::default().event_gap - 0.1, "{req:?}");
    }
    // Never while the user is typing.
    let mut sim = calm(BrainConfig::default());
    sim.park_cursor(Vec2::new(150.0, -40.0));
    sim.user.keys_per_sec = 5.0;
    for _ in 0..(120.0 / DT) as usize {
        sim.brain.mind_mut().needs.boredom = 1.0;
        sim.step();
    }
    assert!(requests(&sim).is_empty());
}

#[test]
fn companion_target_stays_on_the_cursors_monitor() {
    let mut sim = calm(BrainConfig::default());
    // 1920x1080 monitor left of a 2560x1440 primary.
    sim.desk.monitors = vec![
        Monitor { rect: RectI::new(-1920, 0, 0, 1080), work: RectI::new(-1920, 0, 0, 1040), primary: false, scale_milli: 1000 },
        Monitor { rect: RectI::new(0, 0, 2560, 1440), work: RectI::new(0, 0, 2560, 1400), primary: true, scale_milli: 1000 },
    ];
    sim.desktop_changed();
    sim.place(Vec2::new(-684.0, FLOOR));
    sim.cursor = Some(Cursor { pos: Vec2::new(60.0, 1200.0), ..Default::default() });
    sim.run(8.0);
    let goals: Vec<Vec2> = sim.nav.gos.iter().map(|g| sim.goal_point(g.0)).collect();
    assert!(!goals.is_empty(), "follows the cursor to its monitor");
    for g in &goals {
        assert!(g.x >= 20.0 && (g.x - 60.0).abs() <= 450.0, "goal {g:?} must be beside the cursor, on its monitor");
    }
}

#[test]
fn no_re_sit_right_after_getting_up() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.brain.mind_mut().needs.boredom = 0.2;
    sim.place(Vec2::new(500.0, 400.0));
    sim.cursor = Some(Cursor { pos: Vec2::new(560.0, 330.0), ..Default::default() });
    sim.run(70.0);
    assert!(seated_starts(&sim) <= 1, "re-sat within the cooldown: {:?}", sim.started);
}

// --- Pestering -----------------------------------------------------------------------------

#[test]
fn asked_to_pester_he_shows_off_around_the_cursor_then_stops() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    sim.brain.pester_until = 1e9;
    sim.brain.battle_ready_at = 1e9;
    assert!(sim.run_until(4.0, BehaviourKind::Pester), "{}", sim.last_reason());
    let kinds = sim.run(100.0); // persistent: 45-90 s of play
    let jumps = sim.intents.iter().filter(|i| matches!(i.motion, Motion::Direct(m) if m.jump_pressed)).count();
    assert!(jumps >= 3, "jumps {jumps}");
    assert!(sim.intents.iter().all(|i| i.attack.is_none_or(|a| !a.hit)), "never lands a hit");
    assert!(kinds.iter().any(|k| *k != BehaviourKind::Pester), "never stopped: {kinds:?}");
}

#[test]
fn a_click_ends_the_pestering_without_a_fright() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    sim.brain.pester_until = 1e9;
    sim.brain.battle_ready_at = 1e9;
    assert!(sim.run_until(4.0, BehaviourKind::Pester));
    sim.run(1.0);
    sim.pokes.push(Poke::Click { pos: sim.body_center() });
    let kinds = sim.run(4.0);
    assert!(!kinds.contains(&BehaviourKind::Flee), "{kinds:?}");
    assert!(sim.started.iter().any(|a| a == "recoil"), "acts: {:?}", sim.started);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Pester);
}

#[test]
fn bother_level_zero_never_pesters() {
    let mut sim = parked_cursor_sim(BrainConfig { bother_level: 0.0, ..Default::default() });
    sim.brain.pester_until = 1e9;
    let kinds = sim.run(60.0);
    assert!(!kinds.contains(&BehaviourKind::Pester));
}

#[test]
fn a_wall_climb_is_not_dithering() {
    // Climbing kicks left, right, left... in the air: that's the route, not indecision. (The
    // guard used to cancel every climb a second in, so visitors up a wall were never reached.)
    let mut sim = calm(BrainConfig::default());
    sim.cursor = Some(Cursor { pos: Vec2::new(1800.0, 900.0), ..Default::default() });
    sim.nav.trip = 1e9;
    assert!(sim.run_while(3.0, |s| !matches!(s.last_motion, Motion::Navigate { .. })), "follows the cursor");
    let support = sim.knight.support.take();
    for i in 0..8 {
        sim.vel_override = Some(if i % 2 == 0 { -150.0 } else { 150.0 });
        sim.run(0.5);
    }
    sim.vel_override = None;
    sim.knight.support = support;
    assert_eq!(sim.brain.oscillations(), 0);
    assert!(matches!(sim.last_motion, Motion::Navigate { .. }), "still travelling");
}

#[test]
fn a_winding_route_to_one_deposit_is_not_dithering() {
    // The live bug: walking to a deposit, the route turned around on the ground a few times
    // (run-up, a climb on the far side). The guard cancelled the trip and blocked the
    // deposit, so he idled with Geo on screen.
    let mut sim = calm(BrainConfig::default());
    sim.brain.set_places(&[place(2, PlaceKind::Deposit, Vec2::new(1500.0, FLOOR), 0.5, false)]);
    assert!(sim.run_until(5.0, BehaviourKind::Visit), "{}", sim.last_reason());
    sim.nav.trip = 1e9;
    for i in 0..8 {
        sim.vel_override = Some(if i % 2 == 0 { -150.0 } else { 150.0 });
        sim.run(0.5);
    }
    sim.vel_override = None;
    assert_eq!(sim.brain.oscillations(), 0);
    assert_eq!(sim.brain.behaviour(), BehaviourKind::Visit);
}

#[test]
fn idling_he_now_and_then_checks_his_map() {
    // The user missed it: with nothing going on he pulls out his map for a stroll.
    let mut sim = calm(BrainConfig::default());
    sim.run(300.0);
    let maps = sim.started.iter().filter(|a| *a == "map_stroll").count();
    assert!(maps >= 1, "acts: {:?}", sim.started);
    assert!(maps <= 3, "a treat, not a habit: {maps}");
}

#[test]
fn fight_mode_fights_the_cursor_round_after_round_until_switched_off() {
    // Even a calm knight with mischief turned off fights when the user asks for it.
    let mut sim = calm(BrainConfig { bother_level: 0.0, ..Default::default() });
    sim.brain.mind_mut().needs.mischief = 0.0;
    sim.park_cursor(Vec2::new(80.0, -20.0));
    sim.brain.set_fight_mode(true);
    assert!(sim.run_until(2.0, BehaviourKind::Battle), "{}", sim.last_reason());
    // Rounds keep coming (a short breather between them).
    let kinds = sim.run(240.0);
    let rounds = kinds.windows(2).filter(|w| w[0] != BehaviourKind::Battle && w[1] == BehaviourKind::Battle).count();
    assert!(rounds >= 2, "fought again: {rounds} new rounds");
    sim.brain.set_fight_mode(false);
    sim.run(100.0);
    let kinds = sim.run(30.0);
    assert!(!kinds.contains(&BehaviourKind::Battle), "off again");
}

#[test]
fn seated_and_the_cursor_comes_over_he_stays_seated() {
    // The re-sit quirk: idle and sitting on a window edge, then the cursor turns up and
    // keeping company takes over. He used to get up and sit straight back down.
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.place(Vec2::new(500.0, 400.0));
    assert!(sim.run_while(60.0, |s| s.act.as_ref().is_none_or(|a| !matches!(s.act_name(a.act.id).as_str(), "lean_back" | "rest"))), "sat down: {:?}", sim.started);
    let before = sim.started.len();
    sim.cursor = Some(Cursor { pos: Vec2::new(560.0, 330.0), ..Default::default() });
    sim.run(3.0);
    let after: Vec<&String> = sim.started[before..].iter().collect();
    assert!(!after.iter().any(|a| *a == "get_up"), "got up for nothing: {after:?}");
}
