use super::*;
use crate::geom::RectF;
use crate::terrain::{Ledge, WindowBox};

fn terrain() -> Terrain {
    Terrain {
        windows: vec![WindowBox { id: 7, rect: RectF::new(400.0, 600.0, 900.0, 900.0) }],
        ledges: vec![
            Ledge { x0: 400.0, x1: 900.0, y: 600.0, kind: LedgeKind::OneWay, owner: Some(7) },
            Ledge { x0: 0.0, x1: 1920.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
        ],
        bounds: RectF::new(0.0, 0.0, 1920.0, 1080.0),
        ..Default::default()
    }
}

// --- Geo ----------------------------------------------------------------------------

#[test]
fn coins_split_into_denominations() {
    let v: u32 = CoinSize::split(57, 20).iter().map(|c| c.value()).sum();
    assert_eq!(v, 57);
    assert!(CoinSize::split(57, 20).contains(&CoinSize::Large));
    assert!(!CoinSize::split(12, 20).contains(&CoinSize::Large), "small sums are smaller coins");
    assert!(CoinSize::split(1000, 5).len() <= 5);
}

#[test]
fn coins_fall_bounce_and_settle_on_ledges() {
    let t = terrain();
    let mut g = GeoField::new(1);
    g.burst(Vec2::new(650.0, 300.0), 20);
    let n = g.coins().len();
    assert!(n >= 4);
    let mut bounced = false;
    for _ in 0..600 {
        g.step(1.0 / 60.0, &t, &[]);
        bounced |= g.coins().iter().any(|c| !c.resting && c.vel.y < 0.0 && c.pos.y > 400.0);
    }
    assert_eq!(g.coins().len(), n);
    assert!(g.coins().iter().all(|c| c.resting), "all settled after 10 s");
    for c in g.coins() {
        assert!(c.pos.y == 600.0 || c.pos.y == 1040.0, "on a ledge, not floating: {c:?}");
        assert!(c.pos.y != 600.0 || (400.0..=900.0).contains(&c.pos.x));
    }
    assert!(bounced, "coins bounce before settling");
}

#[test]
fn resting_coins_fall_when_their_window_goes_away() {
    let mut t = terrain();
    let mut g = GeoField::new(3);
    g.burst(Vec2::new(650.0, 590.0), 3);
    for _ in 0..300 {
        g.step(1.0 / 60.0, &t, &[]);
    }
    assert!(g.coins().iter().any(|c| c.pos.y == 600.0));
    t.ledges.remove(0);
    for _ in 0..300 {
        g.step(1.0 / 60.0, &t, &[]);
    }
    assert!(g.coins().iter().all(|c| c.resting && c.pos.y == 1040.0));
}

#[test]
fn walking_over_coins_picks_them_up() {
    let t = terrain();
    let mut g = GeoField::new(2);
    g.burst(Vec2::new(1000.0, 1000.0), 7);
    for _ in 0..600 {
        g.step(1.0 / 60.0, &t, &[]);
    }
    let total = g.total();
    assert_eq!(total, 7);
    // A body far away gets nothing; a body sweeping the floor gets everything.
    assert!(g.collect(RectF::new(0.0, 900.0, 50.0, 1040.0)).is_empty());
    let mut got = 0;
    for x in (0..1920).step_by(20) {
        got += g.collect(RectF::from_feet(Vec2::new(x as f32, 1040.0), Vec2::new(30.0, 80.0))).iter().map(|p| p.value).sum::<u32>();
    }
    assert_eq!(got, total);
    assert!(g.is_empty());
}

// --- Wallet + persistence ----------------------------------------------------------

#[test]
fn death_moves_the_wallet_to_the_shade_and_reclaiming_brings_it_back() {
    let mut s = StakesSave { geo: 120, ..Default::default() };
    assert_eq!(s.die(Vec2::new(500.0, 900.0), true), 0);
    assert_eq!(s.geo, 0);
    assert_eq!(s.shade.map(|sh| sh.geo), Some(120));
    s.earn(15);
    // Dying again before reclaiming: the old shade's geo is lost for good.
    assert_eq!(s.die(Vec2::new(800.0, 900.0), true), 120);
    assert_eq!(s.shade.map(|sh| (sh.x, sh.geo)), Some((800.0, 15)));
    assert_eq!(s.reclaim(), 15);
    assert_eq!((s.geo, s.shade, s.deaths, s.shades_defeated), (15, None, 2, 1));
    // Hazards keep everything (no shade).
    assert_eq!(s.die(Vec2::ZERO, false), 0);
    assert_eq!((s.geo, s.shade), (15, None));
}

#[test]
fn save_round_trips_and_old_files_load() {
    let s = StakesSave { geo: 42, shade: Some(ShadeSave { x: 10.5, y: 20.0, geo: 7 }), deaths: 3, shades_defeated: 1 };
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(serde_json::from_str::<StakesSave>(&json).unwrap(), s);
    assert_eq!(serde_json::from_str::<StakesSave>("{}").unwrap(), StakesSave::default());
    assert_eq!(serde_json::from_str::<StakesSave>(r#"{"geo": 5}"#).unwrap().geo, 5);
}

// --- Respawn ------------------------------------------------------------------------

#[test]
fn respawn_prefers_the_nearest_bench_then_solid_ground() {
    let t = terrain();
    let died = Vec2::new(650.0, 600.0);
    let benches = [Vec2::new(1500.0, 1040.0), Vec2::new(200.0, 1040.0)];
    assert_eq!(respawn_point(&t, &benches, died), Vec2::new(200.0, 1040.0));
    assert_eq!(respawn_point(&t, &[], died), Vec2::new(650.0, 1040.0), "the ground, not the window he died on");
    assert_eq!(respawn_point(&t, &[], Vec2::new(5.0, 100.0)), Vec2::new(40.0, 1040.0));
}

// --- Shade --------------------------------------------------------------------------

fn run_shade(sh: &mut Shade, secs: f32, knight: Option<Vec2>, hunted: bool) -> Vec<ShadeTick> {
    let b = [RectF::new(0.0, 0.0, 1920.0, 1080.0)];
    (0..(secs * 60.0) as usize).map(|_| sh.step(1.0 / 60.0, knight, hunted, &b)).collect()
}

#[test]
fn shade_appears_then_floats_near_home() {
    let home = Vec2::new(600.0, 500.0);
    let mut sh = Shade::new(home, 30, true, 9);
    assert_eq!(sh.phase(), ShadePhase::Appear);
    assert!(!sh.vulnerable());
    let ticks = run_shade(&mut sh, 20.0, Some(Vec2::new(620.0, 520.0)), false);
    assert_eq!(ticks[0].clip, "Appear");
    assert_eq!(sh.phase(), ShadePhase::Idle, "not hunted: it only watches, even with the knight close");
    assert!(ticks.iter().all(|t| !t.hit_knight));
    assert!(sh.pos.distance(home) < 70.0, "drifts a little: {:?}", sh.pos);
    let moved = ticks.windows(2).filter(|w| w[0] != w[1] || w[0].clip == "Idle").count();
    assert!(moved > 0);
}

#[test]
fn hunted_shade_attacks_and_bursts_when_beaten() {
    let mut sh = Shade::new(Vec2::new(600.0, 500.0), 30, false, 4);
    let knight = Vec2::new(760.0, 560.0);
    let ticks = run_shade(&mut sh, 6.0, Some(knight), true);
    let clips: Vec<&str> = ticks.iter().map(|t| t.clip).collect();
    for c in ["Startle", "Fly", "Slash Antic", "Slash"] {
        assert!(clips.contains(&c), "{c} in {clips:?}");
    }
    assert!(ticks.iter().any(|t| t.hit_knight), "the lunge connects with a knight standing still");
    assert!(ticks.iter().any(|t| t.effect == Some("Slash Effect")));

    let mut beaten = false;
    for _ in 0..super::shade::SHADE_HP {
        beaten |= sh.strike(1, knight);
    }
    assert!(beaten && !sh.alive());
    let ticks = run_shade(&mut sh, 1.5, Some(knight), true);
    assert_eq!(ticks.iter().filter(|t| t.burst).count(), 1, "bursts exactly once");
    assert_eq!(sh.phase(), ShadePhase::Gone);
}

#[test]
fn shade_goes_home_when_the_knight_leaves() {
    let home = Vec2::new(600.0, 500.0);
    let mut sh = Shade::new(home, 0, false, 5);
    run_shade(&mut sh, 2.0, Some(Vec2::new(700.0, 540.0)), true);
    assert!(sh.fighting());
    run_shade(&mut sh, 8.0, Some(Vec2::new(700.0, 540.0)), false);
    assert_eq!(sh.phase(), ShadePhase::Idle);
    assert!(sh.pos.distance(home) < 90.0);
}

// --- Spells -------------------------------------------------------------------------

fn situation(target: Option<Vec2>) -> SpellSituation {
    SpellSituation { target, grounded: true, just_hit: false, fight_over: false, hurt: 0.0, rolls: [0.0, 0.0], scale: 1.0 }
}

#[test]
fn spells_fit_the_target() {
    let cfg = SpellConfig::default();
    let mut book = SpellBook::new(&SpellConfig { start_soul: 1.0, ..cfg });
    assert_eq!(book.choose(&situation(Some(Vec2::new(500.0, 10.0))), 0.0, &cfg), Some(Spell::Fireball), "level and far");
    assert_eq!(book.choose(&situation(Some(Vec2::new(40.0, -250.0))), 10.0, &cfg), Some(Spell::Scream), "right above");
    assert_eq!(book.choose(&situation(Some(Vec2::new(220.0, 0.0))), 20.0, &cfg), Some(Spell::DashSlash), "mid range");
    let close = book.choose(&situation(Some(Vec2::new(60.0, 0.0))), 30.0, &cfg);
    assert!(matches!(close, Some(Spell::GreatSlash | Spell::Cyclone)), "{close:?}");
    assert_eq!(book.choose(&situation(Some(Vec2::new(2000.0, 0.0))), 40.0, &cfg), None, "out of reach");
}

#[test]
fn spells_are_occasional_and_cost_soul() {
    let cfg = SpellConfig::default();
    let far = situation(Some(Vec2::new(500.0, 0.0)));
    let mut book = SpellBook::new(&SpellConfig { start_soul: 0.0, ..cfg });
    assert_eq!(book.choose(&far, 0.0, &cfg), None, "no soul, no fireball");
    book.gain(3);
    assert_eq!(book.choose(&SpellSituation { rolls: [0.9, 0.0], ..far }, 0.0, &cfg), None, "the roll says plain slashes");
    assert_eq!(book.choose(&far, 0.0, &cfg), Some(Spell::Fireball));
    assert!(book.soul < 0.01, "paid for it");
    book.gain(9);
    assert_eq!(book.choose(&far, 1.0, &cfg), None, "min gap between specials");
    assert_eq!(book.choose(&SpellSituation { grounded: false, ..far }, 5.0, &cfg), None, "grounded only");
    assert_eq!(book.choose(&far, 5.0, &cfg), Some(Spell::Fireball));
}

#[test]
fn thorns_answer_hits_and_focus_heals_after() {
    let cfg = SpellConfig::default();
    let mut book = SpellBook::new(&cfg);
    let hit = SpellSituation { just_hit: true, ..situation(Some(Vec2::new(80.0, 0.0))) };
    assert_eq!(book.choose(&hit, 0.0, &cfg), Some(Spell::Thorns));
    assert_eq!(book.choose(&hit, 1.0, &cfg), None, "thorns cooldown");
    let far_hit = SpellSituation { just_hit: true, ..situation(Some(Vec2::new(600.0, 0.0))) };
    assert_eq!(book.choose(&far_hit, 10.0, &cfg), None, "thorns only reach nearby");

    let over = SpellSituation { fight_over: true, hurt: 0.5, ..situation(None) };
    assert_eq!(book.choose(&over, 10.0, &cfg), Some(Spell::Focus));
    assert_eq!(book.choose(&SpellSituation { hurt: 0.05, ..over }, 30.0, &cfg), None, "not hurt: no focus");
}

// --- Geo sources ------------------------------------------------------------------

#[test]
fn closed_windows_shatter_by_size_with_a_cap_and_a_rate_limit() {
    let mut src = GeoSources::new(GeoSourceConfig::default(), 1);
    let small = src.on_closed(crate::geom::RectI::new(0, 0, 400, 300), 0).expect("small window");
    let mut src = GeoSources::new(GeoSourceConfig::default(), 1);
    let big = src.on_closed(crate::geom::RectI::new(0, 0, 2560, 1400), 0).expect("big window");
    assert!(big.amount > small.amount, "bigger windows hold more: {} vs {}", big.amount, small.amount);
    assert!(big.amount <= GeoSourceConfig::default().shatter_geo[1]);
    assert!(!big.shower);

    let mut src = GeoSources::new(GeoSourceConfig::default(), 2);
    let r = crate::geom::RectI::new(0, 0, 800, 600);
    assert!(src.on_closed(crate::geom::RectI::new(0, 0, 120, 600), 0).is_none(), "popups don't shatter");
    assert!(src.on_closed(r, 0).is_some());
    assert!(src.on_closed(r, 0).is_some());
    assert!(src.on_closed(r, 0).is_none(), "closing a whole app at once: rate limited");
    let b = RectF::new(0.0, 0.0, 1920.0, 1080.0);
    src.tick(21.0, false, false, 0, b);
    assert!(src.on_closed(r, 0).is_some(), "the limit window passed");
    assert!(src.on_closed(r, 10_000).is_none(), "enough geo lying around");
}

#[test]
fn showers_come_every_few_minutes_of_presence_only() {
    let cfg = GeoSourceConfig::default();
    let b = RectF::new(0.0, 0.0, 1920.0, 1080.0);
    let mut src = GeoSources::new(cfg, 5);
    let gap = src.shower_in();
    assert!((cfg.shower_every[0]..=cfg.shower_every[1]).contains(&gap));
    // Away, or fullscreen / typing: the timer doesn't run.
    for _ in 0..1000 {
        assert!(src.tick(1.0, false, false, 0, b).is_none());
        assert!(src.tick(1.0, true, true, 0, b).is_none());
    }
    assert_eq!(src.shower_in(), gap);
    let mut showers = 0;
    for _ in 0..3600 {
        if let Some(d) = src.tick(1.0, true, false, 0, b) {
            assert!(d.shower && d.rect.top <= b.top);
            showers += 1;
        }
    }
    assert!((7..=20).contains(&showers), "an hour of presence: {showers} showers");
}

#[test]
fn coins_stay_out_of_dead_zones_between_monitors() {
    // A 2560x1440 primary with a 1920x1080 monitor to its left: x<0, y>1080 is off-screen.
    let areas = [RectF::new(0.0, 0.0, 2560.0, 1400.0), RectF::new(-1920.0, 0.0, 0.0, 1040.0)];
    let t = Terrain {
        ledges: vec![
            Ledge { x0: 0.0, x1: 2560.0, y: 1400.0, kind: LedgeKind::Ground, owner: None },
            Ledge { x0: -1920.0, x1: 0.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
        ],
        bounds: RectF::new(-1920.0, 0.0, 2560.0, 1440.0),
        ..Default::default()
    };
    let mut g = GeoField::new(9);
    g.spill(&GeoDrop { rect: RectF::new(-300.0, 900.0, 300.0, 1000.0), amount: 40, shower: false });
    g.spill(&GeoDrop { rect: RectF::new(-1000.0, -10.0, -800.0, 0.0), amount: 20, shower: true });
    for _ in 0..900 {
        g.step(1.0 / 60.0, &t, &areas);
        for c in g.coins() {
            assert!(areas.iter().any(|a| a.left <= c.pos.x && c.pos.x <= a.right && a.top <= c.pos.y && c.pos.y <= a.bottom), "{c:?} left the screen");
        }
    }
    assert!(g.coins().iter().all(|c| c.resting && (c.pos.y == 1400.0 || c.pos.y == 1040.0)));
}

#[test]
fn abandoned_coins_fade_out() {
    let t = terrain();
    let mut g = GeoField::new(4);
    g.burst(Vec2::new(1500.0, 1000.0), 1);
    for _ in 0..120 {
        g.step(1.0 / 60.0, &t, &[]);
    }
    let at = g.coins()[0].pos;
    g.abandon(at);
    assert!(g.fading(&g.coins()[0]));
    for _ in 0..(super::geo::FADE_SECS * 60.0) as usize + 5 {
        g.step(1.0 / 60.0, &t, &[]);
    }
    assert!(g.is_empty());
}

#[test]
fn shade_never_floats_off_screen_and_settles_somewhere_reachable() {
    let areas = [RectF::new(0.0, 0.0, 2560.0, 1400.0), RectF::new(-1920.0, 0.0, 0.0, 1040.0)];
    let t = Terrain {
        ledges: vec![
            Ledge { x0: 0.0, x1: 2560.0, y: 1400.0, kind: LedgeKind::Ground, owner: None },
            Ledge { x0: -1920.0, x1: 0.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
        ],
        bounds: RectF::new(-1920.0, 0.0, 2560.0, 1440.0),
        ..Default::default()
    };
    // Died somewhere in the dead zone below the left monitor.
    let mut sh = Shade::new(Vec2::new(-200.0, 1300.0), 10, true, 1);
    assert!(sh.settle(&t, &areas));
    let h = sh.home;
    assert!(areas.iter().any(|a| a.contains(h)), "home {h:?} on screen");
    assert!(t.ledges.iter().any(|l| l.x0 <= h.x && h.x <= l.x1 && l.y > h.y && l.y - h.y < 200.0), "hovering over a ledge: {h:?}");
    // Already fine: stays.
    assert!(!sh.settle(&t, &areas));
    // Lunging and drifting stay inside the work areas.
    for _ in 0..600 {
        sh.step(1.0 / 60.0, Some(Vec2::new(-30.0, 1390.0)), true, &areas);
        assert!(areas.iter().any(|a| a.contains(sh.pos)), "{:?}", sh.pos);
    }
}

#[test]
fn coins_never_rest_inside_a_claimed_prop() {
    use crate::occupancy::{ClaimKey, ClaimKind, Occupancy};
    let t = terrain();
    let mut occ = Occupancy::default();
    // A bench on the window ledge (y = 600) spanning x 600..700.
    occ.claim(ClaimKey::new(ClaimKind::Bench, 1), RectF::new(600.0, 540.0, 700.0, 600.0), 100);
    let mut g = GeoField::new(3);
    g.burst(Vec2::new(650.0, 560.0), 30);
    for _ in 0..900 {
        g.step(1.0 / 60.0, &t, &[]);
        g.evict_from(&occ);
    }
    assert!(g.coins().iter().all(|c| c.resting), "all settled");
    for c in g.coins() {
        assert!(occ.covering(c.pos, 4.0).is_none(), "coin resting inside the bench: {c:?}");
    }
}
