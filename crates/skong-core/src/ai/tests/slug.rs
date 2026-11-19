//! The Unn slug: a very calm, rested knight on a quiet desktop sometimes melts into the
//! slug for a slow crawl. Never while the user is active, never with Geo or visitors
//! about; stirred, he transforms back first.
use super::*;
use crate::ai::stakes::StakesSense;

fn slugs(sim: &Sim) -> usize {
    sim.started.iter().filter(|a| *a == "slug_crawl").count()
}

/// Calm and rested, the user away from the keyboard for a while: due for a slug.
fn serene() -> Sim {
    let mut sim = calm(BrainConfig::default());
    sim.user.idle_secs = 100.0;
    sim
}

#[test]
fn a_serene_idle_knight_sometimes_becomes_the_slug() {
    let mut hits = 0;
    for seed in 0..6 {
        let mut sim = calm(BrainConfig::default());
        sim.brain = Brain::new(BrainConfig { personality: Some(Personality::default()), ..Default::default() }, sim.catalog.clone(), 100 + seed);
        let m = sim.brain.mind_mut();
        m.needs = Needs { energy: 0.85, boredom: 0.5, loneliness: 0.2, mischief: 0.0 };
        // The first 90 s of quiet don't count yet (the user only just left).
        sim.run(80.0);
        assert_eq!(slugs(&sim), 0, "not before the desktop has been quiet a while: {:?}", sim.started);
        sim.run(220.0);
        let n = slugs(&sim);
        assert!(n <= 1, "a few times an hour at most: {n} in 5 min");
        if n == 1 {
            hits += 1;
            assert!(sim.intents.iter().any(|i| i.reason.contains("feeling serene")), "with its reason");
        }
    }
    assert!(hits >= 3, "a serene knight slugs now and then: {hits}/6 runs");
}

#[test]
fn never_slugs_while_the_user_is_active() {
    let mut sim = calm(BrainConfig::default());
    for i in 0..60 {
        // Mouse moving around every few seconds.
        sim.user.idle_secs = 0.0;
        sim.cursor = Some(Cursor { pos: Vec2::new(300.0 + 200.0 * (i % 7) as f32, 600.0), ..Default::default() });
        sim.run(8.0);
    }
    assert_eq!(slugs(&sim), 0, "acts: {:?}", sim.started);
}

#[test]
fn never_slugs_with_geo_or_visitors_around() {
    // Geo lying on the desktop (out of reach: he can't loot it, it just stays there).
    let mut sim = serene();
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(1800.0, 200.0)], ..Default::default() });
    sim.run(300.0);
    assert_eq!(slugs(&sim), 0, "Geo about: {:?}", sim.started);

    // A visitor on screen.
    let mut sim = serene();
    sim.brain.see_visitors(vec![Visitor { id: 1, name: "zote".into(), pos: Vec2::new(1700.0, FLOOR), ..Default::default() }]);
    sim.run(300.0);
    assert_eq!(slugs(&sim), 0, "visitor about: {:?}", sim.started);

    // Excited or tired: not serene either.
    let mut sim = serene();
    sim.brain.mind_mut().needs.energy = 0.3;
    sim.run(200.0);
    assert_eq!(slugs(&sim), 0, "tired: {:?}", sim.started);
}

#[test]
fn stirring_turns_him_back_into_the_knight_first() {
    let mut sim = serene();
    assert!(sim.run_while(400.0, |s| slugs(s) == 0), "slugged: {:?}", sim.started);
    sim.run(2.0);
    assert_eq!(sim.started.last().map(String::as_str), Some("slug_crawl"));
    assert!(sim.last_reason().contains("serene"), "{}", sim.last_reason());
    // The user touches the mouse: he transforms back (not snapped out of it).
    sim.user.idle_secs = 0.0;
    sim.step();
    assert_eq!(sim.started.last().map(String::as_str), Some("slug_up"), "acts: {:?}", sim.started);
    sim.run(30.0);
    assert_eq!(slugs(&sim), 1, "no second crawl straight away");

    // Geo turning up mid-crawl does the same.
    let mut sim = serene();
    assert!(sim.run_while(400.0, |s| slugs(s) == 0));
    sim.run(2.0);
    sim.brain.sense_stakes(StakesSense { coins: vec![Vec2::new(1500.0, FLOOR)], ..Default::default() });
    sim.step();
    assert_eq!(sim.started.last().map(String::as_str), Some("slug_up"), "acts: {:?}", sim.started);
}
