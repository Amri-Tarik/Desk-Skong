//! The training dummy on a real desktop: the user's two monitors (2560×1440 and a 1920×1080
//! to its left), windows whose tops sit high (y ≈ 106–139), the real route planner
//! (`GraphNavigator`) and controller. The app places dummies with `pastime::place_dummy`,
//! checked by the brain's own test (`ai::dummy_stand`), so a dummy only ever stands where he
//! will actually walk over and practise; and he does, within seconds, with the user moving
//! the mouse about.
use super::*;
use crate::discovery::Footprint;
use crate::geom::RectF;
use crate::lasting::LedgeAges;
use crate::occupancy::Occupancy;
use crate::pastime::{DummyHit, place_dummy};
use crate::shiny::ShinySite;

/// The pet's default render scale.
const SCALE: f32 = 0.6;
const PPU: f32 = 64.0 * SCALE;
/// The app's tick.
const TICK: f32 = 1.0 / 120.0;
/// The left monitor's floor (work-area bottom) and the primary's.
const LEFT_FLOOR: f32 = 1040.0;
const MAIN_FLOOR: f32 = 1392.0;

struct Desk {
    desk: DesktopSnapshot,
    terrain: Terrain,
    ages: LedgeAges,
}

fn user_desk() -> Desk {
    let desk = DesktopSnapshot {
        generation: 1,
        monitors: vec![
            Monitor { rect: RectI::new(0, 0, 2560, 1440), work: RectI::new(0, 0, 2560, MAIN_FLOOR as i32), primary: true, scale_milli: 1000 },
            Monitor { rect: RectI::new(-1920, 0, 0, 1080), work: RectI::new(-1920, 0, 0, LEFT_FLOOR as i32), primary: false, scale_milli: 1000 },
        ],
        // Front to back.
        windows: vec![
            window(3, RectI::new(-700, 620, -80, 1010), "notes.exe"),
            window(2, RectI::new(-1150, 139, -260, 1000), "code.exe"),
            window(1, RectI::new(-1880, 106, -1100, 980), "explorer.exe"),
            window(5, RectI::new(1400, 300, 2500, 1350), "chat.exe"),
            window(4, RectI::new(200, 139, 1500, 1300), "browser.exe"),
        ],
    };
    let terrain = Terrain::from_desktop(&desk, &TerrainOptions::default());
    let mut ages = LedgeAges::default();
    ages.observe_aged(&terrain, 0.0, 120.0);
    Desk { desk, terrain, ages }
}

/// The knight standing at `x` on the surface below `y`.
fn standing(t: &Terrain, x: f32, y: f32) -> Character {
    let mut c = Character::new(CharacterParams::hollow_knight(PPU), Abilities::default(), Vec2::new(x, y - 4.0));
    let mut ev = Vec::new();
    for _ in 0..1200 {
        c.step(&MoveInput::default(), t, TICK, &mut ev);
        if c.is_grounded() {
            return c;
        }
    }
    panic!("never landed at {x}");
}

/// The dummy's drawn size at this scale (about the app's art: 7.4 units × 0.34) and what
/// the brain is told about one standing at `foot` (as `PastimeProps::known`).
const DUMMY: (f32, f32) = (36.0, 97.0);

fn known(id: u64, foot: Vec2, span: (f32, f32)) -> KnownDummy {
    let half = (0.4 * DUMMY.0).max(12.0 * SCALE);
    KnownDummy { id, pos: foot, span, body: RectF::new(foot.x - half, foot.y - 0.97 * DUMMY.1, foot.x + half, foot.y - 0.42 * DUMMY.1) }
}

fn footprint() -> Footprint {
    Footprint { w: DUMMY.0 * 0.5, h: DUMMY.1, room: DUMMY.0 + 2.0 * (0.55 * PPU + 30.0 * SCALE) }
}

/// The brain's own test, against the real planner (what the app's placement uses).
fn practisable(nav: &GraphNavigator, k: &Character, d: &KnownDummy) -> bool {
    let g = nav.graph().expect("graph");
    dummy_stand(d, k, &mut |at, st| g.plan(k, NavGoal::Point(at), st).map(|r| Reach { eta: r.eta, end: Some(r.end) })).is_some()
}

/// Where the app would put a dummy now (the knight's monitor, its rules).
fn place(d: &Desk, nav: &GraphNavigator, k: &Character, salt: u64) -> Result<KnownDummy, crate::pastime::DummyMiss> {
    let occ = Occupancy::default();
    let area = d.desk.monitor_at(k.pos.x as i32, k.pos.y as i32 - 1).map(|m| m.work.to_f());
    let site = ShinySite {
        terrain: &d.terrain,
        ages: &d.ages,
        now: 0.0,
        min_age: crate::shiny::LASTING_SECS,
        occupancy: &occ,
        footprint: footprint(),
        knight: k.pos,
        min_distance: 220.0 * SCALE,
        area,
        avoid: &[],
        reachable: &|_| true,
    };
    let (_, foot) = place_dummy(&site, salt, &mut |foot, span| practisable(nav, k, &known(0, foot, span)))?;
    let l = d.terrain.ledges.iter().find(|l| (l.y - foot.y).abs() < 1.0 && l.x0 <= foot.x && foot.x <= l.x1).expect("its ledge");
    Ok(known(7, foot, (l.x0, l.x1)))
}

