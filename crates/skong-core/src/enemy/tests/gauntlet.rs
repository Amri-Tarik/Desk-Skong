//! Gauntlets on the fake desktop: the director with real enemies placed by `plan_arrival`
//! (the app's side), waves in order, the reward, graceful aborts and the start rules.
use super::*;

/// The app's side of a gauntlet: the director, real enemies placed by `plan_arrival`, a
/// knight who strikes down whatever comes within `reach`.
struct Trial {
    set: EnemySet,
    t: Terrain,
    g: GauntletDirector,
    rng: Rng,
    live: Vec<Enemy>,
    next_id: u32,
    knight: Vec2,
    world: GauntletWorld,
    events: Vec<GauntletEvent>,
    reward: Option<u32>,
    /// (wave running when it came, id, placement) of every arrival.
    arrivals: Vec<(usize, u32, EnemyPlacement)>,
    /// The knight strikes enemies within this distance (0: he is busy elsewhere).
    reach: f32,
}

fn quiet(knight: Vec2) -> GauntletWorld {
    GauntletWorld { away_secs: 90.0, user_idle_secs: 5.0, knight_at: knight, ready: true, ..Default::default() }
}

impl Trial {
    fn new(cfg: GauntletConfig) -> Self {
        let set = set();
        let g = GauntletDirector::new(cfg, &set).expect("gauntlet");
        let knight = Vec2::new(960.0, FLOOR);
        Self { set, t: terrain(None), g, rng: Rng::new(11), live: vec![], next_id: 1, knight, world: quiet(knight), events: vec![], reward: None, arrivals: vec![], reach: 0.0 }
    }

    fn alive(&self) -> Vec<u32> {
        self.live.iter().filter(|e| e.alive() && !e.dismissed()).map(|e| e.id).collect()
    }

    fn step(&mut self) {
        let alive = self.alive();
        self.world.others_alive = alive.iter().filter(|&&id| !self.g.is_member(id)).count();
        let step = self.g.tick(DT, &self.world, &|id| alive.contains(&id), &mut self.rng);
        for e in self.live.iter_mut().filter(|e| step.dismiss.contains(&e.id)) {
            e.dismiss();
        }
        if let Some(def) = step.spawn {
            let floor = Footing { x0: 0.0, x1: 1920.0, y: FLOOR, owner: None };
            let placed = plan_arrival(&self.set.defs[def], &[floor], self.knight, &[monitor()], PPU, &mut self.rng, &mut |_| true);
            let id = placed.map(|at| {
                let id = self.next_id;
                self.next_id += 1;
                let mut e = Enemy::new(id, def, &self.set, at, id, PPU);
                e.hold(self.g.hold_secs());
                self.live.push(e);
                let wave = self.g.cue().map_or(0, |c| c.wave);
                self.arrivals.push((wave, id, at));
                id
            });
            self.g.spawned(id);
        }
        if step.reward.is_some() {
            self.reward = step.reward;
        }
        self.events.extend(step.events);
        let env = env(&self.t, Some(self.knight));
        for e in &mut self.live {
            e.tick(DT, &self.set, &env);
        }
        // The knight: a decisive blow on whatever is close and vulnerable.
        let k = self.knight;
        let reach = self.reach;
        if let Some(e) = self.live.iter_mut().find(|e| e.vulnerable() && !e.dismissed() && e.pos.distance(k) < reach)
            && e.strike(9, k, &self.set, PPU)
        {
            let id = e.id;
            self.g.on_kill(id, &mut self.rng);
        }
        self.live.retain(|e| e.phase() != EnemyPhase::Gone);
    }

