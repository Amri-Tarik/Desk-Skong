//! The stakes as a state machine: a lost battle → death → respawn at a bench; hunting the
//! Shade (the real [`Shade`] sim, a fake navigator) → reclaim → focus; dying to the Shade;
//! picking up Geo. Acts are emulated with rough durations.
use std::path::Path;

use super::*;
use crate::acts::ActLibrary;
use crate::anim::ClipLibrary;
use crate::assets::SpriteBank;
use crate::controller::{Abilities, Character, CharacterParams, MoveState, Support};
use crate::desktop::{DesktopSnapshot, Monitor};
use crate::geom::{RectI, Vec2};
use crate::stakes::{DeathKind, Shade};
use crate::terrain::{LedgeKind, Terrain, TerrainOptions};

const DT: f32 = 1.0 / 30.0;
const FLOOR: f32 = 1040.0;

fn catalog() -> ActCatalog {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&data.join("hk")).expect("bank");
    let clips = ClipLibrary::load(&data.join("hk/animations/Knight.json"), &bank).expect("clips");
    let acts = ActLibrary::load(&data.join("acts.json"), &clips).expect("acts + acts/*.json");
    ActCatalog::from_library(&acts)
}

#[derive(Default)]
struct Nav {
    status: NavStatus,
    elapsed: f32,
    goal: Option<NavGoal>,
    /// Every planned route ends here instead (an unreachable goal).
    ends_at: Option<Vec2>,
}

