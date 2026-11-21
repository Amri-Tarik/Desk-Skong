//! Hornet's challenges against the real data: the race (where the barb lands, her course,
//! fairness against the real planner and controller over many desktops, calling it off)
//! and the duel (her moves, she always wins, calling it off).
use std::path::Path;

use super::*;
use crate::ai::{FoeKind, SceneCue, VisitorState};
use crate::controller::{Abilities, Character, CharacterParams, MoveInput, MoveState};
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::{RectF, RectI, Vec2};
use crate::nav::{NavGoal, NavGraph, PathFollower, TravelStyle};
use crate::portal::Footing;
use crate::terrain::{Terrain, TerrainOptions};

use super::race::{Course, LegKind, pick_finish};

const DT: f32 = 1.0 / 120.0;
const PPU: f32 = 64.0;
const FLOOR: f32 = 1040.0;
const AREA: RectF = RectF::new(0.0, 0.0, 1920.0, FLOOR);

fn set() -> NpcSet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    NpcSet::load(&d.join("npcs.json"), &d.join("hk/animations"), &bank).expect("npcs.json")
}

fn win(id: u64, l: i32, t: i32, r: i32, b: i32) -> WindowInfo {
    WindowInfo { id, rect: RectI::new(l, t, r, b), title: format!("w{id}"), class: String::new(), process: String::new(), pid: 0, maximized: false, fullscreen: false, foreground: false, topmost: false }
}

fn desk(windows: Vec<WindowInfo>) -> DesktopSnapshot {
    DesktopSnapshot { generation: 1, monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }], windows }
}

fn terrain(windows: Vec<WindowInfo>) -> Terrain {
    Terrain::from_desktop(&desk(windows), &TerrainOptions::default())
}

fn env(t: &Terrain, knight: Vec2) -> NpcEnv<'_> {
    NpcEnv { terrain: t, knight, ppu: PPU, area: AREA }
}

/// A knight standing at `x` on whatever is below `y`.
fn knight(t: &Terrain, x: f32, y: f32) -> Character {
    let mut c = Character::new(CharacterParams::hollow_knight(PPU), Abilities::default(), Vec2::new(x, y));
    let mut ev = vec![];
    for _ in 0..600 {
        c.step(&MoveInput::default(), t, DT, &mut ev);
        if c.state == MoveState::Grounded {
            return c;
        }
    }
    panic!("never landed");
}

/// Hornet standing at `at` (on the floor), done arriving and ready to talk.
fn hornet(s: &NpcSet, t: &Terrain, at: Vec2, knight: Vec2) -> Npc {
    hornet_seeded(s, t, at, knight, 5)
}

fn hornet_seeded(s: &NpcSet, t: &Terrain, at: Vec2, knight: Vec2, seed: u32) -> Npc {
    let id = s.id("hornet").expect("hornet");
    let mut npc = Npc::new(1, id, s, Placement { feet: at, owner: None, walk_to: None }, seed, PPU);
    npc.attach(&env(t, knight));
    let mut ev = Vec::new();
    for _ in 0..(2.0 / DT) as usize {
        npc.tick(DT, s, &env(t, knight), &mut ev);
    }
    assert!(npc.can_talk(), "{:?}", npc.phase());
    npc
}

/// Talk (one line), then the planned challenge begins.
fn challenged(npc: &mut Npc, s: &NpcSet, t: &Terrain, plan: ChallengePlan, knight: Vec2, ev: &mut Vec<NpcEvent>) {
    npc.plan_challenge(plan, s);
    assert!(npc.challenging());
    assert!(npc.begin_talk(vec![("Ghost.".into(), None)], knight.x, s, ev));
    for _ in 0..(6.0 / DT) as usize {
        npc.tick(DT, s, &env(t, knight), ev);
        if !matches!(npc.phase(), NpcPhase::Talking) {
            break;
        }
    }
}

#[test]
fn hornet_challenges_with_her_own_clips() {
    let s = set();
    let id = s.id("hornet").expect("hornet");
    let def = s.defs[id].challenge.as_ref().expect("hornet challenges");
    assert!(s.clips[id].challenge.is_some(), "every race / duel clip resolves");
    assert!((0.0..=1.0).contains(&def.chance) && (def.race_share - 0.5).abs() < 0.01, "50/50 race and duel");
    // Only the clips she plays came over from the boss library.
    let lib = s.lib(id);
    assert!(lib.id("Boss Barb Loose").is_some() && lib.id("Boss Counter Stance").is_none(), "a trimmed merge");
    let mut rng = crate::ai::rng::Rng::new(1);
    let races = (0..400).filter(|_| def.pick(&mut rng) == ChallengeKind::Race).count();
    assert!((150..250).contains(&races), "{races} races in 400");
}

