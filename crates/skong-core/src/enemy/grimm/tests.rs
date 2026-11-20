//! The troupe master's director and routine, without the brain: data, start rules, the
//! lantern, the show's phases, telegraphs, spots relative to the knight on any desktop,
//! the endings and the aborts. (The knight fighting him for real: `ai/tests/grimm.rs`.)
use std::collections::HashMap;
use std::path::Path;

use super::*;
use crate::desktop::{DesktopSnapshot, WindowInfo};
use crate::geom::{RectF, RectI};
use crate::terrain::{Terrain, TerrainOptions};

const PPU: f32 = 64.0;
const DT: f32 = 1.0 / 60.0;
const FLOOR: f32 = 1040.0;
const AREA: RectF = RectF::new(0.0, 0.0, 1920.0, FLOOR);

fn data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

pub(crate) fn troupe() -> GrimmSet {
    let d = data();
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    GrimmSet::load(&d.join("grimm.json"), &d.join("hk/animations"), &bank).expect("grimm")
}

fn desk(windows: Vec<WindowInfo>) -> Terrain {
    let desk = DesktopSnapshot {
        generation: 1,
        monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
        windows,
    };
    Terrain::from_desktop(&desk, &TerrainOptions::default())
}

fn window(id: WindowId, rect: RectI) -> WindowInfo {
    WindowInfo { id, rect, title: "w".into(), class: "T".into(), process: "app.exe".into(), pid: id as u32, maximized: false, fullscreen: false, foreground: false, topmost: false }
}

fn knight_at(feet: Vec2) -> RectF {
    RectF::from_feet(feet, Vec2::new(0.5, 1.28) * PPU)
}

fn world(at: Vec2) -> GrimmWorld {
    GrimmWorld { achievements: 20, user_idle_secs: 3.0, knight_at: at, ready: true, ..Default::default() }
}