impl Navigator for Nav {
    fn reach(&mut self, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<navigator::Reach> {
        self.estimate(from, goal, style).map(|eta| navigator::Reach { eta, end: self.ends_at })
    }
    fn estimate(&mut self, from: &Character, goal: NavGoal, _: TravelStyle) -> Option<f32> {
        Some(match goal {
            NavGoal::Point(p) => from.pos.distance(p) / 400.0,
            NavGoal::OnWindow(_) => 1.5,
        })
    }
    fn go(&mut self, _: &Character, goal: NavGoal, _: TravelStyle) -> bool {
        self.goal = Some(goal);
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

fn act_secs(name: &str) -> f32 {
    match name {
        "death" => 1.8,
        "respawn_wake" => 3.5,
        "hazard_respawn" | "reclaim_shade" => 1.3,
        "focus_heal" | "focus" => 2.5,
        "slash" | "slash_alt" | "up_slash" | "down_slash" | "recoil" | "thorns" => 0.3,
        _ => 1.2,
    }
}

struct Sim {
    brain: Brain,
    nav: Nav,
    catalog: ActCatalog,
    desk: DesktopSnapshot,
    terrain: Terrain,
    knight: Character,
    benches: Vec<Vec2>,
    cursor: Option<Cursor>,
    pokes: Vec<Poke>,
    act: Option<(RunningAct, f32)>,
    shade: Option<Shade>,
    coins: Vec<Vec2>,
    started: Vec<String>,
    events: Vec<StakeEvent>,
    hits_from_shade: u32,
    activity: Activity,
    /// A Shade that never moves (overrides `shade`).
    fixed_shade: Option<ShadeSense>,
    /// Controller events for the next tick (e.g. a hard landing).
    moves: Vec<crate::controller::MoveEvent>,
}

impl Sim {
    fn new() -> Self {
        let personality = Personality { boldness: 0.8, ..Default::default() };
        let cfg = BrainConfig { personality: Some(personality), bother_level: 1.0, ..Default::default() };
        let catalog = catalog();
        let desk = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows: vec![],
        };
        let terrain = Terrain::from_desktop(&desk, &TerrainOptions::default());
        let mut knight = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(960.0, FLOOR));
        Self::ground(&mut knight);
        Self {
            brain: Brain::new(cfg, catalog.clone(), 7),
            nav: Nav::default(),
            catalog,
            desk,
            terrain,
            knight,
            benches: vec![],
            cursor: None,
            pokes: vec![],
            act: None,
            shade: None,
            coins: vec![],
            started: vec![],
            events: vec![],
            hits_from_shade: 0,
            activity: Activity::default(),
            fixed_shade: None,
            moves: vec![],
        }
    }

    fn ground(k: &mut Character) {
        k.pos.y = FLOOR;
        k.state = MoveState::Grounded;
        k.support = Some(Support { owner: None, kind: LedgeKind::Ground, y: FLOOR, x0: 0.0, x1: 1920.0 });
    }

    fn step(&mut self) -> Intent {
        // The app's side: shade sim + percepts.
        let hunted = self.brain.behaviour() == BehaviourKind::ShadeHunt;
        let centre = self.knight.body().center();
        if let Some(sh) = &mut self.shade {
            let t = sh.step(DT, Some(centre), hunted, &[self.terrain.bounds]);
            self.hits_from_shade += u32::from(t.hit_knight);
            if t.burst {
                self.shade = None;
            }
        }
        let shade = self.shade.as_ref().filter(|s| s.alive()).map(|s| ShadeSense { pos: s.pos, vulnerable: s.vulnerable(), fighting: s.fighting(), geo: s.geo });
        let shade = self.fixed_shade.or(shade);
        let shade_hits = std::mem::take(&mut self.hits_from_shade);
        self.brain.sense_stakes(StakesSense { shade, coins: self.coins.clone(), shade_hits, scale: 1.0 });

        let at_bench = self.benches.iter().any(|b| b.distance(self.knight.pos) < 6.0);
        let moves = std::mem::take(&mut self.moves);
        let p = Percepts {
            dt: DT,
            knight: &self.knight,
            terrain: &self.terrain,
            desktop: &self.desk,
            cursor: self.cursor,
            user: UserActivity { idle_secs: 2.0, keys_per_sec: 0.0 },
            fullscreen_front: false,
            pokes: &self.pokes,
            act: self.act.map(|a| a.0),
            benches: &self.benches,
            perch: at_bench,
            move_events: &moves,
            activity: &self.activity,
            last_input: MoveInput::default(),
            local_hour: None,
            dark: false,
            selection: None,
        };
        let intent = self.brain.tick(&p, &mut self.nav);
        self.pokes.clear();

        if intent.cancel_act && self.act.is_some_and(|a| a.0.interruptible) {
            self.act = None;
        }
        let start = intent.act.or_else(|| intent.attack.and_then(|a| self.catalog.id(a.dir.act_name())));
        if let Some(id) = start
            && self.act.is_none()
        {
            let name = self.catalog.get(id).map(|a| a.name.clone()).unwrap_or_default();
            let interruptible = !matches!(name.as_str(), "death" | "respawn_wake" | "hazard_respawn" | "reclaim_shade");
            self.act = Some((RunningAct { id, interruptible }, act_secs(&name)));
            self.started.push(name);
        }
        if let Some(a) = &mut self.act {
            a.1 -= DT;
            if a.1 <= 0.0 {
                self.act = None;
            }
        }
        for e in &intent.stakes {
            match *e {
                StakeEvent::Respawn { at } => {
                    self.knight.pos = at;
                    Self::ground(&mut self.knight);
                    self.act = None;
                }
                StakeEvent::StrikeShade { damage, from } => {
                    if let Some(sh) = &mut self.shade {
                        sh.strike(damage, from);
                    }
                }
                _ => {}
            }
        }
        self.events.extend(intent.stakes.iter().copied());
        // Movement: navigation arrives after 1.5 s; direct input walks.
        if self.nav.status == NavStatus::Moving {
            self.nav.elapsed += DT;
            if self.nav.elapsed > 1.5 {
                self.nav.status = NavStatus::Arrived;
                if let Some(NavGoal::Point(p)) = self.nav.goal {
                    self.knight.pos.x = p.x;
                }
            }
        }
        if let Motion::Direct(i) = intent.motion {
            self.knight.pos.x += i.x * 300.0 * DT;
        }
        // Coins under the knight get picked up.
        let k = self.knight.pos;
        self.coins.retain(|c| c.distance(k) > 10.0);
        intent
    }

    fn run_until(&mut self, secs: f32, mut done: impl FnMut(&Sim) -> bool) -> bool {
        for _ in 0..(secs / DT) as usize {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn died(&self) -> Option<(DeathKind, bool)> {
        self.events.iter().find_map(|e| match *e {
            StakeEvent::Died { kind, cracked, .. } => Some((kind, cracked)),
            _ => None,
        })
    }

    fn respawned_at(&self) -> Option<Vec2> {
        self.events.iter().find_map(|e| match *e {
            StakeEvent::Respawn { at } => Some(at),
            _ => None,
        })
    }

    /// Start a cursor battle and lose it 0-3.
    fn lose_battle(&mut self) {
        self.brain.mind_mut().needs.mischief = 1.0;
        self.cursor = Some(Cursor { pos: self.knight.body().center() + Vec2::new(150.0, -20.0), ..Default::default() });
        assert!(self.run_until(10.0, |s| s.brain.behaviour() == BehaviourKind::Battle), "battle starts");
        assert!(self.run_until(5.0, |s| s.brain.battle_score().is_some() && s.started.iter().any(|a| a == "challenge") && s.act.is_none()));
        // Three hits in one go, before he lands any.
        for _ in 0..3 {
            self.pokes.push(Poke::Click { pos: self.knight.pos });
        }
        self.step();
    }
}

#[test]
fn badly_lost_battle_kills_and_he_respawns_at_the_bench_sore() {
    let mut sim = Sim::new();
    sim.benches = vec![Vec2::new(300.0, FLOOR)];
    sim.lose_battle();
    assert!(sim.run_until(5.0, |s| s.brain.behaviour() == BehaviourKind::Dead), "a 0-3 loss is lethal");
    assert!(sim.run_until(10.0, |s| s.respawned_at().is_some()));
    assert_eq!(sim.died(), Some((DeathKind::Fight, true)), "shut out: cracked mask");
    assert!(sim.started.iter().any(|a| a == "death"));
    assert!(!sim.started.iter().any(|a| a == "bow_down"));
    assert_eq!(sim.respawned_at(), Some(Vec2::new(300.0, FLOOR)));
    assert!(sim.run_until(10.0, |s| s.brain.behaviour() != BehaviourKind::Dead));
    assert!(sim.started.iter().any(|a| a == "respawn_wake"), "wakes up on the bench: {:?}", sim.started);
    let m = sim.brain.mind();
    assert!(m.emotions.hurt > 0.3, "sore after dying");
    assert!(m.needs.mischief < 0.5);
}

#[test]
fn gentle_lethality_only_knocks_him_down() {
    let mut sim = Sim::new();
    let mut cfg = sim.brain.config().clone();
    cfg.stakes.lethal = Lethality::Never;
    sim.brain = Brain::new(cfg, sim.catalog.clone(), 7);
    sim.lose_battle();
    sim.run_until(6.0, |_| false);
    assert!(sim.died().is_none());
    assert!(sim.started.iter().any(|a| a == "bow_down"));
}

#[test]
fn debug_kill_respawns_on_the_ground_without_a_bench() {
    let mut sim = Sim::new();
    sim.knight.pos.x = 700.0;
    sim.brain.kill(DeathKind::Fight);
    assert!(sim.run_until(10.0, |s| s.respawned_at().is_some()));
    assert_eq!(sim.died(), Some((DeathKind::Fight, false)));
    assert_eq!(sim.respawned_at().map(|p| p.y), Some(FLOOR));
    assert!(sim.run_until(6.0, |s| s.started.iter().any(|a| a == "hazard_respawn")));
}

#[test]
fn he_hunts_the_shade_beats_it_and_focuses() {
    let mut sim = Sim::new();
    sim.knight.pos.x = 300.0;
    sim.shade = Some(Shade::new(Vec2::new(1300.0, FLOOR - 70.0), 80, false, 3));
    sim.brain.hunt_shade();
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt));
    assert!(sim.nav.goal.is_some_and(|g| matches!(g, NavGoal::Point(p) if (p.x - 1300.0).abs() < 1.0 && p.y == FLOOR)), "travels to the floor under it");
    assert!(sim.run_until(60.0, |s| s.shade.is_none()), "beats the shade: {:?}", sim.started);
    assert!(sim.events.iter().any(|e| matches!(e, StakeEvent::StrikeShade { .. })));
    assert!(sim.run_until(10.0, |s| s.brain.behaviour() != BehaviourKind::ShadeHunt));
    assert!(sim.started.iter().any(|a| a == "reclaim_shade"));
    assert!(sim.died().is_none(), "won without dying: {:?}", sim.started);
}

#[test]
fn spells_show_up_in_shade_fights() {
    let mut casts = Vec::new();
    for seed in 0..6 {
        let mut sim = Sim::new();
        sim.brain = Brain::new(sim.brain.config().clone(), sim.catalog.clone(), seed);
        sim.knight.pos.x = 300.0;
        sim.shade = Some(Shade::new(Vec2::new(1300.0, FLOOR - 70.0), 0, false, seed));
        sim.brain.hunt_shade();
        sim.run_until(40.0, |s| s.shade.is_none());
        casts.extend(sim.events.iter().filter_map(|e| match e {
            StakeEvent::Cast { spell, .. } => Some(*spell),
            _ => None,
        }));
    }
    assert!(!casts.is_empty(), "some specials across a few fights");
}

#[test]
fn the_shade_can_kill_him_again() {
    let mut sim = Sim::new();
    sim.knight.pos.x = 1200.0;
    sim.shade = Some(Shade::new(Vec2::new(1300.0, FLOOR - 70.0), 10, false, 3));
    sim.brain.hunt_shade();
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt));
    sim.hits_from_shade = 5;
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Dead), "out of masks");
    assert!(sim.run_until(10.0, |s| s.died().is_some()));
}