fn graph_nav(d: &Desk, k: &Character) -> GraphNavigator {
    let mut nav = GraphNavigator::new();
    nav.rebuild(&d.terrain, k);
    nav
}

#[test]
fn dummies_are_only_placed_where_he_will_go_and_practise() {
    let d = user_desk();
    // On the left monitor's floor (where he was in the log), on a high window top, and on
    // the primary's floor.
    for (x, y) in [(-1328.0, LEFT_FLOOR), (-700.0, 139.0), (900.0, MAIN_FLOOR)] {
        let k = standing(&d.terrain, x, y);
        let nav = graph_nav(&d, &k);
        let mut level = 0;
        for salt in 0..5 {
            let dummy = place(&d, &nav, &k, salt).unwrap_or_else(|m| panic!("from ({x},{y}): {m}"));
            assert!(practisable(&nav, &k, &dummy), "the brain agrees: {dummy:?}");
            let mon = d.desk.monitor_at(k.pos.x as i32, k.pos.y as i32 - 1).unwrap();
            assert!(mon.work.to_f().contains(Vec2::new(dummy.pos.x, dummy.pos.y - 1.0)), "on his monitor: {dummy:?}");
            assert!(dummy.pos.distance(k.pos) >= 220.0 * SCALE, "not under his feet");
            level += usize::from((dummy.pos.y - k.pos.y).abs() < 4.0);
        }
        assert!(level >= 3, "mostly on his own level ({level}/5 from ({x},{y}))");
    }
    // No spot he'd practise at: none, and the log says why.
    let k = standing(&d.terrain, -1328.0, LEFT_FLOOR);
    let occ = Occupancy::default();
    let site = ShinySite {
        terrain: &d.terrain,
        ages: &d.ages,
        now: 0.0,
        min_age: crate::shiny::LASTING_SECS,
        occupancy: &occ,
        footprint: footprint(),
        knight: k.pos,
        min_distance: 220.0 * SCALE,
        area: d.desk.monitor_at(-1328, 1000).map(|m| m.work.to_f()),
        avoid: &[],
        reachable: &|_| true,
    };
    let miss = place_dummy(&site, 1, &mut |_, _| false).expect_err("nowhere");
    assert!(miss.spots > 0 && (1..=16).contains(&miss.tried), "{miss:?}");
    assert!(miss.to_string().contains("stand spot"), "{miss}");
}

/// The brain, the real planner and controller, acts emulated; the user wiggling the mouse
/// near him. Returns the first second he was practising and the blows that landed.
struct Live {
    brain: Brain,
    nav: GraphNavigator,
    catalog: ActCatalog,
    d: Desk,
    knight: Character,
    act: Option<(RunningAct, f32)>,
    events: Vec<MoveEvent>,
    last_input: MoveInput,
    t: f32,
    hits: Vec<DummyHit>,
    dummy: Option<KnownDummy>,
    reason: String,
}

impl Live {
    fn new(d: Desk, x: f32, y: f32, seed: u32) -> Live {
        let knight = standing(&d.terrain, x, y);
        let nav = graph_nav(&d, &knight);
        let catalog = catalog();
        let mut brain = Brain::new(BrainConfig { personality: Some(Personality::default()), ..Default::default() }, catalog.clone(), seed);
        brain.mind_mut().needs = Needs { energy: 0.85, boredom: 0.1, loneliness: 0.6, mischief: 0.1 };
        Live { brain, nav, catalog, d, knight, act: None, events: Vec::new(), last_input: MoveInput::default(), t: 0.0, hits: Vec::new(), dummy: None, reason: String::new() }
    }