/// A short run of Grimm against a knight standing still at `feet`; what happened.
#[derive(Default)]
struct Log {
    stages: Vec<GrimmStage>,
    moves: Vec<Move>,
    says: Vec<LineKind>,
    hits: u32,
    phases: Vec<u8>,
    /// Per move: was it winding up before it lunged / before its flames were armed?
    order: Vec<(Move, &'static str)>,
    spots: Vec<(Move, Vec2)>,
    burst: Option<Vec2>,
}

fn run(g: &mut Grimm, set: &GrimmSet, terrain: &Terrain, knight: Option<RectF>, secs: f32, log: &mut Log, mut each: impl FnMut(&mut Grimm, &GrimmSet, f32)) {
    let env = GrimmEnv { terrain, knight, ppu: PPU, area: AREA };
    let mut last_move = None;
    // Flames already about when a move began are the previous move's.
    let mut old: Vec<u32> = Vec::new();
    for i in 0..(secs / DT) as usize {
        each(g, set, i as f32 * DT);
        let t = g.tick(DT, set, &env);
        log.hits += t.hits;
        log.says.extend(t.say);
        log.phases.extend(t.phase);
        if t.burst.is_some() {
            log.burst = t.burst;
        }
        if log.stages.last() != Some(&g.stage()) {
            log.stages.push(g.stage());
        }
        let mv = g.current_move();
        if mv != last_move
            && let Some(m) = mv
        {
            log.moves.push(m);
            log.spots.push((m, g.pos));
            old = g.hazards.iter().map(|h| h.id).collect();
        }
        if let Some(m) = mv {
            let tag = if g.winding_up() {
                "wind"
            } else if g.lunging() {
                "lunge"
            } else if g.hazards.iter().any(|h| h.armed() && !old.contains(&h.id)) {
                "armed"
            } else {
                ""
            };
            if !tag.is_empty() && log.order.last() != Some(&(m, tag)) {
                log.order.push((m, tag));
            }
        }
        last_move = mv;
        if g.stage() == GrimmStage::Gone {
            break;
        }
    }
}

#[test]
fn the_data_loads_and_every_clip_resolves() {
    let set = troupe();
    assert!(set.cfg.hp >= 10, "a proper boss");
    assert!(set.cfg.lantern.min_achievements >= 5, "it's a secret: earned");
    assert!(set.lib.get(set.clips.firebat).frames.len() > 1);
    assert!(set.flash.is_some(), "teleport flash clip");
    // Tuning sanity: phases descend, the reward is big.
    assert!(set.cfg.phases[0] > set.cfg.phases[1]);
    assert!(set.cfg.geo[0] >= 100, "a big Geo burst");
}

#[test]
fn every_kind_of_line_has_something_to_say_even_with_nothing_to_fill() {
    let set = troupe();
    let mut rng = Rng::new(3);
    let empty = TalkContext::default();
    for kind in [LineKind::Intro, LineKind::Phase2, LineKind::Phase3, LineKind::Defeat, LineKind::Triumph, LineKind::BowOut] {
        for _ in 0..10 {
            let line = set.cfg.lines.pick(kind, &empty, &mut rng).unwrap_or_else(|| panic!("{kind:?}"));
            assert!(!line.contains('{'), "unfilled: {line}");
            assert!(line.len() <= 80, "short lines: {line}");
        }
    }
    let ctx = TalkContext { titles: vec![], apps: vec!["Excel".into()], hour: Some(20.0) };
    let intros: Vec<String> = (0..60).filter_map(|_| set.cfg.lines.pick(LineKind::Intro, &ctx, &mut rng)).collect();
    assert!(intros.iter().any(|l| l.contains("evening") || l.contains("Excel")), "placeholders get filled: {intros:?}");
}

#[test]
fn bad_data_is_refused() {
    let mut cfg = troupe().cfg;
    cfg.phases = [0.3, 0.6];
    assert!(cfg.validate().is_err(), "phases must descend");
    let mut cfg = troupe().cfg;
    cfg.lines.defeat.clear();
    assert!(cfg.validate().is_err(), "every kind of line");
    let mut cfg = troupe().cfg;
    cfg.lines.intro = vec!["{title} only".into()];
    assert!(cfg.validate().is_err(), "a plain fallback line");
}

#[test]
fn start_rules_keep_him_out_of_the_way() {
    let cfg = troupe().cfg;
    let ok = world(Vec2::new(960.0, FLOOR));
    assert_eq!(start_blocked(&cfg, &ok, true), None);
    let blocked = |w: GrimmWorld| start_blocked(&cfg, &w, true).is_some() && start_blocked(&cfg, &w, false).is_some();
    assert!(blocked(GrimmWorld { fullscreen: true, ..ok }));
    assert!(blocked(GrimmWorld { visitor_present: true, ..ok }));
    assert!(blocked(GrimmWorld { gauntlet: true, ..ok }));
    assert!(blocked(GrimmWorld { others_alive: 1, ..ok }));
    assert!(blocked(GrimmWorld { keys_per_sec: 4.0, ..ok }));
    assert!(blocked(GrimmWorld { knight_down: true, ..ok }));
    // The natural flame also waits for the Shade, a free knight, a present user, and
    // enough achievements; the debug summon doesn't.
    for w in [GrimmWorld { shade: true, ..ok }, GrimmWorld { knight_busy: true, ..ok }, GrimmWorld { user_idle_secs: 600.0, ..ok }, GrimmWorld { achievements: 2, ..ok }] {
        assert!(start_blocked(&cfg, &w, true).is_some(), "{w:?}");
        assert_eq!(start_blocked(&cfg, &w, false), None, "{w:?}");
    }
}

/// Run the director with the app's side: lantern placement right where asked.
fn direct(d: &mut GrimmDirector, set: &GrimmSet, terrain: &Terrain, w: &GrimmWorld, knight: Option<RectF>, secs: f32, lantern_at: Option<Vec2>) -> Vec<GrimmEvent> {
    let env = GrimmEnv { terrain, knight, ppu: PPU, area: AREA };
    let mut ev = Vec::new();
    for _ in 0..(secs / DT) as usize {
        let step = d.tick(DT, w, &env, set);
        ev.extend(step.events);
        if step.place_lantern {
            ev.extend(d.lantern_placed(lantern_at.map(|p| (p, None)), &env));
        }
    }
    ev
}

#[test]
fn the_lantern_needs_achievements_then_shows_now_and_then() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut cfg = set.cfg.clone();
    cfg.lantern.schedule.first = [10.0, 10.0];
    let mut d = GrimmDirector::new(&cfg, 7);
    let set = GrimmSet { cfg, ..troupe() };
    // Too few achievements: the clock never runs.
    let few = GrimmWorld { achievements: 3, ..world(at) };
    let ev = direct(&mut d, &set, &terrain, &few, Some(knight_at(at)), 30.0, Some(Vec2::new(1400.0, FLOOR)));
    assert!(ev.is_empty() && d.lantern().is_none(), "{ev:?}");
    // Enough: after its presence time, a flame lights where the app put it.
    let ev = direct(&mut d, &set, &terrain, &world(at), Some(knight_at(at)), 12.0, Some(Vec2::new(1400.0, FLOOR)));
    assert!(ev.contains(&GrimmEvent::LanternLit { natural: true }), "{ev:?}");
    let l = *d.lantern().expect("lantern");
    assert_eq!(l.at, Vec2::new(1400.0, FLOOR));
    assert!(d.active());
    // It fades in, then it is something to strike.
    direct(&mut d, &set, &terrain, &world(at), Some(knight_at(at)), 2.5, None);
    let foe = d.foes(&set, PPU)[0];
    assert!(foe.kind == crate::ai::FoeKind::Lantern && foe.vulnerable);
    // Nobody strikes it: it goes out after its time.
    let ev = direct(&mut d, &set, &terrain, &world(at), Some(knight_at(at)), set.cfg.lantern.secs + 3.0, None);
    assert!(ev.contains(&GrimmEvent::LanternFaded), "{ev:?}");
    assert!(d.lantern().is_none() && !d.active());
}

