//! The Troupe Master's friendly visit: its flow (flash, flourish, bow, line, teleports
//! around the knight, farewell), the flame rule, aborts, and that he is never a foe.
use std::path::Path;

use super::*;
use crate::desktop::DesktopSnapshot;
use crate::geom::{RectF, RectI};
use crate::terrain::{Terrain, TerrainOptions};

const PPU: f32 = 64.0;
const DT: f32 = 1.0 / 60.0;
const FLOOR: f32 = 1040.0;
const AREA: RectF = RectF::new(0.0, 0.0, 1920.0, FLOOR);

fn troupe() -> GrimmSet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    GrimmSet::load(&d.join("grimm.json"), &d.join("hk/animations"), &bank).expect("grimm")
}

fn floor() -> Terrain {
    let desk = DesktopSnapshot { generation: 1, monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }], windows: vec![] };
    Terrain::from_desktop(&desk, &TerrainOptions::default())
}

fn knight_at(feet: Vec2) -> RectF {
    RectF::from_feet(feet, Vec2::new(0.5, 1.28) * PPU)
}

fn world(achievements: usize) -> GrimmWorld {
    GrimmWorld { achievements, user_idle_secs: 3.0, knight_at: Vec2::new(960.0, FLOOR), ready: true, ..Default::default() }
}

#[derive(Default)]
struct Log {
    events: Vec<GrimmEvent>,
    says: Vec<LineKind>,
    flashes: usize,
    spots: Vec<Vec2>,
    beats: Vec<(u32, Option<String>)>,
    foes: usize,
    lanterns: usize,
}

/// Run the director (lantern placement where asked) and log what the visit did.
fn run(d: &mut GrimmDirector, set: &GrimmSet, t: &Terrain, w: &GrimmWorld, secs: f32, log: &mut Log) {
    let env = GrimmEnv { terrain: t, knight: Some(knight_at(w.knight_at)), ppu: PPU, area: AREA };
    for _ in 0..(secs / DT) as usize {
        let step = d.tick(DT, w, &env, set);
        log.events.extend(step.events.iter().copied());
        log.says.extend(step.say);
        log.flashes += step.flashes.len();
        if step.place_lantern {
            log.lanterns += 1;
            log.events.extend(d.lantern_placed(Some((Vec2::new(1400.0, FLOOR), None)), &env));
        }
        log.foes += d.foes(set, PPU).len();
        if let Some(c) = d.cameo() {
            if c.alpha >= 1.0 && log.spots.last().is_none_or(|p| p.distance(c.pos) > 1.0) {
                log.spots.push(c.pos);
            }
            if let Some(SceneCue::Watch { beat, react, .. }) = d.scene_cue(set)
                && log.beats.last().is_none_or(|b| b.0 != beat)
            {
                log.beats.push((beat, react));
            }
        }
    }
}

fn with_flame(set: &GrimmSet, chance: f32) -> GrimmSet {
    let mut cfg = set.cfg.clone();
    cfg.cameo.flame_chance = chance;
    GrimmSet { cfg, ..troupe() }
}

#[test]
fn the_cameo_data_and_lines_are_there() {
    let set = troupe();
    assert!(set.cfg.cameo.enabled);
    assert!(set.cfg.cameo.schedule.first[0] >= 1200.0, "rare");
    let empty = TalkContext::default();
    let mut rng = Rng::new(2);
    for kind in [LineKind::Cameo, LineKind::CameoBye] {
        let line = set.cfg.lines.pick(kind, &empty, &mut rng).expect("a line");
        assert!(!line.contains('{') && line.len() <= 80, "{line}");
    }
}

#[test]
fn a_friendly_visit_flourishes_bows_hops_around_the_knight_and_leaves() {
    let set = with_flame(&troupe(), 0.0);
    let t = floor();
    let mut d = GrimmDirector::new(&set.cfg, 5);
    d.request_cameo();
    assert!(d.busy(), "nothing else starts meanwhile");
    let mut log = Log::default();
    run(&mut d, &set, &t, &world(30), 30.0, &mut log);
    assert_eq!(log.events.first(), Some(&GrimmEvent::CameoArrived));
    assert!(log.events.contains(&GrimmEvent::CameoOver { finished: true, flame: false }), "{:?}", log.events);
    assert_eq!(log.says, [LineKind::Cameo, LineKind::CameoBye], "a line after the bow, a farewell");
    assert_eq!(log.foes, 0, "never hostile: no foes, no hits");
    assert!(log.spots.len() >= 3, "appears, then teleports at least twice: {:?}", log.spots);
    assert!(log.flashes > log.spots.len() - 1, "a flash with every appearance: {}", log.flashes);
    let k = 960.0;
    assert!(log.spots.iter().any(|p| p.x < k) && log.spots.iter().any(|p| p.x > k), "around him: {:?}", log.spots);
    assert!(log.spots.iter().all(|p| (p.x - k).abs() <= 7.0 * PPU && (p.y - FLOOR).abs() < 1.0), "close, on his ground line: {:?}", log.spots);
    // The knight's beats: a startle at the entrance, a bow for his bow, a glance per hop.
    let reacts: Vec<String> = log.beats.iter().filter_map(|b| b.1.clone()).collect();
    assert!(reacts.first().is_some_and(|r| r.contains("startled")), "{reacts:?}");
    assert!(reacts.iter().any(|r| r.contains("challenge")), "answers the bow: {reacts:?}");
    assert!(reacts.iter().filter(|r| r.contains("glance")).count() >= 2, "{reacts:?}");
    assert!(d.cameo().is_none() && !d.busy() && d.lantern().is_none() && log.lanterns == 0, "no flame left");
}

