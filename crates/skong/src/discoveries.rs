//! Thing found in windows, on screen: the persistent [`World`] (`%APPDATA%\Desk-Skong\
//! world.json`), one small sprite window per find, reveal fades, deposit hits, how long
//! each surface has lasted ([`LedgeAges`]), collisions with other props through the shared
//! [`Occupancy`], and what the brain needs to know ([`Discoveries::places`]).
//!
//! Art (game sprites, exported into `skins/Vanilla/props`) is named in `data/props.json`
//! under `discoveries`; anything missing degrades to plain shapes.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use skong_core::ai::KnownPlace;
use skong_core::assets::{Atlas, read_json};
use skong_core::canvas::{Image, over, premul};
use skong_core::controller::Character;
use skong_core::desktop::{DesktopSnapshot, WindowId};
use skong_core::discovery::{self, DepositDrop, FindId, FindKind, FindSpot, Footprint, LASTING_SECS, Rules, Shown, Site, World};
use skong_core::geom::{RectI, Vec2};
use skong_core::lasting::LedgeAges;
use skong_core::occupancy::Occupancy;
use skong_core::terrain::Terrain;
use skong_win::sprite_window::{MouseEvent, SpriteWindow};

use crate::benches::Benches;
use crate::fx::Fx;
use crate::grubs::GrubArt;

/// A revealed find fades in over this long (s).
const REVEAL_SECS: f32 = 1.2;
/// ... and counts as "just found" for the brain this long (s).
const FRESH_SECS: f32 = 6.0;
/// A find whose ledge vanished stays this long while its window is open (s).
const HIDE_GRACE: f32 = 1.5;
/// How often surfaces are re-observed between terrain rebuilds (s).
const OBSERVE_EVERY: f32 = 0.25;
/// Effect played where something is revealed (see `data/effects.json`).
const REVEAL_FX: &str = "reveal_dust";

// Sizes in px at scale 1 (the knight is ~88 px tall).
const DEPOSIT_W: f32 = 124.0;
/// Where the knight stands to mine a deposit (from its center), at least...
const DEPOSIT_STAND: f32 = 82.0;
/// ... and at least this far beyond a (bigger) rock's edge.
const STAND_CLEAR: f32 = 28.0;
/// A trapped grub cries for help once the knight is this close (px at scale 1).
const GRUB_CRY_DIST: f32 = 700.0;

/// Seconds between a deposit's gleams (the game's own shimmer), varied per rock.
const GLEAM_EVERY: f32 = 6.0;
/// Gleam speed when a clip doesn't say.
const GLEAM_FPS: f32 = 12.0;
/// Regional rocks are much bigger than the crossroads one (the Resting Grounds chest is
/// 3.5 units tall): a region is drawn smaller so its largest idle fits this box (game
/// units; the crossroads rock is ~1.7 × 1.6).
const ROCK_MAX_UNITS: [f32; 2] = [2.2, 2.3];

/// A region's Geo Rock: an exported library (`data/hk/animations/<lib>.json`) and which of
/// its shapes (`Idle n` / `Gleam n`) belong to that region.
struct Region {
    name: &'static str,
    lib: &'static str,
    shapes: &'static [u8],
}

/// Every app gets one region's look ([`region_of`]). The first is the fallback: a region
/// whose art is missing shows the crossroads rock.
const REGIONS: &[Region] = &[
    Region { name: "Forgotten Crossroads", lib: "Coin_Rocks", shapes: &[1, 2] },
    Region { name: "City of Tears", lib: "Geo_Rock_City", shapes: &[1] },
    Region { name: "Greenpath", lib: "Geo_Rocks_Green", shapes: &[1, 2] },
    Region { name: "Fungal Wastes", lib: "Geo_Rocks_Fung", shapes: &[1, 2] },
    Region { name: "Deepnest", lib: "Geo_Rocks_Deep_Outskirts", shapes: &[1] },
    Region { name: "Kingdom's Edge", lib: "Geo_Rocks_Deep_Outskirts", shapes: &[2] },
    Region { name: "Resting Grounds", lib: "Geo_Rocks_Grave", shapes: &[1, 2] },
    Region { name: "Abyss", lib: "Geo_Rocks_Extra", shapes: &[1] },
    Region { name: "Crystal Peak", lib: "Geo_Rocks_Extra", shapes: &[2] },
    Region { name: "Hive", lib: "Geo_Rocks_Extra", shapes: &[3] },
];
const FALLBACK_REGION: usize = 0;

/// The region whose rocks an app's windows hold: a stable hash (FNV-1a) of its app key
/// ([`discovery::app_key`]), so each app keeps its look across runs without saving it.
fn region_of(app: &str) -> usize {
    let h = app.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    (h % REGIONS.len() as u64) as usize
}

/// The region whose art is drawn for `region`: itself when loaded, else the fallback, else
/// none (plain deposit).
fn art_region(loaded: impl Fn(usize) -> bool, region: usize) -> Option<usize> {
    [region, FALLBACK_REGION].into_iter().find(|&r| loaded(r))
}

/// One shape of a Geo Rock (`Idle n` / `Gleam n`), at the current scale. Frames carry
/// their anchors so the gleam lines up with the idle pose.
struct RockArt {
    idle: (Image, [f32; 2]),
    gleam: Vec<(Image, [f32; 2])>,
    gleam_fps: f32,
}

type Frames = std::collections::HashMap<skong_core::assets::SpriteKey, skong_core::assets::Frame>;

/// The sprites of `region`'s shapes in `lib`.
fn shape_keys(lib: &skong_core::anim::ClipLibrary, region: &Region) -> Vec<skong_core::assets::SpriteKey> {
    let mut keys: Vec<_> = region
        .shapes
        .iter()
        .flat_map(|n| [format!("Idle {n}"), format!("Gleam {n}")])
        .filter_map(|c| lib.id(&c))
        .flat_map(|id| lib.get(id).frames.iter().filter_map(|f| f.sprite))
        .collect();
    keys.sort_by_key(|k| (k.collection, k.sprite));
    keys.dedup();
    keys
}

/// `region`'s shapes from rasterized `frames` (shapes without an idle frame are skipped).
fn region_rocks(lib: &skong_core::anim::ClipLibrary, region: &Region, frames: &Frames) -> Vec<RockArt> {
    let clip = |name: &str| lib.id(name).map(|id| lib.get(id));
    let images = |name: &str| -> Vec<(Image, [f32; 2])> {
        clip(name).map(|c| c.frames.iter().filter_map(|f| f.sprite.and_then(|k| frames.get(&k))).map(|f| (f.image.clone(), f.anchor)).collect()).unwrap_or_default()
    };
    region
        .shapes
        .iter()
        .filter_map(|n| {
            let idle = images(&format!("Idle {n}")).into_iter().next()?;
            let gleam_name = format!("Gleam {n}");
            let gleam_fps = clip(&gleam_name).map_or(GLEAM_FPS, |c| c.fps).clamp(4.0, 30.0);
            Some(RockArt { idle, gleam: images(&gleam_name), gleam_fps })
        })
        .collect()
}

