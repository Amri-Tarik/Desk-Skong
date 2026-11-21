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
        let lesson_lines = def.lesson.iter().flat_map(|l| l.praise.iter().chain(&l.critique).chain(&l.closing));
        let reacts = def.conversations.iter().flatten().chain(lesson_lines).filter_map(|l| l.react()).chain(def.greet_react.as_deref());
        // Drills are built-in moves or knight acts.
        for d in def.lesson.iter().flat_map(|l| &l.drills) {
            let builtin = ["slash", "up_slash", "down_slash", "dash", "jump"].contains(&d.drill.as_str());
            assert!(builtin || acts.id(&d.drill).is_some(), "{}: unknown drill '{}'", def.name, d.drill);
        }
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
fn a_visitor_asked_to_talk_again_waits_for_the_knight() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("zote").expect("zote");
    let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    npc.attach(&env(&t));
    let mut ev = Vec::new();
    assert!(!npc.talk_again(), "still arriving: nothing to ask yet");
    for _ in 0..60 {
        if npc.can_talk() {
            break;
        }
        run(&mut npc, &s, &t, 0.5, &mut ev);
    }
    assert!(npc.can_talk() && npc.talk_again() && npc.can_talk());

    // After the talk he lingers; asked again, he waits for a second one, and long enough.
    let ctx = TalkContext::from_desktop(&desk(500), Some(10.0));
    let lines = ctx.pick(&s.defs[id].conversations, &mut Rng::new(9)).expect("lines");
    assert!(npc.begin_talk(lines, 700.0, &s, &mut ev));
    assert!(!npc.talk_again(), "mid-talk: left alone");
    while !npc.greeted {
        run(&mut npc, &s, &t, 0.5, &mut ev);
    }
    assert!(!npc.can_talk());
    assert!(npc.talk_again() && npc.can_talk());
    run(&mut npc, &s, &t, 40.0, &mut ev);
    assert!(npc.can_talk(), "stays at least {} s", actor::TALK_AGAIN_STAY);
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

/// Sly arrives, talks, then teaches: announce → (demo) → practice; the knight's report gets
/// feedback; after the closing line he bows and fades away.
#[test]
fn teacher_runs_a_lesson_after_the_talk() {
    use crate::ai::LessonStage;
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("sly").expect("sly");
    let def = &s.defs[id];
    let lesson_def = def.lesson.as_ref().expect("sly teaches");
    let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    npc.attach(&env(&t));
    let mut ev = Vec::new();
    run(&mut npc, &s, &t, 1.0, &mut ev);
    let ctx = TalkContext::from_desktop(&desk(500), Some(10.0));
    let mut rng = Rng::new(4);
    let lesson = Lesson::plan(lesson_def, &ctx, &mut rng).expect("lesson");
    let drills = lesson.drills.len();
    assert!(drills >= 4 && lesson.drills[0].drill == "slash", "{:?}", lesson.drills);
    npc.set_lesson(lesson);
    let lines = ctx.pick(&def.conversations, &mut rng).expect("lines");
    assert!(npc.begin_talk(lines, 700.0, &s, &mut ev));
    run(&mut npc, &s, &t, 30.0, &mut ev);
    assert!(ev.contains(&NpcEvent::ConversationOver));
    assert_eq!(npc.phase(), NpcPhase::Teaching);
    let p = npc.percept(&s, 64.0);
    assert_eq!(p.state, VisitorState::Teaching);
    assert!(p.greeted, "the talk is over: no second greeting");

    // Each drill: wait for the knight's turn, report it, hear the feedback.
    let mut served = Vec::new();
    for _ in 0..(120.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t), &mut ev);
        let p = npc.percept(&s, 64.0);
        if let Some(cue) = &p.lesson
            && cue.stage == LessonStage::Practice
            && !served.contains(&cue.serial)
        {
            served.push(cue.serial);
            npc.drilled(cue.serial, true, &s, &mut ev);
            let p = npc.percept(&s, 64.0);
            assert_eq!(p.lesson.as_ref().map(|c| c.stage), Some(LessonStage::Feedback));
            assert!(p.react.is_some(), "feedback carries the knight's answer");
            // A stale report changes nothing.
            npc.drilled(cue.serial, false, &s, &mut ev);
        }
        if npc.phase() == NpcPhase::Gone {
            break;
        }
    }
    assert_eq!(served, (1..=drills as u32).collect::<Vec<_>>(), "every drill in turn");
    assert!(ev.contains(&NpcEvent::LessonOver { finished: true }));
    assert_eq!(ev.last(), Some(&NpcEvent::Gone), "bowed and faded away");
}