#[test]
fn the_barb_lands_far_on_a_settled_reachable_ledge() {
    let s = set();
    let race = &s.defs[s.id("hornet").unwrap()].challenge.as_ref().unwrap().race;
    let t = terrain(vec![win(1, 1200, 600, 1700, 1000), win(2, 300, 350, 800, 900)]);
    let footings: Vec<Footing> = t.ledges.iter().map(|l| Footing { x0: l.x0, x1: l.x1, y: l.y, owner: l.owner }).collect();
    let start = Vec2::new(200.0, FLOOR);
    let mut rng = crate::ai::rng::Rng::new(3);
    for _ in 0..20 {
        let mut random = || rng.f32();
        let (p, _) = pick_finish(&footings, start, AREA, PPU, race.distance, &mut random, &mut |_| true).expect("a finish");
        let d = p.distance(start) / PPU;
        assert!(d >= race.distance[0] - 0.01 && d <= race.distance[1] + 0.01, "{d} units");
        assert!(footings.iter().any(|f| (f.y - p.y).abs() < 0.5 && f.x0 <= p.x && p.x <= f.x1), "on a ledge");
    }
    // Higher ledges are preferred (a climb makes a race).
    let mut random = || rng.f32();
    let highs = (0..20).filter(|_| pick_finish(&footings, start, AREA, PPU, race.distance, &mut random, &mut |_| true).is_some_and(|(p, _)| p.y < FLOOR - 1.0)).count();
    assert!(highs >= 12, "{highs} of 20 up on a window");
    // Only where the knight can get to; nowhere when nothing is reachable.
    let mut random = || rng.f32();
    let (p, owner) = pick_finish(&footings, start, AREA, PPU, race.distance, &mut random, &mut |p| p.y > 900.0).expect("reachable one");
    assert!(p.y > 900.0 && owner.is_none(), "on the floor: {p:?}");
    let mut random = || rng.f32();
    assert!(pick_finish(&footings, start, AREA, PPU, race.distance, &mut random, &mut |_| false).is_none());
}

#[test]
fn her_course_runs_dashes_and_leaps_to_the_barb() {
    let s = set();
    let race = &s.defs[s.id("hornet").unwrap()].challenge.as_ref().unwrap().race;
    let start = Vec2::new(200.0, FLOOR);
    let finish = Vec2::new(1450.0, 600.0);
    let c = Course::plan(start, (0.0, 1920.0), finish, PPU, race);
    let kinds: Vec<LegKind> = (0..200).map(|i| c.at(c.secs() * i as f32 / 200.0).kind).collect();
    assert!(kinds.contains(&LegKind::Run) && kinds.contains(&LegKind::Dash), "{kinds:?}");
    assert!(kinds.iter().any(|k| matches!(k, LegKind::Leap { .. })), "a leap up to the window");
    let end = c.at(c.secs() + 1.0);
    assert!(end.done && end.pos.distance(finish) < 1.0, "{end:?}");
    // The leap clears the window top on its way (apex above the finish).
    let apex = (0..200).map(|i| c.at(c.secs() * i as f32 / 200.0).pos.y).fold(f32::INFINITY, f32::min);
    assert!(apex < finish.y - 20.0, "apex {apex}");
    // Pacing stretches every leg.
    let slow = c.clone().paced(1.5);
    assert!((slow.secs() - 1.5 * c.secs()).abs() < 1e-3);
}

