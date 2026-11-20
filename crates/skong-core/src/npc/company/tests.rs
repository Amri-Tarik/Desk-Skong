//! Calm scenes on a fake desktop: the real `npcs.json` scenes resolve, the spot checks
//! (seats, the view, deposits), and the visitor's side of a scene from invitation to parting.
use std::path::Path;

use super::*;
use crate::acts::ActLibrary;
use crate::ai::{CompanyStage, VisitorState};
use crate::assets::SpriteBank;
use crate::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use crate::geom::RectI;
use crate::npc::{Npc, NpcEnv, NpcEvent, NpcPhase, NpcSet, TalkContext};
use crate::terrain::TerrainOptions;

const DT: f32 = 1.0 / 60.0;
const PPU: f32 = 64.0;

fn data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn set() -> NpcSet {
    let d = data();
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    NpcSet::load(&d.join("npcs.json"), &d.join("hk/animations"), &bank).expect("npcs.json")
}

fn window(id: u64, rect: RectI) -> WindowInfo {
    WindowInfo {
        id,
        rect,
        title: "notes.txt - Notepad".into(),
        class: "Test".into(),
        process: "notepad.exe".into(),
        pid: id as u32,
        maximized: false,
        fullscreen: false,
        foreground: false,
        topmost: false,
    }
}

/// One 1920x1080 monitor with the given windows.
fn desk(windows: Vec<WindowInfo>) -> DesktopSnapshot {
    DesktopSnapshot {
        generation: 1,
        monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
        windows,
    }
}

fn terrain(d: &DesktopSnapshot) -> Terrain {
    Terrain::from_desktop(d, &TerrainOptions::default())
}

fn env(t: &Terrain, knight: Vec2) -> NpcEnv<'_> {
    NpcEnv { terrain: t, knight, ppu: PPU, area: RectF::new(0.0, 0.0, 1920.0, 1040.0) }
}

fn run(npc: &mut Npc, s: &NpcSet, t: &Terrain, knight: Vec2, secs: f32, ev: &mut Vec<NpcEvent>) {
    for _ in 0..(secs / DT) as usize {
        npc.tick(DT, s, &env(t, knight), ev);
    }
}

/// `who` placed at `feet`, waiting, talked with by a knight at `knight`, with a scene planned.
fn after_talk(s: &NpcSet, t: &Terrain, who: &str, feet: Vec2, knight: Vec2, face: Option<f32>, ev: &mut Vec<NpcEvent>) -> Npc {
    let id = s.id(who).expect(who);
    let owner = t.ledges.iter().find(|l| (l.y - feet.y).abs() < 2.0 && l.x0 <= feet.x && feet.x <= l.x1).and_then(|l| l.owner);
    let mut npc = Npc::new(1, id, s, Placement { feet, owner, walk_to: None }, 5, PPU);
    npc.attach(&env(t, knight));
    run(&mut npc, s, t, knight, 1.0, ev);
    assert!(npc.can_talk(), "{who} waits");
    let lines = TalkContext::from_desktop(&desk(vec![]), Some(10.0)).pick(&s.defs[id].conversations, &mut Rng::new(3)).expect("lines");
    npc.plan_company(s, face);
    assert!(npc.company_planned());
    assert!(npc.begin_talk(lines, knight.x, s, ev));
    // Through the conversation, into the invitation.
    for _ in 0..(40.0 / DT) as usize {
        npc.tick(DT, s, &env(t, knight), ev);
        if npc.phase() == NpcPhase::Company {
            break;
        }
    }
    assert_eq!(npc.phase(), NpcPhase::Company, "{who}: the talk ends in an invitation");
    npc
}

fn stage(npc: &Npc, s: &NpcSet) -> Option<CompanyStage> {
    let v = npc.percept(s, PPU);
    assert_eq!(v.state, VisitorState::Company);
    v.company.map(|c| c.stage)
}

fn says(ev: &[NpcEvent]) -> Vec<String> {
    ev.iter().filter_map(|e| if let NpcEvent::Say { text, .. } = e { Some(text.clone()) } else { None }).collect()
}