#[test]
fn he_walks_over_loose_geo() {
    let mut sim = Sim::new();
    sim.coins = vec![Vec2::new(1200.0, FLOOR), Vec2::new(1250.0, FLOOR)];
    assert!(sim.run_until(5.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    assert!(sim.run_until(15.0, |s| s.coins.is_empty()), "picked everything up");
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() != BehaviourKind::Loot));
}

#[test]
fn a_shade_out_of_reach_is_retried_within_seconds_but_not_frantically() {
    let mut sim = Sim::new();
    sim.knight.pos.x = 900.0;
    // High up over the floor, never coming down: nothing to slash at.
    sim.fixed_shade = Some(ShadeSense { pos: Vec2::new(1000.0, FLOOR - 600.0), vulnerable: true, fighting: false, geo: 40 });
    sim.brain.hunt_shade();
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt));
    // Out of reach: he goes under it again a few times, then steps back for a moment.
    assert!(sim.run_until(40.0, |s| s.brain.behaviour() != BehaviourKind::ShadeHunt), "steps back after re-planning a few times");
    // Never shelved for minutes (it used to be 6 min), never re-started every tick.
    let mut restarts = 0;
    let mut was = false;
    let mut first_back = None;
    for i in 0..(120.0 / DT) as usize {
        sim.step();
        let now = sim.brain.behaviour() == BehaviourKind::ShadeHunt;
        if now && !was {
            restarts += 1;
            first_back.get_or_insert(i as f32 * DT);
        }
        was = now;
    }
    assert!(first_back.is_some_and(|t| t < 10.0), "back at it within seconds: {first_back:?}");
    assert!((2..=8).contains(&restarts), "keeps at it, calmly: {restarts} restarts in 2 min");
}