/// One whole race on a desktop: the knight follows the real planner's route with the real
/// controller (after a short reaction), Hornet runs her course. `Some(knight_won)`, or
/// `None` when there was no finish to race to.
fn race(windows: Vec<WindowInfo>, hornet_x: f32, seed: u32) -> Option<(bool, Vec<NpcEvent>)> {
    let s = set();
    let t = terrain(windows);
    let mut k = knight(&t, hornet_x + 2.0 * PPU, 900.0);
    let graph = NavGraph::build(&t, &k);
    let race = s.defs[s.id("hornet").unwrap()].challenge.as_ref().unwrap().race.clone();
    let footings: Vec<Footing> = graph.surfaces().iter().map(|f| Footing { x0: f.x0, x1: f.x1, y: f.y, owner: f.owner }).collect();
    let mut rng = crate::ai::rng::Rng::new(seed);
    let mut random = || rng.f32();
    let mut eta = None;
    let start = Vec2::new(hornet_x, FLOOR);
    let kk = k.clone();
    let (finish, owner) = pick_finish(&footings, start, AREA, PPU, race.distance, &mut random, &mut |p| match graph.plan(&kk, NavGoal::Point(p), TravelStyle::Hurry) {
        Some(r) if r.end.distance(p) <= race.reach * PPU => {
            eta = Some(r.eta);
            true
        }
        _ => false,
    })?;
    let mut npc = hornet_seeded(&s, &t, start, k.pos, seed);
    let mut ev = Vec::new();
    challenged(&mut npc, &s, &t, ChallengePlan::Race { finish, owner, knight_eta: eta }, k.pos, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Racing);
    assert_eq!(npc.percept(&s, PPU).state, VisitorState::Racing);
    let mut follower: Option<PathFollower> = None;
    let mut since_go: Option<f32> = None;
    let mut mev = Vec::new();
    for _ in 0..(60.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, k.pos), &mut ev);
        if let Some(SceneCue::Race { go: true, finish: f, .. }) = npc.scene_cue() {
            let waited = since_go.get_or_insert(0.0);
            *waited += DT;
            // The brain takes a moment to switch and plan.
            if follower.is_none() && *waited >= 0.3 {
                let r = graph.plan(&k, NavGoal::Point(f), TravelStyle::Hurry).expect("route");
                follower = Some(PathFollower::new(r, NavGoal::Point(f), TravelStyle::Hurry));
            }
        }
        if let Some(f) = follower.as_mut() {
            let (input, _) = f.next_input(&k, &graph, &t, DT);
            mev.clear();
            k.step(&input, &t, DT, &mut mev);
        }
        if let Some(won) = ev.iter().find_map(|e| if let NpcEvent::RaceOver { knight_won } = e { *knight_won } else { None }) {
            eprintln!("race: planned {:.2} s, knight ran {:.2} s, {} won", eta.unwrap_or(0.0), since_go.unwrap_or(0.0), if won { "knight" } else { "hornet" });
            return Some((won, ev));
        }
    }
    panic!("the race never ended: {:?} {ev:?}", npc.phase());
}

/// A few desktops: windows here and there to climb, hop and dash across.
fn layouts() -> Vec<Vec<WindowInfo>> {
    vec![
        vec![win(1, 1200, 600, 1700, 1000), win(2, 300, 350, 800, 900)],
        vec![win(1, 900, 300, 1500, 760), win(2, 1300, 700, 1800, 980), win(3, 120, 160, 1000, 900)],
        vec![win(1, 1000, 500, 1900, 1040)],
        vec![win(1, 600, 700, 1000, 1040), win(2, 1100, 450, 1500, 800), win(3, 1550, 250, 1900, 600)],
        vec![],
    ]
}

#[test]
fn races_are_fair_the_knight_wins_some_and_loses_some() {
    let mut won = 0;
    let mut lost = 0;
    for (i, l) in layouts().into_iter().enumerate() {
        for seed in 0..6u32 {
            let x = 150.0 + 300.0 * ((seed + i as u32) % 4) as f32;
            if let Some((w, _)) = race(l.clone(), x, seed * 7 + i as u32) {
                if w { won += 1 } else { lost += 1 }
            }
        }
    }
    eprintln!("races: the knight won {won}, Hornet won {lost}");
    assert!(won + lost >= 20, "most desktops have a finish: {}", won + lost);
    assert!(won >= 2, "the knight wins some ({won} of {})", won + lost);
    assert!(lost > won, "she's Hornet: she wins most ({lost} of {})", won + lost);
}

