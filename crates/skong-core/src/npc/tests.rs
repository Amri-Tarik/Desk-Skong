//! Real `data/npcs.json` against the exported clips and `acts.json`, plus the visitor
//! state machine on a fake desktop.
use std::path::Path;

use super::*;
use crate::acts::ActLibrary;
use crate::ai::VisitorState;
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::{RectF, RectI, Vec2};
use crate::portal::Footing;
use crate::terrain::{Terrain, TerrainOptions};

const DT: f32 = 1.0 / 60.0;

fn data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn set() -> NpcSet {
    let d = data();
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    NpcSet::load(&d.join("npcs.json"), &d.join("hk/animations"), &bank).expect("npcs.json must resolve against the exported clips")
}

fn desk(win_top: i32) -> DesktopSnapshot {
    DesktopSnapshot {
        generation: 1,
        monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
        windows: vec![WindowInfo {
            id: 7,
            rect: RectI::new(400, win_top, 1200, win_top + 400),
            title: "notes.txt - Notepad".into(),
            class: "Test".into(),
            process: "notepad.exe".into(),
            pid: 7,
            maximized: false,
            fullscreen: false,
            foreground: true,
            topmost: false,
        }],
    }
}

fn env(t: &Terrain) -> NpcEnv<'_> {
    NpcEnv { terrain: t, knight: Vec2::new(100.0, 1040.0), ppu: 64.0, area: RectF::new(0.0, 0.0, 1920.0, 1040.0) }
}

fn run(npc: &mut Npc, set: &NpcSet, t: &Terrain, secs: f32, events: &mut Vec<NpcEvent>) {
    for _ in 0..(secs / DT) as usize {
        npc.tick(DT, set, &env(t), events);
    }
}

#[test]
fn real_npcs_resolve_and_reacts_exist() {
    let s = set();
    assert!(s.defs.len() >= 3, "a few visitors");
    let d = data();
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    let knight = crate::anim::ClipLibrary::load(&d.join("hk/animations/Knight.json"), &bank).expect("knight clips");
    let acts = ActLibrary::load(&d.join("acts.json"), &knight).expect("acts");
    for (i, def) in s.defs.iter().enumerate() {
        // Every frame of the idle clip has art metadata.
        let lib = s.lib(i);
        assert!(lib.get(s.clips[i].idle).frames.iter().all(|f| f.sprite.is_some()), "{}: idle frames", def.name);
        for c in &def.conversations {
            assert!((2..=4).contains(&c.len()), "{}: 2-4 lines per conversation", def.name);
        }
        let reacts = def.conversations.iter().flatten().filter_map(|l| l.react()).chain(def.greet_react.as_deref());
        for r in reacts {
            for name in r.split('|') {
                assert!(acts.id(name.trim()).is_some(), "{}: unknown react act '{name}'", def.name);
            }
        }
    }
    assert!(acts.id("dream_nail").is_some() && acts.id("receive_map").is_some());
    assert!(s.dream.0.iter().any(|r| r.matches.is_empty()), "a generic dream pool");
}

#[test]
fn portal_visitor_arrives_talks_and_leaves() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("cornifer").expect("cornifer");
    let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    npc.attach(&env(&t));
    let mut ev = Vec::new();
    assert_eq!(npc.percept(&s, 64.0).state, VisitorState::Arriving);
    run(&mut npc, &s, &t, 1.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Waiting);
    assert_eq!(npc.alpha, 1.0);
    assert!(npc.can_talk());

    let ctx = TalkContext::from_desktop(&desk(500), Some(10.0));
    let mut rng = Rng::new(9);
    let lines = ctx.pick(&s.defs[id].conversations, &mut rng).expect("lines");
    let n = lines.len();
    ev.clear();
    assert!(npc.begin_talk(lines, 700.0, &s, &mut ev));
    assert_eq!(npc.percept(&s, 64.0).state, VisitorState::Talking);
    assert_eq!(npc.percept(&s, 64.0).line, Some(0));
    run(&mut npc, &s, &t, 40.0, &mut ev);
    let says = ev.iter().filter(|e| matches!(e, NpcEvent::Say { .. })).count();
    let dones = ev.iter().filter(|e| matches!(e, NpcEvent::LineDone)).count();
    assert!(says >= n && dones == n, "{says} says, {dones} done for {n} lines");
    assert!(ev.contains(&NpcEvent::ConversationOver));
    assert!(npc.greeted && !npc.can_talk());
    // Lingers, then leaves through a portal and fades away.
    assert!(ev.iter().any(|e| matches!(e, NpcEvent::OpenPortal(_))));
    assert_eq!(ev.last(), Some(&NpcEvent::Gone));
    assert_eq!(npc.phase(), NpcPhase::Gone);
}