#[test]
fn real_scenes_resolve_with_knight_acts() {
    let s = set();
    let d = data();
    let bank = SpriteBank::load_metadata(&d.join("hk")).expect("bank");
    let knight = crate::anim::ClipLibrary::load(&d.join("hk/animations/Knight.json"), &bank).expect("knight clips");
    let acts = ActLibrary::load(&d.join("acts.json"), &knight).expect("acts");
    for (who, needs) in [("cornifer", CompanyNeeds::Any), ("quirrel", CompanyNeeds::View), ("myla", CompanyNeeds::Deposit)] {
        let i = s.id(who).expect(who);
        let def = s.defs[i].company.as_ref().unwrap_or_else(|| panic!("{who}: a calm scene"));
        assert_eq!(def.needs, needs, "{who}");
        assert!(s.clips[i].company.is_some(), "{who}: clips");
        for a in def.knight.split('|') {
            let id = acts.id(a.trim()).unwrap_or_else(|| panic!("{who}: unknown knight act '{a}'"));
            // His part holds until the visitor parts.
            assert!(acts.get(id).steps.iter().any(|st| st.until.as_deref() == Some(crate::ai::PARTED)), "{who}: '{a}' waits for 'parted'");
        }
    }
    // Quirrel looks out at the view and turns back at the end.
    let q = s.id("quirrel").unwrap();
    let qc = s.clips[q].company.unwrap();
    assert_eq!(s.lib(q).get(qc.clip).name, "Bench Look");
    assert_eq!(qc.end.map(|e| s.lib(q).get(e).name.clone()).as_deref(), Some("Bench Look End"));
}

#[test]
fn validation_rejects_bad_scenes() {
    let ok = CompanyDef {
        chance: 0.5,
        needs: CompanyNeeds::Any,
        invite: vec!["Sit?".into()],
        knight: "company_maps".into(),
        clip: None,
        clip_end: None,
        secs: [10.0, 20.0],
        lines: vec!["Hm.".into()],
        every: [5.0, 6.0],
        parting: vec!["Bye.".into()],
        abandon: vec!["Oh.".into()],
    };
    assert!(ok.validate().is_ok());
    assert!(CompanyDef { chance: 2.0, ..ok.clone() }.validate().is_err());
    assert!(CompanyDef { parting: vec![], ..ok.clone() }.validate().is_err());
    assert!(CompanyDef { lines: vec!["On {title}".into()], ..ok.clone() }.validate().is_err());
    assert!(CompanyDef { knight: "a||b".into(), ..ok.clone() }.validate().is_err());
    assert!(CompanyDef { secs: [0.0, 1.0], ..ok }.validate().is_err());
}

#[test]
fn seats_beside_visitors_need_a_window_edge_and_the_view_needs_height() {
    // A high window, a low one, and (in front) a window just under the high one's left part.
    let d = desk(vec![window(3, RectI::new(300, 220, 500, 600)), window(1, RectI::new(300, 200, 1100, 700)), window(2, RectI::new(1200, 800, 1800, 1000))]);
    let t = terrain(&d);
    let work = RectF::new(0.0, 0.0, 1920.0, 1040.0);
    // On the high window's top, the knight beside: a seat with a view.
    let v = Vec2::new(800.0, 200.0);
    let l = seat_beside(&t, v, 800.0 - 1.6 * PPU, PPU).expect("seat on the window top");
    assert_eq!(l.owner, Some(1));
    assert!(high_up(l.y, work));
    // Off the end of the ledge: no seat.
    assert!(seat_beside(&t, v, 1150.0, PPU).is_none());
    // Something right below his seat (the window just under the edge): his legs would dangle into it.
    assert!(seat_beside(&t, Vec2::new(520.0, 200.0), 400.0, PPU).is_none());
    // The desktop floor is not a seat.
    assert!(seat_beside(&t, Vec2::new(800.0, 1040.0), 700.0, PPU).is_none());
    // The low window: a seat, but no view.
    let low = seat_beside(&t, Vec2::new(1500.0, 800.0), 1400.0, PPU).expect("seat on the low window");
    assert!(!high_up(low.y, work));
}