#[test]
fn a_race_throws_the_barb_says_go_and_ends_with_her_leaping_away() {
    let (_, ev) = race(layouts()[0].clone(), 200.0, 3).expect("a race");
    let says: Vec<&str> = ev.iter().filter_map(|e| if let NpcEvent::Say { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    assert!(says.len() >= 3, "challenge, go, and the reaction: {says:?}");
    let s = set();
    let race = &s.defs[s.id("hornet").unwrap()].challenge.as_ref().unwrap().race;
    assert!(race.go.iter().any(|g| says.contains(&g.as_str())), "a go line: {says:?}");
}

/// The sprites of one of Hornet's clips.
fn clip_keys(s: &NpcSet, clip: &str) -> Vec<crate::assets::SpriteKey> {
    let lib = s.lib(s.id("hornet").unwrap());
    lib.get(lib.id(clip).expect(clip)).frames.iter().filter_map(|f| f.sprite).collect()
}

#[test]
fn the_barb_flies_sits_on_the_finish_and_bursts_when_reached() {
    let s = set();
    let (barb, burst) = (clip_keys(&s, "Boss Barb Loose"), clip_keys(&s, "Boss Barb Break"));
    let t = terrain(vec![]);
    let start = Vec2::new(200.0, FLOOR);
    let k = Vec2::new(330.0, FLOOR);
    let mut npc = hornet(&s, &t, start, k);
    let mut ev = Vec::new();
    let finish = Vec2::new(1300.0, FLOOR);
    challenged(&mut npc, &s, &t, ChallengePlan::Race { finish, owner: None, knight_eta: Some(3.0) }, k, &mut ev);
    assert!(npc.props(&s, PPU).is_empty(), "still in her hand");
    let (mut flying, mut lofted, mut resting, mut bursting) = (false, false, false, false);
    for _ in 0..(40.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, k), &mut ev);
        let props = npc.props(&s, PPU);
        if let Some(p) = props.first() {
            assert_eq!(props.len(), 1, "just the barb");
            assert!(matches!(p.drawn, Some(DrawnProp::Barb { .. })), "drawn plainly if its art is missing");
            if burst.contains(&p.key) {
                assert!(!bursting || resting, "it bursts where it sat");
                assert!((p.at.x - finish.x).abs() < 2.0, "{p:?}");
                assert!(ev.iter().any(|e| matches!(e, NpcEvent::RaceOver { .. })), "only once it's reached");
                bursting = true;
            } else {
                assert!(barb.contains(&p.key), "{p:?}");
                assert!(!bursting, "gone once burst");
                if (p.at.x - finish.x).abs() < 2.0 {
                    // Sitting on the ledge: its spikes' tips at the ledge line.
                    assert!(((finish.y - p.at.y) / PPU - 0.82).abs() < 0.05, "{p:?}");
                    resting = true;
                } else {
                    assert!(!resting, "it stays where it landed");
                    lofted |= p.at.y < FLOOR - 3.0 * PPU;
                    flying = true;
                }
            }
        }
        if npc.phase() == NpcPhase::Gone {
            break;
        }
    }
    assert!(flying && lofted, "lobbed through the air");
    assert!(resting && bursting, "sits at the finish, then bursts");
    assert!(ev.contains(&NpcEvent::RaceOver { knight_won: Some(false) }), "the knight stood still: she wins");
    assert_eq!(npc.phase(), NpcPhase::Gone, "she leaps away");
    assert!(npc.props(&s, PPU).is_empty(), "nothing left behind");
}

#[test]
fn a_race_called_off_ends_politely() {
    let s = set();
    let t = terrain(vec![]);
    let start = Vec2::new(200.0, FLOOR);
    let k = Vec2::new(330.0, FLOOR);
    let mut npc = hornet(&s, &t, start, k);
    let mut ev = Vec::new();
    challenged(&mut npc, &s, &t, ChallengePlan::Race { finish: Vec2::new(1300.0, FLOOR), owner: None, knight_eta: Some(3.0) }, k, &mut ev);
    for _ in 0..(5.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, k), &mut ev);
    }
    ev.clear();
    let barb = clip_keys(&s, "Boss Barb Loose");
    assert!(npc.props(&s, PPU).first().is_some_and(|p| barb.contains(&p.key)), "the barb sits at the finish");
    assert!(npc.call_off(&s, &mut ev));
    let burst = clip_keys(&s, "Boss Barb Break");
    assert!(npc.props(&s, PPU).first().is_some_and(|p| burst.contains(&p.key)), "called off: the barb bursts");
    assert!(ev.contains(&NpcEvent::RaceOver { knight_won: None }), "{ev:?}");
    assert!(ev.iter().any(|e| matches!(e, NpcEvent::Say { .. })), "a parting word");
    assert!(matches!(npc.scene_cue(), Some(SceneCue::Race { over: true, won: None, .. })));
    for _ in 0..(8.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, k), &mut ev);
    }
    assert_eq!(npc.phase(), NpcPhase::Gone);
    assert!(npc.props(&s, PPU).is_empty());
}