/// Rasterize every region's Geo Rock (indexed like [`REGIONS`]); a region whose library or
/// atlas is missing gets no shapes (it shows the fallback's).
fn rock_art(data: &Path, vanilla: &Path, px_per_unit: f32) -> anyhow::Result<Vec<Vec<RockArt>>> {
    use skong_core::anim::ClipLibrary;
    use skong_core::assets::{Skin, SpriteBank};
    let skin = Skin { name: "Vanilla".into(), dir: vanilla.to_path_buf() };
    let bank = SpriteBank::load_metadata(data)?;
    let libs: Vec<Option<ClipLibrary>> = REGIONS
        .iter()
        .map(|r| ClipLibrary::load(&data.join(format!("animations/{}.json", r.lib)), &bank).map_err(|e| log::debug!("geo rocks of {}: {e:#}", r.name)).ok())
        .collect();
    let keys: Vec<Vec<_>> = libs.iter().zip(REGIONS).map(|(l, r)| l.as_ref().map(|l| shape_keys(l, r)).unwrap_or_default()).collect();
    let build = |sets: &[Frames]| -> Vec<Vec<RockArt>> {
        libs.iter().zip(REGIONS).zip(sets).map(|((l, r), f)| l.as_ref().map(|l| region_rocks(l, r, f)).unwrap_or_default()).collect()
    };
    let sets: Vec<(&[_], f32)> = keys.iter().map(|k| (k.as_slice(), px_per_unit)).collect();
    let mut rocks = build(&bank.rasterize_sets(&skin, None, &sets)?);
    // Shrink oversized regions to fit the box (each atlas decoded once more at most).
    let max = ROCK_MAX_UNITS.map(|u| u * px_per_unit);
    let fit: Vec<f32> = rocks
        .iter()
        .map(|shapes| shapes.iter().map(|s| (max[0] / s.idle.0.width.max(1) as f32).min(max[1] / s.idle.0.height.max(1) as f32)).fold(1.0f32, f32::min))
        .collect();
    if fit.iter().any(|&f| f < 0.999) {
        let sets: Vec<(&[_], f32)> = keys.iter().zip(&fit).map(|(k, &f)| (if f < 0.999 { k.as_slice() } else { &[][..] }, px_per_unit * f)).collect();
        for (i, shapes) in build(&bank.rasterize_sets(&skin, None, &sets)?).into_iter().enumerate() {
            if fit[i] < 0.999 {
                rocks[i] = shapes;
            }
        }
    }
    anyhow::ensure!(rocks.iter().any(|r| !r.is_empty()), "no Geo Rock clips");
    let missing: Vec<&str> = REGIONS.iter().zip(&rocks).filter(|(_, r)| r.is_empty()).map(|(g, _)| g.name).collect();
    if !missing.is_empty() {
        log::info!("geo rocks: no art for {} (they show the crossroads rock)", missing.join(", "));
    }
    Ok(rocks)
}

/// Which rock shape a find uses, and the frame to show at `clock` (`None` = idle).
fn rock_frame(id: FindId, rocks: &[RockArt], clock: f64) -> (usize, Option<usize>) {
    let variant = (id % rocks.len().max(1) as u64) as usize;
    let Some(rock) = rocks.get(variant) else { return (0, None) };
    let period = GLEAM_EVERY + (id % 5) as f32 * 0.7;
    let t = ((clock + (id % 97) as f64 * 0.37) % period as f64) as f32;
    let f = (t * rock.gleam_fps) as usize;
    (variant, (f < rock.gleam.len()).then_some(f))
}

/// How far from a deposit's center the knight stands to mine it (px), given the rock's
/// drawn width.
fn stand_off(rock_w: f32, scale: f32) -> f32 {
    (DEPOSIT_STAND * scale).max(rock_w * 0.5 + STAND_CLEAR * scale)
}

#[derive(Deserialize)]
#[serde(default)]
struct ArtNames {
    deposit_rock: String,
    grub_jar: String,
}

impl Default for ArtNames {
    fn default() -> Self {
        Self { deposit_rock: "mines_small_rock".into(), grub_jar: "brk_bottle".into() }
    }
}

#[derive(Deserialize, Default)]
struct PropsFile {
    #[serde(default)]
    discoveries: ArtNames,
}

struct Prop {
    shown: Shown,
    win: SpriteWindow,
    /// Clock time its reveal started (None: it was always there).
    revealed: Option<f64>,
    /// Clock time it last resolved on its window.
    seen_at: f64,
    /// Seconds of hit shake left.
    shake: f32,
    drawn_at: f64,
    dirty: bool,
    /// Deposit frame last drawn (art region, rock shape, gleam frame).
    frame: (usize, usize, Option<usize>),
    /// The region of its app's rocks ([`region_of`]).
    region: usize,
}

pub struct Discoveries {
    world: World,
    rules: Rules,
    path: Option<PathBuf>,
    deposit_src: Option<Image>,
    /// The grub jar sprite (64 px per unit), and the jar with its grub at the current scale.
    jar_src: Option<Image>,
    grub: GrubArt,
    /// Jars broken since the pet last looked (the app plays the glass, the grub, a mimic).
    pub freed: Vec<FreedJar>,
    /// The plain deposit at the current scale (drawn when there is no Geo Rock art).
    deposit: Image,
    /// Room every deposit is given (px): the largest rock of any region, or `deposit`.
    deposit_box: (f32, f32),
    /// The game's Geo Rocks at the current scale, per [`REGIONS`] (empty: none loaded).
    rocks: Vec<Vec<RockArt>>,
    /// Exported metadata (`data/hk`) and the vanilla skin, to re-rasterize on rescale.
    data_dir: PathBuf,
    vanilla_dir: Option<PathBuf>,
    scale: f32,
    props: Vec<Prop>,
    shown: Vec<Shown>,
    /// Recently revealed finds with their reveal time.
    reveals: Vec<(FindId, f64)>,
    clock: f64,
    ages: LedgeAges,
    /// Terrain generation and clock of the last [`LedgeAges::observe`].
    observed: (u64, f64),
    /// The window the knight stood on last update (arrivals place pending finds).
    knight_on: Option<WindowId>,
    /// Monitors running a fullscreen game: nothing shows there.
    game_monitors: Vec<RectI>,
    /// Clock of the last check for broken deposits due to turn up elsewhere.
    relocate_checked: f64,
    pub capturable: bool,
    /// Regions ([`REGIONS`] index) of the Geo rocks broken since the pet last looked.
    pub mined: Vec<usize>,
    /// The find in the user's hand (or falling from it) and its foot: drawn there, not
    /// claimed, not known to the brain until it lands.
    held: Option<(FindId, Vec2)>,
}

