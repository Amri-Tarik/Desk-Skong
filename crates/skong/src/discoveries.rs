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
use skong_core::discovery::{self, DepositDrop, FindId, FindKind, Footprint, LASTING_SECS, Rules, Shown, Site, World};
use skong_core::geom::{RectI, Vec2};
use skong_core::lasting::LedgeAges;
use skong_core::occupancy::Occupancy;
use skong_core::terrain::Terrain;
use skong_win::sprite_window::SpriteWindow;

use crate::benches::Benches;
use crate::fx::Fx;

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
/// Where the knight stands to mine a deposit (from its center).
const DEPOSIT_STAND: f32 = 82.0;

/// Seconds between a deposit's gleams (the game's own shimmer), varied per rock.
const GLEAM_EVERY: f32 = 6.0;
const GLEAM_FPS: f32 = 12.0;

/// One shape of the game's Geo Rock ("Coin Rocks": `Idle n` / `Gleam n`), at the current
/// scale. Frames carry their anchors so the gleam lines up with the idle pose.
struct RockArt {
    idle: (Image, [f32; 2]),
    gleam: Vec<(Image, [f32; 2])>,
}

/// Rasterize both Geo Rock shapes from the exported "Coin Rocks" library.
fn rock_art(data: &Path, vanilla: &Path, px_per_unit: f32) -> anyhow::Result<Vec<RockArt>> {
    use skong_core::anim::ClipLibrary;
    use skong_core::assets::{Skin, SpriteBank};
    let skin = Skin { name: "Vanilla".into(), dir: vanilla.to_path_buf() };
    let mut bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Coin_Rocks.json"), &bank)?;
    bank.rasterize_skin(&skin, None, &lib.sprite_keys(), px_per_unit)?;
    let frames = |name: &str| -> Vec<(Image, [f32; 2])> {
        lib.id(name)
            .map(|id| lib.get(id).frames.iter().filter_map(|f| f.sprite.and_then(|k| bank.frame(k))).map(|f| (f.image.clone(), f.anchor)).collect())
            .unwrap_or_default()
    };
    let rocks: Vec<RockArt> = (1..=2)
        .filter_map(|n| {
            let idle = frames(&format!("Idle {n}")).into_iter().next()?;
            Some(RockArt { idle, gleam: frames(&format!("Gleam {n}")) })
        })
        .collect();
    anyhow::ensure!(!rocks.is_empty(), "no Geo Rock clips");
    Ok(rocks)
}

/// Which rock shape a find uses, and the frame to show at `clock` (`None` = idle).
fn rock_frame(id: FindId, rocks: &[RockArt], clock: f32) -> (usize, Option<usize>) {
    let variant = (id % rocks.len().max(1) as u64) as usize;
    let Some(rock) = rocks.get(variant) else { return (0, None) };
    let period = GLEAM_EVERY + (id % 5) as f32 * 0.7;
    let t = (clock + (id % 97) as f32 * 0.37) % period;
    let f = (t * GLEAM_FPS) as usize;
    (variant, (f < rock.gleam.len()).then_some(f))
}

#[derive(Deserialize)]
#[serde(default)]
struct ArtNames {
    deposit_rock: String,
}