#[test]
fn the_lantern_goes_out_when_the_moment_passes() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut d = GrimmDirector::new(&set.cfg, 7);
    d.request(Summon::Lantern);
    direct(&mut d, &set, &terrain, &world(at), Some(knight_at(at)), 3.0, Some(Vec2::new(1400.0, FLOOR)));
    assert!(d.lantern().is_some());
    let ev = direct(&mut d, &set, &terrain, &GrimmWorld { visitor_present: true, ..world(at) }, Some(knight_at(at)), 2.0, None);
    assert!(ev.contains(&GrimmEvent::LanternFaded), "a visitor came: {ev:?}");
}

#[test]
fn striking_the_lantern_brings_him_out_of_the_flame_with_a_bow_and_a_line() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut d = GrimmDirector::new(&set.cfg, 7);
    d.request(Summon::Lantern);
    direct(&mut d, &set, &terrain, &world(at), Some(knight_at(at)), 3.0, Some(Vec2::new(1300.0, FLOOR)));
    assert_eq!(d.strike_lantern(&set, at.x, PPU), Some(GrimmEvent::Summoned { via: Summon::Lantern }));
    let g = d.grimm().expect("grimm");
    assert_eq!(g.pos, Vec2::new(1300.0, FLOOR), "out of the flame");
    assert!(!g.facing_right, "facing the knight");
    assert_eq!(g.stage(), GrimmStage::Intro);
    // No cheap shots during the bow, and nothing hurts.
    let foe = g.foe(&set, PPU).expect("foe");
    assert!(!foe.vulnerable && !foe.attacking);
    let mut log = Log::default();
    let mut g = Grimm::new(&set, Vec2::new(1300.0, FLOOR), at.x, 1, PPU);
    run(&mut g, &set, &terrain, Some(knight_at(at)), 9.0, &mut log, |_, _, _| {});
    assert_eq!(log.says.first(), Some(&LineKind::Intro));
    assert_eq!(log.stages[..2], [GrimmStage::Intro, GrimmStage::Fight]);
    assert!(!log.moves.is_empty(), "the show began");
}

