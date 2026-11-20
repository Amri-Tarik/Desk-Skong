//! Expressions from whole brain runs: which situations show which bubble, and that a long
//! stretch of ordinary life stays quiet (see `ai::expression`).
use super::*;

fn emotes(sim: &Sim, from: usize) -> Vec<Expression> {
    sim.intents[from..].iter().filter_map(|i| i.emote).collect()
}

fn visitor(id: u32, x: f32) -> Visitor {
    Visitor { id, name: "cornifer".into(), pos: Vec2::new(x, FLOOR), state: VisitorState::Waiting, excitement: 1.0, ..Default::default() }
}

#[test]
fn a_cursor_worth_fighting_makes_him_angry_once() {
    let mut sim = parked_cursor_sim(BrainConfig::default());
    assert!(sim.run_until(4.0, BehaviourKind::Battle), "{}", sim.last_reason());
    sim.run(20.0);
    let got = emotes(&sim, 0);
    assert_eq!(got.iter().filter(|e| **e == Expression::Angry).count(), 1, "{got:?}");
}

#[test]
fn a_nice_chat_ends_with_a_heart() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    let mut v = visitor(5, 1500.0);
    sim.brain.see_visitors(vec![v.clone()]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet));
    assert_eq!(emotes(&sim, 0), [Expression::Alert], "a visitor!");
    sim.run(8.0);
    v.state = VisitorState::Talking;
    v.line = Some(0);
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(3.0);
    let n = sim.intents.len();
    v.state = VisitorState::Waiting;
    v.line = None;
    v.greeted = true;
    sim.brain.see_visitors(vec![v.clone()]);
    sim.run(6.0);
    assert!(emotes(&sim, n).contains(&Expression::Fond), "{:?}", emotes(&sim, n));
}

#[test]
fn an_unreachable_visitor_gets_a_question() {
    let mut sim = calm(BrainConfig::default());
    sim.run(1.0);
    sim.wall = true;
    sim.brain.see_visitors(vec![visitor(3, 1500.0)]);
    assert!(sim.run_until(1.0, BehaviourKind::Greet));
    assert!(sim.run_while(6.0, |s| s.brain.behaviour() == BehaviourKind::Greet), "gave up");
    sim.run(1.0);
    assert!(emotes(&sim, 0).contains(&Expression::Lost), "{:?}", emotes(&sim, 0));
}

#[test]
fn a_tired_knight_shows_one_z_before_sleeping() {
    let mut sim = sleepy_with_bench();
    sim.run(60.0);
    let got = emotes(&sim, 0);
    assert_eq!(got, [Expression::Drowsy], "{got:?}");
}

#[test]
fn bored_with_nothing_to_do_sighs_once() {
    let mut sim = calm(BrainConfig::default());
    sim.brain.mind_mut().needs.boredom = 0.95;
    sim.run(120.0);
    let got = emotes(&sim, 0);
    assert_eq!(got.iter().filter(|e| **e == Expression::Bored).count(), 1, "{got:?}");
}

/// Twenty minutes of a user moving about two windows, a cursor that sometimes parks, a
/// visitor: bubbles stay rare, never repeat inside their cooldown, and soft ones keep their gap.
#[test]
fn a_long_ordinary_session_is_never_noisy() {
    let mut sim = with_window(calm(BrainConfig::default()));
    sim.open(window(2, RectI::new(1100, 600, 1700, 1000), "browser.exe"));
    let spots = [Vec2::new(700.0, 300.0), Vec2::new(1400.0, 500.0), Vec2::new(400.0, 950.0), Vec2::new(1600.0, 950.0)];
    let minutes = 20;
    for i in 0..minutes * 2 {
        sim.cursor = Some(Cursor { pos: spots[i % spots.len()], ..Default::default() });
        sim.user.idle_secs = if i % 5 == 4 { 400.0 } else { 0.0 };
        if i == 10 {
            sim.brain.see_visitors(vec![visitor(7, 1500.0)]);
        }
        if i == 12 {
            sim.brain.see_visitors(Vec::new());
        }
        sim.run(30.0);
    }
    let shown: Vec<(f32, Expression)> = sim.intents.iter().enumerate().filter_map(|(n, i)| i.emote.map(|e| (n as f32 * DT, e))).collect();
    assert!(shown.len() <= 2 * minutes, "{} bubbles in {minutes} min: {shown:?}", shown.len());
    for (k, &(t, e)) in shown.iter().enumerate() {
        if let Some(&(t0, _)) = shown[..k].iter().rev().find(|(_, e0)| *e0 == e) {
            assert!(t - t0 >= e.cooldown() - 0.01, "{e:?} again after {:.1} s", t - t0);
        }
        if !e.reaction()
            && let Some(&(t0, _)) = shown[..k].iter().rev().find(|(_, e0)| !e0.reaction())
        {
            assert!(t - t0 >= crate::ai::expression::SOFT_GAP - 0.01, "{e:?} {:.1} s after a soft bubble", t - t0);
        }
    }
}
