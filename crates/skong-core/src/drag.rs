//! The user's hand on the props: the training dummy, visitors, Geo deposits and benches
//! (placed and found) can be picked up with the mouse, carried, and let go. Let go, a thing
//! falls (a simple body under gravity, keeping a little of the hand's swing) and lands on
//! the first good enough ledge below it ([`Fall`]):
//!
//! * wide enough for its footprint (it's moved in from the ledge's ends to fit);
//! * a window's title bar, the monitor floor, or a content line that has **settled** (a
//!   line vision only just found may be gone in a moment: it falls past). A title bar that
//!   moved lately still counts: it lands, and the usual rules (props hide on unsettled
//!   ledges, visitors ride windows) take over from there;
//! * never on a monitor running a fullscreen game: let go over one, it drops from the
//!   nearest point of the nearest other monitor instead (a game's screen is never ours);
//! * nothing below: the floor of its monitor's work area.
//!
//! Not draggable: enemies, the Troupe Master and his flames (the scarlet lantern's click
//! summons him), the Shade, Geo coins, shinies, lumaflies. Pure: the app reads the mouse
//! (never moves it) and applies the consequences (`skong/src/app/drag.rs`).
use crate::desktop::{Monitor, WindowId};
use crate::geom::{RectF, RectI, Vec2};
use crate::terrain::{Ledge, LedgeKind, Terrain};

/// What the user can pick up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DragKind {
    Dummy,
    Visitor,
    Deposit,
    /// A bench found in a window.
    FoundBench,
    /// A bench the user placed (tray / panel).
    Bench,
}

impl DragKind {
    pub fn label(self) -> &'static str {
        match self {
            DragKind::Dummy => "training dummy",
            DragKind::Visitor => "visitor",
            DragKind::Deposit => "geo deposit",
            DragKind::FoundBench => "found bench",
            DragKind::Bench => "bench",
        }
    }
}

/// A grab further than this (px) from the press is a drag; less is a click (as the knight).
pub const DRAG_START: f32 = 6.0;
/// A drop right on a ledge's top edge (up to this many px above it) lands on it.
pub const ABOVE_SLACK: f32 = 6.0;
/// Kept this far from a ledge's ends (px), when it is wide enough.
pub const EDGE_MARGIN: f32 = 8.0;
/// Gravity and terminal speed, in game units (× px per unit): the visitors' own fall.
const GRAVITY: f32 = 60.0;
const TERMINAL: f32 = 30.0;
/// Share of the hand's last velocity a dropped thing keeps (no throwing), and its cap
/// (units/s).
const CARRY: f32 = 0.15;
const MAX_CARRY: f32 = 6.0;
/// Horizontal drift dies out at this rate (1/s).
const DRAG_X: f32 = 3.0;

/// Where a dropped thing may land.
pub struct DropSite<'a> {
    pub terrain: &'a Terrain,
    pub monitors: &'a [Monitor],
    pub game_monitors: &'a [RectI],
    /// Half its width (px): the ledge must be at least twice as wide.
    pub half: f32,
    /// Has this ledge been on screen long enough to carry something (content lines must)?
    pub settled: &'a dyn Fn(&Ledge) -> bool,
    /// Any further rule of its own (finds: only windows that are rooms, or the floor).
    pub accept: &'a dyn Fn(&Ledge) -> bool,
}

impl DropSite<'_> {
    fn usable(&self, m: &Monitor) -> bool {
        !self.game_monitors.iter().any(|g| *g == m.rect || g.contains(m.rect.left, m.rect.top))
    }

    /// The monitor a drop at `p` belongs to: the one under it, else (off screen, or over a
    /// game) the nearest usable one.
    fn monitor_for(&self, p: Vec2) -> Option<&Monitor> {
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if let Some(m) = self.monitors.iter().find(|m| m.rect.contains(x, y)).filter(|m| self.usable(m)) {
            return Some(m);
        }
        self.monitors.iter().filter(|m| self.usable(m)).min_by(|a, b| distance(a.rect.to_f(), p).total_cmp(&distance(b.rect.to_f(), p)))
    }

    /// Can the thing stand on `l` (wide enough, inside `work`, settled or a title bar /
    /// floor, its own rule)?
    fn fits(&self, l: &Ledge, work: RectF) -> bool {
        let (x0, x1) = (l.x0.max(work.left), l.x1.min(work.right));
        x1 - x0 >= 2.0 * self.half && l.y > work.top && l.y <= work.bottom + 1.0 && (l.kind != LedgeKind::Content || (self.settled)(l)) && (self.accept)(l)
    }
}