#[test]
fn myla_stands_beside_the_deposit_away_from_where_the_knight_mines() {
    let footing = [Footing { x0: 300.0, x1: 1100.0, y: 500.0, owner: Some(4) }];
    // Rock at 700, the knight mines from its left (at 640).
    let deposits = [(Vec2::new(700.0, 500.0), Vec2::new(640.0, 500.0))];
    let (at, rock) = beside_deposit(&deposits, &footing, PPU, &mut Rng::new(1), &mut |_| true).expect("a spot");
    assert_eq!(rock, deposits[0].0);
    assert_eq!(at.owner, Some(4));
    assert!(at.feet.x > 700.0 && at.feet.x - 700.0 <= 2.0 * PPU, "right of the rock, close by: {}", at.feet);
    assert_eq!(at.feet.y, 500.0);
    // That side blocked: further along on the knight's side, past his mining spot.
    let (at, _) = beside_deposit(&deposits, &footing, PPU, &mut Rng::new(1), &mut |p| p.x < 700.0).expect("a spot");
    assert!(at.feet.x < 640.0 - PPU, "{}", at.feet);
    // No deposit, or one off every footing: nowhere.
    assert!(beside_deposit(&[], &footing, PPU, &mut Rng::new(1), &mut |_| true).is_none());
    let off = [(Vec2::new(1500.0, 500.0), Vec2::new(1440.0, 500.0))];
    assert!(beside_deposit(&off, &footing, PPU, &mut Rng::new(1), &mut |_| true).is_none());
}

#[test]
fn cornifer_invites_hums_while_the_knight_is_near_then_parts() {
    let s = set();
    let d = desk(vec![window(7, RectI::new(400, 500, 1200, 900))]);
    let t = terrain(&d);
    let feet = Vec2::new(800.0, 500.0);
    let knight = Vec2::new(800.0 - 2.4 * PPU, 500.0);
    let mut ev = Vec::new();
    let mut npc = after_talk(&s, &t, "cornifer", feet, knight, None, &mut ev);
    assert_eq!(stage(&npc, &s), Some(CompanyStage::Invite));
    let v = npc.percept(&s, PPU);
    assert_eq!(v.company.as_ref().map(|c| c.act.as_str()), Some("company_maps|company_maps_stand"));
    assert!(v.greeted, "the talk is done: not greeted again");
    let invite = says(&ev).pop().expect("the invitation");
    assert!(s.defs[npc.def].company.as_ref().unwrap().invite.contains(&invite));

    // Together: he draws (his idle) and hums now and then.
    ev.clear();
    run(&mut npc, &s, &t, knight, 6.0, &mut ev);
    assert_eq!(stage(&npc, &s), Some(CompanyStage::Together));
    assert_eq!(npc.clip(), s.clips[npc.def].idle);
    run(&mut npc, &s, &t, knight, 20.0, &mut ev);
    let lines = &s.defs[npc.def].company.as_ref().unwrap().lines;
    let hums = says(&ev);
    assert!(!hums.is_empty() && hums.iter().all(|h| lines.contains(h)), "{hums:?}");

    // The knight steps away briefly: the clock stops, nothing is said.
    ev.clear();
    let away = Vec2::new(1500.0, 1040.0);
    run(&mut npc, &s, &t, away, 8.0, &mut ev);
    assert_eq!(stage(&npc, &s), Some(CompanyStage::Together), "waits for him");
    assert!(says(&ev).is_empty());

    // Back beside him: the scene runs out, the parting line, then he lingers and leaves.
    ev.clear();
    run(&mut npc, &s, &t, knight, 60.0, &mut ev);
    let parting = &s.defs[npc.def].company.as_ref().unwrap().parting;
    assert!(says(&ev).iter().any(|l| parting.contains(l)), "{:?}", says(&ev));
    assert!(ev.contains(&NpcEvent::CompanyOver { finished: true }));
    assert!(!ev.contains(&NpcEvent::CompanyOver { finished: false }));
    assert!(matches!(npc.phase(), NpcPhase::Leaving | NpcPhase::Gone), "{:?}", npc.phase());
}