/// A duel with a knight standing `gap` units to her right, striking whenever he can.
fn duel(seed_strikes: bool, gap: f32) -> (Npc, Vec<NpcEvent>, u32, u32) {
    let s = set();
    let t = terrain(vec![]);
    let start = Vec2::new(900.0, FLOOR);
    let k = Vec2::new(900.0 + gap * PPU, FLOOR);
    let mut npc = hornet(&s, &t, start, k);
    let mut ev = Vec::new();
    challenged(&mut npc, &s, &t, ChallengePlan::Duel, k, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Sparring);
    let (mut touches, mut landed) = (0, 0);
    let mut cd = 0.0;
    for _ in 0..(60.0 / DT) as usize {
        let n = ev.len();
        npc.tick(DT, &s, &env(&t, k), &mut ev);
        touches += ev[n..].iter().filter(|e| **e == NpcEvent::HitKnight).count() as u32;
        cd -= DT;
        if let Some(f) = npc.foe(&s, PPU) {
            assert_eq!(f.kind, FoeKind::Sparring);
            if seed_strikes && f.vulnerable && cd <= 0.0 && (f.feet.x - k.x).abs() < 2.5 * PPU {
                cd = 0.45;
                npc.spar_strike(1, k.x, &s, &env(&t, k));
                landed += 1;
            }
        }
        if ev.iter().any(|e| matches!(e, NpcEvent::DuelOver { .. })) && npc.phase() != NpcPhase::Sparring {
            break;
        }
    }
    (npc, ev, touches, landed)
}

#[test]
fn hornet_duels_fast_and_always_wins() {
    let mut taken = 0;
    for (strikes, gap) in [(true, 3.0), (false, 3.0), (true, 1.5), (true, 7.0)] {
        let (npc, ev, touches, landed) = duel(strikes, gap);
        assert!(ev.contains(&NpcEvent::DuelOver { finished: true }), "she ends it, the winner: {ev:?}");
        assert_ne!(npc.phase(), NpcPhase::Sparring);
        assert!(touches >= 1, "she lands her blows ({gap} units away)");
        eprintln!("duel gap {gap} strikes {strikes}: {touches} touches, {landed} blows taken");
        taken += landed;
    }
    assert!(taken >= 1, "she can be struck, on guard");
}

/// Everything said in a full visit ending in a duel (a three-line talk whose last line is a
/// goodbye), the knight `gap` units to her right striking whenever she can be struck (if
/// `strikes`). Checks on the way that she is a `waiting` foe — never struck-able — while
/// her challenge is being said. Returns the lines in order and his blows that landed.
fn duel_visit(strikes: bool, seed: u32) -> (Vec<String>, u32, Vec<NpcEvent>) {
    let s = set();
    let t = terrain(vec![]);
    let start = Vec2::new(900.0, FLOOR);
    let k = Vec2::new(900.0 + 2.0 * PPU, FLOOR);
    let mut npc = hornet_seeded(&s, &t, start, k, seed);
    let mut ev = Vec::new();
    npc.plan_challenge(ChallengePlan::Duel, &s);
    let talk = ["Little ghost.", "Stay sharp up here.", "Farewell, little ghost."];
    assert!(npc.begin_talk(talk.iter().map(|l| (l.to_string(), None)).collect(), k.x, &s, &mut ev));
    let challenge = &s.defs[npc.def].challenge.as_ref().unwrap().duel.challenge;
    let mut said: Vec<String> = ev.iter().filter_map(|e| if let NpcEvent::Say { text, .. } = e { Some(text.clone()) } else { None }).collect();
    let (mut landed, mut cd, mut in_challenge) = (0, 0.0, false);
    for _ in 0..(90.0 / DT) as usize {
        let n = ev.len();
        npc.tick(DT, &s, &env(&t, k), &mut ev);
        for e in &ev[n..] {
            match e {
                NpcEvent::Say { text, .. } => {
                    in_challenge = challenge.contains(text);
                    said.push(text.clone());
                }
                NpcEvent::LineDone => in_challenge = false,
                _ => {}
            }
        }
        cd -= DT;
        if let Some(f) = npc.foe(&s, PPU) {
            if in_challenge {
                assert!(f.waiting && !f.vulnerable, "still saying her challenge: no fight yet ({f:?})");
            } else {
                assert!(!f.waiting, "her line is over: the duel is on");
            }
            if strikes && f.vulnerable && cd <= 0.0 {
                cd = 0.3;
                npc.spar_strike(1, k.x, &s, &env(&t, k));
                landed += 1;
            }
        } else {
            assert!(!in_challenge || said.is_empty(), "a foe while she says her challenge");
        }
        if npc.phase() == NpcPhase::Gone {
            break;
        }
    }
    assert_eq!(npc.phase(), NpcPhase::Gone, "she leaves in the end: {said:?}");
    (said, landed, ev)
}