fn distance(r: RectF, p: Vec2) -> f32 {
    let dx = (r.left - p.x).max(p.x - r.right).max(0.0);
    let dy = (r.top - p.y).max(p.y - r.bottom).max(0.0);
    dx.hypot(dy)
}

/// Where a dropped thing came to rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    /// Feet point (bottom centre) on the ledge.
    pub at: Vec2,
    /// The window whose edge it is (`None`: the floor).
    pub owner: Option<WindowId>,
    /// The ledge's extent (x0, x1).
    pub span: (f32, f32),
    pub kind: LedgeKind,
}

/// The first ledge `site` accepts that a thing at `x` crosses falling from `from_y` to
/// `to_y` (inclusive), on `work`. Its feet are moved in from the ledge's ends to fit.
pub fn ledge_between(site: &DropSite, work: RectF, x: f32, from_y: f32, to_y: f32) -> Option<Landing> {
    let l = site
        .terrain
        .ledges
        .iter()
        .filter(|l| l.x0 <= x && x <= l.x1 && l.y >= from_y && l.y <= to_y && site.fits(l, work))
        .min_by(|a, b| a.y.total_cmp(&b.y))?;
    Some(land_on(site, l, work, x))
}

fn land_on(site: &DropSite, l: &Ledge, work: RectF, x: f32) -> Landing {
    let (x0, x1) = (l.x0.max(work.left), l.x1.min(work.right));
    let keep = site.half + EDGE_MARGIN;
    let x = if x0 + keep <= x1 - keep { x.clamp(x0 + keep, x1 - keep) } else { (x.clamp(x0 + site.half, x1 - site.half)).clamp(x0, x1) };
    Landing { at: Vec2::new(x, l.y), owner: l.owner, span: (l.x0, l.x1), kind: l.kind }
}

/// The floor of `work` under `x`: its ground ledge when there is one, else the work-area
/// bottom itself.
pub fn floor_at(site: &DropSite, work: RectF, x: f32) -> Landing {
    let x = x.clamp(work.left + site.half, (work.right - site.half).max(work.left + site.half));
    let ground = site.terrain.ledges.iter().filter(|l| l.kind == LedgeKind::Ground && l.x0 <= x && x <= l.x1 && l.y > work.top && l.y <= work.bottom + 1.0).max_by(|a, b| a.y.total_cmp(&b.y));
    match ground {
        Some(l) => land_on(site, l, work, x),
        None => Landing { at: Vec2::new(x, work.bottom), owner: None, span: (work.left, work.right), kind: LedgeKind::Ground },
    }
}

/// A thing let go, falling until it lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fall {
    /// Feet point (bottom centre).
    pub pos: Vec2,
    pub vel: Vec2,
    /// The work area it falls in (it never drifts off it).
    work: RectF,
    /// The first step also takes a ledge just above the feet ([`ABOVE_SLACK`]).
    started: bool,
}

impl Fall {
    /// Let go at `feet` with the hand moving at `hand_vel` (px/s; only a little of it is
    /// kept). Over a game monitor or off screen it starts from the nearest point of the
    /// nearest usable monitor. `None`: no monitor it may go to (games on every one).
    pub fn start(site: &DropSite, feet: Vec2, hand_vel: Vec2, ppu: f32) -> Option<Fall> {
        let m = site.monitor_for(feet)?;
        let work = m.work.to_f();
        let lo = work.left + site.half;
        let hi = (work.right - site.half).max(lo);
        let pos = Vec2::new(feet.x.clamp(lo, hi), feet.y.clamp(work.top + 1.0, work.bottom));
        let mut vel = hand_vel * CARRY;
        let cap = MAX_CARRY * ppu;
        if vel.length() > cap {
            vel = vel.normalize() * cap;
        }
        Some(Fall { pos, vel, work, started: false })
    }