impl Default for ArtNames {
    fn default() -> Self {
        Self { deposit_rock: "mines_small_rock".into() }
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
    revealed: Option<f32>,
    /// Clock time it last resolved on its window.
    seen_at: f32,
    /// Seconds of hit shake left.
    shake: f32,
    drawn_at: f32,
    dirty: bool,
    /// Deposit frame last drawn (rock shape, gleam frame).
    frame: (usize, Option<usize>),
}

pub struct Discoveries {
    world: World,
    rules: Rules,
    path: Option<PathBuf>,
    deposit_src: Option<Image>,
    /// The deposit at the current scale (footprint; the drawn art when no Geo Rock).
    deposit: Image,
    /// The game's Geo Rock at the current scale (empty: fall back to `deposit`).
    rocks: Vec<RockArt>,
    /// Exported metadata (`data/hk`) and the vanilla skin, to re-rasterize on rescale.
    data_dir: PathBuf,
    vanilla_dir: Option<PathBuf>,
    scale: f32,
    props: Vec<Prop>,
    shown: Vec<Shown>,
    /// Recently revealed finds with their reveal time.
    reveals: Vec<(FindId, f32)>,
    clock: f32,
    ages: LedgeAges,
    /// Terrain generation and clock of the last [`LedgeAges::observe`].
    observed: (u64, f32),
    /// The window the knight stood on last update (arrivals place pending finds).
    knight_on: Option<WindowId>,
    /// Monitors running a fullscreen game: nothing shows there.
    game_monitors: Vec<RectI>,
    /// Clock of the last check for broken deposits due to turn up elsewhere.
    relocate_checked: f32,
    pub capturable: bool,
}

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

/// Footprint of a find at `scale`: the bench art / the deposit rock (plus room to stand
/// beside it to mine it).
fn footprint(scale: f32, benches: &Benches, deposit: &Image, kind: FindKind) -> Footprint {
    match kind {
        FindKind::Bench { style } => {
            let (w, h) = benches.art(style).map_or((120.0 * scale, 60.0 * scale), |i| (i.width as f32, i.height as f32));
            Footprint { w, h, room: w }
        }
        FindKind::Deposit => {
            let (w, h) = (deposit.width as f32, deposit.height as f32);
            Footprint { w, h, room: 2.0 * (DEPOSIT_STAND + 24.0) * scale }
        }
    }
}

impl Discoveries {
    pub fn load(data: &Path, props_json: &Path, vanilla: Option<&Path>, scale: f32) -> Discoveries {
        let names = read_json::<PropsFile>(props_json).map(|p| p.discoveries).unwrap_or_default();
        let deposit_src = vanilla.and_then(|v| load_sprite(&v.join("props"), &names.deposit_rock));
        let path = world_path();
        let world = match path.as_deref().map(std::fs::read_to_string) {
            Some(Ok(s)) => World::from_json(&s).unwrap_or_else(|e| {
                log::warn!("ignoring malformed world.json: {e}");
                World::new(now_unix() ^ 0x5eed)
            }),
            _ => World::new(now_unix() ^ 0x5eed),
        };
        let found: usize = world.rooms.values().map(|r| r.finds.len()).sum();
        log::info!("discoveries: {} rooms known, {found} things found", world.rooms.len());
        let mut d = Discoveries {
            world,
            rules: Rules::default(),
            path,
            deposit_src,
            deposit: Image::default(),
            rocks: Vec::new(),
            data_dir: data.to_path_buf(),
            vanilla_dir: vanilla.map(Path::to_path_buf),
            scale: 0.0,
            props: Vec::new(),
            shown: Vec::new(),
            reveals: Vec::new(),
            clock: 0.0,
            ages: LedgeAges::default(),
            observed: (u64::MAX, f32::NEG_INFINITY),
            knight_on: None,
            game_monitors: Vec::new(),
            relocate_checked: 0.0,
            capturable: false,
        };
        d.rescale(scale);
        d
    }

    pub fn rescale(&mut self, scale: f32) {
        self.scale = scale;
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
        if let Some(r) = self.rocks.first() {
            self.deposit = r.idle.0.clone();
        }
        for p in &mut self.props {
            p.dirty = true;
        }
    }

    /// Resolve what is on screen (lasting surfaces, collisions with other claims already in
    /// `occ`), create / move / animate the prop windows, and place pending finds when the
    /// knight comes onto their window.
    pub fn update(&mut self, dt: f32, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &mut Occupancy, fx: &mut Fx, (knight, knight_win): (&Character, &SpriteWindow)) {
        self.clock += dt;
        let clock = self.clock;
        if terrain.generation != self.observed.0 || clock - self.observed.1 >= OBSERVE_EVERY {
            self.ages.observe(terrain, clock);
            self.observed = (terrain.generation, clock);
        }
        let (scale, deposit) = (self.scale, self.deposit.clone());
        let fp = |k: FindKind| footprint(scale, benches, &deposit, k);

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
            let placed = self.world.relocate_due(desk, &site, now_unix());
            for (fid, wid) in placed {
                log::info!("discoveries: a new geo deposit turned up (window {wid})");
                self.revealed(vec![fid], desk, wid);
                self.save();
            }
        }