/// A grub jar the knight just broke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreedJar {
    /// Where the jar stood (bottom centre) and the window it stood on.
    pub foot: Vec2,
    pub owner: Option<WindowId>,
    /// Where the grub was in it.
    pub grub: Vec2,
    /// The jar's middle (the glass flies from here).
    pub centre: Vec2,
    /// A Grub Mimic, not a grub.
    pub mimic: bool,
    /// The grub faced right (toward the knight).
    pub right: bool,
}

/// How many regions Geo rocks come from.
pub const REGION_COUNT: usize = REGIONS.len();

fn world_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Desk-Skong").join("world.json"))
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn load_sprite(dir: &Path, name: &str) -> Option<Image> {
    let atlas = Atlas::load_png(&dir.join(format!("{name}.png"))).ok()?;
    let img = Image::from_rgba(atlas.width, atlas.height, &atlas.rgba);
    // Normalize to 64 px per game unit (some sprites are authored at other densities).
    #[derive(Deserialize)]
    struct Meta {
        pixels_per_unit: f32,
    }
    let ppu = read_json::<Meta>(&dir.join(format!("{name}.json"))).map_or(64.0, |m| m.pixels_per_unit);
    Some(if (ppu - 64.0).abs() > 0.5 { img.scaled(64.0 / ppu) } else { img })
}

/// Footprint of a find at `scale`: the bench art / the deposit rock (`deposit`: the largest
/// any region draws, so every app's rocks fit where one was placed), plus room to stand
/// beside it to mine it. `boxes`: the deposit's and the grub jar's size.
fn footprint(scale: f32, benches: &Benches, boxes: [(f32, f32); 2], kind: FindKind) -> Footprint {
    let beside = |(w, h): (f32, f32)| Footprint { w, h, room: 2.0 * (stand_off(w, scale) + 24.0 * scale) };
    match kind {
        FindKind::Bench { style } => {
            let (w, h) = benches.art(style).map_or((120.0 * scale, 60.0 * scale), |i| (i.width as f32, i.height as f32));
            Footprint { w, h, room: w }
        }
        FindKind::Deposit => beside(boxes[0]),
        FindKind::Grub { .. } => beside(boxes[1]),
    }
}

/// The shapes drawn for `region` (its own, else the fallback's; empty: plain deposit), and
/// the region they belong to.
fn rocks_for(rocks: &[Vec<RockArt>], region: usize) -> (usize, &[RockArt]) {
    art_region(|r| rocks.get(r).is_some_and(|v| !v.is_empty()), region).map_or((FALLBACK_REGION, &[][..]), |r| (r, rocks[r].as_slice()))
}

/// The region of the app owning window `id` (the fallback for unknown windows).
fn window_region(desk: &DesktopSnapshot, id: WindowId) -> usize {
    desk.window(id).and_then(discovery::app_key).map_or(FALLBACK_REGION, |k| region_of(&k))
}

/// The region of find `id`'s rocks: its room's app (a rock dragged onto the floor keeps
/// its look; one dragged into another app's window takes that app's).
fn find_region(world: &World, id: FindId) -> usize {
    world.room_of(id).map_or(FALLBACK_REGION, region_of)
}

impl Discoveries {
    pub fn load(data: &Path, props_json: &Path, vanilla: Option<&Path>, scale: f32) -> Discoveries {
        let names = read_json::<PropsFile>(props_json).map(|p| p.discoveries).unwrap_or_default();
        let deposit_src = vanilla.and_then(|v| load_sprite(&v.join("props"), &names.deposit_rock));
        let jar_src = vanilla.and_then(|v| load_sprite(&v.join("props"), &names.grub_jar));
        let path = world_path();
        let fresh = || World::new(now_unix() ^ 0x5eed);
        let world = path.as_deref().map_or_else(fresh, |p| load_world(p, fresh));
        let found: usize = world.rooms.values().map(|r| r.finds.len()).sum();
        log::info!("discoveries: {} rooms known, {found} things found", world.rooms.len());
        let mut d = Discoveries {
            world,
            rules: Rules::default(),
            path,
            deposit_src,
            grub: GrubArt::load(data, None, None, scale), // the real art: `rescale` below
            jar_src,
            freed: Vec::new(),
            deposit: Image::default(),
            deposit_box: (0.0, 0.0),
            rocks: Vec::new(),
            data_dir: data.to_path_buf(),
            vanilla_dir: vanilla.map(Path::to_path_buf),
            scale: 0.0,
            props: Vec::new(),
            shown: Vec::new(),
            reveals: Vec::new(),
            clock: 0.0,
            ages: LedgeAges::default(),
            observed: (u64::MAX, f64::NEG_INFINITY),
            knight_on: None,
            game_monitors: Vec::new(),
            relocate_checked: 0.0,
            capturable: false,
            mined: Vec::new(),
            held: None,
        };
        d.rescale(scale);
        d
    }

    pub fn rescale(&mut self, scale: f32) {
        self.scale = scale;
        self.grub = GrubArt::load(&self.data_dir, self.vanilla_dir.as_deref(), self.jar_src.as_ref(), scale);
        let w = ((DEPOSIT_W * scale).round() as u32).max(1);
        let h = self.deposit_src.as_ref().map_or(((50.0 * scale).round() as u32).max(1), |r| ((r.height as f32 * w as f32 / r.width as f32).round() as u32).max(1));
        self.deposit = deposit_art(self.deposit_src.as_ref(), w, h, scale);
        self.rocks = match &self.vanilla_dir {
            Some(v) => rock_art(&self.data_dir, v, 64.0 * scale).unwrap_or_else(|e| {
                log::warn!("geo rock art: {e:#}; drawing a plain deposit");
                Vec::new()
            }),
            None => Vec::new(),
        };
        let idles = self.rocks.iter().flatten().map(|r| &r.idle.0);
        self.deposit_box = idles.fold(None, |b: Option<(f32, f32)>, i| {
            let (w, h) = (i.width as f32, i.height as f32);
            Some(b.map_or((w, h), |(bw, bh)| (bw.max(w), bh.max(h))))
        })
        .unwrap_or((self.deposit.width as f32, self.deposit.height as f32));
        for p in &mut self.props {
            p.dirty = true;
        }
    }