#[test]
fn an_unreachable_cursor_ends_the_battle() {
    let mut sim = Sim::new();
    sim.brain.mind_mut().needs.mischief = 1.0;
    sim.cursor = Some(Cursor { pos: sim.knight.body().center() + Vec2::new(150.0, -20.0), ..Default::default() });
    assert!(sim.run_until(10.0, |s| s.brain.behaviour() == BehaviourKind::Battle));
    assert!(sim.run_until(5.0, |s| s.started.iter().any(|a| a == "challenge") && s.act.is_none()));
    // The cursor parks way up where he can't jump.
    sim.cursor = Some(Cursor { pos: sim.knight.body().center() + Vec2::new(0.0, -700.0), ..Default::default() });
    assert!(sim.run_until(15.0, |s| s.brain.behaviour() != BehaviourKind::Battle), "gives up");
}

#[test]
fn gathering_goes_for_the_easy_coins_first() {
    use super::shade_hunt::next_coin;
    let from = Vec2::new(500.0, FLOOR);
    let up_close = Vec2::new(560.0, FLOOR - 300.0);
    let along = Vec2::new(800.0, FLOOR);
    let far = Vec2::new(1500.0, FLOOR);
    let coins = [far, up_close, along];
    assert_eq!(next_coin(&coins, from, 3000.0, |_| false), Some(along), "walking 300 beats climbing 300");
    assert_eq!(next_coin(&coins, from, 3000.0, |c| c == along), Some(up_close));
    assert_eq!(next_coin(&coins, from, 200.0, |_| false), None, "out of range");

    // In the brain: the floor coins in order of distance, the high one last.
    let mut sim = Sim::new();
    sim.knight.pos.x = 500.0;
    sim.coins = vec![far, up_close, along, Vec2::new(650.0, FLOOR)];
    assert!(sim.run_until(5.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    let mut order = Vec::new();
    sim.run_until(30.0, |s| {
        if let Some(NavGoal::Point(p)) = s.nav.goal
            && order.last() != Some(&p)
        {
            order.push(p);
        }
        s.coins.is_empty()
    });
    assert_eq!(order.first(), Some(&Vec2::new(650.0, FLOOR)), "{order:?}");
    assert_eq!(order.get(1), Some(&along), "{order:?}");
}

#[test]
fn a_big_haul_ends_with_a_happy_act() {
    let mut sim = Sim::new();
    sim.coins = (0..8).map(|i| Vec2::new(1000.0 + 40.0 * i as f32, FLOOR)).collect();
    assert!(sim.run_until(5.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    assert!(sim.run_until(40.0, |s| s.started.iter().any(|a| a == "collect_happy")), "{:?}", sim.started);
}

/// Has he set off for (a point under) `x`?
fn heading_for(s: &Sim, x: f32) -> bool {
    s.nav.goal.is_some_and(|g| matches!(g, NavGoal::Point(p) if (p.x - x).abs() < 40.0))
}

#[test]
fn a_shade_with_no_way_there_is_looked_for_again_within_seconds() {
    // No route and no Dream Gate (disabled here): no frantic chase, but no shelving it for
    // minutes either — the moment a way opens (a window moved), he goes.
    let mut sim = Sim::new();
    let mut cfg = sim.brain.config().clone();
    cfg.dream_gate.enabled = false;
    sim.brain = Brain::new(cfg, sim.catalog.clone(), 7);
    sim.knight.pos.x = 300.0;
    sim.shade = Some(Shade::new(Vec2::new(1300.0, FLOOR - 70.0), 40, false, 3));
    // Routes stop far from the Shade: he can't get there.
    sim.nav.ends_at = Some(Vec2::new(320.0, FLOOR));
    sim.brain.hunt_shade();
    let (mut restarts, mut was) = (0, false);
    for _ in 0..(10.0 / DT) as usize {
        sim.step();
        assert!(!heading_for(&sim, 1300.0), "never sets off with no way there");
        let now = sim.brain.behaviour() == BehaviourKind::ShadeHunt;
        restarts += u32::from(now && !was);
        was = now;
    }
    assert!(restarts <= 1, "no re-trying every tick: {restarts}");
    sim.nav.ends_at = None;
    assert!(sim.run_until(8.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt && heading_for(s, 1300.0)), "goes as soon as there's a way: {:?} {:?} {:?}", sim.brain.behaviour(), sim.nav.goal, sim.brain.utilities());
}

#[test]
fn he_goes_after_his_shade_seconds_after_respawning() {
    // It used to wait 90 s after every respawn.
    let mut sim = Sim::new();
    sim.knight.pos.x = 300.0;
    sim.brain.kill(DeathKind::Fall);
    assert!(sim.run_until(10.0, |s| s.respawned_at().is_some()));
    sim.fixed_shade = Some(ShadeSense { pos: Vec2::new(1500.0, FLOOR - 40.0), vulnerable: true, fighting: false, geo: 120 });
    assert!(sim.run_until(15.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt), "hunts within seconds: {:?}", sim.brain.behaviour());
}

#[test]
fn after_the_shade_kills_him_he_takes_a_breather_then_goes_again() {
    let mut sim = Sim::new();
    sim.knight.pos.x = 1200.0;
    sim.shade = Some(Shade::new(Vec2::new(1300.0, FLOOR - 70.0), 10, false, 3));
    sim.brain.hunt_shade();
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt));
    sim.hits_from_shade = 5;
    assert!(sim.run_until(15.0, |s| s.respawned_at().is_some()));
    assert!(sim.run_until(10.0, |s| s.brain.behaviour() != BehaviourKind::Dead));
    assert!(!sim.run_until(8.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt), "not straight back into it");
    assert!(sim.run_until(30.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt), "then after it again");
}

#[test]
fn a_failed_route_to_the_shade_is_re_planned_not_given_up() {
    let mut sim = Sim::new();
    sim.knight.pos.x = 300.0;
    sim.fixed_shade = Some(ShadeSense { pos: Vec2::new(1500.0, FLOOR - 40.0), vulnerable: true, fighting: false, geo: 60 });
    assert!(sim.run_until(2.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt && s.nav.status == NavStatus::Moving));
    // The route breaks down on the way (stuck, a window moved).
    sim.nav.status = NavStatus::Failed;
    sim.nav.goal = None;
    sim.step();
    assert_eq!(sim.brain.behaviour(), BehaviourKind::ShadeHunt, "still on it");
    assert!(sim.run_until(1.0, |s| heading_for(s, 1500.0)), "a fresh route under it");
}

// --- Priority tiers -------------------------------------------------------------------

#[test]
fn loose_geo_preempts_ambient_behaviour_at_once() {
    let mut sim = Sim::new();
    sim.run_until(8.0, |_| false);
    let before = sim.brain.behaviour();
    assert_eq!(before.tier(), crate::ai::Tier::Ambient, "settled into something ambient: {before:?}");
    sim.coins = vec![Vec2::new(1300.0, FLOOR)];
    // Within one think interval (+ a tick), no commitment period applies.
    assert!(sim.run_until(0.4, |s| s.brain.behaviour() == BehaviourKind::Loot), "loot preempts {before:?}");
}

#[test]
fn a_worn_out_sore_knight_gathers_far_geo_before_resting() {
    // Live: after a rough fight Geo counted as a pastime (unless a coin lay at his feet)
    // and recovering outranked it; he sat while the coins faded. Only sleep puts Geo off.
    let mut sim = Sim::new();
    sim.brain.mind_mut().needs.energy = 0.05;
    sim.brain.mind_mut().emotions.hurt = 0.8;
    sim.coins = vec![Vec2::new(1850.0, FLOOR)];
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Loot), "geo first: {:?}", sim.brain.behaviour());
    assert!(sim.run_until(10.0, |s| s.coins.is_empty()), "collected");
    assert!(sim.run_until(3.0, |s| matches!(s.brain.behaviour(), BehaviourKind::Recover | BehaviourKind::Rest | BehaviourKind::Nap)), "then he sees to himself");
}