#[test]
fn quirrel_takes_in_the_view_and_turns_back_at_the_end() {
    let s = set();
    let d = desk(vec![window(7, RectI::new(400, 200, 1200, 700))]);
    let t = terrain(&d);
    let feet = Vec2::new(800.0, 200.0);
    let knight = Vec2::new(800.0 - 1.6 * PPU, 200.0);
    let mut ev = Vec::new();
    let mut npc = after_talk(&s, &t, "quirrel", feet, knight, None, &mut ev);
    let q = npc.def;
    let qc = s.clips[q].company.unwrap();
    run(&mut npc, &s, &t, knight, 6.0, &mut ev);
    assert_eq!(npc.clip(), qc.clip, "looks out at the view");
    // Parting, then his look ends.
    let mut saw_end = false;
    for _ in 0..(80.0 / DT) as usize {
        npc.tick(DT, &s, &env(&t, knight), &mut ev);
        saw_end |= Some(npc.clip()) == qc.end;
        if npc.phase() != NpcPhase::Company {
            break;
        }
    }
    assert!(saw_end, "Bench Look End plays");
    assert!(ev.contains(&NpcEvent::CompanyOver { finished: true }));
}

#[test]
fn myla_faces_her_rock_while_she_sings() {
    let s = set();
    let d = desk(vec![window(7, RectI::new(400, 500, 1200, 900))]);
    let t = terrain(&d);
    let feet = Vec2::new(800.0, 500.0);
    // The knight on her left, the rock on her right.
    let knight = Vec2::new(800.0 - 1.8 * PPU, 500.0);
    let mut ev = Vec::new();
    let mut npc = after_talk(&s, &t, "myla", feet, knight, Some(870.0), &mut ev);
    assert!(!npc.looking_right, "turned to the knight for the invitation");
    run(&mut npc, &s, &t, knight, 6.0, &mut ev);
    assert_eq!(stage(&npc, &s), Some(CompanyStage::Together));
    assert!(npc.looking_right, "back to the rock, mining");
    assert_eq!(npc.clip(), s.clips[npc.def].idle, "her idle is the mining swing");
    ev.clear();
    run(&mut npc, &s, &t, knight, 20.0, &mut ev);
    assert!(says(&ev).len() >= 2, "sings: {:?}", says(&ev));
}

#[test]
fn a_scene_cut_short_says_goodbye_and_an_abandoned_one_ends() {
    let s = set();
    let d = desk(vec![window(7, RectI::new(400, 500, 1200, 900))]);
    let t = terrain(&d);
    let feet = Vec2::new(800.0, 500.0);
    let knight = Vec2::new(800.0 - 2.4 * PPU, 500.0);

    // Called away: an abandon line, the scene is over (not finished), he leaves as usual.
    let mut ev = Vec::new();
    let mut npc = after_talk(&s, &t, "cornifer", feet, knight, None, &mut ev);
    run(&mut npc, &s, &t, knight, 8.0, &mut ev);
    ev.clear();
    npc.end_company(&s, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Waiting);
    let abandon = &s.defs[npc.def].company.as_ref().unwrap().abandon;
    assert!(says(&ev).iter().all(|l| abandon.contains(l)) && says(&ev).len() == 1, "{:?}", says(&ev));
    assert!(ev.contains(&NpcEvent::CompanyOver { finished: false }));
    assert!(!npc.company_planned());
    run(&mut npc, &s, &t, knight, 30.0, &mut ev);
    assert_eq!(npc.phase(), NpcPhase::Gone);

    // The knight wanders off and never comes back: given up after a while.
    let mut ev = Vec::new();
    let mut npc = after_talk(&s, &t, "cornifer", feet, knight, None, &mut ev);
    run(&mut npc, &s, &t, Vec2::new(1700.0, 1040.0), 15.0, &mut ev);
    assert!(ev.contains(&NpcEvent::CompanyOver { finished: false }));
    assert_ne!(npc.phase(), NpcPhase::Company);

    // Leaving outright (visits switched off) mid-scene also reports it cut short.
    let mut ev = Vec::new();
    let mut npc = after_talk(&s, &t, "quirrel", Vec2::new(800.0, 500.0), knight, None, &mut ev);
    ev.clear();
    npc.leave(&s, &env(&t, knight), &mut ev);
    assert!(ev.contains(&NpcEvent::CompanyOver { finished: false }));
}