    /// Resolve what is on screen (lasting surfaces, collisions with other claims already in
    /// `occ`), create / move / animate the prop windows, and place pending finds when the
    /// knight comes onto their window.
    pub fn update(&mut self, dt: f32, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &mut Occupancy, fx: &mut Fx, (knight, knight_win): (&Character, &SpriteWindow)) {
        self.clock += dt as f64;
        let clock = self.clock;
        if self.observed.1 == f64::NEG_INFINITY {
            // What is on screen at start-up was there before we looked: settled already
            // (finds don't vanish for 15 s on every launch). Not "lasting" for new finds.
            self.ages.observe_aged(terrain, clock, skong_core::lasting::SETTLED_SECS);
            self.observed = (terrain.generation, clock);
        } else if terrain.generation != self.observed.0 || (clock - self.observed.1) as f32 >= OBSERVE_EVERY {
            self.ages.observe(terrain, clock);
            self.observed = (terrain.generation, clock);
        }
        let (scale, deposit) = (self.scale, [self.deposit_box, self.grub.size()]);
        let fp = |k: FindKind| footprint(scale, benches, deposit, k);

        // Arriving on a window of an app with finds that had no lasting room yet.
        let on = knight.support.and_then(|s| s.owner).filter(|_| knight.is_grounded());
        if on != self.knight_on && let Some(id) = on {
            let site = Site { terrain, ages: &self.ages, now: clock, min_age: LASTING_SECS, occupancy: occ, footprint: &fp };
            let found = self.world.arrived(id, desk, &site);
            self.revealed(found, desk, id);
        }
        self.knight_on = on;

        // Broken deposits turn up again in another app's window once due.
        if clock - self.relocate_checked >= 1.0 {
            self.relocate_checked = clock;
            let site = Site { terrain, ages: &self.ages, now: clock, min_age: LASTING_SECS, occupancy: occ, footprint: &fp };
            let placed = self.world.relocate_due(desk, &site, now_unix(), &self.rules);
            for (fid, wid) in placed {
                let what = self.world.find(fid).map_or("find", |f| f.kind.label());
                log::info!("discoveries: a new {what} turned up (window {wid})");
                self.revealed(vec![fid], desk, wid);
                self.save();
            }
        }

        // Finds only stand on ledges that have lasted (a line that just appeared where the
        // old one was doesn't carry them yet).
        let ages = &self.ages;
        let settled = |l: &skong_core::terrain::Ledge| ages.settled(terrain, l, clock);
        let mut raw = self.world.shown(desk, terrain, now_unix(), &self.rules, &fp, &settled);
        // Really on its ledge this tick (the grace copies below must not count as seen, or
        // the grace never ends: broken rocks stayed, finds floated where a ledge had been).
        let seen_now: Vec<FindId> = raw.iter().map(|s| s.id).collect();
        // Content ledges flicker (re-detection): a find keeps its last spot a moment while
        // its window is open.
        for p in &self.props {
            if !raw.iter().any(|s| s.id == p.shown.id) && desk.window(p.shown.window).is_some() && ((clock - p.seen_at) as f32) < HIDE_GRACE {
                raw.push(p.shown);
            }
        }
        raw.retain(|s| !self.game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1)));
        // In the user's hand: where the hand is (not settled against the others, no claim).
        let held = self.held.and_then(|(id, at)| {
            let p = self.props.iter().find(|p| p.shown.id == id)?;
            Some(Shown { pos: at, span: (at.x, at.x), ..p.shown })
        });
        if self.held.is_some() && held.is_none() {
            self.held = None; // gone meanwhile (broken, forgotten)
        }
        raw.retain(|s| held.is_none_or(|h| h.id != s.id));
        let mut shown = discovery::settle(raw, occ, &fp);
        shown.extend(held);

        self.props.retain(|p| {
            let keep = shown.iter().any(|s| s.id == p.shown.id);
            if !keep {
                log::debug!("discoveries: {} #{} hidden", p.shown.kind.label(), p.shown.id);
            }
            keep
        });
        let mut created = false;
        for s in &shown {
            let fresh_seen = seen_now.contains(&s.id);
            match self.props.iter_mut().find(|p| p.shown.id == s.id) {
                Some(p) => {
                    if p.shown.pos != s.pos {
                        p.dirty = true;
                    }
                    let region = find_region(&self.world, s.id);
                    if p.region != region {
                        p.region = region;
                        p.dirty = true;
                    }
                    p.shown = *s;
                    if fresh_seen {
                        p.seen_at = clock;
                    }
                }
                None => {
                    let Ok(win) = SpriteWindow::new() else { continue };
                    win.set_capturable(self.capturable);
                    // Its opaque pixels catch the mouse: the user can pick it up.
                    log::debug!("discoveries: {} #{} shown at ({:.0},{:.0})", s.kind.label(), s.id, s.pos.x, s.pos.y);
                    let revealed = self.reveals.iter().find(|r| r.0 == s.id).map(|r| r.1);
                    if revealed.is_some() {
                        fx.spawn(REVEAL_FX, s.pos, knight.pos.x < s.pos.x);
                    }
                    let region = find_region(&self.world, s.id);
                    if s.kind == FindKind::Deposit {
                        log::debug!("discoveries: deposit #{} is a {} rock", s.id, REGIONS[rocks_for(&self.rocks, region).0].name);
                    }
                    self.props.push(Prop { shown: *s, win, revealed, shake: 0.0, seen_at: clock, drawn_at: f64::NEG_INFINITY, dirty: true, frame: (0, 0, None), region });
                    created = true;
                }
            }
        }
        self.shown = shown;
        self.reveals.retain(|r| ((clock - r.1) as f32) < FRESH_SECS);

        for p in &mut self.props {
            p.shake = (p.shake - dt).max(0.0);
            let revealing = p.revealed.is_some_and(|t| (clock - t) as f32 <= REVEAL_SECS + 0.1);
            // Geo rocks gleam now and then: redraw when the frame changes.
            if p.shown.kind == FindKind::Deposit && deposit_frame(p, &self.rocks, clock) != p.frame {
                p.dirty = true;
            }
            // A trapped grub idles, and cries for help once the knight is near; it turns
            // to him.
            if let FindKind::Grub { .. } = p.shown.kind {
                let crying = knight.pos.distance(p.shown.pos) < GRUB_CRY_DIST * scale;
                let (cry, i) = self.grub.frame(crying, clock + (p.shown.id % 13) as f64 * 0.41);
                let frame = (cry as usize, i, Some((knight.pos.x > p.shown.pos.x) as usize));
                if frame != p.frame {
                    p.frame = frame;
                    p.dirty = true;
                }
            }
            if p.dirty || ((revealing || p.shake > 0.0) && clock - p.drawn_at >= 1.0 / 30.0) {
                if let Err(e) = draw(p, &self.deposit, &self.rocks, &self.grub, benches, scale, clock) {
                    log::warn!("discovery prop: {e:#}");
                }
                p.drawn_at = clock;
                p.dirty = false;
            }
        }
        if created {
            knight_win.raise(); // the knight stays in front of new props
        }
    }

    /// Hide everything on monitors running a fullscreen game (like `Benches::hide_on`).
    pub fn hide_on(&mut self, game_monitors: &[RectI]) {
        self.game_monitors = game_monitors.to_vec();
        let held = self.held.map(|h| h.0);
        let on_game = |s: &Shown| held != Some(s.id) && game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1));
        self.props.retain(|p| !on_game(&p.shown));
        self.shown.retain(|s| !on_game(s));
    }

    /// Mouse events on the finds' windows since the last call: (find, a bench?, event) (the
    /// user's hand, `app/drag.rs`). Only the art's opaque pixels catch the mouse.
    pub fn take_mouse(&self) -> Vec<(FindId, bool, MouseEvent)> {
        self.props.iter().flat_map(|p| {
            let bench = matches!(p.shown.kind, FindKind::Bench { .. });
            p.win.take_mouse().into_iter().map(move |e| (p.shown.id, bench, e))
        })
        .collect()
    }

    /// Where find `id` stands (or hangs in the user's hand).
    pub fn foot(&self, id: FindId) -> Option<Vec2> {
        match self.held {
            Some((h, at)) if h == id => Some(at),
            _ => self.props.iter().find(|p| p.shown.id == id).map(|p| p.shown.pos),
        }
    }

    /// Half the width of find `id` on screen (px).
    pub fn half_width(&self, id: FindId, benches: &Benches) -> f32 {
        let kind = self.world.find(id).map_or(FindKind::Deposit, |f| f.kind);
        0.5 * footprint(self.scale, benches, [self.deposit_box, self.grub.size()], kind).w
    }

    /// The user picked find `id` up (the brain loses sight of it until it lands).
    pub fn pick_up(&mut self, id: FindId) -> bool {
        let Some(at) = self.foot(id) else { return false };
        self.held = Some((id, at));
        true
    }

    /// Carried / falling: its foot is at `foot`.
    pub fn hold(&mut self, foot: Vec2) {
        if let Some((_, at)) = self.held.as_mut() {
            *at = foot;
        }
    }

    /// Find `id` landed at `at` on window `owner`'s edge (`None`: the floor): anchored there
    /// (see [`World::move_find`]), its visits and hits kept, saved. Returns the app whose
    /// room it is in now.
    pub fn set_down(&mut self, id: FindId, at: Vec2, owner: Option<WindowId>, (desk, terrain): (&DesktopSnapshot, &Terrain)) -> Option<String> {
        if self.held.is_some_and(|h| h.0 == id) {
            self.held = None;
        }
        let spot = match owner {
            Some(w) => FindSpot::Window { id: w, at },
            None => FindSpot::Floor { at },
        };
        let moved = self.world.move_find(id, spot, desk, terrain, &self.rules);
        if moved {
            self.save();
            // Its prop shows there right away (a fresh line resolves once settled).
            if let Some(p) = self.props.iter_mut().find(|p| p.shown.id == id) {
                p.shown.pos = at;
                p.dirty = true;
            }
        }
        moved.then(|| self.world.room_of(id).unwrap_or("?").to_string())
    }

    /// How long each surface has lasted, and the clock it is measured on (shinies use the
    /// same rule).
    pub fn ages(&self) -> (&LedgeAges, f64) {
        (&self.ages, self.clock)
    }

    /// Prop windows (call before raising the knight).
    /// Has the ledge at this footing (same owner and height, overlapping) been on screen
    /// long enough to stand a visitor on ([`skong_core::lasting::SETTLED_SECS`])? Floors
    /// (no owner) always have.
    pub fn settled(&self, terrain: &Terrain, owner: Option<WindowId>, y: f32, x0: f32, x1: f32) -> bool {
        owner.is_none()
            || terrain
                .ledges
                .iter()
                .filter(|l| l.owner == owner && (l.y - y).abs() <= 1.0 && l.x0 < x1 && x0 < l.x1)
                .any(|l| self.ages.settled(terrain, l, self.clock))
    }

    /// Has `l` been on screen long enough to carry something ([`skong_core::lasting::SETTLED_SECS`])?
    pub fn ledge_settled(&self, terrain: &Terrain, l: &skong_core::terrain::Ledge) -> bool {
        self.ages.settled(terrain, l, self.clock)
    }

    pub fn raise_all(&self) {
        for p in &self.props {
            p.win.raise();
        }
    }

    /// Found benches' standing spots (they count as benches for the brain).
    pub fn bench_spots(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.shown.iter().filter(|s| matches!(s.kind, FindKind::Bench { .. }) && !self.is_held(s.id)).map(|s| s.pos)
    }

    fn is_held(&self, id: FindId) -> bool {
        self.held.is_some_and(|h| h.0 == id)
    }

    /// Where the knight would sit right now on a found bench.
    pub fn seat_here(&self, ch: &Character, benches: &Benches) -> Option<f32> {
        let sup = ch.support.as_ref().filter(|_| ch.is_grounded())?;
        self.shown.iter().filter(|s| !self.is_held(s.id) && (s.pos.y - sup.y).abs() < 1.0 && (s.pos.x - ch.pos.x).abs() < 6.0).find_map(|s| match s.kind {
            FindKind::Bench { style } => {
                let h = benches.art(style).map_or(0.0, |i| i.height as f32);
                Some(s.pos.y - h * benches.styles.get(style).map_or(0.45, |st| st.seat))
            }
            FindKind::Deposit | FindKind::Grub { .. } => None,
        })
    }

    /// What the brain knows about the finds on screen.
    pub fn places(&self) -> Vec<KnownPlace> {
        // Only finds that still exist (a broken rock lingers on screen a moment; the knight
        // must not keep swinging at it).
        let live: Vec<Shown> = self.shown.iter().filter(|s| self.world.find(s.id).is_some() && !self.is_held(s.id)).copied().collect();
        discovery::known_places(
            &live,
            |id| self.reveals.iter().any(|r| r.0 == id),
            |f| {
                let reach = self.stand_off(f.id);
                match f.kind {
                    // Mine it from whichever side has room.
                    FindKind::Deposit | FindKind::Grub { .. } if f.pos.x - reach - 20.0 >= f.span.0 => f.pos - Vec2::new(reach, 0.0),
                    FindKind::Deposit | FindKind::Grub { .. } => f.pos + Vec2::new(reach, 0.0),
                    FindKind::Bench { .. } => f.pos,
                }
            },
        )
    }

    /// How far from deposit `id`'s center the knight stands to mine it (its rock's width).
    fn stand_off(&self, id: FindId) -> f32 {
        if self.world.find(id).is_some_and(|f| matches!(f.kind, FindKind::Grub { .. })) {
            return stand_off(self.grub.size().0, self.scale);
        }
        let w = self
            .props
            .iter()
            .find(|p| p.shown.id == id)
            .and_then(|p| {
                let rocks = rocks_for(&self.rocks, p.region).1;
                rocks.get((id % rocks.len().max(1) as u64) as usize)
            })
            .map_or(self.deposit.width as f32, |r| r.idle.0.width as f32);
        stand_off(w, self.scale)
    }

    /// The knight explored a never-visited window: roll / reveal its room.
    pub fn explored(&mut self, id: WindowId, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &Occupancy) {
        let (scale, deposit) = (self.scale, [self.deposit_box, self.grub.size()]);
        let fp = |k: FindKind| footprint(scale, benches, deposit, k);
        let site = Site { terrain, ages: &self.ages, now: self.clock, min_age: LASTING_SECS, occupancy: occ, footprint: &fp };
        let found = self.world.explored(id, desk, &site, &self.rules);
        self.revealed(found, desk, id);
    }

    fn revealed(&mut self, found: Vec<FindId>, desk: &DesktopSnapshot, id: WindowId) {
        let app = desk.window(id).map_or("?", |w| w.process.as_str());
        for f in &found {
            if let Some(find) = self.world.find(*f) {
                log::info!("discovered a {} in {app}", find.kind.label());
            }
            self.reveals.push((*f, self.clock));
        }
        if !found.is_empty() {
            self.save();
        }
    }

    /// The style of found bench `id` (`None`: not a bench, or gone).
    pub fn bench_style(&self, id: FindId) -> Option<usize> {
        match self.world.find(id)?.kind {
            FindKind::Bench { style } => Some(style),
            FindKind::Deposit | FindKind::Grub { .. } => None,
        }
    }

    /// Bench styles that can turn up in windows (rolled at all, with art to show).
    pub fn bench_style_count(&self, benches: &Benches) -> usize {
        self.rules.bench_styles.iter().enumerate().filter(|&(i, w)| *w > 0.0 && benches.available(i)).count()
    }

    pub fn used(&mut self, id: FindId) {
        self.world.used(id);
        self.save();
    }

    /// A slash at deposit `id` (only counts when the knight is next to it).
    /// Returns the Geo it drops when it breaks.
    pub fn struck(&mut self, id: FindId, knight: &Character, fx: &mut Fx) -> Option<DepositDrop> {
        let near = self.stand_off(id) + (160.0 - DEPOSIT_STAND) * self.scale;
        let p = self.props.iter_mut().find(|p| p.shown.id == id)?;
        if (p.shown.pos.x - knight.pos.x).abs() > near || (p.shown.pos.y - knight.pos.y).abs() > 8.0 {
            return None;
        }
        p.shake = 0.25;
        let (at, region) = (p.shown.pos, p.region);
        if let FindKind::Grub { .. } = p.shown.kind {
            // One blow: the glass gives, and the grub (or what was in its place) is out.
            let owner = Some(p.shown.window).filter(|&w| w != discovery::FLOOR_WINDOW);
            let right = knight.pos.x > at.x;
            if let Some(mimic) = self.world.free_grub(id, now_unix(), &self.rules) {
                let (_, h) = self.grub.size();
                self.freed.push(FreedJar { foot: at, owner, grub: self.grub.grub_at(at), centre: at - Vec2::new(0.0, h * 0.4), mimic, right });
                self.props.retain(|p| p.shown.id != id);
                self.shown.retain(|s| s.id != id);
                log::info!("grub jar broken at ({:.0},{:.0}): {}", at.x, at.y, if mimic { "a Grub Mimic!" } else { "a grub, free" });
                self.save();
            }
            return None;
        }
        let drop = self.world.strike(id, at, now_unix(), &self.rules);
        if let Some(d) = drop {
            self.mined.push(region);
            fx.spawn(REVEAL_FX, at, knight.pos.x < at.x);
            log::info!("geo deposit broken at ({:.0},{:.0}): {} geo", d.at.x, d.at.y, d.geo);
        }
        self.save();
        drop
    }

    /// Debug / gift: put a find of kind `k` (0 bench, 1 geo deposit) into window `id`, on any
    /// surface of it (lasting or not).
    pub fn force(&mut self, k: u8, id: WindowId, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &Occupancy) {
        let kind = match k {
            0 => {
                let avail: Vec<usize> = (0..benches.styles.len()).filter(|&i| benches.available(i)).collect();
                FindKind::Bench { style: avail.get(now_unix() as usize % avail.len().max(1)).copied().unwrap_or(0) }
            }
            _ => FindKind::Deposit,
        };
        let (scale, deposit) = (self.scale, [self.deposit_box, self.grub.size()]);
        let fp = |k: FindKind| footprint(scale, benches, deposit, k);
        let site = Site { terrain, ages: &self.ages, now: self.clock, min_age: 0.0, occupancy: occ, footprint: &fp };
        match self.world.force(kind, id, desk, &site) {
            Some(f) => self.revealed(vec![f], desk, id),
            None => log::warn!("discover: no room for a {} on that window", kind.label()),
        }
    }

    /// Test panel: a Geo deposit right at `at` on window `id`'s ledge. Returns the region
    /// its rocks come from (it follows the window's app).
    pub fn force_deposit_at(&mut self, id: WindowId, at: Vec2, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &Occupancy) -> Result<&'static str, &'static str> {
        let (scale, deposit) = (self.scale, [self.deposit_box, self.grub.size()]);
        let fp = |k: FindKind| footprint(scale, benches, deposit, k);
        let site = Site { terrain, ages: &self.ages, now: self.clock, min_age: 0.0, occupancy: occ, footprint: &fp };
        let f = self.world.force_at(FindKind::Deposit, id, at, desk, &site)?;
        self.revealed(vec![f], desk, id);
        Ok(REGIONS[rocks_for(&self.rocks, window_region(desk, id)).0].name)
    }

    /// Test panel: a grub jar (`mimic`: holding a Grub Mimic) right at `at` on window
    /// `id`'s ledge.
    pub fn force_grub_at(&mut self, id: WindowId, at: Vec2, mimic: bool, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &Occupancy) -> Result<(), &'static str> {
        let (scale, deposit) = (self.scale, [self.deposit_box, self.grub.size()]);
        let fp = |k: FindKind| footprint(scale, benches, deposit, k);
        let site = Site { terrain, ages: &self.ages, now: self.clock, min_age: 0.0, occupancy: occ, footprint: &fp };
        let f = self.world.force_at(FindKind::Grub { mimic }, id, at, desk, &site)?;
        self.revealed(vec![f], desk, id);
        Ok(())
    }

    /// A grub jar's half width (px), for placement.
    pub fn jar_half_width(&self) -> f32 {
        0.5 * self.grub.size().0
    }

    /// A deposit's half width (px), for placement.
    pub fn deposit_half_width(&self) -> f32 {
        0.5 * self.deposit_box.0
    }

    pub fn forget(&mut self) {
        self.world.clear();
        self.held = None;
        self.props.clear();
        self.shown.clear();
        self.save();
        log::info!("discoveries forgotten");
    }

    fn save(&self) {
        let Some(p) = &self.path else { return };
        if let Err(e) = save_world(p, &self.world) {
            log::warn!("could not save {}: {e}", p.display());
        }
    }
}