    fn run_until(&mut self, secs: f32, mut done: impl FnMut(&Trial) -> bool) -> bool {
        for _ in 0..(secs / DT) as usize {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn names(&self) -> Vec<&'static str> {
        self.events.iter().map(|e| e.name()).collect()
    }
}

#[test]
fn the_real_gauntlet_has_three_waves_of_two_to_four_from_the_roster() {
    let s = set();
    let mix = s.gauntlet.resolve(&s).expect("resolves against the roster");
    assert_eq!(mix.len(), 3, "three waves");
    for w in &s.gauntlet.waves {
        assert!(w.count[0] >= 2 && w.count[1] <= 4, "2-4 enemies a wave: {:?}", w.count);
    }
    let used: Vec<&str> = mix.iter().flatten().map(|(d, _)| s.defs[*d].name.as_str()).collect();
    for n in ["crawlid", "vengefly", "husk"] {
        assert!(used.contains(&n), "{n} takes part");
    }
    assert!(s.gauntlet.reward[0] > s.defs.iter().map(|d| d.geo[1]).max().unwrap(), "a bigger burst than any single enemy");
    // A bad name is caught at load.
    let mut cfg = s.gauntlet.clone();
    cfg.waves[0].mix.insert("primal aspid".into(), 1.0);
    assert!(cfg.resolve(&s).is_err());
}

#[test]
fn gauntlet_waves_arrive_in_order_each_after_a_clear_then_the_reward() {
    let mut tr = Trial::new(GauntletConfig::default());
    tr.run_until(2.0, |_| false);
    assert!(tr.events.is_empty() && tr.live.is_empty(), "nothing on its own this soon");
    tr.g.request();
    assert!(tr.run_until(0.5, |t| !t.events.is_empty()));
    assert!(matches!(tr.events[0], GauntletEvent::Started { trigger: GauntletTrigger::Requested, waves: 3, .. }), "{:?}", tr.events);
    assert_eq!(tr.g.cue().map(|c| c.stage), Some(GauntletStage::Announce));
    // The announcement: nobody arrives yet.
    tr.run_until(2.5, |_| false);
    assert!(tr.live.is_empty(), "the announcement comes first");
    // Wave 1 arrives; the knight is busy elsewhere (reach 0): it stays, no wave 2.
    assert!(tr.run_until(10.0, |t| t.live.len() >= 2));
    tr.run_until(30.0, |_| false);
    assert_eq!(tr.g.cue().map(|c| (c.stage, c.wave)), Some((GauntletStage::Wave, 1)), "wave 2 waits for wave 1 to be cleared");
    assert!(!tr.names().contains(&"gauntlet_wave_cleared"));
    assert!(tr.live.iter().all(|e| e.alive()), "wave members don't wander off while he's busy");
    // He fights: everything that comes near falls.
    tr.reach = 20.0 * PPU;
    assert!(tr.run_until(120.0, |t| t.reward.is_some()), "completes: {:?}", tr.names());
    assert_eq!(
        tr.names(),
        [
            "gauntlet_started",
            "gauntlet_wave_started",
            "gauntlet_wave_cleared",
            "gauntlet_wave_started",
            "gauntlet_wave_cleared",
            "gauntlet_wave_started",
            "gauntlet_wave_cleared",
            "gauntlet_completed"
        ]
    );
    // Each wave arrived complete, only once the previous one was cleared.
    let mut counts = [0usize; 3];
    for (wave, id, at) in &tr.arrivals {
        assert!((1..=3).contains(wave), "{id} arrived outside a wave");
        counts[wave - 1] += 1;
        let d = at.at.distance(tr.knight);
        assert!((4.0 * PPU..=14.0 * PPU).contains(&d), "arrives at a fair distance: {d:.0}");
    }
    for (i, e) in tr.events.iter().enumerate() {
        if let GauntletEvent::WaveStarted { wave, count, .. } = *e {
            assert!((2..=4).contains(&count));
            assert_eq!(counts[wave - 1], count, "wave {wave} brought all its enemies");
            if wave > 1 {
                assert!(matches!(tr.events[i - 1], GauntletEvent::WaveCleared { wave: w, .. } if w == wave - 1), "wave {wave} only after wave {} was cleared", wave - 1);
            }
        }
    }
    let Some(GauntletEvent::Completed { geo, beaten, waves: 3, .. }) = tr.events.last().copied() else { panic!("{:?}", tr.events) };
    assert!((40..=60).contains(&geo) && tr.reward == Some(geo), "the reward: {geo}");
    assert_eq!(beaten as usize, counts.iter().sum::<usize>(), "every one he beat counts");
    assert_eq!(tr.g.cue().map(|c| c.stage), Some(GauntletStage::Reward), "then his cheer");
    assert!(tr.run_until(5.0, |t| t.g.cue().is_none()), "and it's over");
    assert_eq!(tr.g.kills(), 0, "gauntlet kills don't count towards the next natural one");
}

#[test]
fn gauntlet_ends_gracefully_when_the_user_types_or_goes_fullscreen() {
    for (what, reason) in [("typing", AbortReason::Typing), ("fullscreen", AbortReason::Fullscreen), ("hurt", AbortReason::Hurt), ("down", AbortReason::KnightDown)] {
        let mut tr = Trial::new(GauntletConfig::default());
        tr.g.request();
        assert!(tr.run_until(15.0, |t| t.live.len() >= 2), "{what}: wave 1 is in");
        match what {
            "typing" => {
                // A short burst is fine...
                tr.world.keys_per_sec = 5.0;
                tr.run_until(1.0, |_| false);
                tr.world.keys_per_sec = 0.0;
                tr.run_until(1.0, |_| false);
                assert!(tr.g.active(), "a short burst of typing is fine");
                // ... typing hard ends it.
                tr.world.keys_per_sec = 5.0;
            }
            "fullscreen" => tr.world.fullscreen = true,
            "hurt" => tr.world.knight_hurt = true,
            _ => tr.world.knight_down = true,
        }
        assert!(tr.run_until(3.0, |t| !t.g.active()), "{what}: ends");
        assert!(matches!(tr.events.last(), Some(GauntletEvent::Aborted { reason: r, wave: 1, .. }) if *r == reason), "{what}: {:?}", tr.events);
        assert!(tr.reward.is_none(), "{what}: no reward");
        assert!(!tr.live.is_empty() && tr.live.iter().all(|e| e.dismissed() && e.foe(&tr.set, PPU).is_none()), "{what}: they're sent away, no foes any more");
        assert!(tr.run_until(40.0, |t| t.live.is_empty()), "{what}: they wander off and fade");
        // Nothing more comes.
        tr.world = quiet(tr.knight);
        tr.run_until(20.0, |_| false);
        assert!(tr.live.is_empty() && tr.g.cue().is_none(), "{what}: no further waves");
    }
}

#[test]
fn a_gauntlet_never_starts_during_a_visit_or_another_fight() {
    let mut tr = Trial::new(GauntletConfig::default());
    // Another fight: a single enemy (not part of any gauntlet) is about.
    let def = tr.set.id("crawlid").unwrap();
    let at = EnemyPlacement { at: Vec2::new(500.0, FLOOR), owner: None, entry: EntryKind::Edge { from: Vec2::new(490.0, FLOOR) } };
    tr.live.push(Enemy::new(99, def, &tr.set, at, 3, PPU));
    tr.g.request();
    tr.run_until(10.0, |_| false);
    assert!(tr.events.is_empty() && !tr.g.active(), "waits for the fight to end");
    assert_eq!(tr.g.pending(), Some(GauntletTrigger::Requested));
    tr.live.clear();
    // A visitor.
    tr.world.visitor_present = true;
    tr.run_until(10.0, |_| false);
    assert!(tr.events.is_empty(), "not during a visit");
    // The knight fighting (a spar, a lesson), or a fullscreen app.
    tr.world.visitor_present = false;
    tr.world.knight_fighting = true;
    tr.run_until(5.0, |_| false);
    tr.world.knight_fighting = false;
    tr.world.fullscreen = true;
    tr.run_until(5.0, |_| false);
    assert!(tr.events.is_empty());
    tr.world.fullscreen = false;
    // His Shade waits to be reclaimed: that comes first, even for a requested gauntlet.
    tr.world.shade = true;
    tr.run_until(5.0, |_| false);
    assert!(tr.events.is_empty() && !tr.g.active(), "not while his Shade waits");
    assert_eq!(tr.g.pending(), Some(GauntletTrigger::Requested));
    tr.world.shade = false;
    assert!(tr.run_until(1.0, |t| t.g.active()), "starts once it's quiet");
    // A visitor turning up mid-gauntlet ends it.
    tr.world.visitor_present = true;
    assert!(tr.run_until(1.0, |t| !t.g.active()));
    assert!(matches!(tr.events.last(), Some(GauntletEvent::Aborted { reason: AbortReason::Visitor, .. })));
    // A request that never finds its moment lapses.
    let mut tr = Trial::new(GauntletConfig { wait: 5.0, ..Default::default() });
    tr.world.visitor_present = true;
    tr.g.request();
    tr.run_until(6.0, |_| false);
    assert_eq!(tr.g.pending(), None, "gave up waiting");
    tr.world.visitor_present = false;
    tr.run_until(2.0, |_| false);
    assert!(!tr.g.active());
}

#[test]
fn a_gauntlet_comes_on_its_own_after_several_single_kills() {
    let mut cfg = GauntletConfig::default();
    cfg.natural.chance = 1.0;
    cfg.natural.after_kills = 3;
    cfg.natural.cooldown = 600.0;
    let mut tr = Trial::new(cfg);
    let mut rng = Rng::new(1);
    tr.g.on_kill(101, &mut rng);
    tr.g.on_kill(102, &mut rng);
    assert_eq!(tr.g.pending(), None, "two single kills: not yet");
    tr.g.on_kill(103, &mut rng);
    assert_eq!(tr.g.pending(), Some(GauntletTrigger::Natural));
    // Not while he's asleep / held.
    tr.world.knight_busy = true;
    tr.run_until(3.0, |_| false);
    assert!(!tr.g.active());
    tr.world.knight_busy = false;
    assert!(tr.run_until(1.0, |t| t.g.active()));
    assert!(matches!(tr.events[0], GauntletEvent::Started { trigger: GauntletTrigger::Natural, .. }));
    // Ends (typing); then a cooldown: more single kills bring none for a while.
    tr.world.keys_per_sec = 5.0;
    assert!(tr.run_until(5.0, |t| !t.g.active()));
    tr.world.keys_per_sec = 0.0;
    for id in 200..210 {
        tr.g.on_kill(id, &mut rng);
    }
    assert_eq!(tr.g.pending(), None, "cooling down");
    // Natural gauntlets can be switched off; requests still work.
    let mut cfg = GauntletConfig::default();
    cfg.natural.enabled = false;
    let mut tr = Trial::new(cfg);
    for id in 1..50 {
        tr.g.on_kill(id, &mut rng);
    }
    assert_eq!(tr.g.pending(), None);
    tr.g.request();
    assert_eq!(tr.g.pending(), Some(GauntletTrigger::Requested));
}

/// Days of uptime at the app's 120 Hz: the natural gauntlet's cooldown still runs out
/// (an f32 clock stops at 2^18 s, 72.8 h, and it never would).
#[test]
fn the_natural_cooldown_still_runs_out_after_days_of_uptime() {
    const TICK: f32 = 1.0 / 120.0;
    let mut cfg = GauntletConfig::default();
    cfg.natural.chance = 1.0;
    cfg.natural.after_kills = 1;
    cfg.natural.cooldown = 30.0;
    let mut tr = Trial::new(cfg);
    let mut rng = Rng::new(3);
    tr.g.skip_clock(80.0 * 3600.0);
    // One ends (typing): the cooldown starts now.
    tr.g.request();
    assert!(tr.run_until(1.0, |t| t.g.active()));
    tr.world.keys_per_sec = 5.0;
    assert!(tr.run_until(5.0, |t| !t.g.active()));
    tr.world.keys_per_sec = 0.0;
    let quiet = tr.world;
    for _ in 0..(31.0 / TICK) as usize {
        tr.g.tick(TICK, &quiet, &|_| false, &mut rng);
    }
    tr.g.on_kill(500, &mut rng);
    assert_eq!(tr.g.pending(), Some(GauntletTrigger::Natural), "cooled down after 31 s");
}

#[test]
fn a_gauntlet_without_room_for_its_arrivals_ends() {
    let mut tr = Trial::new(GauntletConfig { no_room_secs: 3.0, ..Default::default() });
    tr.g.request();
    // The app finds no spot, again and again.
    let mut asked = 0;
    for _ in 0..(8.0 / DT) as usize {
        let step = tr.g.tick(DT, &tr.world, &|_| false, &mut tr.rng);
        if step.spawn.is_some() {
            asked += 1;
            tr.g.spawned(None);
        }
        tr.events.extend(step.events);
    }
    assert!(asked >= 2, "asks again a moment later ({asked})");
    assert!(matches!(tr.events.last(), Some(GauntletEvent::Aborted { reason: AbortReason::NoRoom, wave: 1, .. })), "{:?}", tr.events);
}
