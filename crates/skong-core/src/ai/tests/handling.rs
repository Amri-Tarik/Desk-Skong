//! How the user handles him: clicks try his patience (a look, an anger mark, then a fight
//! with the cursor), a shake leaves him dizzy, repeated throws make him wary, a gentle carry
//! is fine. See `ai::handling`.
use super::*;
use crate::ai::handling::{CLICK_MEMORY, PROVOKED_SECS, WARY_SECS};

/// Calm and content (no mischief, not bored), the cursor resting beside him, and no
/// ordinary battle for ages: any fight is a provoked one.
fn content_with_cursor() -> Sim {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.boredom = 0.2;
    sim.park_cursor(Vec2::new(60.0, -10.0));
    sim.brain.battle_ready_at = 1e9;
    sim.brain.pester_ready_at = 1e9;
    sim.run(2.0);
    sim
}

/// `n` clicks, `gap` seconds apart; the intents of the clicks' own ticks.
fn clicks(sim: &mut Sim, n: usize, gap: f32) -> Vec<Intent> {
    let mut out = Vec::new();
    for i in 0..n {
        if i > 0 {
            sim.run(gap);
        }
        sim.pokes.push(Poke::Click { pos: sim.body_center() });
        out.push(sim.step());
    }
    out
}

fn emotes(sim: &Sim, from: usize, e: Expression) -> usize {
    sim.intents[from..].iter().filter(|i| i.emote == Some(e)).count()
}

#[test]
fn a_click_turns_his_head() {
    let mut sim = calm(BrainConfig::default());
    sim.run(2.0);
    for (dx, side) in [(-40.0, Side::Left), (40.0, Side::Right)] {
        sim.pokes.push(Poke::Click { pos: sim.body_center() + Vec2::new(dx, 0.0) });
        let i = sim.step();
        assert_eq!(i.face, Some(side), "looks toward the click");
        assert_eq!(i.emote, None, "a single click: just a look");
        sim.run(CLICK_MEMORY + 1.0);
    }
    assert!(!sim.switches.iter().any(|s| s.to == BehaviourKind::Flee), "a click is no fright");
}

#[test]
fn a_second_click_earns_an_anger_mark() {
    let mut sim = content_with_cursor();
    let got = clicks(&mut sim, 2, 3.0);
    assert_eq!(got[0].emote, None);
    assert_eq!(got[1].emote, Some(Expression::Angry), "{}", sim.last_reason());
    // Forgotten after a while: a click is just a look again.
    sim.run(CLICK_MEMORY + 1.0);
    assert_eq!(clicks(&mut sim, 1, 0.0)[0].emote, None);
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Battle);
}

#[test]
fn five_quick_clicks_start_a_fight_with_the_cursor() {
    let mut sim = content_with_cursor();
    assert_ne!(sim.brain.behaviour(), BehaviourKind::Battle);
    clicks(&mut sim, 5, 0.6);
    assert!(sim.run_until(1.0, BehaviourKind::Battle), "provoked: {}", sim.last_reason());
    assert!(sim.last_reason().contains("provoked"), "{}", sim.last_reason());
    sim.run(6.0);
    assert!(sim.started.iter().any(|a| a == "challenge"), "{:?}", sim.started);
    assert!(!sim.switches.iter().any(|s| s.to == BehaviourKind::Flee), "clicks don't scare him off");
}

#[test]
fn slow_or_few_clicks_never_start_a_fight() {
    let mut sim = content_with_cursor();
    clicks(&mut sim, 4, 0.6);
    let kinds = sim.run(5.0);
    assert!(!kinds.contains(&BehaviourKind::Battle), "four is not enough");
    // A click every two seconds for a minute: never five within the window.
    sim.run(CLICK_MEMORY + 1.0);
    let n = sim.intents.len();
    clicks(&mut sim, 30, 2.0);
    let kinds: Vec<_> = sim.intents[n..].iter().filter_map(|i| i.behaviour).collect();
    assert!(!kinds.contains(&BehaviourKind::Battle), "slow clicks decay");
    // ... and the anger mark stays rare (its own cooldown).
    assert!(emotes(&sim, n, Expression::Angry) <= 2, "{} anger marks in a minute", emotes(&sim, n, Expression::Angry));
}

#[test]
fn no_provoked_fight_under_a_fullscreen_app() {
    let mut sim = calm(BrainConfig { hide_for_fullscreen: true, ..Default::default() });
    sim.park_cursor(Vec2::new(60.0, -10.0));
    sim.brain.battle_ready_at = 1e9;
    sim.run(1.0);
    let mut w = window(20, RectI::new(0, 0, 1920, 1080), "game.exe");
    w.fullscreen = true;
    w.foreground = true;
    sim.open(w);
    assert!(sim.run_until(0.5, BehaviourKind::Hide));
    clicks(&mut sim, 6, 0.4);
    let kinds = sim.run(5.0);
    assert!(!kinds.contains(&BehaviourKind::Battle), "{kinds:?}");
}