/// The world in `path`; missing → `fresh()`, malformed → kept as `world.json.bak`, `fresh()`.
fn load_world(path: &Path, fresh: impl FnOnce() -> World) -> World {
    skong_core::persist::load_or_fresh(path, World::from_json, fresh)
}

/// Save atomically (temp file, synced, renamed): a crash mid-save keeps the old world whole.
fn save_world(path: &Path, world: &World) -> std::io::Result<()> {
    skong_core::persist::write_atomic(path, world.to_json().as_bytes())
}

// --- drawing ------------------------------------------------------------------------

/// The deposit frame `p` shows at `clock`: (art region, rock shape, gleam frame).
fn deposit_frame(p: &Prop, rocks: &[Vec<RockArt>], clock: f64) -> (usize, usize, Option<usize>) {
    let (region, rocks) = rocks_for(rocks, p.region);
    let (shape, gleam) = rock_frame(p.shown.id, rocks, clock);
    (region, shape, gleam)
}

fn draw(p: &mut Prop, deposit: &Image, all_rocks: &[Vec<RockArt>], grub: &GrubArt, benches: &Benches, s: f32, clock: f64) -> anyhow::Result<()> {
    let alpha = p.revealed.map_or(1.0, |t| ((clock - t) as f32 / REVEAL_SECS).clamp(0.0, 1.0));
    let pos = p.shown.pos;
    let rocks = rocks_for(all_rocks, p.region).1;
    match p.shown.kind {
        FindKind::Bench { style } => {
            let Some(img) = benches.art(style) else { return Ok(()) };
            let img = faded(img, alpha);
            p.win.present(&img, (pos.x - img.width as f32 * 0.5).round() as i32, (pos.y - img.height as f32).round() as i32)
        }
        FindKind::Deposit if !rocks.is_empty() => {
            // The game's Geo Rock (the app's region); every frame placed by its anchor
            // relative to the idle pose, whose bottom center sits (sunk a little) on the ledge.
            p.frame = deposit_frame(p, all_rocks, clock);
            let (_, variant, gleam) = p.frame;
            let rock = &rocks[variant];
            let (idle, idle_anchor) = (&rock.idle.0, rock.idle.1);
            let (img, anchor) = gleam.and_then(|i| rock.gleam.get(i)).map_or((idle, idle_anchor), |(i, a)| (i, *a));
            let dx = if p.shake > 0.0 { (p.shake * 90.0).sin() * 3.0 * s } else { 0.0 };
            let ax = pos.x - idle.width as f32 * 0.5 + idle_anchor[0] + dx;
            let ay = pos.y + 2.0 * s - idle.height as f32 + idle_anchor[1];
            let img = faded(img, alpha);
            p.win.present(&img, (ax - anchor[0]).round() as i32, (ay - anchor[1]).round() as i32)
        }
        FindKind::Grub { .. } => {
            let (crying, i, right) = p.frame;
            let img = faded(&grub.compose((crying == 1, i), right == Some(1)), alpha);
            let dx = if p.shake > 0.0 { (p.shake * 90.0).sin() * 3.0 * s } else { 0.0 };
            // Its bulb rests on the ledge, sunk a hair like something that was always there.
            p.win.present(&img, (pos.x - img.width as f32 * 0.5 + dx).round() as i32, (pos.y - img.height as f32 + 2.0 * s).round() as i32)
        }
        FindKind::Deposit => {
            let img = faded(deposit, alpha);
            let dx = if p.shake > 0.0 { (p.shake * 90.0).sin() * 3.0 * s } else { 0.0 };
            // Sunk a little into its ledge, like a rock that was always there.
            p.win.present(&img, (pos.x - img.width as f32 * 0.5 + dx).round() as i32, (pos.y - img.height as f32 + 3.0 * s).round() as i32)
        }
    }
}

