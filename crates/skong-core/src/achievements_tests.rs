use std::path::Path;

use super::AchievementEvent as E;
use super::*;

fn defs() -> Vec<AchievementDef> {
    load_defs(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/achievements.json")).expect("data/achievements.json")
}

fn tracker() -> Tracker {
    let mut t = Tracker::new(defs(), AchievementSave::default());
    t.set_totals(Totals { visitors: 3, regions: 2, bench_styles: 2, enemies: 2 });
    t
}

/// Feed `events`; the ids they unlocked, in order.
fn feed(t: &mut Tracker, events: &[E]) -> Vec<String> {
    events.iter().flat_map(|e| t.on(e, 100).into_iter().map(|d| d.id.clone()).collect::<Vec<_>>()).collect()
}

#[test]
fn every_definition_has_a_criterion_and_original_text() {
    let d = defs();
    let ids: BTreeSet<&str> = d.iter().map(|d| d.id.as_str()).collect();
    let known: BTreeSet<&str> = ALL.iter().copied().collect();
    assert_eq!(ids, known, "data/achievements.json and ids::ALL must match");
    assert!((28..=48).contains(&d.len()), "a few dozen achievements: {}", d.len());
    for a in &d {
        assert!(!a.title.trim().is_empty() && !a.description.trim().is_empty(), "{}", a.id);
        // Hints stay vague: a short category, never the how.
        assert!(a.hint.as_deref().is_none_or(|h| h.split_whitespace().count() <= 2), "{}: hint too specific", a.id);
    }
}

#[test]
fn duplicate_ids_are_rejected() {
    let json = r#"{"achievements":[{"id":"pogo","title":"a","description":"b"},{"id":"pogo","title":"c","description":"d"}]}"#;
    assert!(parse_defs(json).is_err());
}

#[test]
fn one_shot_achievements_unlock_once_with_their_time() {
    let mut t = tracker();
    let cases = [
        (E::WindowDreamed, DREAM_WINDOW),
        (E::VisitorDreamed { who: "zote".into() }, DREAM_VISITOR),
        (E::SpellCast, FIRST_SPELL),
        (E::Pogo, POGO),
        (E::WindowRode, WINDOW_RIDE),
        (E::CursorBattleLost, CURSOR_DEFEAT),
        (E::DreamGateWarp, DREAM_GATE),
        (E::LessonFinished { who: "sly".into() }, SLY_LESSON),
        (E::GauntletCleared, GAUNTLET),
        (E::SlugForm, UNN_FORM),
        (E::TextHighlighted, HIGHLIGHTED_TEXT),
        (E::WindowShattered, SHATTERED_WINDOW),
        (E::PokedWhileNapping, NAP_POKE),
        (E::SuperHardLanding, SUPER_HARD_LANDING),
        (E::SteppedBehind, STEP_BEHIND),
        (E::TroupeMasterBeaten, TROUPE_MASTER),
        (E::FellToTroupeMaster, TROUPE_ENCORE),
    ];
    for (ev, id) in cases {
        assert_eq!(feed(&mut t, std::slice::from_ref(&ev)), [id], "{ev:?}");
        assert!(feed(&mut t, &[ev]).is_empty(), "{id} unlocks only once");
        assert_eq!(t.unlocked_at(id), Some(100));
    }
    assert!(t.take_dirty() && !t.take_dirty());
}

#[test]
fn hornet_is_the_one_to_talk_to() {
    let mut t = tracker();
    assert!(feed(&mut t, &[E::VisitorTalked { who: "zote".into() }]).is_empty());
    assert_eq!(feed(&mut t, &[E::VisitorTalked { who: "hornet".into() }]), [HORNET_TALK]);
}

#[test]
fn zote_must_lose_the_spar() {
    let mut t = tracker();
    assert!(feed(&mut t, &[E::SparWon { who: "someone".into() }]).is_empty());
    assert_eq!(feed(&mut t, &[E::SparWon { who: "zote".into() }]), [ZOTE_SPAR]);
}

#[test]
fn each_calm_scene_has_its_own_achievement() {
    let mut t = tracker();
    assert!(feed(&mut t, &[E::CompanyKept { who: "zote".into() }]).is_empty());
    let kept = |who: &str| E::CompanyKept { who: who.into() };
    assert_eq!(feed(&mut t, &[kept("cornifer"), kept("quirrel"), kept("myla"), kept("myla")]), [CORNIFER_MAPS, QUIRREL_VIEW, MYLA_SONG]);
}

#[test]
fn a_vengefly_felled_without_touching_the_floor() {
    // Struck only from ledges and mid-air, never landing on the floor: unlocked.
    let mut t = tracker();
    let clean = [
        E::EnemyStruck { foe: 1, on_floor: false },
        E::Landed { monitor: 0, floor: false },
        E::EnemyStruck { foe: 1, on_floor: false },
        E::EnemyKilled { foe: 1, kind: "vengefly".into() },
    ];
    assert_eq!(feed(&mut t, &clean), [VENGEFLY_AIRBORNE]);

    // One blow from the floor spoils it.
    let mut t = tracker();
    let from_floor = [E::EnemyStruck { foe: 2, on_floor: false }, E::EnemyStruck { foe: 2, on_floor: true }, E::EnemyKilled { foe: 2, kind: "vengefly".into() }];
    assert!(feed(&mut t, &from_floor).is_empty());

    // So does touching down on the floor between blows.
    let mut t = tracker();
    let touched = [E::EnemyStruck { foe: 3, on_floor: false }, E::Landed { monitor: 0, floor: true }, E::EnemyStruck { foe: 3, on_floor: false }, E::EnemyKilled { foe: 3, kind: "vengefly".into() }];
    assert!(feed(&mut t, &touched).is_empty());

    // A kill without any recorded blow doesn't count; nor does another kind.
    let mut t = tracker();
    assert!(feed(&mut t, &[E::EnemyKilled { foe: 4, kind: "vengefly".into() }]).is_empty());
    let mut t = tracker();
    assert!(feed(&mut t, &[E::EnemyStruck { foe: 5, on_floor: false }, E::EnemyKilled { foe: 5, kind: "crawlid".into() }]).is_empty());

    // Each fly is judged on its own: the floor before the second fight doesn't matter.
    let mut t = tracker();
    let separate = [
        E::EnemyStruck { foe: 6, on_floor: true },
        E::EnemyKilled { foe: 6, kind: "vengefly".into() },
        E::Landed { monitor: 0, floor: true },
        E::EnemyStruck { foe: 7, on_floor: false },
        E::EnemyKilled { foe: 7, kind: "vengefly".into() },
    ];
    assert_eq!(feed(&mut t, &separate), [VENGEFLY_AIRBORNE]);
}

#[test]
fn counting_achievements_reach_their_goal_and_persist() {
    let mut t = tracker();
    for _ in 0..4 {
        assert!(feed(&mut t, &[E::CursorBattleWon]).is_empty());
    }
    assert_eq!(t.save().counts[CURSOR_VICTOR], 4);
    assert_eq!(feed(&mut t, &[E::CursorBattleWon]), [CURSOR_VICTOR]);

    assert!(feed(&mut t, &[E::GeoCollected(600)]).is_empty());
    // A restart keeps the count.
    let mut t = Tracker::new(defs(), t.save().clone());
    assert!(feed(&mut t, &[E::GeoCollected(399)]).is_empty());
    assert_eq!(feed(&mut t, &[E::GeoCollected(25)]), [GEO_HOARD]);
    assert_eq!(t.save().counts[GEO_HOARD], 1000, "counts stop at the goal");
}

#[test]
fn wall_jumps_must_chain_without_landing() {
    let mut t = tracker();
    let four = [E::WallJumped, E::WallJumped, E::WallJumped, E::WallJumped];
    assert!(feed(&mut t, &four).is_empty());
    assert!(feed(&mut t, &[E::Landed { monitor: 0, floor: false }, E::WallJumped]).is_empty(), "landing starts over");
    assert!(feed(&mut t, &four[..3]).is_empty());
    assert_eq!(feed(&mut t, &[E::WallJumped]), [WALL_JUMP_CHAIN]);
}

#[test]
fn every_kind_counts_distinct_things_against_the_totals() {
    let mut t = tracker();
    let talk = |w: &str| E::VisitorTalked { who: w.into() };
    assert!(feed(&mut t, &[talk("zote"), talk("zote"), talk("cornifer")]).is_empty());
    assert_eq!(feed(&mut t, &[talk("elderbug")]), [EVERY_VISITOR]);

    assert!(feed(&mut t, &[E::DepositMined { region: 4 }, E::DepositMined { region: 4 }]).is_empty());
    assert_eq!(feed(&mut t, &[E::DepositMined { region: 7 }]), [EVERY_REGION]);

    assert!(feed(&mut t, &[E::BenchRested { style: 1 }]).is_empty());
    assert_eq!(feed(&mut t, &[E::BenchRested { style: 0 }]), [EVERY_BENCH]);

    let kill = |foe: u32, k: &str| E::EnemyKilled { foe, kind: k.into() };
    assert!(feed(&mut t, &[kill(1, "husk"), kill(2, "husk")]).is_empty());
    assert_eq!(feed(&mut t, &[kill(3, "crawlid")]), [EVERY_ENEMY]);

    // Unknown totals (nothing loaded) never unlock.
    let mut t = Tracker::new(defs(), AchievementSave::default());
    assert!(feed(&mut t, &[talk("a"), talk("b"), E::DepositMined { region: 0 }]).is_empty());
}

#[test]
fn a_fatal_fall_then_the_shade_won_back() {
    let mut t = tracker();
    // A fight death's Shade doesn't count.
    assert!(feed(&mut t, &[E::Died { shade: true, fall: false }, E::ShadeReclaimed]).is_empty());
    // A fall: it does, even across a restart before the Shade is beaten.
    assert!(feed(&mut t, &[E::Died { shade: true, fall: true }]).is_empty());
    let mut t = Tracker::new(defs(), t.save().clone());
    assert_eq!(feed(&mut t, &[E::ShadeReclaimed]), [FALL_AND_RECLAIM]);
    // A later hazard death (no Shade) doesn't spoil a waiting fall Shade.
    let mut t = tracker();
    assert!(feed(&mut t, &[E::Died { shade: true, fall: true }, E::Died { shade: false, fall: false }]).is_empty());
    assert_eq!(feed(&mut t, &[E::ShadeReclaimed]), [FALL_AND_RECLAIM]);
}

#[test]
fn crystal_dashes_off_walls() {
    let mut t = tracker();
    let plain = [E::SuperdashLaunched { monitor: 0, from_wall: false }, E::SuperdashFlying { monitor: 0 }, E::SuperdashOver];
    assert!(feed(&mut t, &plain).is_empty());
    // Crossing monitors is no achievement (everyone can reach 100 %).
    assert!(feed(&mut t, &[E::SuperdashLaunched { monitor: 0, from_wall: false }, E::SuperdashFlying { monitor: 1 }]).is_empty());
    assert_eq!(feed(&mut t, &[E::SuperdashLaunched { monitor: 1, from_wall: true }]), [WALL_CRYSTAL_DASH]);
}

#[test]
fn throws_and_midair_catches() {
    let mut t = tracker();
    assert_eq!(feed(&mut t, &[E::Thrown { monitor: 0 }]), [THROW]);
    assert!(feed(&mut t, &[E::Landed { monitor: 1, floor: true }]).is_empty(), "another monitor is nothing special");
    assert!(feed(&mut t, &[E::Grabbed]).is_empty(), "landed: a plain pick-up");
    assert_eq!(feed(&mut t, &[E::Thrown { monitor: 1 }, E::Grabbed]), [MIDAIR_CATCH]);
}

#[test]
fn no_achievement_needs_special_hardware_or_apps() {
    for gone in ["dash_across_monitors", "air_mail", "game_retreat"] {
        assert!(!ALL.contains(&gone), "{gone}");
    }
    let mut t = tracker();
    assert!(feed(&mut t, &[E::HidFromGame]).is_empty());
}

#[test]
fn removed_definitions_never_unlock_and_reset_forgets() {
    let mut d = defs();
    d.retain(|a| a.id != POGO);
    let mut t = Tracker::new(d, AchievementSave::default());
    assert!(feed(&mut t, &[E::Pogo]).is_empty());
    assert_eq!(feed(&mut t, &[E::SpellCast]), [FIRST_SPELL]);
    assert_eq!(t.unlocked_count(), 1);
    t.reset();
    assert_eq!(t.unlocked_count(), 0);
    assert_eq!(*t.save(), AchievementSave::default());
}

#[test]
fn progress_round_trips_through_the_file() {
    let mut t = tracker();
    feed(&mut t, &[E::Pogo, E::CursorBattleWon, E::DepositMined { region: 3 }, E::Died { shade: true, fall: true }]);
    let dir = std::env::temp_dir().join(format!("skong-ach-{}", std::process::id()));
    let path = dir.join("achievements.json");
    write_save(&path, t.save()).expect("write");
    let back = read_save(&path);
    assert_eq!(&back, t.save());
    assert!(back.unlocked.contains_key(POGO) && back.counts[CURSOR_VICTOR] == 1 && back.shade_from_fall);
    // Missing or malformed files start fresh.
    std::fs::write(&path, "{ nope").expect("write");
    assert_eq!(read_save(&path), AchievementSave::default());
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(read_save(&path), AchievementSave::default());
}
