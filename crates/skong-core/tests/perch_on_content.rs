//! End to end: the knight perches on a content ledge (a line drawn inside a window) for
//! 30 s while vision keeps rescanning the area around him, terrain is rebuilt from the
//! content at 30 Hz and the controller re-validates his support every time.
//!
//! Worst case: capture *sees* his animated sprite (legs dangling over the ledge like in
//! the seated acts). With the region under our own window frozen he must never fall.
use skong_core::controller::{Abilities, Character, CharacterParams, MoveEvent, MoveInput};
use skong_core::desktop::{DesktopSnapshot, Monitor, WindowInfo};
use skong_core::geom::{RectI, Vec2};
use skong_core::terrain::{ContentLayer, LedgeKind, Terrain, TerrainOptions};
use skong_core::vision::{PixelView, SurfaceMap, VisionParams};

const W: usize = 1920;
const H: usize = 1080;
const LEDGE_Y: i32 = 600;
const DT: f32 = 1.0 / 120.0;

struct World {
    px: Vec<u32>,
    desk: DesktopSnapshot,
    map: SurfaceMap,
    layer: ContentLayer,
    terrain: Terrain,
    t: f64,
}

impl World {
    fn new() -> Self {
        let mut px = vec![0xff20_2020u32; W * H];
        // A window (light) with a 2 px dark line drawn inside it.
        for y in 200..900 {
            px[y * W + 300..y * W + 1500].fill(0xfff0_f0f0);
        }
        for y in LEDGE_Y as usize..LEDGE_Y as usize + 2 {
            px[y * W + 500..y * W + 1300].fill(0xff10_1010);
        }
        let monitor = Monitor { rect: RectI::new(0, 0, W as i32, H as i32), work: RectI::new(0, 0, W as i32, 1040), primary: true, scale_milli: 1000 };
        let window = WindowInfo {
            id: 7,
            rect: RectI::new(300, 200, 1500, 900),
            title: "Paint".into(),
            class: String::new(),
            process: String::new(),
            pid: 0,
            maximized: false,
            fullscreen: false,
            foreground: true,
            topmost: false,
        };
        let desk = DesktopSnapshot { generation: 1, monitors: vec![monitor], windows: vec![window] };
        let mut w = Self { px, desk, map: SurfaceMap::new(W, H, VisionParams::default()), layer: ContentLayer::default(), terrain: Terrain::default(), t: 0.0 };
        w.layer.observe_desktop(&w.desk, 0.0);
        w.vision(&[RectI::new(0, 0, W as i32, H as i32)]);
        w.t = 0.1;
        w.vision(&[]);
        w
    }

    /// One capture pass: rescan `dirty`, hysteresis, ingest, rebuild terrain.
    fn vision(&mut self, dirty: &[RectI]) -> bool {
        let view = PixelView::new(&self.px, W, H, W).expect("view");
        self.map.rescan(&view, dirty);
        let changed = self.map.update(self.t);
        self.layer.ingest(0, self.map.surfaces(), self.map.walls(), &self.desk, self.t);
        let opts = TerrainOptions::default();
        self.terrain = Terrain::from_desktop_and_content(&self.desk, &self.layer.ledges(&self.desk), &self.layer.walls(&self.desk), &opts);
        changed
    }

    /// Paint an animated knight-like sprite over `r` (what a capture that sees our
    /// windows would contain).
    fn paint_sprite(&mut self, r: RectI, frame: usize) {
        for y in r.top..r.bottom {
            for x in r.left..r.right {
                let v = ((x * 7 + y * 13 + frame as i32 * 11) % 251) as u32;
                self.px[y as usize * W + x as usize] = 0xff00_0000 | v << 16 | (255 - v) << 8 | (v / 2);
            }
        }
    }
}