#[test]
fn the_duel_goes_talk_challenge_fight_outcome_then_goodbye() {
    let s = set();
    let duel = &s.defs[s.id("hornet").unwrap()].challenge.as_ref().unwrap().duel;
    let mut respected = false;
    for (strikes, seed) in [(false, 5), (true, 5), (true, 11), (false, 23)] {
        let (said, landed, ev) = duel_visit(strikes, seed);
        // The goodbye comes last, never before the challenge.
        assert_eq!(said.len(), 5, "talk (2), challenge, outcome, goodbye: {said:?}");
        assert_eq!(said[..2], ["Little ghost.", "Stay sharp up here."], "{said:?}");
        assert!(duel.challenge.contains(&said[2]), "then the challenge: {said:?}");
        let outcome = if landed >= duel.respect_after { &duel.respect } else { &duel.win };
        respected |= landed >= duel.respect_after;
        assert!(outcome.contains(&said[3]), "the outcome fits ({landed} blows landed): {said:?}");
        assert_eq!(said[4], "Farewell, little ghost.", "the goodbye last: {said:?}");
        // The duel is over before her word on it.
        let over = ev.iter().position(|e| *e == NpcEvent::DuelOver { finished: true }).expect("she won");
        let word = ev.iter().position(|e| matches!(e, NpcEvent::Say { text, .. } if *text == said[3])).unwrap();
        assert!(over < word);
    }
    assert!(respected, "landing several blows earns her grudging word");
}

#[test]
fn a_challenge_called_off_during_the_talk_keeps_its_goodbye() {
    let s = set();
    let t = terrain(vec![]);
    let k = Vec2::new(1100.0, FLOOR);
    let mut npc = hornet(&s, &t, Vec2::new(900.0, FLOOR), k);
    let mut ev = Vec::new();
    npc.plan_challenge(ChallengePlan::Duel, &s);
    assert!(npc.begin_talk(vec![("Little ghost.".into(), None), ("Farewell, little ghost.".into(), None)], k.x, &s, &mut ev));
    assert!(npc.call_off(&s, &mut ev), "the planned one is dropped");
    for _ in 0..(12.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, k), &mut ev);
    }
    let said: Vec<&str> = ev.iter().filter_map(|e| if let NpcEvent::Say { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    assert_eq!(said, ["Little ghost.", "Farewell, little ghost."]);
}

#[test]
fn a_duel_called_off_or_walked_away_from_ends_without_a_winner() {
    let s = set();
    let t = terrain(vec![]);
    let start = Vec2::new(900.0, FLOOR);
    let k = Vec2::new(1100.0, FLOOR);
    let mut npc = hornet(&s, &t, start, k);
    let mut ev = Vec::new();
    challenged(&mut npc, &s, &t, ChallengePlan::Duel, k, &mut ev);
    for _ in 0..(3.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, k), &mut ev);
    }
    ev.clear();
    assert!(npc.call_off(&s, &mut ev));
    assert!(ev.contains(&NpcEvent::DuelOver { finished: false }), "{ev:?}");
    assert!(npc.foe(&s, PPU).is_none(), "no longer a foe");
    // Walked away from: she gives up after a while.
    let mut npc = hornet(&s, &t, start, k);
    let mut ev = Vec::new();
    challenged(&mut npc, &s, &t, ChallengePlan::Duel, k, &mut ev);
    let far = Vec2::new(1900.0, 300.0);
    for _ in 0..(15.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, far), &mut ev);
    }
    assert!(ev.contains(&NpcEvent::DuelOver { finished: false }), "{ev:?}");
}