    fn step(&mut self) {
        let wiggle = Vec2::new(150.0 + 60.0 * ((self.t * 2.0) as i32 % 3) as f32, -120.0);
        let cursor = Some(Cursor { pos: self.knight.body().center() + wiggle, ..Default::default() });
        let perch = self.knight.support.is_some_and(|s| s.kind != LedgeKind::Ground);
        let activity = Activity::default();
        let p = Percepts {
            dt: TICK,
            knight: &self.knight,
            terrain: &self.d.terrain,
            desktop: &self.d.desk,
            cursor,
            user: UserActivity { idle_secs: 0.0, keys_per_sec: 0.0 },
            fullscreen_front: false,
            pokes: &[],
            act: self.act.map(|a| a.0),
            benches: &[],
            perch,
            move_events: &self.events,
            activity: &activity,
            last_input: self.last_input,
            local_hour: Some(15.0),
            dark: false,
            selection: None,
        };
        self.brain.set_pastimes(self.dummy, &[]);
        let intent = self.brain.tick(&p, &mut self.nav);
        self.reason.clone_from(&intent.reason);
        for ev in self.brain.take_pastime_events() {
            if let PastimeEvent::DummyHit { hit, .. } = ev {
                self.hits.push(hit);
            }
        }
        if intent.cancel_act && self.act.is_some_and(|a| a.0.interruptible) {
            self.act = None;
        }
        if let Some(id) = intent.act
            && self.act.is_none()
            && self.knight.is_grounded()
        {
            let name = self.catalog.get(id).map(|a| a.name.clone()).unwrap_or_default();
            self.act = Some((RunningAct { id, interruptible: true }, act_secs(&name)));
        }
        if let Some(f) = intent.face
            && self.act.is_none()
            && self.knight.is_grounded()
        {
            self.knight.facing = f;
        }
        let input = match intent.motion {
            _ if self.act.is_some() => MoveInput::default(),
            Motion::Hold => MoveInput::default(),
            Motion::Direct(i) => i,
            Motion::Navigate { .. } => self.nav.next_input(&self.knight, &self.d.terrain, TICK),
        };
        self.events.clear();
        self.knight.step(&input, &self.d.terrain, TICK, &mut self.events);
        self.last_input = input;
        if let Some(a) = &mut self.act {
            a.1 -= TICK;
            if a.1 <= 0.0 {
                self.act = None;
            }
        }
        self.t += TICK;
    }

    /// Run until `done` (true) or `secs` pass (false).
    fn run_until(&mut self, secs: f32, mut done: impl FnMut(&Live) -> bool) -> bool {
        for _ in 0..(secs / TICK) as usize {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }
}

#[test]
fn on_the_users_two_monitor_desk_he_practises_on_a_placed_dummy_within_seconds() {
    // Where he stood in the live log (the left monitor's floor, under high windows), and on
    // the primary. The app places the dummy; the mouse moves about near him all along.
    for (x, y, seed) in [(-1328.0, LEFT_FLOOR, 3), (900.0, MAIN_FLOOR, 4)] {
        let mut live = Live::new(user_desk(), x, y, seed);
        assert!(live.run_until(3.0, |l| l.brain.behaviour() == BehaviourKind::Companion), "keeping company: {}", live.reason);
        let dummy = place(&live.d, &live.nav, &live.knight, 9).unwrap_or_else(|m| panic!("{m}"));
        live.dummy = Some(dummy);
        assert!(live.run_until(3.0, |l| l.brain.pastime() == Some(PastimeKind::Dummy)), "takes it up at once from ({x},{y}): {} / dummy {dummy:?}", live.reason);
        assert!(live.run_until(40.0, |l| l.hits.len() >= 3), "goes over and practises: {} / knight {:?} / dummy {dummy:?}", live.reason, live.knight.pos);
        assert!((live.knight.pos.x - dummy.pos.x).abs() < 3.0 * PPU, "beside it (or pogoing off it): t {} knight {:?} dummy {dummy:?} hits {:?} / {}", live.t, live.knight.pos, live.hits, live.reason);
    }
}

#[test]
fn a_dummy_on_a_high_window_top_is_climbed_to_when_his_routes_get_there() {
    // A dummy up on the explorer window's top (y = 106, as in the log), at a spot the brain's
    // test accepts: he climbs up and practises.
    let d = user_desk();
    let k = standing(&d.terrain, -1328.0, LEFT_FLOOR);
    let nav = graph_nav(&d, &k);
    let top = d.terrain.ledges.iter().find(|l| l.owner == Some(1) && (l.y - 106.0).abs() < 2.0).copied().expect("the high title bar");
    let dummy = (0..=20)
        .map(|i| top.x0 + 60.0 + (top.x1 - top.x0 - 120.0) * i as f32 / 20.0)
        .map(|x| known(11, Vec2::new(x, top.y), (top.x0, top.x1)))
        .find(|dm| practisable(&nav, &k, dm));
    let Some(dummy) = dummy else {
        // Then the placement never puts one there, which is the point.
        return;
    };
    let mut live = Live::new(d, -1328.0, LEFT_FLOOR, 5);
    assert!(live.run_until(3.0, |l| l.brain.behaviour() == BehaviourKind::Companion), "{}", live.reason);
    live.dummy = Some(dummy);
    assert!(live.run_until(3.0, |l| l.brain.pastime() == Some(PastimeKind::Dummy)), "takes it up: {}", live.reason);
    assert!(live.run_until(60.0, |l| !l.hits.is_empty()), "climbs up and practises: {} / knight {:?} / dummy {dummy:?}", live.reason, live.knight.pos);
}