#[test]
fn geo_where_something_else_failed_is_still_gathered() {
    // Live: a Crawlid's Geo lay ignored (loot utility 0) while he sat. Coins were filtered
    // by the shared "unreachable target" map, keyed on a 64 px grid: a seat, a cursor spot,
    // a fight approach or a stuck goal that failed nearby hid fresh coins for 20–60 s.
    let mut sim = Sim::new();
    sim.run_until(1.0, |_| false);
    let coin = Vec2::new(1300.0, FLOOR);
    sim.brain.interest.block(PoiTarget::Spot(coin + Vec2::new(10.0, 0.0)), sim.brain.now + 60.0);
    sim.coins = vec![coin];
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Loot), "loot: {:?}", sim.brain.utilities());
    assert!(sim.run_until(10.0, |s| s.coins.is_empty()));
}
#[test]
fn an_exhausted_knight_still_grabs_a_coin_at_his_feet() {
    let mut sim = Sim::new();
    sim.brain.mind_mut().needs.energy = 0.05;
    sim.coins = vec![Vec2::new(sim.knight.pos.x + 120.0, FLOOR)];
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Loot), "a coin right there beats resting");
}

#[test]
fn urgent_reactions_beat_engagements() {
    let mut sim = Sim::new();
    sim.coins = (0..6).map(|i| Vec2::new(1400.0 + 60.0 * i as f32, FLOOR)).collect();
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    // Shaken about mid-loot: he reels once set down (a reflex), then gets back to the Geo.
    let home = sim.knight.pos;
    sim.pokes.push(Poke::Grab);
    for i in 0..45 {
        sim.knight.pos = home + Vec2::new(100.0 * (i as f32 * DT * 18.0).sin(), -200.0);
        sim.step();
    }
    sim.knight.pos = home;
    sim.pokes.push(Poke::Release { vel: Vec2::ZERO });
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Dizzy), "a shake makes him reel mid-loot");
    assert!(sim.run_until(4.0, |s| s.brain.behaviour() == BehaviourKind::Loot), "back to the Geo");
    assert!(sim.started.iter().any(|a| a == "stunned"), "{:?}", sim.started);
}