/// The deposit: a rock with geo glinting in it.
fn deposit_art(rock: Option<&Image>, w: u32, h: u32, s: f32) -> Image {
    let mut img = match rock {
        Some(r) => fit(r, w, h),
        None => {
            let mut i = Image::new(w, h);
            fill(&mut i, 0, (h / 3) as i32, w as i32, h as i32, |_, _| premul(40, 44, 60, 255));
            i
        }
    };
    let px = |v: f32| (v * s).round().max(1.0) as i32;
    // Geo: small gold lozenges with a pale glint.
    for &(fx, fy, r) in &[(0.32, 0.55, 5.0), (0.55, 0.42, 6.0), (0.7, 0.66, 4.5), (0.45, 0.75, 4.0), (0.22, 0.75, 3.5)] {
        let (cx, cy, r) = ((fx * w as f32) as i32, (fy * h as f32) as i32, px(r));
        for dy in -r..=r {
            let half = r - dy.abs();
            for dx in -half..=half {
                let shade = if dx + dy < 0 { premul(250, 222, 120, 255) } else { premul(205, 160, 60, 255) };
                put(&mut img, cx + dx, cy + dy, shade);
            }
        }
        put(&mut img, cx - r / 3, cy - r / 3, premul(255, 250, 220, 255));
    }
    img
}