#[test]
fn lines_play_in_order() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("elderbug").expect("elderbug");
    let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    let mut ev = Vec::new();
    run(&mut npc, &s, &t, 1.0, &mut ev);
    let lines = vec![("one".to_string(), Some("look_up".to_string())), ("two".to_string(), None)];
    ev.clear();
    assert!(npc.begin_talk(lines, 900.0, &s, &mut ev));
    assert_eq!(npc.percept(&s, 64.0).react.as_deref(), Some("look_up"));
    let mut seen = Vec::new();
    for _ in 0..(12.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t), &mut ev);
        let p = npc.percept(&s, 64.0);
        if let Some(l) = p.line
            && seen.last() != Some(&l)
        {
            seen.push(l);
        }
    }
    assert_eq!(seen, [0, 1]);
    let texts: Vec<&str> = ev.iter().filter_map(|e| if let NpcEvent::Say { text, .. } = e { Some(text.as_str()) } else { None }).collect();
    assert_eq!(texts, ["one", "two"]);
}

#[test]
fn rides_its_window_and_drops_when_it_closes() {
    let s = set();
    let t0 = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("quirrel").expect("quirrel");
    let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    npc.attach(&env(&t0));
    let mut ev = Vec::new();
    run(&mut npc, &s, &t0, 0.5, &mut ev);
    let t1 = Terrain::from_desktop(&desk(300), &TerrainOptions::default());
    run(&mut npc, &s, &t1, 0.1, &mut ev);
    assert_eq!(npc.pos, Vec2::new(800.0, 300.0));
    let mut gone = desk(300);
    gone.windows.clear();
    let t2 = Terrain::from_desktop(&gone, &TerrainOptions::default());
    run(&mut npc, &s, &t2, 2.0, &mut ev);
    assert_eq!((npc.pos.y, npc.owner), (1040.0, None), "fell to the floor");
}

#[test]
fn drop_visitor_falls_then_lands() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("zote").expect("zote");
    let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    assert!(npc.pos.y < 500.0, "starts above the ledge");
    assert_eq!(npc.clip(), s.clips[id].fall.expect("fall"));
    let mut ev = Vec::new();
    run(&mut npc, &s, &t, 0.5, &mut ev);
    assert_eq!(npc.pos.y, 500.0);
    assert_eq!(npc.owner, Some(7));
    assert_eq!(npc.phase(), NpcPhase::Arriving, "landing clips");
    run(&mut npc, &s, &t, 3.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Waiting);
    assert_eq!(npc.clip(), s.clips[id].idle);
}

#[test]
fn walker_walks_in_and_out() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("hornet").expect("hornet");
    let footings: Vec<Footing> = t.ledges.iter().map(|l| Footing { x0: l.x0, x1: l.x1, y: l.y, owner: l.owner }).collect();
    let mut rng = Rng::new(4);
    let knight = Vec2::new(900.0, 1040.0);
    let p = plan_placement(Arrival::Walk, &footings, knight, &desk(500).monitors, &mut rng, &mut |_| true).expect("placement");
    assert_eq!(p.feet.y, 1040.0);
    assert!(p.feet.x <= 2.0 || p.feet.x >= 1917.0, "starts at a screen edge: {}", p.feet.x);
    let to = p.walk_to.expect("walks");
    assert!((to - knight.x).abs() >= 200.0, "keeps some distance so the knight hurries over");
    let mut npc = Npc::new(1, id, &s, p, 3, 64.0);
    let mut ev = Vec::new();
    run(&mut npc, &s, &t, 4.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Waiting);
    assert_eq!(npc.pos.x, to);
    npc.leave(&s, &env(&t), &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Leaving);
    run(&mut npc, &s, &t, 6.0, &mut ev);
    assert_eq!(ev.last(), Some(&NpcEvent::Gone));
}

#[test]
fn placement_avoids_the_knight_and_perches_on_windows() {
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let footings: Vec<Footing> = t.ledges.iter().map(|l| Footing { x0: l.x0, x1: l.x1, y: l.y, owner: l.owner }).collect();
    let mut rng = Rng::new(11);
    for _ in 0..20 {
        let knight = Vec2::new(800.0, 500.0);
        let p = plan_placement(Arrival::Portal, &footings, knight, &desk(500).monitors, &mut rng, &mut |_| true).expect("spot");
        assert!(p.feet.distance(knight) >= 350.0);
        let perch = plan_placement(Arrival::Perch, &footings, Vec2::new(100.0, 1040.0), &desk(500).monitors, &mut rng, &mut |_| true).expect("perch");
        assert_eq!(perch.owner, Some(7), "perches on a window top");
    }
}