#[test]
fn every_attack_is_telegraphed_and_he_uses_his_whole_routine() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut log = Log::default();
    let mut g = Grimm::new(&set, Vec2::new(1300.0, FLOOR), at.x, 11, PPU);
    // Phase 1 at full health, then half of it gone (phase 2: spikes come in).
    run(&mut g, &set, &terrain, Some(knight_at(at)), 80.0, &mut log, |g, set, t| {
        if t > 30.0 && g.hp == g.max_hp && g.vulnerable() {
            g.hp = g.max_hp / 2 + 1;
            g.strike(1, set);
        }
    });
    for m in [Move::AirDash, Move::Slash, Move::Firebats, Move::Pillars, Move::Spikes, Move::Balloon] {
        assert!(log.moves.contains(&m), "{m:?} in {:?}", log.moves);
    }
    // A wind-up always comes before the danger of each attack.
    let mut seen_wind: HashMap<Move, bool> = HashMap::new();
    let mut last: Option<Move> = None;
    for &(m, tag) in &log.order {
        if last != Some(m) {
            seen_wind.insert(m, false);
        }
        last = Some(m);
        match tag {
            "wind" => {
                seen_wind.insert(m, true);
            }
            _ if m != Move::Balloon => assert!(seen_wind.get(&m) == Some(&true), "{m:?} struck without a wind-up: {:?}", log.order),
            _ => {}
        }
    }
}

/// The flames' tells: pillars glow before they erupt, spikes peek before they rise.
#[test]
fn pillars_and_spikes_warn_before_they_hurt() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut g = Grimm::new(&set, Vec2::new(1300.0, FLOOR), at.x, 5, PPU);
    g.phase = 2;
    let env = GrimmEnv { terrain: &terrain, knight: Some(knight_at(at)), ppu: PPU, area: AREA };
    let mut seen: HashMap<u32, (HazardKind, bool, bool)> = HashMap::new();
    for _ in 0..(90.0 / DT) as usize {
        g.tick(DT, &set, &env);
        for h in &g.hazards {
            let e = seen.entry(h.id).or_insert((h.kind, false, false));
            if h.armed() {
                e.2 = true;
            } else {
                e.1 = true;
                assert!(!e.2, "warned after it was armed");
            }
            if matches!(h.kind, HazardKind::Pillar | HazardKind::Spikes) {
                assert!((h.pos.y + h.half.y - FLOOR).abs() < 1.0, "rises from his ground line");
            }
        }
    }
    for kind in [HazardKind::Pillar, HazardKind::Spikes, HazardKind::Firebat, HazardKind::Fireball] {
        assert!(seen.values().any(|v| v.0 == kind), "{kind:?} never came: {:?}", seen.values().map(|v| v.0).collect::<Vec<_>>());
    }
    assert!(seen.values().filter(|v| matches!(v.0, HazardKind::Pillar | HazardKind::Spikes)).all(|v| v.1), "ground flames always warn first");
}

