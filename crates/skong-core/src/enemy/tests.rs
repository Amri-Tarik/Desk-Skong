//! Real `data/enemies.json` against the exported clips, the enemy sim on a fake desktop,
//! and arrival planning.
mod gauntlet;

use std::path::Path;

use super::*;
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::{RectF, RectI, Vec2};
use crate::terrain::{Terrain, TerrainOptions};

const DT: f32 = 1.0 / 60.0;
const PPU: f32 = 38.4;
const FLOOR: f32 = 1040.0;

fn set() -> EnemySet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    EnemySet::load(&d.join("enemies.json"), &d.join("hk/animations"), &bank).expect("enemies.json must resolve against the exported clips")
}

fn monitor() -> Monitor {
    Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }
}

fn desk(win: Option<RectI>) -> DesktopSnapshot {
    DesktopSnapshot {
        generation: 1,
        monitors: vec![monitor()],
        windows: win
            .into_iter()
            .map(|rect| WindowInfo {
                id: 7,
                rect,
                title: "notes.txt - Notepad".into(),
                class: "Test".into(),
                process: "notepad.exe".into(),
                pid: 7,
                maximized: false,
                fullscreen: false,
                foreground: true,
                topmost: false,
            })
            .collect(),
    }
}

fn terrain(win: Option<RectI>) -> Terrain {
    Terrain::from_desktop(&desk(win), &TerrainOptions::default())
}

fn env(t: &Terrain, knight: Option<Vec2>) -> EnemyEnv<'_> {
    // The knight's body: ~0.5 × 1.3 units over his feet.
    let knight = knight.map(|k| RectF::from_feet(k, Vec2::new(0.5, 1.3) * PPU));
    EnemyEnv { terrain: t, knight, ppu: PPU, area: RectF::new(0.0, 0.0, 1920.0, FLOOR) }
}

fn spawn(s: &EnemySet, name: &str, at: Vec2, entry: EntryKind, owner: Option<WindowId>) -> Enemy {
    let def = s.id(name).expect(name);
    Enemy::new(1, def, s, EnemyPlacement { at, owner, entry }, 5, PPU)
}

/// Run `secs`; returns how many times it hit the knight.
fn run(e: &mut Enemy, s: &EnemySet, t: &Terrain, knight: Option<Vec2>, secs: f32) -> u32 {
    let mut hits = 0;
    for _ in 0..(secs / DT) as usize {
        hits += u32::from(e.tick(DT, s, &env(t, knight)).hit_knight);
    }
    hits
}

#[test]
fn real_enemies_resolve_with_art_metadata() {
    let s = set();
    assert!(s.defs.len() >= 3, "a small roster");
    for (i, d) in s.defs.iter().enumerate() {
        let lib = s.lib(i);
        let c = &s.clips[i];
        for clip in [c.idle, c.moving, c.chase, c.death_air, c.death_land] {
            assert!(lib.get(clip).frames.iter().any(|f| f.sprite.is_some()), "{}: clip {} has art", d.name, lib.get(clip).name);
        }
        assert!(d.geo[0] >= 1, "{}: every kill drops some Geo", d.name);
    }
    assert!(s.defs.iter().any(|d| d.moves == Moves::Fly), "a flyer");
    assert!(s.defs.iter().any(|d| d.aggro == 0.0), "a harmless crawler");
    assert!(s.defs.iter().enumerate().any(|(i, d)| d.lunge > 0.0 && s.clips[i].attack.is_some()), "one that attacks");
}

#[test]
fn a_crawler_walks_in_patrols_its_ledge_and_stays_on_it() {
    let s = set();
    let t = terrain(None);
    let mut e = spawn(&s, "crawlid", Vec2::new(500.0, FLOOR), EntryKind::Edge { from: Vec2::new(31.0, FLOOR) }, None);
    e.attach(&env(&t, None));
    run(&mut e, &s, &t, None, 0.2);
    assert_eq!(e.phase(), EnemyPhase::Arriving);
    assert!(!e.vulnerable(), "can't be struck before it has faded in");
    run(&mut e, &s, &t, None, 15.0);
    assert!(matches!(e.phase(), EnemyPhase::Roaming | EnemyPhase::Turning), "{:?}", e.phase());
    assert_eq!(e.alpha, 1.0);
    // A long patrol: always on the floor, always on the monitor, and it gets about.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut turned = false;
    for _ in 0..(60.0 / DT) as usize {
        e.tick(DT, &s, &env(&t, None));
        lo = lo.min(e.pos.x);
        hi = hi.max(e.pos.x);
        turned |= e.phase() == EnemyPhase::Turning;
        assert_eq!(e.pos.y, FLOOR);
        assert!(e.pos.x >= 0.0 && e.pos.x <= 1920.0);
    }
    assert!(hi - lo > 300.0, "it roams ({lo:.0}..{hi:.0})");
    assert!(turned || hi - lo > 1700.0, "turns at the end of its ledge");
}