#[test]
fn clicked_mid_loot_he_finishes_the_geo_first() {
    // Clicked once too often while gathering Geo: provoked, but Geo comes before the cursor.
    let mut sim = Sim::new();
    sim.coins = (0..6).map(|i| Vec2::new(1400.0 + 60.0 * i as f32, FLOOR)).collect();
    sim.cursor = Some(Cursor { pos: sim.knight.body().center() + Vec2::new(150.0, -20.0), ..Default::default() });
    assert!(sim.run_until(3.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    for _ in 0..6 {
        sim.pokes.push(Poke::Click { pos: sim.knight.body().center() });
        sim.step();
    }
    for _ in 0..(3.0 / DT) as usize {
        sim.step();
        if sim.coins.is_empty() {
            break;
        }
        assert_ne!(sim.brain.behaviour(), BehaviourKind::Battle, "the cursor waits until the Geo is in");
        assert_ne!(sim.brain.behaviour(), BehaviourKind::Flee, "clicks don't scare him off");
    }
}

#[test]
fn geo_ends_a_cursor_battle_and_the_cursor_never_interrupts_looting() {
    // The cursor is a boredom killer: with Geo to collect he's busy, not bored.
    let mut sim = Sim::new();
    sim.brain.mind_mut().needs.mischief = 1.0;
    sim.cursor = Some(Cursor { pos: sim.knight.body().center() + Vec2::new(150.0, -20.0), ..Default::default() });
    assert!(sim.run_until(10.0, |s| s.brain.behaviour() == BehaviourKind::Battle), "battle starts");
    sim.coins = vec![Vec2::new(1300.0, FLOOR), Vec2::new(1340.0, FLOOR)];
    assert!(sim.run_until(0.5, |s| s.brain.behaviour() == BehaviourKind::Loot), "Geo takes over the fight at once");
    // Looting with a mischievous knight and the cursor waved right at him: he keeps looting.
    for _ in 0..(2.0 / DT) as usize {
        sim.brain.mind_mut().needs.mischief = 1.0;
        sim.cursor = Some(Cursor { pos: sim.knight.body().center() + Vec2::new(60.0, -10.0), ..Default::default() });
        sim.step();
        if sim.coins.is_empty() {
            break;
        }
        assert!(!matches!(sim.brain.behaviour(), BehaviourKind::Battle | BehaviourKind::Pester), "cursor interrupted looting");
    }
}

#[test]
fn an_unbroken_fall_kills_and_leaves_a_shade_to_hunt() {
    let mut sim = Sim::new();
    sim.run_until(2.0, |_| false);
    sim.moves = vec![crate::controller::MoveEvent::Landed { hard: true }];
    assert!(sim.run_until(5.0, |s| s.brain.behaviour() == BehaviourKind::Dead), "a hard landing is fatal");
    assert!(sim.run_until(10.0, |s| s.respawned_at().is_some()));
    assert_eq!(sim.died(), Some((DeathKind::Fall, false)));
    assert!(sim.started.iter().any(|a| a == "death"));
    assert!(DeathKind::Fall.leaves_shade(), "a Shade with his Geo: something to go and do");
}

#[test]
fn a_coin_landing_close_on_the_way_is_grabbed_first() {
    // He was heading for a far coin; another lands right by him. He used to run past it to
    // the far one (the target was chosen once). Now the quicker coin takes over.
    let mut sim = Sim::new();
    sim.coins = vec![Vec2::new(1900.0, FLOOR)];
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    sim.run_until(0.2, |_| false);
    sim.coins.push(Vec2::new(1000.0, FLOOR));
    let near_first = sim.run_until(6.0, |s| !s.coins.iter().any(|c| c.x == 1000.0) || !s.coins.iter().any(|c| c.x == 1900.0));
    assert!(near_first);
    assert!(sim.coins.iter().any(|c| c.x == 1900.0), "took the near coin first: {:?} at {:?}", sim.coins, sim.knight.pos);
    assert!(sim.run_until(10.0, |s| s.coins.is_empty()), "then the far one");
}

#[test]
fn geo_is_worth_running_for() {
    // Two coins aren't a stroll any more: he runs (and hurries for far ones).
    let mut sim = Sim::new();
    sim.coins = vec![Vec2::new(1200.0, FLOOR)];
    let mut style = None;
    for _ in 0..(1.0 / DT) as usize {
        if let Motion::Navigate { style: s, .. } = sim.step().motion {
            style = Some(s);
            break;
        }
    }
    assert!(matches!(style, Some(TravelStyle::Normal | TravelStyle::Hurry)), "{style:?}");
}

#[test]
fn coins_on_both_sides_are_not_dithering_and_a_calm_down_never_blocks_geo() {
    // Live: gathering coins left and right tripped the dithering guard; its calm-down then
    // zeroed every behaviour, so he sat with Geo and an enemy on screen.
    let mut sim = Sim::new();
    sim.coins = vec![Vec2::new(500.0, FLOOR), Vec2::new(1400.0, FLOOR), Vec2::new(700.0, FLOOR), Vec2::new(1250.0, FLOOR)];
    assert!(sim.run_until(20.0, |s| s.coins.is_empty()), "gathered them all: {:?}", sim.coins);
    assert_eq!(sim.brain.oscillations(), 0, "collecting isn't dithering");
    // A calm-down in progress (from some earlier pastime) doesn't hold Geo back.
    sim.brain.calm_until = sim.brain.now + 100.0;
    sim.coins = vec![Vec2::new(1300.0, FLOOR)];
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Loot), "Geo during a calm-down");
}