#[test]
fn on_a_cooked_desktop_he_works_around_the_knight_wherever_he_stands() {
    let set = troupe();
    // A small window high up; the knight stands on its top. Everything is placed from there.
    let terrain = desk(vec![window(1, RectI::new(700, 400, 1100, 700)), window(2, RectI::new(1500, 820, 1800, 1000))]);
    let at = Vec2::new(900.0, 400.0);
    let mut log = Log::default();
    let mut g = Grimm::new(&set, Vec2::new(1000.0, 400.0), at.x, 21, PPU);
    run(&mut g, &set, &terrain, Some(knight_at(at)), 80.0, &mut log, |_, _, _| {});
    assert!(log.spots.len() > 8, "{:?}", log.moves);
    for (m, p) in &log.spots {
        assert!(AREA.left <= p.x && p.x <= AREA.right && p.y <= AREA.bottom, "{m:?} off stage at {p:?}");
        assert!(p.distance(at) < 13.0 * PPU, "{m:?} far from the knight: {p:?}");
        match m {
            // On his ledge (the window top), not the desktop floor.
            Move::Slash => assert!((p.y - 400.0).abs() < 1.0, "slash on his ledge: {p:?}"),
            Move::AirDash | Move::Pillars | Move::Balloon => assert!(p.y < 400.0, "{m:?} above him: {p:?}"),
            _ => {}
        }
    }
    // The window top is 400 px (6 units) wide: too short for spikes, slashes still fit.
    assert!(!log.moves.contains(&Move::Spikes) || g.phase == 1);
}

#[test]
fn a_ledge_narrower_than_a_slash_spot_never_crashes_him() {
    use super::sim::slash_x;
    // Regression (review): the slash spot was clamped to [x0 + 0.7u, x1 − 0.7u], which
    // panics when the ledge under the knight is narrower than 1.4 units (a 40 px content
    // ledge at 38.4 px per unit did).
    let u = 38.4;
    assert_eq!(slash_x(940.0, 920.0, 960.0, 1.0, u), None);
    assert_eq!(slash_x(940.0, 920.0, 960.0, -1.0, u), None);
    // A wide ledge: a few steps away on the asked side, or none when it's too short there.
    let x = slash_x(900.0, 0.0, 1920.0, 1.0, PPU).expect("fits");
    assert!((x - (900.0 + 4.5 * PPU)).abs() < 0.01);
    assert_eq!(slash_x(1900.0, 0.0, 1920.0, 1.0, PPU), None, "too close to the end on that side");
}

#[test]
fn flames_hurt_him_but_not_twice_in_a_breath() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut g = Grimm::new(&set, Vec2::new(1300.0, FLOOR), at.x, 9, PPU);
    let env = GrimmEnv { terrain: &terrain, knight: Some(knight_at(at)), ppu: PPU, area: AREA };
    // Skip the intro.
    for _ in 0..(6.0 / DT) as usize {
        g.tick(DT, &set, &env);
    }
    g.hazards.push(Hazard { id: 999, kind: HazardKind::Pillar, pos: Vec2::new(at.x, FLOOR - 3.5 * PPU), vel: Vec2::ZERO, half: Vec2::new(0.7, 3.5) * PPU, warn: 0.2, left: 2.0, gravity: 0.0, age: 0.0, lit: 0.0 });
    let mut hits = Vec::new();
    for i in 0..(2.0 / DT) as usize {
        if g.tick(DT, &set, &env).hits > 0 {
            hits.push(i as f32 * DT);
        }
    }
    assert!(!hits.is_empty() && hits[0] >= 0.2 - 1e-3, "not while it warns: {hits:?}");
    assert!(hits.windows(2).all(|w| w[1] - w[0] >= set.cfg.invuln - 0.02), "i-frames: {hits:?}");
}