#[test]
fn struck_dead_it_falls_lies_there_and_fades() {
    let s = set();
    let t = terrain(None);
    let mut e = spawn(&s, "crawlid", Vec2::new(800.0, FLOOR), EntryKind::Edge { from: Vec2::new(790.0, FLOOR) }, None);
    run(&mut e, &s, &t, None, 1.0);
    let from = e.pos - Vec2::new(60.0, 20.0);
    assert!(!e.strike(1, from, &s, PPU), "2 hp: the first blow doesn't kill");
    assert_eq!(e.flash, 1.0, "flashes when struck");
    let x = e.pos.x;
    run(&mut e, &s, &t, None, 0.3);
    assert!(e.pos.x > x + 10.0, "knocked back away from the blow");
    assert!(e.strike(1, from, &s, PPU), "the second blow kills");
    assert_eq!(e.phase(), EnemyPhase::Dying);
    assert!(!e.vulnerable() && e.foe(&s, PPU).is_none(), "dead foes are no foes");
    assert!(!e.strike(1, from, &s, PPU), "no hitting a dead crawlid");
    run(&mut e, &s, &t, None, 1.5);
    assert_eq!(e.phase(), EnemyPhase::Corpse);
    assert_eq!(e.pos.y, FLOOR, "the corpse lies on the floor");
    let mut gone = false;
    for _ in 0..(6.0 / DT) as usize {
        gone |= e.tick(DT, &s, &env(&t, None)).gone;
    }
    assert!(gone && e.phase() == EnemyPhase::Gone);
}

#[test]
fn a_husk_notices_the_knight_winds_up_and_lunges() {
    let s = set();
    let t = terrain(None);
    let mut e = spawn(&s, "husk", Vec2::new(900.0, FLOOR), EntryKind::Edge { from: Vec2::new(890.0, FLOOR) }, None);
    run(&mut e, &s, &t, None, 1.0);
    let knight = Some(Vec2::new(1100.0, FLOOR));
    let mut seen = Vec::new();
    let mut hits = 0;
    for _ in 0..(8.0 / DT) as usize {
        hits += u32::from(e.tick(DT, &s, &env(&t, knight)).hit_knight);
        if seen.last() != Some(&e.phase()) {
            seen.push(e.phase());
        }
    }
    assert!(seen.contains(&EnemyPhase::Chasing) && seen.contains(&EnemyPhase::Antic) && seen.contains(&EnemyPhase::Attacking), "{seen:?}");
    assert!(hits >= 1, "its lunge catches a knight who just stands there");
    assert!(hits <= 8, "but not every frame ({hits})");
    // A knight on another level is ignored.
    let mut e = spawn(&s, "husk", Vec2::new(900.0, FLOOR), EntryKind::Edge { from: Vec2::new(890.0, FLOOR) }, None);
    run(&mut e, &s, &t, None, 1.0);
    run(&mut e, &s, &t, Some(Vec2::new(1000.0, 700.0)), 3.0);
    assert!(!matches!(e.phase(), EnemyPhase::Chasing | EnemyPhase::Antic | EnemyPhase::Attacking), "{:?}", e.phase());
}

#[test]
fn a_vengefly_flies_in_hovers_then_chases_but_stays_on_the_monitor() {
    let s = set();
    let t = terrain(None);
    let hover = Vec2::new(600.0, FLOOR - 2.4 * PPU);
    let mut e = spawn(&s, "vengefly", hover, EntryKind::Edge { from: Vec2::new(27.0, hover.y) }, None);
    run(&mut e, &s, &t, None, 8.0);
    assert_eq!(e.phase(), EnemyPhase::Roaming);
    assert!(e.pos.distance(hover) < 4.0 * PPU, "hovers around where it arrived");
    assert!(e.pos.y < FLOOR - PPU, "in the air");
    let knight = Some(Vec2::new(700.0, FLOOR));
    let hits = run(&mut e, &s, &t, knight, 6.0);
    assert!(hits >= 1, "dives at the knight");
    // Chasing a knight near the monitor edge never takes it off the monitor.
    for _ in 0..(10.0 / DT) as usize {
        e.tick(DT, &s, &env(&t, Some(Vec2::new(5.0, FLOOR))));
        assert!(e.pos.x >= 0.0 && e.pos.x <= 1920.0 && e.pos.y >= 0.0);
    }
    // Struck dead in the air, it falls to the floor.
    let from = e.body(&s, PPU).center() + Vec2::new(50.0, 0.0);
    let mut killed = false;
    for _ in 0..6 {
        killed |= e.strike(1, from, &s, PPU);
    }
    assert!(killed);
    run(&mut e, &s, &t, None, 3.0);
    assert_eq!(e.phase(), EnemyPhase::Corpse);
    assert_eq!(e.pos.y, FLOOR);
}