    /// Advance `dt` seconds; `Some` once it has landed (the fall is over).
    pub fn step(&mut self, site: &DropSite, dt: f32, ppu: f32) -> Option<Landing> {
        let from = if self.started { self.pos.y } else { self.pos.y - ABOVE_SLACK };
        self.started = true;
        self.vel.y = (self.vel.y + GRAVITY * ppu * dt).min(TERMINAL * ppu);
        self.vel.x *= (-DRAG_X * dt).exp();
        let lo = self.work.left + site.half;
        let hi = (self.work.right - site.half).max(lo);
        let x = (self.pos.x + self.vel.x * dt).clamp(lo, hi);
        if x == lo || x == hi {
            self.vel.x = 0.0;
        }
        let y = self.pos.y + self.vel.y * dt;
        // Rising (a little carried swing): nothing to land on until it comes down.
        if self.vel.y > 0.0
            && let Some(l) = ledge_between(site, self.work, x, from.min(y), y)
        {
            self.pos = l.at;
            return Some(l);
        }
        if y >= self.work.bottom {
            let l = floor_at(site, self.work, x);
            self.pos = l.at;
            return Some(l);
        }
        self.pos = Vec2::new(x, y);
        None
    }

    /// Run to the end (tests, or a drop that must land right away).
    pub fn settle(mut self, site: &DropSite, ppu: f32) -> Landing {
        for _ in 0..100_000 {
            if let Some(l) = self.step(site, 1.0 / 120.0, ppu) {
                return l;
            }
        }
        floor_at(site, self.work, self.pos.x)
    }
}