#[test]
fn coins_he_may_not_go_for_dont_restart_loot_every_tick() {
    // Live: every coin was skipped for now (unreachable / kept for a Dream Gate); Loot
    // started, found nothing and finished — every tick.
    let mut sim = Sim::new();
    sim.coins = vec![Vec2::new(1300.0, FLOOR)];
    sim.brain.interest.skip_coin(Vec2::new(1300.0, FLOOR), sim.brain.now + 100.0);
    let mut switches = 0;
    let mut last = sim.brain.behaviour();
    for _ in 0..(3.0 / DT) as usize {
        sim.step();
        if sim.brain.behaviour() != last {
            switches += 1;
            last = sim.brain.behaviour();
        }
    }
    assert!(switches <= 2, "behaviour churned {switches} times");
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Loot);
}

#[test]
fn the_shade_comes_first_even_mid_fight_or_loot() {
    // Lore: his Shade (his Geo, his soul) is priority number one, whatever he's doing.
    let mut sim = Sim::new();
    sim.coins = vec![Vec2::new(1300.0, FLOOR)];
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::Loot));
    sim.fixed_shade = Some(ShadeSense { pos: Vec2::new(400.0, FLOOR - 40.0), vulnerable: true, fighting: false, geo: 50 });
    assert!(sim.run_until(1.0, |s| s.brain.behaviour() == BehaviourKind::ShadeHunt), "hunts the Shade over loot");
}