#[test]
fn the_flame_is_left_behind_only_when_the_secret_allows() {
    let set = with_flame(&troupe(), 1.0);
    let t = floor();
    let min = set.cfg.lantern.min_achievements;
    // Not enough achievements: never a flame, however often he visits.
    for seed in 0..4 {
        let mut d = GrimmDirector::new(&set.cfg, seed);
        d.request_cameo();
        let mut log = Log::default();
        run(&mut d, &set, &t, &world(min - 1), 35.0, &mut log);
        assert!(log.events.contains(&GrimmEvent::CameoOver { finished: true, flame: false }), "{:?}", log.events);
        assert!(d.lantern().is_none() && log.lanterns == 0);
    }
    // Enough: his flame stays behind, the same click-to-summon flame.
    let mut d = GrimmDirector::new(&set.cfg, 9);
    d.request_cameo();
    let mut log = Log::default();
    run(&mut d, &set, &t, &world(min), 35.0, &mut log);
    assert!(log.events.contains(&GrimmEvent::CameoOver { finished: true, flame: true }), "{:?}", log.events);
    assert!(log.events.contains(&GrimmEvent::LanternLit { natural: true }), "{:?}", log.events);
    assert!(d.lantern().is_some_and(|l| l.lit()));
    assert!(matches!(d.scene_cue(&set), Some(SceneCue::Flame { .. })), "he lingers by it");
    assert!(d.foes(&set, PPU).is_empty());
    assert!(d.click_lantern());
    let mut log = Log::default();
    run(&mut d, &set, &t, &world(min), 0.5, &mut log);
    assert!(log.events.contains(&GrimmEvent::Summoned { via: Summon::Lantern }), "{:?}", log.events);
}

#[test]
fn a_visit_is_cut_short_by_a_visitor_typing_or_a_fullscreen_app() {
    let set = with_flame(&troupe(), 1.0);
    let t = floor();
    let ok = world(40);
    let cuts = [GrimmWorld { visitor_present: true, ..ok }, GrimmWorld { keys_per_sec: 6.0, ..ok }, GrimmWorld { fullscreen: true, ..ok }, GrimmWorld { others_alive: 1, ..ok }, GrimmWorld { shade: true, ..ok }];
    for w in cuts {
        let mut d = GrimmDirector::new(&set.cfg, 4);
        d.request_cameo();
        let mut log = Log::default();
        run(&mut d, &set, &t, &ok, 3.0, &mut log);
        assert!(d.cameo().is_some(), "on stage");
        run(&mut d, &set, &t, &w, 5.0, &mut log);
        assert!(log.events.contains(&GrimmEvent::CameoOver { finished: false, flame: false }), "{w:?}: {:?}", log.events);
        assert!(d.cameo().is_none() && d.lantern().is_none(), "gone, no flame");
    }
    // And it doesn't start in such a moment (a debug request waits).
    let mut d = GrimmDirector::new(&set.cfg, 4);
    d.request_cameo();
    let mut log = Log::default();
    run(&mut d, &set, &t, &GrimmWorld { visitor_present: true, ..ok }, 5.0, &mut log);
    assert!(d.cameo().is_none() && log.events.is_empty());
    run(&mut d, &set, &t, &ok, 1.0, &mut log);
    assert_eq!(log.events.first(), Some(&GrimmEvent::CameoArrived));
}

#[test]
fn visits_come_on_their_own_rarely() {
    let mut set = troupe();
    set.cfg.cameo.schedule.first = [30.0, 30.0];
    let t = floor();
    let mut d = GrimmDirector::new(&set.cfg, 3);
    let mut log = Log::default();
    run(&mut d, &set, &t, &world(0), 25.0, &mut log);
    assert!(log.events.is_empty(), "not yet");
    run(&mut d, &set, &t, &world(0), 10.0, &mut log);
    assert!(log.events.contains(&GrimmEvent::CameoArrived), "everyone gets visited, no achievements needed: {:?}", log.events);
}