#[test]
fn phase_changes_bring_a_roar_a_line_and_the_balloon() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut log = Log::default();
    let mut g = Grimm::new(&set, Vec2::new(1300.0, FLOOR), at.x, 4, PPU);
    let mut struck = 0;
    run(&mut g, &set, &terrain, Some(knight_at(at)), 70.0, &mut log, |g, set, _| {
        // Whittle him down whenever he can be struck, but keep him alive.
        if g.hp > 3 && g.vulnerable() && g.strike(1, set).is_some() {
            struck += 1;
        }
    });
    assert!(struck > 0);
    assert_eq!(log.phases, vec![2, 3], "{:?}", log.phases);
    assert!(log.says.contains(&LineKind::Phase2) && log.says.contains(&LineKind::Phase3), "{:?}", log.says);
    assert!(log.moves.iter().filter(|m| **m == Move::Balloon).count() >= 2, "a balloon per phase: {:?}", log.moves);
    // Ballooned, he can't be struck.
    let mut g = Grimm::new(&set, Vec2::new(1300.0, FLOOR), at.x, 4, PPU);
    g.phase = 1;
    let env = GrimmEnv { terrain: &terrain, knight: Some(knight_at(at)), ppu: PPU, area: AREA };
    let mut checked = false;
    for _ in 0..(40.0 / DT) as usize {
        g.tick(DT, &set, &env);
        if g.current_move() == Some(Move::Balloon) && g.alpha >= 1.0 {
            assert!(!g.vulnerable());
            checked = true;
        }
        if g.vulnerable() && g.hp > set.cfg.hp / 2 {
            g.strike(1, &set);
        }
    }
    assert!(checked, "saw a balloon");
}

#[test]
fn beaten_he_bursts_into_bats_with_a_line_and_a_big_geo_burst() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut d = GrimmDirector::new(&set.cfg, 3);
    d.request(Summon::Direct);
    let w = world(at);
    let env = GrimmEnv { terrain: &terrain, knight: Some(knight_at(at)), ppu: PPU, area: AREA };
    let mut events = Vec::new();
    let mut reward = None;
    let mut says = Vec::new();
    for _ in 0..(120.0 / DT) as usize {
        let step = d.tick(DT, &w, &env, &set);
        events.extend(step.events);
        says.extend(step.say);
        reward = reward.or(step.reward);
        if let Some(g) = d.grimm()
            && g.vulnerable()
        {
            d.strike(2, &set);
        }
        if d.grimm().is_none() && !events.is_empty() {
            break;
        }
    }
    assert_eq!(events.first(), Some(&GrimmEvent::Summoned { via: Summon::Direct }));
    let geo = events.iter().find_map(|e| if let GrimmEvent::Defeated { geo } = e { Some(*geo) } else { None }).expect("defeated");
    assert!((set.cfg.geo[0]..=set.cfg.geo[1]).contains(&geo));
    assert_eq!(reward.map(|r| r.1), Some(geo));
    assert!(says.contains(&LineKind::Defeat), "{says:?}");
    assert!(d.grimm().is_none() && !d.active(), "gone");
}

#[test]
fn if_the_knight_falls_he_bows_says_something_kind_and_leaves() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut d = GrimmDirector::new(&set.cfg, 3);
    d.request(Summon::Direct);
    let env = GrimmEnv { terrain: &terrain, knight: Some(knight_at(at)), ppu: PPU, area: AREA };
    for _ in 0..(10.0 / DT) as usize {
        d.tick(DT, &world(at), &env, &set);
    }
    let down = GrimmWorld { knight_down: true, ..world(at) };
    let env = GrimmEnv { knight: None, ..env };
    let mut events = Vec::new();
    let mut says = Vec::new();
    let mut hits = 0;
    for _ in 0..(8.0 / DT) as usize {
        let s = d.tick(DT, &down, &env, &set);
        events.extend(s.events);
        says.extend(s.say);
        hits += s.hits;
    }
    assert_eq!(events, vec![GrimmEvent::KnightFell]);
    assert_eq!(says, vec![LineKind::Triumph]);
    assert_eq!(hits, 0);
    assert!(d.grimm().is_none(), "gone");
}