#[test]
fn typing_buys_patience_and_the_fight_waits_for_the_typing_to_stop() {
    let mut sim = content_with_cursor();
    sim.user.keys_per_sec = 6.0;
    sim.run(1.0);
    let got = clicks(&mut sim, 3, 0.6);
    assert!(got.iter().all(|i| i.emote.is_none()), "typing: these clicks aren't aimed at him");
    clicks(&mut sim, 3, 0.6);
    assert!(!sim.run(2.0).contains(&BehaviourKind::Battle), "six while typing: still patient");
    clicks(&mut sim, 1, 0.0);
    let kinds = sim.run(3.0);
    assert!(!kinds.contains(&BehaviourKind::Battle), "not while the user types");
    sim.user.keys_per_sec = 0.0;
    assert!(sim.run_until(2.0, BehaviourKind::Battle), "provoked, once the typing stops: {}", sim.last_reason());
}

#[test]
fn a_provocation_lapses_if_it_has_to_wait_too_long() {
    let mut sim = content_with_cursor();
    // Provoked between bursts of typing, and the typing goes on.
    sim.user.keys_per_sec = 6.0;
    clicks(&mut sim, 7, 0.6);
    let kinds = sim.run(PROVOKED_SECS + 1.0);
    assert!(!kinds.contains(&BehaviourKind::Battle));
    sim.user.keys_per_sec = 0.0;
    let kinds = sim.run(5.0);
    assert!(!kinds.contains(&BehaviourKind::Battle), "the provocation has lapsed");
}

#[test]
fn a_visitor_comes_before_a_provoked_fight() {
    let mut sim = content_with_cursor();
    sim.brain.see_visitors(vec![Visitor { id: 7, name: "cornifer".into(), pos: Vec2::new(1500.0, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() }]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet));
    clicks(&mut sim, 6, 0.4);
    let kinds = sim.run(2.0);
    assert!(kinds.iter().all(|k| *k == BehaviourKind::Greet), "a visitor comes first: {kinds:?}");
}

#[test]
fn shaken_he_reels_dizzily() {
    let mut sim = calm(BrainConfig::default());
    sim.run(2.0);
    let n = sim.intents.len();
    sim.shake();
    assert!(sim.run_until(1.0, BehaviourKind::Dizzy), "{}", sim.last_reason());
    sim.run(0.2);
    assert_eq!(sim.started.last().map(String::as_str), Some("stunned"));
    assert_eq!(emotes(&sim, n, Expression::Lost), 1, "a dazed \"?\"");
    assert!(sim.run_while(3.0, |s| s.brain.behaviour() == BehaviourKind::Dizzy), "it passes");
}

#[test]
fn a_gentle_carry_is_fine() {
    let mut sim = calm(BrainConfig::default());
    sim.run(2.0);
    let annoyed = sim.brain.mind().emotions.annoyance;
    sim.handle(2.0, |t| Vec2::new(200.0 * t, -100.0 + 5.0 * (t * 20.0).sin()), Vec2::new(40.0, 0.0));
    let kinds = sim.run(4.0);
    assert!(!kinds.contains(&BehaviourKind::Dizzy) && !kinds.contains(&BehaviourKind::Flee), "{kinds:?}");
    assert!(sim.brain.mind().emotions.annoyance <= annoyed, "no annoyance");
    assert!(!sim.intents.iter().any(|i| i.emote == Some(Expression::Angry)));
}

#[test]
fn thrown_around_he_turns_wary_of_the_cursor() {
    let mut sim = content_with_cursor();
    sim.brain.pester_ready_at = 0.0;
    sim.brain.battle_ready_at = 0.0;
    let fling = |t: f32| Vec2::new(300.0 * t, -150.0 * t);
    sim.handle(0.4, fling, Vec2::new(2500.0, -600.0));
    sim.run(10.0);
    sim.handle(0.4, fling, Vec2::new(2500.0, -600.0));
    sim.run(10.0);
    assert!(!sim.brain.senses().wary, "two throws: not yet");
    let mischief = sim.brain.mind().needs.mischief;
    let n = sim.intents.len();
    sim.handle(0.4, fling, Vec2::new(2500.0, -600.0));
    assert_eq!(emotes(&sim, n, Expression::Angry), 1, "grumpy");
    sim.step();
    assert!(sim.brain.senses().wary);
    assert!(sim.brain.mind().needs.mischief > mischief + 0.2, "he'll get his own back later");
    // Wary: no playing with the cursor for a while, however mischievous.
    sim.brain.mind_mut().needs.mischief = 1.0;
    let kinds = sim.run(30.0);
    assert!(!kinds.contains(&BehaviourKind::Battle) && !kinds.contains(&BehaviourKind::Pester), "{kinds:?}");
    sim.run(WARY_SECS);
    assert!(!sim.brain.senses().wary, "it wears off");
}