#[test]
fn it_rides_its_window_and_falls_when_the_window_goes() {
    let s = set();
    let win = RectI::new(400, 500, 1200, 900);
    let t = terrain(Some(win));
    let mut e = spawn(&s, "crawlid", Vec2::new(800.0, 500.0), EntryKind::Drop, Some(7));
    e.attach(&env(&t, None));
    run(&mut e, &s, &t, None, 2.0);
    assert_eq!(e.pos.y, 500.0, "dropped onto the window top");
    // The window moves down 100 px: it rides along.
    let moved = terrain(Some(RectI::new(400, 600, 1200, 1000)));
    run(&mut e, &s, &moved, None, 0.5);
    assert_eq!(e.pos.y, 600.0);
    assert!(e.pos.x >= 400.0 && e.pos.x <= 1200.0);
    // Gone: it falls to the floor.
    let bare = terrain(None);
    run(&mut e, &s, &bare, None, 2.0);
    assert_eq!(e.pos.y, FLOOR);
}

#[test]
fn left_alone_it_wanders_off() {
    let mut s = set();
    let id = s.id("crawlid").unwrap();
    s.defs[id].stay = [2.0, 2.0];
    let t = terrain(None);
    let mut e = spawn(&s, "crawlid", Vec2::new(300.0, FLOOR), EntryKind::Edge { from: Vec2::new(290.0, FLOOR) }, None);
    run(&mut e, &s, &t, None, 4.0);
    assert_eq!(e.phase(), EnemyPhase::Leaving);
    let mut gone = false;
    for _ in 0..(30.0 / DT) as usize {
        gone |= e.tick(DT, &s, &env(&t, None)).gone;
    }
    assert!(gone, "walked off and faded");
}

#[test]
fn arrivals_walk_in_from_an_edge_drop_onto_window_ledges_or_fly_in() {
    let s = set();
    let mut rng = Rng::new(3);
    let knight = Vec2::new(960.0, FLOOR);
    let floor = Footing { x0: 0.0, x1: 1920.0, y: FLOOR, owner: None };
    let ledge = Footing { x0: 400.0, x1: 900.0, y: 700.0, owner: Some(7) };
    let crawlid = &s.defs[s.id("crawlid").unwrap()];
    let fly = &s.defs[s.id("vengefly").unwrap()];
    let mut yes = |_: Vec2| true;
    for _ in 0..20 {
        let p = plan_arrival(crawlid, &[floor], knight, &[monitor()], PPU, &mut rng, &mut yes).expect("a spot on the floor");
        let d = p.at.distance(knight);
        assert!((4.0 * PPU..=14.0 * PPU).contains(&d), "not on top of him, not miles away: {d:.0}");
        let EntryKind::Edge { from } = p.entry else { panic!("walks in from an edge: {p:?}") };
        assert!(from.x < 60.0 || from.x > 1860.0, "from the monitor edge: {from:?}");
        assert_eq!(from.y, FLOOR);
        let p = plan_arrival(crawlid, &[ledge], knight, &[monitor()], PPU, &mut rng, &mut yes).expect("a spot on the ledge");
        assert_eq!(p.entry, EntryKind::Drop, "a ledge off the edges: drops in");
        assert_eq!((p.at.y, p.owner), (700.0, Some(7)));
        let p = plan_arrival(fly, &[floor], knight, &[monitor()], PPU, &mut rng, &mut yes).expect("a spot to hover");
        assert!((p.at.y - (FLOOR - fly.hover * PPU)).abs() < 0.5, "hovers above the floor");
        assert!(matches!(p.entry, EntryKind::Edge { from } if from.y == p.at.y && (from.x < 60.0 || from.x > 1860.0)));
    }
    // Nothing reachable, nothing wide enough, nothing at all: no arrival.
    let mut no = |_: Vec2| false;
    assert!(plan_arrival(crawlid, &[floor], knight, &[monitor()], PPU, &mut rng, &mut no).is_none());
    let sliver = Footing { x0: 900.0, x1: 950.0, y: 600.0, owner: Some(7) };
    assert!(plan_arrival(crawlid, &[sliver], knight, &[monitor()], PPU, &mut rng, &mut yes).is_none());
    assert!(plan_arrival(crawlid, &[], knight, &[monitor()], PPU, &mut rng, &mut yes).is_none());
}