fn put(img: &mut Image, x: i32, y: i32, c: u32) {
    if x >= 0 && y >= 0 && (x as u32) < img.width && (y as u32) < img.height {
        let i = (y as u32 * img.width + x as u32) as usize;
        img.px[i] = over(c, img.px[i]);
    }
}

/// Source-over fill of [x0,x1)×[y0,y1) with a per-pixel color.
fn fill(img: &mut Image, x0: i32, y0: i32, x1: i32, y1: i32, color: impl Fn(i32, i32) -> u32) {
    for y in y0.max(0)..y1.min(img.height as i32) {
        for x in x0.max(0)..x1.min(img.width as i32) {
            let i = (y as u32 * img.width + x as u32) as usize;
            img.px[i] = over(color(x, y), img.px[i]);
        }
    }
}

/// Premultiplied image times `a`.
fn faded(img: &Image, a: f32) -> Image {
    if a >= 0.999 {
        return img.clone();
    }
    let k = (a.clamp(0.0, 1.0) * 256.0) as u32;
    let px = img.px.iter().map(|&p| {
        let ch = |sh: u32| ((((p >> sh) & 0xff) * k) >> 8) << sh;
        ch(24) | ch(16) | ch(8) | ch(0)
    });
    Image { width: img.width, height: img.height, px: px.collect() }
}