#[test]
fn a_lesson_cut_short_says_goodbye_and_an_ignored_one_ends() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let id = s.id("sly").expect("sly");
    let start = |seed: u32| {
        let mut npc = Npc::new(1, id, &s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, seed, 64.0);
        let mut ev = Vec::new();
        run(&mut npc, &s, &t, 1.0, &mut ev);
        let mut rng = Rng::new(seed);
        npc.set_lesson(Lesson::plan(s.defs[id].lesson.as_ref().unwrap(), &TalkContext::default(), &mut rng).unwrap());
        assert!(npc.begin_talk(vec![("Hm.".into(), None)], 700.0, &s, &mut ev));
        run(&mut npc, &s, &t, 3.0, &mut ev);
        assert_eq!(npc.phase(), NpcPhase::Teaching);
        npc
    };
    // Stopped by the knight's side: a parting word, then he leaves.
    let mut npc = start(5);
    let mut ev = Vec::new();
    npc.end_lesson(&mut ev);
    assert!(ev.contains(&NpcEvent::LessonOver { finished: false }));
    assert!(ev.iter().any(|e| matches!(e, NpcEvent::Say { .. })), "says goodbye");
    run(&mut npc, &s, &t, 12.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Gone);

    // Nobody practises: after two unanswered drills he gives up.
    let mut npc = start(6);
    let mut ev = Vec::new();
    run(&mut npc, &s, &t, 90.0, &mut ev);
    assert!(ev.contains(&NpcEvent::LessonOver { finished: false }), "gave up on an absent student");
    assert_eq!(npc.phase(), NpcPhase::Gone);
}

fn env_at(t: &Terrain, knight: Vec2) -> NpcEnv<'_> {
    NpcEnv { terrain: t, knight, ppu: 64.0, area: RectF::new(0.0, 0.0, 1920.0, 1040.0) }
}

/// Zote, landed on the window top and done talking, about to challenge the knight.
fn zote_after_talk(s: &NpcSet, t: &Terrain, knight: Vec2, ev: &mut Vec<NpcEvent>) -> Npc {
    let id = s.id("zote").expect("zote");
    assert!(s.clips[id].spar.is_some(), "zote spars");
    let mut npc = Npc::new(1, id, s, Placement { feet: Vec2::new(800.0, 500.0), owner: Some(7), walk_to: None }, 3, 64.0);
    npc.attach(&env_at(t, knight));
    for _ in 0..(4.0 / DT) as usize {
        npc.tick(DT, s, &env_at(t, knight), ev);
    }
    assert!(npc.can_talk(), "{:?}", npc.phase());
    npc.plan_spar(s);
    assert!(npc.begin_talk(vec![("Halt!".into(), None), ("Now begone.".into(), None)], knight.x, s, ev));
    for _ in 0..(10.0 / DT) as usize {
        npc.tick(DT, s, &env_at(t, knight), ev);
        if npc.phase() == NpcPhase::Sparring {
            break;
        }
    }
    assert_eq!(npc.phase(), NpcPhase::Sparring, "the talk ends in a challenge");
    assert!(!said(ev, "Now begone."), "the goodbye waits until after the spar");
    npc
}

fn said(ev: &[NpcEvent], line: &str) -> bool {
    ev.iter().any(|e| matches!(e, NpcEvent::Say { text, .. } if text == line))
}

