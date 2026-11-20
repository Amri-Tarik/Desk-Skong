//! Visitors picked up by the user (`Npc::grab` / `set_down`), against the real data: a talk
//! pauses and goes on where it stopped, scenes in progress end and they take their leave,
//! arrivals and departures carry on.
use std::path::Path;

use super::*;
use crate::ai::VisitorState;
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::{RectF, RectI, Vec2};
use crate::terrain::{Terrain, TerrainOptions};

const DT: f32 = 1.0 / 60.0;
const PPU: f32 = 64.0;

fn set() -> NpcSet {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    NpcSet::load(&d.join("npcs.json"), &d.join("hk/animations"), &bank).expect("npcs.json")
}

fn terrain() -> Terrain {
    let desk = DesktopSnapshot {
        generation: 1,
        monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
        windows: vec![WindowInfo {
            id: 7,
            rect: RectI::new(400, 500, 1200, 900),
            title: "notes.txt - Notepad".into(),
            class: "Test".into(),
            process: "notepad.exe".into(),
            pid: 7,
            maximized: false,
            fullscreen: false,
            foreground: true,
            topmost: false,
        }],
    };
    Terrain::from_desktop(&desk, &TerrainOptions::default())
}

fn env(t: &Terrain) -> NpcEnv<'_> {
    NpcEnv { terrain: t, knight: Vec2::new(700.0, 500.0), ppu: PPU, area: RectF::new(0.0, 0.0, 1920.0, 1040.0) }
}

fn run(npc: &mut Npc, s: &NpcSet, t: &Terrain, secs: f32, ev: &mut Vec<NpcEvent>) {
    for _ in 0..(secs / DT) as usize {
        npc.tick(DT, s, &env(t), ev);
        if npc.phase() == NpcPhase::Gone {
            break;
        }
    }
}

/// `who`, arrived on the window top and waiting.
fn arrived(s: &NpcSet, t: &Terrain, who: &str) -> Npc {
    let id = s.id(who).expect(who);
    let mut npc = Npc::new(1, id, s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, PPU);
    npc.attach(&env(t));
    let mut ev = Vec::new();
    run(&mut npc, s, t, 4.0, &mut ev);
    assert!(npc.can_talk(), "{who}: {:?}", npc.phase());
    npc
}

fn says(ev: &[NpcEvent]) -> Vec<String> {
    ev.iter().filter_map(|e| if let NpcEvent::Say { text, .. } = e { Some(text.clone()) } else { None }).collect()
}

#[test]
fn every_visitor_has_something_to_say_about_being_picked_up() {
    let s = set();
    for d in &s.defs {
        assert!(!d.handled.picked_up.is_empty(), "{}: handled.picked_up", d.name);
        assert!(!d.handled.set_down.is_empty(), "{}: handled.set_down", d.name);
    }
}

#[test]
fn picked_up_mid_talk_the_talk_goes_on_where_it_stopped() {
    let s = set();
    let t = terrain();
    let mut npc = arrived(&s, &t, "elderbug");
    let lines = vec![("one".to_string(), None), ("two".to_string(), None), ("three".to_string(), None)];
    let mut ev = Vec::new();
    assert!(npc.begin_talk(lines, 700.0, &s, &mut ev));
    // Into the second line.
    for _ in 0..(20.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t), &mut ev);
        if npc.percept(&s, PPU).line == Some(1) {
            break;
        }
    }
    assert_eq!(npc.percept(&s, PPU).line, Some(1));
    ev.clear();
    assert!(npc.grab(&s, &mut ev));
    assert!(ev.contains(&NpcEvent::LineDone), "the line's bubble closes");
    let picked = &s.defs[npc.def].handled.picked_up;
    assert!(says(&ev).iter().any(|l| picked.contains(l)), "a surprised word: {ev:?}");
    assert_eq!((npc.phase(), npc.percept(&s, PPU).state), (NpcPhase::Held, VisitorState::Held));
    // Carried far off; nothing of their own meanwhile (they don't fall or walk).
    npc.hold_at(Vec2::new(300.0, 200.0));
    run(&mut npc, &s, &t, 2.0, &mut ev);
    assert_eq!(npc.pos, Vec2::new(300.0, 200.0));
    // Set down on the floor: they wait for the knight, not yet greeted.
    ev.clear();
    npc.set_down(Vec2::new(300.0, 1040.0), None, &s, &env(&t), &mut ev);
    assert_eq!(npc.percept(&s, PPU).state, VisitorState::Waiting);
    assert!(npc.can_talk() && npc.resuming() && !npc.greeted);
    run(&mut npc, &s, &t, 5.0, &mut ev);
    assert_eq!(npc.pos, Vec2::new(300.0, 1040.0), "stays where it was put");
    // He comes over: the talk goes on from the line it stopped at (fresh lines ignored).
    ev.clear();
    assert!(npc.begin_talk(vec![("fresh".into(), None), ("lines".into(), None)], 400.0, &s, &mut ev));
    run(&mut npc, &s, &t, 20.0, &mut ev);
    let heard = says(&ev);
    assert_eq!(heard.iter().filter(|l| ["one", "two", "three", "fresh", "lines"].contains(&l.as_str())).cloned().collect::<Vec<_>>(), ["two", "three"]);
    assert!(ev.contains(&NpcEvent::ConversationOver));
    assert!(npc.greeted);
}