/// Is the knight sitting on the bench whose foot is at `bench` (`half`: half its width)?
/// `seat`: his feet while a seated act plays (`None`: not seated).
pub fn sitting_on(seat: Option<Vec2>, bench: Vec2, half: f32) -> bool {
    seat.is_some_and(|k| (k.y - bench.y).abs() < 2.0 && (k.x - bench.x).abs() <= half.max(6.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::WindowBox;

    fn monitors() -> Vec<Monitor> {
        vec![
            Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 },
            Monitor { rect: RectI::new(1920, 0, 3840, 1080), work: RectI::new(1920, 0, 3840, 1040), primary: false, scale_milli: 1000 },
        ]
    }

    /// Window 7 (title bar y 400, x 100..500) with a content line at y 600 (x 150..450), a
    /// narrow window 9 (title bar y 300, x 700..760), floors at 1040.
    fn terrain() -> Terrain {
        Terrain {
            windows: vec![WindowBox { id: 7, rect: RectF::new(100.0, 400.0, 500.0, 900.0) }, WindowBox { id: 9, rect: RectF::new(700.0, 300.0, 760.0, 500.0) }],
            ledges: vec![
                Ledge { x0: 100.0, x1: 500.0, y: 400.0, kind: LedgeKind::OneWay, owner: Some(7) },
                Ledge { x0: 150.0, x1: 450.0, y: 600.0, kind: LedgeKind::Content, owner: Some(7) },
                Ledge { x0: 700.0, x1: 760.0, y: 300.0, kind: LedgeKind::OneWay, owner: Some(9) },
                Ledge { x0: 0.0, x1: 1920.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
                Ledge { x0: 1920.0, x1: 3840.0, y: 1040.0, kind: LedgeKind::Ground, owner: None },
            ],
            bounds: RectF::new(0.0, 0.0, 3840.0, 1080.0),
            ..Default::default()
        }
    }

    const PPU: f32 = 38.4;

    fn drop_at(t: &Terrain, games: &[RectI], half: f32, settled: bool, at: Vec2) -> Landing {
        let m = monitors();
        let st = move |_: &Ledge| settled;
        let site = DropSite { terrain: t, monitors: &m, game_monitors: games, half, settled: &st, accept: &|_| true };
        Fall::start(&site, at, Vec2::ZERO, PPU).expect("a monitor").settle(&site, PPU)
    }

    #[test]
    fn lands_on_the_first_wide_enough_ledge_below() {
        let t = terrain();
        // Above window 7: its title bar.
        let l = drop_at(&t, &[], 20.0, true, Vec2::new(300.0, 100.0));
        assert_eq!((l.at, l.owner, l.kind), (Vec2::new(300.0, 400.0), Some(7), LedgeKind::OneWay));
        // Let go inside the window, above its (settled) content line: the line.
        assert_eq!(drop_at(&t, &[], 20.0, true, Vec2::new(300.0, 500.0)).at.y, 600.0);
        // Right on the title bar's top edge still counts.
        assert_eq!(drop_at(&t, &[], 20.0, true, Vec2::new(300.0, 397.0)).at.y, 400.0);
        // Window 9's title bar is 60 px wide: a 40 px thing lands, an 80 px one falls past.
        assert_eq!(drop_at(&t, &[], 20.0, true, Vec2::new(730.0, 100.0)).owner, Some(9));
        let wide = drop_at(&t, &[], 40.0, true, Vec2::new(730.0, 100.0));
        assert_eq!((wide.at.y, wide.owner), (1040.0, None));
        // Near a ledge's end: moved in to fit.
        assert_eq!(drop_at(&t, &[], 30.0, true, Vec2::new(105.0, 100.0)).at.x, 100.0 + 30.0 + EDGE_MARGIN);
    }

    #[test]
    fn content_lines_must_have_settled_title_bars_need_not() {
        let t = terrain();
        // An unsettled content line: falls past it to the floor (beside the window's box
        // the title bar is above, not below).
        let l = drop_at(&t, &[], 20.0, false, Vec2::new(300.0, 500.0));
        assert_eq!((l.at.y, l.owner), (1040.0, None));
        // An unsettled title bar is still landed on.
        assert_eq!(drop_at(&t, &[], 20.0, false, Vec2::new(300.0, 100.0)).at.y, 400.0);
    }

    #[test]
    fn nothing_below_means_the_floor() {
        let t = terrain();
        let l = drop_at(&t, &[], 20.0, true, Vec2::new(1200.0, 50.0));
        assert_eq!((l.at, l.owner, l.kind), (Vec2::new(1200.0, 1040.0), None, LedgeKind::Ground));
        // No ground ledge at all (a terrain still building): the work-area bottom.
        let mut bare = terrain();
        bare.ledges.clear();
        assert_eq!(drop_at(&bare, &[], 20.0, true, Vec2::new(1200.0, 50.0)).at, Vec2::new(1200.0, 1040.0));
        // Let go below the floor (over the taskbar): straight onto it.
        assert_eq!(drop_at(&t, &[], 20.0, true, Vec2::new(1200.0, 1070.0)).at.y, 1040.0);
    }

    #[test]
    fn never_on_a_game_monitor() {
        let t = terrain();
        let game = [RectI::new(1920, 0, 3840, 1080)];
        // Let go over the game: dropped from the nearest point of the other monitor.
        let l = drop_at(&t, &game, 20.0, true, Vec2::new(2500.0, 200.0));
        assert!(l.at.x <= 1920.0 - 20.0 && l.at.x > 1800.0, "{l:?}");
        assert_eq!(l.at.y, 1040.0);
        // Games everywhere: nowhere to go.
        let m = monitors();
        let site = DropSite { terrain: &t, monitors: &m, game_monitors: &[m[0].rect, m[1].rect], half: 20.0, settled: &|_| true, accept: &|_| true };
        assert!(Fall::start(&site, Vec2::new(300.0, 100.0), Vec2::ZERO, PPU).is_none());
    }

    #[test]
    fn a_little_swing_is_kept_never_a_throw() {
        let t = terrain();
        let m = monitors();
        let site = DropSite { terrain: &t, monitors: &m, game_monitors: &[], half: 20.0, settled: &|_| true, accept: &|_| true };
        let f = Fall::start(&site, Vec2::new(1000.0, 100.0), Vec2::new(4000.0, -3000.0), PPU).unwrap();
        assert!(f.vel.length() <= MAX_CARRY * PPU + 0.01);
        let l = f.settle(&site, PPU);
        // It drifted right a little, and stayed on its monitor.
        assert!(l.at.x > 1000.0 && l.at.x < 1300.0, "{l:?}");
        // Its own rule: a thing that may only go on the floor falls past the title bar.
        let floor_only = |l: &Ledge| l.owner.is_none();
        let site = DropSite { accept: &floor_only, ..site };
        assert_eq!(Fall::start(&site, Vec2::new(300.0, 100.0), Vec2::ZERO, PPU).unwrap().settle(&site, PPU).owner, None);
    }

    #[test]
    fn a_seated_knight_is_on_the_bench_under_him() {
        let bench = Vec2::new(500.0, 400.0);
        assert!(sitting_on(Some(Vec2::new(503.0, 400.0)), bench, 40.0));
        assert!(!sitting_on(None, bench, 40.0));
        assert!(!sitting_on(Some(Vec2::new(600.0, 400.0)), bench, 40.0));
        assert!(!sitting_on(Some(Vec2::new(500.0, 300.0)), bench, 40.0));
    }
}