        let mut raw = self.world.shown(desk, terrain, now_unix(), &self.rules, &fp);
        // Really on its ledge this tick (the grace copies below must not count as seen, or
        // the grace never ends: broken rocks stayed, finds floated where a ledge had been).
        let seen_now: Vec<FindId> = raw.iter().map(|s| s.id).collect();
        // Content ledges flicker (re-detection): a find keeps its last spot a moment while
        // its window is open.
        for p in &self.props {
            if !raw.iter().any(|s| s.id == p.shown.id) && desk.window(p.shown.window).is_some() && clock - p.seen_at < HIDE_GRACE {
                raw.push(p.shown);
            }
        }
        raw.retain(|s| !self.game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1)));
        let shown = discovery::settle(raw, occ, &fp);

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
                    p.shown = *s;
                    if fresh_seen {
                        p.seen_at = clock;
                    }
                }
                None => {
                    let Ok(win) = SpriteWindow::new() else { continue };
                    win.set_capturable(self.capturable);
                    win.set_click_through(true);
                    log::debug!("discoveries: {} #{} shown at ({:.0},{:.0})", s.kind.label(), s.id, s.pos.x, s.pos.y);
                    let revealed = self.reveals.iter().find(|r| r.0 == s.id).map(|r| r.1);
                    if revealed.is_some() {
                        fx.spawn(REVEAL_FX, s.pos, knight.pos.x < s.pos.x);
                    }
                    self.props.push(Prop { shown: *s, win, revealed, shake: 0.0, seen_at: clock, drawn_at: f32::NEG_INFINITY, dirty: true, frame: (0, None) });
                    created = true;
                }
            }
        }
        self.shown = shown;
        self.reveals.retain(|r| clock - r.1 < FRESH_SECS);

        for p in &mut self.props {
            p.shake = (p.shake - dt).max(0.0);
            let revealing = p.revealed.is_some_and(|t| clock - t <= REVEAL_SECS + 0.1);
            // Geo rocks gleam now and then: redraw when the frame changes.
            if p.shown.kind == FindKind::Deposit && !self.rocks.is_empty() && rock_frame(p.shown.id, &self.rocks, clock) != p.frame {
                p.dirty = true;
            }
            if p.dirty || ((revealing || p.shake > 0.0) && clock - p.drawn_at >= 1.0 / 30.0) {
                if let Err(e) = draw(p, &self.deposit, &self.rocks, benches, scale, clock) {
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
        let on_game = |s: &Shown| game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1));
        self.props.retain(|p| !on_game(&p.shown));
        self.shown.retain(|s| !on_game(s));
    }

    /// How long each surface has lasted, and the clock it is measured on (shinies use the
    /// same rule).
    pub fn ages(&self) -> (&LedgeAges, f32) {
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

    pub fn raise_all(&self) {
        for p in &self.props {
            p.win.raise();
        }
    }

    /// Found benches' standing spots (they count as benches for the brain).
    pub fn bench_spots(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.shown.iter().filter(|s| matches!(s.kind, FindKind::Bench { .. })).map(|s| s.pos)
    }

    /// Where the knight would sit right now on a found bench.
    pub fn seat_here(&self, ch: &Character, benches: &Benches) -> Option<f32> {
        let sup = ch.support.as_ref().filter(|_| ch.is_grounded())?;
        self.shown.iter().filter(|s| (s.pos.y - sup.y).abs() < 1.0 && (s.pos.x - ch.pos.x).abs() < 6.0).find_map(|s| match s.kind {
            FindKind::Bench { style } => {
                let h = benches.art(style).map_or(0.0, |i| i.height as f32);
                Some(s.pos.y - h * benches.styles.get(style).map_or(0.45, |st| st.seat))
            }
            FindKind::Deposit => None,
        })
    }

    /// What the brain knows about the finds on screen.
    pub fn places(&self) -> Vec<KnownPlace> {
        let reach = DEPOSIT_STAND * self.scale;
        discovery::known_places(
            &self.shown,
            |id| self.reveals.iter().any(|r| r.0 == id),
            |f| match f.kind {
                // Mine it from whichever side has room.
                FindKind::Deposit if f.pos.x - reach - 20.0 >= f.span.0 => f.pos - Vec2::new(reach, 0.0),
                FindKind::Deposit => f.pos + Vec2::new(reach, 0.0),
                FindKind::Bench { .. } => f.pos,
            },
        )
    }

    /// The knight explored a never-visited window: roll / reveal its room.
    pub fn explored(&mut self, id: WindowId, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, occ: &Occupancy) {
        let (scale, deposit) = (self.scale, &self.deposit);
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

    pub fn used(&mut self, id: FindId) {
        self.world.used(id);
        self.save();
    }

    /// A slash at deposit `id` (only counts when the knight is next to it).
    /// Returns the Geo it drops when it breaks.
    pub fn struck(&mut self, id: FindId, knight: &Character, fx: &mut Fx) -> Option<DepositDrop> {
        let p = self.props.iter_mut().find(|p| p.shown.id == id)?;
        if (p.shown.pos.x - knight.pos.x).abs() > 160.0 * self.scale || (p.shown.pos.y - knight.pos.y).abs() > 8.0 {
            return None;
        }
        p.shake = 0.25;
        let at = p.shown.pos;
        let drop = self.world.strike(id, at, now_unix(), &self.rules);
        if let Some(d) = drop {
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
        let (scale, deposit) = (self.scale, &self.deposit);
        let fp = |k: FindKind| footprint(scale, benches, deposit, k);
        let site = Site { terrain, ages: &self.ages, now: self.clock, min_age: 0.0, occupancy: occ, footprint: &fp };
        match self.world.force(kind, id, desk, &site) {
            Some(f) => self.revealed(vec![f], desk, id),
            None => log::warn!("discover: no room for a {} on that window", kind.label()),
        }
    }

    pub fn forget(&mut self) {
        self.world.clear();
        self.props.clear();
        self.shown.clear();
        self.save();
        log::info!("discoveries forgotten");
    }

    fn save(&self) {
        let Some(p) = &self.path else { return };
        let r = p.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(p, self.world.to_json()));
        if let Err(e) = r {
            log::warn!("could not save {}: {e}", p.display());
        }
    }
}

// --- drawing ------------------------------------------------------------------------

fn draw(p: &mut Prop, deposit: &Image, rocks: &[RockArt], benches: &Benches, s: f32, clock: f32) -> anyhow::Result<()> {
    let alpha = p.revealed.map_or(1.0, |t| ((clock - t) / REVEAL_SECS).clamp(0.0, 1.0));
    let pos = p.shown.pos;
    match p.shown.kind {
        FindKind::Bench { style } => {
            let Some(img) = benches.art(style) else { return Ok(()) };
            let img = faded(img, alpha);
            p.win.present(&img, (pos.x - img.width as f32 * 0.5).round() as i32, (pos.y - img.height as f32).round() as i32)
        }
        FindKind::Deposit if !rocks.is_empty() => {
            // The game's Geo Rock; every frame placed by its anchor relative to the idle
            // pose, whose bottom center sits (sunk a little) on the ledge.
            let (variant, gleam) = rock_frame(p.shown.id, rocks, clock);
            p.frame = (variant, gleam);
            let rock = &rocks[variant];
            let (idle, idle_anchor) = (&rock.idle.0, rock.idle.1);
            let (img, anchor) = gleam.and_then(|i| rock.gleam.get(i)).map_or((idle, idle_anchor), |(i, a)| (i, *a));
            let dx = if p.shake > 0.0 { (p.shake * 90.0).sin() * 3.0 * s } else { 0.0 };
            let ax = pos.x - idle.width as f32 * 0.5 + idle_anchor[0] + dx;
            let ay = pos.y + 2.0 * s - idle.height as f32 + idle_anchor[1];
            let img = faded(img, alpha);
            p.win.present(&img, (ax - anchor[0]).round() as i32, (ay - anchor[1]).round() as i32)
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