#[test]
fn picked_up_mid_lesson_the_lesson_ends_and_they_leave_politely() {
    let s = set();
    let t = terrain();
    let mut npc = arrived(&s, &t, "sly");
    let mut rng = crate::ai::rng::Rng::new(5);
    npc.set_lesson(Lesson::plan(s.defs[npc.def].lesson.as_ref().unwrap(), &TalkContext::default(), &mut rng).unwrap());
    let mut ev = Vec::new();
    assert!(npc.begin_talk(vec![("Hm.".into(), None)], 700.0, &s, &mut ev));
    run(&mut npc, &s, &t, 3.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Teaching);
    ev.clear();
    assert!(npc.grab(&s, &mut ev));
    assert!(ev.contains(&NpcEvent::LessonOver { finished: false }));
    assert_eq!(npc.percept(&s, PPU).lesson, None, "no drills while held");
    ev.clear();
    npc.set_down(Vec2::new(1500.0, 1040.0), None, &s, &env(&t), &mut ev);
    let parting = &s.defs[npc.def].handled.set_down;
    assert!(says(&ev).iter().any(|l| parting.contains(l)), "a parting word: {ev:?}");
    assert!(npc.greeted && !npc.can_talk(), "no second greeting");
    run(&mut npc, &s, &t, 20.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Gone);
}

#[test]
fn picked_up_mid_spar_it_fizzles_out_without_the_held_back_goodbye() {
    let s = set();
    let t = terrain();
    let mut npc = arrived(&s, &t, "zote");
    npc.plan_spar(&s);
    let mut ev = Vec::new();
    assert!(npc.begin_talk(vec![("Halt!".into(), None), ("Now begone.".into(), None)], 700.0, &s, &mut ev));
    for _ in 0..(10.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t), &mut ev);
        if npc.phase() == NpcPhase::Sparring {
            break;
        }
    }
    assert_eq!(npc.phase(), NpcPhase::Sparring);
    ev.clear();
    assert!(npc.grab(&s, &mut ev));
    assert!(ev.contains(&NpcEvent::SparOver { lost: false }));
    assert!(npc.foe(&s, PPU).is_none(), "not a foe while held");
    npc.set_down(Vec2::new(1500.0, 1040.0), None, &s, &env(&t), &mut ev);
    run(&mut npc, &s, &t, 20.0, &mut ev);
    assert!(!says(&ev).contains(&"Now begone.".to_string()));
    assert_eq!(npc.phase(), NpcPhase::Gone);
}

#[test]
fn hornets_challenge_is_called_off_and_her_needle_put_away() {
    let s = set();
    let t = terrain();
    let mut npc = arrived(&s, &t, "hornet");
    npc.plan_challenge(ChallengePlan::Duel, &s);
    let mut ev = Vec::new();
    assert!(npc.begin_talk(vec![("Ghost.".into(), None)], 700.0, &s, &mut ev));
    for _ in 0..(8.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t), &mut ev);
        if npc.challenging() && npc.phase() == NpcPhase::Sparring {
            break;
        }
    }
    assert_eq!(npc.phase(), NpcPhase::Sparring, "the duel is on");
    ev.clear();
    assert!(npc.grab(&s, &mut ev));
    assert!(ev.contains(&NpcEvent::DuelOver { finished: false }));
    assert!(!npc.challenging() && npc.props(&s, PPU).is_empty());
    npc.set_down(Vec2::new(1500.0, 1040.0), None, &s, &env(&t), &mut ev);
    run(&mut npc, &s, &t, 20.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Gone);
}

#[test]
fn arrivals_waits_and_departures_carry_on_after_the_drop() {
    let s = set();
    let t = terrain();
    // Waiting: picked up and set down, still waiting (and greeted as usual).
    let mut npc = arrived(&s, &t, "cornifer");
    let mut ev = Vec::new();
    assert!(npc.grab(&s, &mut ev));
    npc.set_down(Vec2::new(1000.0, 500.0), Some(7), &s, &env(&t), &mut ev);
    assert!(npc.can_talk() && !npc.resuming());
    run(&mut npc, &s, &t, 5.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Waiting);
    // Sent away while held: they go once down.
    let mut npc = arrived(&s, &t, "quirrel");
    let mut ev = Vec::new();
    assert!(npc.grab(&s, &mut ev));
    npc.leave(&s, &env(&t), &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Held, "not while in the hand");
    npc.set_down(Vec2::new(1000.0, 1040.0), None, &s, &env(&t), &mut ev);
    run(&mut npc, &s, &t, 30.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Gone);
}