/// Sprite window rect for a seated knight with feet at `feet`: body above, legs and
/// cloak dangling ~28 px below the ledge.
fn sprite_rect(c: &Character) -> RectI {
    let b = c.body();
    RectI::new(b.left as i32 - 20, b.top as i32 - 20, b.right as i32 + 20, b.bottom as i32 + 28)
}

/// Run `secs` of perching. `freeze`: freeze vision under the sprite window.
/// `extra`: called once per capture frame to change the screen elsewhere.
fn perch(secs: f32, freeze: bool, mut extra: impl FnMut(&mut World, usize) -> Option<RectI>) -> Result<(), String> {
    let mut w = World::new();
    assert!(w.terrain.ledges.iter().any(|l| l.kind == LedgeKind::Content && l.y == LEDGE_Y as f32), "content ledge detected");
    let mut c = Character::new(CharacterParams::hollow_knight(64.0), Abilities::default(), Vec2::new(900.0, 450.0));
    let mut ev = Vec::new();
    for _ in 0..240 {
        c.step(&MoveInput::default(), &w.terrain, DT, &mut ev);
        if c.is_grounded() {
            break;
        }
    }
    if !(c.is_grounded() && c.pos.y == LEDGE_Y as f32) {
        return Err(format!("never landed on the content ledge: {:?}", c.pos));
    }
    let mut frozen: Vec<RectI> = Vec::new();
    let ticks = (secs / DT) as usize;
    for tick in 0..ticks {
        if tick % 4 == 0 {
            let frame = tick / 4;
            w.t += 4.0 * DT as f64;
            let own = sprite_rect(&c);
            let mut dirty = vec![own];
            if freeze {
                let grown = RectI::new(own.left - 16, own.top - 16, own.right + 16, own.bottom + 16);
                let left = w.map.set_frozen(&[grown]);
                dirty.extend(left);
                frozen = vec![grown];
            }
            w.paint_sprite(own, frame);
            if let Some(r) = extra(&mut w, frame) {
                dirty.push(r);
            }
            let old = w.terrain.clone();
            w.vision(&dirty);
            ev.clear();
            c.on_terrain_rebuilt(&old, &w.terrain, &mut ev);
            if ev.contains(&MoveEvent::LostSupport) {
                return Err(format!("lost support at {:.2} s (frozen {frozen:?})", tick as f32 * DT));
            }
        }
        c.step(&MoveInput::default(), &w.terrain, DT, &mut ev);
        if !c.is_grounded() {
            return Err(format!("fell at {:.2} s", tick as f32 * DT));
        }
    }
    Ok(())
}

#[test]
fn perched_knight_never_falls_for_30_seconds() {
    perch(30.0, true, |_, _| None).expect("stable perch");
}

#[test]
fn without_the_freeze_a_visible_sprite_would_drop_him() {
    // Control: proves the test bites (only matters where capture sees our windows).
    assert!(perch(5.0, false, |_, _| None).is_err());
}

#[test]
fn a_line_appearing_just_above_does_not_drop_him() {
    perch(10.0, true, |w, frame| {
        // At 2 s a focus outline appears 4 rows above the ledge, outside the knight.
        (frame == 60).then(|| {
            let y = (LEDGE_Y - 4) as usize;
            w.px[y * W + 500..y * W + 1300].fill(0xff30_70ff);
            RectI::new(500, LEDGE_Y - 4, 1300, LEDGE_Y - 3)
        })
    })
    .expect("stable perch");
}

#[test]
fn busy_screen_elsewhere_does_not_disturb_him() {
    perch(10.0, true, |w, frame| {
        // A blinking block 40 px above the ledge, 200 px to his left.
        let r = RectI::new(650, LEDGE_Y - 60, 700, LEDGE_Y - 40);
        let c = if frame % 2 == 0 { 0xff10_1010 } else { 0xfff0_f0f0 };
        for y in r.top..r.bottom {
            w.px[y as usize * W + r.left as usize..y as usize * W + r.right as usize].fill(c);
        }
        Some(r)
    })
    .expect("stable perch");
}