#[test]
fn a_fullscreen_app_mid_fight_and_he_bows_out_without_penalty() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut d = GrimmDirector::new(&set.cfg, 3);
    d.request(Summon::Direct);
    let env = GrimmEnv { terrain: &terrain, knight: Some(knight_at(at)), ppu: PPU, area: AREA };
    for _ in 0..(15.0 / DT) as usize {
        d.tick(DT, &world(at), &env, &set);
    }
    assert!(d.fighting());
    let full = GrimmWorld { fullscreen: true, ..world(at) };
    let mut events = Vec::new();
    let mut says = Vec::new();
    let mut hits = 0;
    for _ in 0..(6.0 / DT) as usize {
        let s = d.tick(DT, &full, &env, &set);
        events.extend(s.events);
        says.extend(s.say);
        hits += s.hits;
        assert!(d.foes(&set, PPU).iter().all(|f| f.kind != crate::ai::FoeKind::Hazard), "his flames go out at once");
    }
    assert_eq!(events, vec![GrimmEvent::BowedOut { reason: BowOutReason::Fullscreen }]);
    assert_eq!(says, vec![LineKind::BowOut]);
    assert_eq!(hits, 0, "no parting shots");
    assert!(!d.fighting());
    // Hard typing does it too; a game makes him vanish on the spot.
    let mut d = GrimmDirector::new(&set.cfg, 3);
    d.request(Summon::Direct);
    for _ in 0..(10.0 / DT) as usize {
        d.tick(DT, &world(at), &env, &set);
    }
    let typing = GrimmWorld { keys_per_sec: 5.0, ..world(at) };
    let ev: Vec<GrimmEvent> = (0..(5.0 / DT) as usize).flat_map(|_| d.tick(DT, &typing, &env, &set).events).collect();
    assert_eq!(ev, vec![GrimmEvent::BowedOut { reason: BowOutReason::Typing }]);
    let mut d = GrimmDirector::new(&set.cfg, 3);
    d.request(Summon::Direct);
    d.tick(DT, &world(at), &env, &set);
    assert_eq!(d.vanish(), vec![GrimmEvent::BowedOut { reason: BowOutReason::Game }]);
    assert!(!d.active());
}

#[test]
fn a_summon_waits_for_a_good_moment() {
    let set = troupe();
    let terrain = desk(vec![]);
    let at = Vec2::new(960.0, FLOOR);
    let mut d = GrimmDirector::new(&set.cfg, 3);
    d.request(Summon::Direct);
    let full = GrimmWorld { fullscreen: true, ..world(at) };
    assert!(direct(&mut d, &set, &terrain, &full, Some(knight_at(at)), 5.0, None).is_empty());
    assert!(!d.fighting());
    let ev = direct(&mut d, &set, &terrain, &world(at), Some(knight_at(at)), 0.1, None);
    assert_eq!(ev, vec![GrimmEvent::Summoned { via: Summon::Direct }]);
    let g = d.grimm().expect("grimm");
    assert!((g.pos.x - at.x).abs() > 3.0 * PPU && g.pos.y == FLOOR, "beside him: {:?}", g.pos);
}

#[test]
fn the_lantern_is_placed_near_him_on_a_reachable_ledge() {
    let foot = |x0: f32, x1: f32, y: f32| Footing { x0, x1, y, owner: None };
    let mon = Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 };
    let mut rng = Rng::new(1);
    let footings = [foot(0.0, 1920.0, FLOOR), foot(1200.0, 1500.0, 600.0)];
    let knight = Vec2::new(960.0, FLOOR);
    let mons = [mon];
    let (p, _) = plan_lantern(&footings, knight, &mons, PPU, &mut rng, &mut |_| true).expect("a spot");
    let d = p.distance(knight);
    assert!((5.0 * PPU..=16.0 * PPU).contains(&d), "{p:?}");
    // Only reachable spots.
    let only_window = plan_lantern(&footings, knight, &mons, PPU, &mut rng, &mut |p| p.y < 700.0).expect("on the window");
    assert_eq!(only_window.0.y, 600.0);
    assert!(plan_lantern(&footings, knight, &mons, PPU, &mut rng, &mut |_| false).is_none());
}