/// Resize to exactly `w`×`h`: area-average down to about the size, then bilinear.
fn fit(src: &Image, w: u32, h: u32) -> Image {
    let f = (w as f32 / src.width.max(1) as f32).max(h as f32 / src.height.max(1) as f32);
    let base = if f < 0.9 { src.scaled(f) } else { src.clone() };
    let mut out = Image::new(w, h);
    let (sx, sy) = (base.width as f32 / w as f32, base.height as f32 / h as f32);
    for y in 0..h {
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (base.width - 1) as f32);
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (base.height - 1) as f32);
            let (x0, y0) = (fx as u32, fy as u32);
            let (x1, y1) = ((x0 + 1).min(base.width - 1), (y0 + 1).min(base.height - 1));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let at = |xx: u32, yy: u32| base.px[(yy * base.width + xx) as usize];
            let mut c = 0u32;
            for sh in [24u32, 16, 8, 0] {
                let g = |p: u32| ((p >> sh) & 0xff) as f32;
                let top = g(at(x0, y0)) * (1.0 - tx) + g(at(x1, y0)) * tx;
                let bot = g(at(x0, y1)) * (1.0 - tx) + g(at(x1, y1)) * tx;
                c |= ((top * (1.0 - ty) + bot * ty).round() as u32).min(255) << sh;
            }
            out.px[(y * w + x) as usize] = c;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rock(w: u32, h: u32) -> RockArt {
        RockArt { idle: (Image::new(w, h), [w as f32 * 0.5, h as f32]), gleam: vec![(Image::new(w, h), [0.0, 0.0]); 4], gleam_fps: 15.0 }
    }

    #[test]
    fn the_world_save_is_atomic_and_a_malformed_one_is_kept() {
        let dir = std::env::temp_dir().join(format!("skong-world-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("world.json");
        let world = World::new(42);
        save_world(&path, &world).expect("save");
        save_world(&path, &world).expect("save again");
        assert_eq!(load_world(&path, || World::new(1)), world);
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1, "no temporary file left");
        std::fs::write(&path, "{\"seed\": 4").expect("truncate");
        assert_eq!(load_world(&path, || World::new(1)).seed, 1, "malformed: fresh");
        assert_eq!(std::fs::read_to_string(dir.join("world.json.bak")).expect("kept"), "{\"seed\": 4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_app_keeps_its_region() {
        for app in ["code", "chrome", "explorer", "notepad", "spotify"] {
            assert_eq!(region_of(app), region_of(app));
            assert!(region_of(app) < REGIONS.len());
        }
        // Pinned: a stable hash (not std's, which may change between Rust versions), so an
        // app's rocks look the same after an update.
        assert_eq!(region_of("code"), 8);
        assert_eq!(region_of("chrome"), 3);
    }

    #[test]
    fn apps_spread_over_the_regions() {
        let apps = [
            "code", "chrome", "firefox", "msedge", "explorer", "notepad", "spotify", "discord", "slack", "teams", "outlook", "winword", "excel", "powerpnt", "steam", "obs64", "windowsterminal",
            "devenv", "rider64", "idea64", "blender", "gimp", "vlc", "zoom", "telegram", "whatsapp", "notion", "obsidian", "figma", "postman",
        ];
        let mut used: Vec<usize> = apps.iter().map(|a| region_of(a)).collect();
        used.sort_unstable();
        used.dedup();
        assert!(used.len() >= REGIONS.len() / 2, "only {} regions used: {used:?}", used.len());
    }

    #[test]
    fn regions_name_known_libraries_and_shapes() {
        assert_eq!(REGIONS[FALLBACK_REGION].lib, "Coin_Rocks");
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = skong_core::assets::SpriteBank::load_metadata(&data).unwrap();
        for r in REGIONS {
            let lib = skong_core::anim::ClipLibrary::load(&data.join(format!("animations/{}.json", r.lib)), &bank).unwrap();
            for n in r.shapes {
                assert!(lib.id(&format!("Idle {n}")).is_some(), "{}: no Idle {n}", r.name);
                assert!(lib.id(&format!("Gleam {n}")).is_some(), "{}: no Gleam {n}", r.name);
            }
            assert!(!shape_keys(&lib, r).is_empty(), "{}: no sprites", r.name);
        }
    }

    #[test]
    fn missing_art_falls_back_to_the_crossroads_rock() {
        let mut rocks: Vec<Vec<RockArt>> = (0..REGIONS.len()).map(|_| Vec::new()).collect();
        rocks[0] = vec![rock(60, 50), rock(66, 52)];
        rocks[3] = vec![rock(80, 70)];
        assert_eq!(rocks_for(&rocks, 3).0, 3);
        assert_eq!(rocks_for(&rocks, 3).1.len(), 1);
        // Region 5's art is missing: the crossroads rock stands in.
        assert_eq!(rocks_for(&rocks, 5).0, FALLBACK_REGION);
        assert_eq!(rocks_for(&rocks, 5).1.len(), 2);
        // Out of range (an older table) too.
        assert_eq!(rocks_for(&rocks, 99).0, FALLBACK_REGION);
        // Nothing loaded at all: no rock (the plain deposit is drawn).
        let none: Vec<Vec<RockArt>> = (0..REGIONS.len()).map(|_| Vec::new()).collect();
        assert!(rocks_for(&none, 3).1.is_empty());
        assert_eq!(art_region(|_| false, 3), None);
        assert_eq!(art_region(|r| r == 0, 3), Some(0));
    }

    #[test]
    fn bigger_rocks_stand_the_knight_further_out() {
        let s = 0.6;
        // The crossroads rock keeps the old spot ...
        assert_eq!(stand_off(1.6 * 64.0 * s, s), DEPOSIT_STAND * s);
        // ... a wide one moves him out so he clears its edge.
        let w = ROCK_MAX_UNITS[0] * 64.0 * s;
        assert!(stand_off(w, s) >= w * 0.5 + STAND_CLEAR * s);
        // The footprint leaves room to stand on either side of the widest rock.
        let benches_free = |w: f32| 2.0 * (stand_off(w, s) + 24.0 * s);
        assert!(benches_free(w) >= w + 2.0 * STAND_CLEAR * s);
    }

    #[test]
    fn gleams_play_at_the_clip_speed() {
        let rocks = vec![rock(60, 50)];
        // Somewhere in every period the gleam runs through all 4 frames, then rests.
        let frames: Vec<Option<usize>> = (0..2000).map(|i| rock_frame(1, &rocks, i as f64 / 100.0).1).collect();
        assert!(frames.contains(&Some(3)));
        assert!(frames.iter().filter(|f| f.is_some()).count() < frames.len() / 5);
    }
}