#[test]
fn zote_challenges_spars_goes_down_and_concedes() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let knight = Vec2::new(950.0, 500.0);
    let mut ev = Vec::new();
    let mut npc = zote_after_talk(&s, &t, knight, &mut ev);
    assert_eq!(npc.percept(&s, 64.0).state, VisitorState::Sparring);
    let f = npc.foe(&s, 64.0).expect("a sparring partner");
    assert_eq!(f.kind, crate::ai::FoeKind::Sparring);
    assert!(!f.vulnerable && f.waiting, "not while he is still saying his challenge");
    // On guard: now he can be struck; his charges catch a knight who stands there.
    let mut hits = 0;
    let mut charged = false;
    for _ in 0..(16.0 / DT) as usize {
        let mut e = Vec::new();
        npc.tick(DT, &s, &env_at(&t, knight), &mut e);
        hits += e.iter().filter(|e| matches!(e, NpcEvent::HitKnight)).count();
        charged |= npc.foe(&s, 64.0).is_some_and(|f| f.attacking);
    }
    assert!(charged && hits >= 1, "he charges at the knight ({hits} hits)");
    let x0 = npc.pos.x;
    assert!((400.0..=1200.0).contains(&x0), "never charges off his window: {x0}");
    // Five blows put him down (no death: he gets up and concedes, then leaves).
    let hits_needed = s.defs[npc.def].spar.as_ref().unwrap().hits;
    let mut down = false;
    let mut blows = 0;
    for _ in 0..(20.0 / DT) as usize {
        if npc.foe(&s, 64.0).is_some_and(|f| f.vulnerable) && !down {
            blows += 1;
            down = npc.spar_strike(1, knight.x, &s, &env_at(&t, knight));
            assert!(npc.flash > 0.9, "flashes when struck");
        }
        npc.tick(DT, &s, &env_at(&t, knight), &mut ev);
        if down {
            break;
        }
    }
    assert!(down && blows == hits_needed as usize, "{blows} blows");
    assert!(npc.foe(&s, 64.0).is_none(), "down: no longer a foe");
    ev.clear();
    for _ in 0..(30.0 / DT) as usize {
        npc.tick(DT, &s, &env_at(&t, knight), &mut ev);
    }
    assert!(ev.contains(&NpcEvent::SparOver { lost: true }));
    let lose = &s.defs[npc.def].spar.as_ref().unwrap().lose;
    let face = ev.iter().position(|e| matches!(e, NpcEvent::Say { text, .. } if lose.contains(text)));
    let bye = ev.iter().position(|e| matches!(e, NpcEvent::Say { text, .. } if text == "Now begone."));
    assert!(face.is_some(), "saves face: {ev:?}");
    assert!(bye > face, "then the talk's goodbye, last: {ev:?}");
    assert_eq!(npc.phase(), NpcPhase::Gone, "and leaves");
}

#[test]
fn a_spar_the_knight_walks_away_from_fizzles_out() {
    let s = set();
    let t = Terrain::from_desktop(&desk(500), &TerrainOptions::default());
    let mut ev = Vec::new();
    let mut npc = zote_after_talk(&s, &t, Vec2::new(950.0, 500.0), &mut ev);
    ev.clear();
    let far = Vec2::new(1900.0, 1040.0);
    for _ in 0..(12.0 / DT) as usize {
        npc.tick(DT, &s, &env_at(&t, far), &mut ev);
    }
    assert!(ev.contains(&NpcEvent::SparOver { lost: false }));
    let abandon = &s.defs[npc.def].spar.as_ref().unwrap().abandon;
    assert!(ev.iter().any(|e| matches!(e, NpcEvent::Say { text, .. } if abandon.contains(text))));
    assert!(npc.foe(&s, 64.0).is_none());
}

#[test]
fn every_scene_the_test_panel_forces_exists_in_the_data() {
    use super::forced::{ForcedScene, supports};
    let s = set();
    let def = |n: &str| &s.defs[s.id(n).unwrap_or_else(|| panic!("{n}"))];
    for (who, scene) in [
        ("sly", ForcedScene::Lesson),
        ("zote", ForcedScene::Spar),
        ("cornifer", ForcedScene::Company),
        ("quirrel", ForcedScene::Company),
        ("myla", ForcedScene::Company),
        ("hornet", ForcedScene::Race),
        ("hornet", ForcedScene::Duel),
    ] {
        assert_eq!(supports(def(who), scene), Ok(()), "{who}");
    }
    for d in &s.defs {
        assert_eq!(supports(d, ForcedScene::TalkOnly), Ok(()));
    }
    assert_eq!(supports(def("elderbug"), ForcedScene::Spar), Err("Elderbug has no spar scene".into()));
    // The calm scenes' hard conditions, as the data sets them.
    assert_eq!(def("myla").company.as_ref().map(|c| c.needs), Some(CompanyNeeds::Deposit));
    assert_eq!(def("quirrel").company.as_ref().map(|c| c.needs), Some(CompanyNeeds::View));
    assert_eq!(def("cornifer").company.as_ref().map(|c| c.needs), Some(CompanyNeeds::Any));
}
