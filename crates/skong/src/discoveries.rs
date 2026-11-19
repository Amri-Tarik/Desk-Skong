//! Things found in windows, on screen: the persistent [`World`] (`%APPDATA%\Desk-Skong\
//! world.json`), one small sprite window per find (a hot spring has two: the basin behind
//! the knight and the water + steam in front of him), reveal fades, deposit hits, and what
//! the brain needs to know ([`Discoveries::places`]).
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
use skong_core::discovery::{self, FindId, DepositDrop, FindKind, Rules, Shown, World};
use skong_core::geom::{RectI, Vec2};
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
/// Effect played where something is revealed (see `data/effects.json`).
const REVEAL_FX: &str = "reveal_dust";

// Sizes in px at scale 1 (the knight is ~88 px tall).
const SPRING_W: f32 = 300.0;
/// Water depth: the swimming pose's cloak line sits ~48 px below the sprite origin and
/// the legs reach ~88 px, so this much water hides them.
const SPRING_DEPTH: f32 = 44.0;
/// Room above the water for steam.
const STEAM_ROOM: f32 = 110.0;
/// The bather floats this much above the water line (his cloak spreads on the surface).
const SPRING_FLOAT: f32 = 7.0;
const SPRING_PAD: f32 = 26.0;
const DEPOSIT_W: f32 = 124.0;
/// Deposit footprint: the rock plus room to stand on either side.
const DEPOSIT_ROOM: f32 = 250.0;
/// Where the knight stands to mine a deposit (from its center).
const DEPOSIT_STAND: f32 = 82.0;

#[derive(Deserialize)]
#[serde(default)]
struct ArtNames {
    spring_back_rim: String,
    spring_front_rim: String,
    spring_glow: String,
    spring_steam: String,
    spring_rock: String,
    deposit_rock: String,
}

impl Default for ArtNames {
    fn default() -> Self {
        Self {
            spring_back_rim: "hot_spring_layers_0054_01-copy-24".into(),
            spring_front_rim: "hot_spring_layers_0055_01-copy-26".into(),
            spring_glow: "hot_spring_layers_0033_water_glow".into(),
            spring_steam: "hot_spring_layers_0029_Layer-10".into(),
            spring_rock: "hot_spring_layers_0019_Layer-16".into(),
            deposit_rock: "mines_small_rock".into(),
        }
    }
}

#[derive(Deserialize, Default)]
struct PropsFile {
    #[serde(default)]
    discoveries: ArtNames,
}

#[derive(Default)]
struct Art<T> {
    back_rim: Option<T>,
    front_rim: Option<T>,
    glow: Option<T>,
    steam: Option<T>,
    rock: Option<T>,
    deposit: Option<T>,
}

/// Art prepared at its drawn size for the current scale.
#[derive(Default)]
struct Ready {
    back_rim: Option<Image>,
    front_rim: Option<Image>,
    glow: Option<Image>,
    /// Steam puffs, small to large.
    steam: Vec<Image>,
    rock_l: Option<Image>,
    rock_r: Option<Image>,
    deposit: Option<Image>,
}

struct Prop {
    shown: Shown,
    back: SpriteWindow,
    front: Option<SpriteWindow>,
    /// Clock time its reveal started (None: it was always there).
    revealed: Option<f32>,
    /// Clock time it last resolved on its window.
    seen_at: f32,
    /// Seconds of hit shake left.
    shake: f32,
    drawn_at: f32,
    dirty: bool,
}

pub struct Discoveries {
    world: World,
    rules: Rules,
    path: Option<PathBuf>,
    src: Art<Image>,
    ready: Ready,
    scale: f32,
    props: Vec<Prop>,
    shown: Vec<Shown>,
    /// Recently revealed finds with their reveal time.
    reveals: Vec<(FindId, f32)>,
    clock: f32,
    /// The knight was inside a spring last update.
    in_spring: bool,
    /// Monitors running a fullscreen game: nothing shows there.
    game_monitors: Vec<RectI>,
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
    // Normalize to 64 px per game unit (some layers are authored at other densities).
    #[derive(Deserialize)]
    struct Meta {
        pixels_per_unit: f32,
    }
    let ppu = read_json::<Meta>(&dir.join(format!("{name}.json"))).map_or(64.0, |m| m.pixels_per_unit);
    Some(if (ppu - 64.0).abs() > 0.5 { img.scaled(64.0 / ppu) } else { img })
}

impl Discoveries {
    pub fn load(props_json: &Path, vanilla: Option<&Path>, scale: f32) -> Discoveries {
        let names = read_json::<PropsFile>(props_json).map(|p| p.discoveries).unwrap_or_default();
        let dir = vanilla.map(|v| v.join("props"));
        let get = |n: &str| dir.as_deref().and_then(|d| load_sprite(d, n));
        let src = Art {
            back_rim: get(&names.spring_back_rim),
            front_rim: get(&names.spring_front_rim),
            glow: get(&names.spring_glow),
            steam: get(&names.spring_steam),
            rock: get(&names.spring_rock),
            deposit: get(&names.deposit_rock),
        };
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
            src,
            ready: Ready::default(),
            scale: 0.0,
            props: Vec::new(),
            shown: Vec::new(),
            reveals: Vec::new(),
            clock: 0.0,
            in_spring: false,
            game_monitors: Vec::new(),
            capturable: false,
        };
        d.rescale(scale);
        d
    }

    pub fn rescale(&mut self, scale: f32) {
        self.scale = scale;
        let s = scale;
        let px = |v: f32| ((v * s).round() as u32).max(1);
        let fit = |img: &Option<Image>, w: u32, h: u32| img.as_ref().map(|i| fit(i, w, h));
        let rock_h = px(52.0);
        let rock_w = self.src.rock.as_ref().map_or(rock_h, |r| ((r.width as f32 * rock_h as f32 / r.height as f32).round() as u32).max(1));
        let dep_h = self.src.deposit.as_ref().map_or(px(50.0), |r| ((r.height as f32 * DEPOSIT_W * s / r.width as f32).round() as u32).max(1));
        self.ready = Ready {
            back_rim: fit(&self.src.back_rim, px(SPRING_W + 8.0), px(26.0)),
            front_rim: fit(&self.src.front_rim, px(SPRING_W + 16.0), px(26.0)),
            glow: fit(&self.src.glow, px(SPRING_W - 20.0), px(16.0)),
            steam: (0..3).filter_map(|i| fit(&self.src.steam, px(70.0 + 25.0 * i as f32), px(24.0 + 8.0 * i as f32))).collect(),
            rock_l: fit(&self.src.rock, rock_w, rock_h),
            rock_r: fit(&self.src.rock, rock_w, rock_h).map(|i| mirrored(&i)),
            deposit: Some(deposit_art(self.src.deposit.as_ref(), px(DEPOSIT_W), dep_h, s)),
        };
        for p in &mut self.props {
            p.dirty = true;
        }
    }

    /// Footprint width (px) of a find of `kind`.
    fn width(scale: f32, benches: &Benches, kind: FindKind) -> f32 {
        match kind {
            FindKind::Bench { style } => benches.art(style).map_or(120.0 * scale, |i| i.width as f32),
            FindKind::Spring => (SPRING_W + 2.0 * SPRING_PAD) * scale,
            FindKind::Deposit => DEPOSIT_ROOM * scale,
        }
    }

    /// Resolve what is on screen, create / move / animate the prop windows.
    pub fn update(&mut self, dt: f32, (desk, terrain): (&DesktopSnapshot, &Terrain), benches: &Benches, fx: &mut Fx, knight: &Character, knight_win: &SpriteWindow) {
        self.clock += dt;
        let scale = self.scale;
        let mut shown = self.world.shown(desk, terrain, now_unix(), &self.rules, |k| Self::width(scale, benches, k));
        shown.retain(|s| !self.game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1)));
        let clock = self.clock;
        self.props.retain(|p| {
            // Content ledges flicker (re-detection): keep a prop a moment while its window is open.
            let keep = shown.iter().any(|s| s.id == p.shown.id) || (desk.window(p.shown.window).is_some() && clock - p.seen_at < HIDE_GRACE);
            if !keep {
                log::debug!("discoveries: {} #{} hidden", p.shown.kind.label(), p.shown.id);
            }
            keep
        });
        let mut created = false;
        for s in &shown {
            match self.props.iter_mut().find(|p| p.shown.id == s.id) {
                Some(p) => {
                    if p.shown.pos != s.pos {
                        p.dirty = true;
                    }
                    p.shown = *s;
                    p.seen_at = clock;
                }
                None => {
                    let Ok(back) = SpriteWindow::new() else { continue };
                    let front = if s.kind == FindKind::Spring { SpriteWindow::new().ok() } else { None };
                    for w in std::iter::once(&back).chain(front.as_ref()) {
                        w.set_capturable(self.capturable);
                        w.set_click_through(true);
                    }
                    log::debug!("discoveries: {} #{} shown at ({:.0},{:.0})", s.kind.label(), s.id, s.pos.x, s.pos.y);
                    let revealed = self.reveals.iter().find(|r| r.0 == s.id).map(|r| r.1);
                    if revealed.is_some() {
                        let right = knight.pos.x < s.pos.x;
                        fx.spawn(REVEAL_FX, s.pos, right);
                    }
                    self.props.push(Prop { shown: *s, back, front, revealed, shake: 0.0, seen_at: clock, drawn_at: f32::NEG_INFINITY, dirty: true });
                    created = true;
                }
            }
        }
        self.shown = self.props.iter().map(|p| p.shown).collect();
        self.reveals.retain(|r| self.clock - r.1 < FRESH_SECS);

        for p in &mut self.props {
            p.shake = (p.shake - dt).max(0.0);
            let revealing = p.revealed.is_some_and(|t| clock - t <= REVEAL_SECS + 0.1);
            let animated = p.shown.kind == FindKind::Spring || revealing || p.shake > 0.0;
            if p.dirty || (animated && clock - p.drawn_at >= 1.0 / 12.0) {
                if let Err(e) = draw(p, &self.ready, benches, scale, clock) {
                    log::warn!("discovery prop: {e:#}");
                }
                p.drawn_at = clock;
                p.dirty = false;
            }
        }
        // Keep the knight in front of new props, and spring water in front of the knight.
        let in_spring = self.spring_at(knight).is_some();
        if created {
            knight_win.raise();
        }
        if created || (in_spring && !self.in_spring) {
            self.raise_fronts();
        }
        self.in_spring = in_spring;
    }

    fn spring_at(&self, ch: &Character) -> Option<&Shown> {
        let half = 0.5 * SPRING_W * self.scale;
        self.shown.iter().find(|s| s.kind == FindKind::Spring && (ch.pos.x - s.pos.x).abs() < half && (ch.pos.y - s.pos.y).abs() < 2.0)
    }

    /// Hide everything on monitors running a fullscreen game (like `Benches::hide_on`).
    pub fn hide_on(&mut self, game_monitors: &[RectI]) {
        self.game_monitors = game_monitors.to_vec();
        let on_game = |s: &Shown| game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1));
        self.props.retain(|p| !on_game(&p.shown));
        self.shown.retain(|s| !on_game(s));
    }

    /// Behind-the-knight layers (call before raising the knight).
    pub fn raise_backs(&self) {
        for p in &self.props {
            p.back.raise();
        }
    }

    /// In-front-of-the-knight layers (spring water; call after raising the knight).
    pub fn raise_fronts(&self) {
        for p in &self.props {
            if let Some(f) = &p.front {
                f.raise();
            }
        }
    }

    /// Found benches' standing spots (they count as benches for the brain).
    pub fn bench_spots(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.shown.iter().filter(|s| matches!(s.kind, FindKind::Bench { .. })).map(|s| s.pos)
    }

    /// Where the knight would sit right now: on a found bench, or afloat in a spring.
    pub fn seat_here(&self, ch: &Character, benches: &Benches) -> Option<f32> {
        let sup = ch.support.as_ref().filter(|_| ch.is_grounded())?;
        self.shown.iter().filter(|s| (s.pos.y - sup.y).abs() < 1.0).find_map(|s| {
            let dx = (s.pos.x - ch.pos.x).abs();
            match s.kind {
                FindKind::Bench { style } if dx < 6.0 => {
                    let h = benches.art(style).map_or(0.0, |i| i.height as f32);
                    Some(s.pos.y - h * benches.styles.get(style).map_or(0.45, |st| st.seat))
                }
                // The swimming pose's cloak line floats on the water.
                FindKind::Spring if dx < 8.0 => Some(s.pos.y - (SPRING_DEPTH + SPRING_FLOAT) * self.scale),
                _ => None,
            }
        })
    }

    /// What the brain knows about the finds on screen.
    pub fn places(&self) -> Vec<KnownPlace> {
        let s = self.scale;
        discovery::known_places(
            &self.shown,
            |id| self.reveals.iter().any(|r| r.0 == id),
            |f| match f.kind {
                FindKind::Deposit => f.pos - Vec2::new(DEPOSIT_STAND * s, 0.0),
                _ => f.pos,
            },
        )
    }

    /// The knight explored a never-visited window: roll / reveal its room.
    pub fn explored(&mut self, id: WindowId, desk: &DesktopSnapshot, terrain: &Terrain, benches: &Benches) {
        let scale = self.scale;
        let found = self.world.explored(id, desk, terrain, &self.rules, |k| Self::width(scale, benches, k));
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

    /// Debug / gift: put a find of kind `k` (0 bench, 1 spring, 2 deposit) into window `id`.
    pub fn force(&mut self, k: u8, id: WindowId, desk: &DesktopSnapshot, terrain: &Terrain, benches: &Benches) {
        let kind = match k {
            0 => {
                let avail: Vec<usize> = (0..benches.styles.len()).filter(|&i| benches.available(i)).collect();
                FindKind::Bench { style: avail.get(now_unix() as usize % avail.len().max(1)).copied().unwrap_or(0) }
            }
            1 => FindKind::Spring,
            _ => FindKind::Deposit,
        };
        let scale = self.scale;
        match self.world.force(kind, id, desk, terrain, |k| Self::width(scale, benches, k)) {
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

fn draw(p: &mut Prop, art: &Ready, benches: &Benches, s: f32, clock: f32) -> anyhow::Result<()> {
    let alpha = p.revealed.map_or(1.0, |t| ((clock - t) / REVEAL_SECS).clamp(0.0, 1.0));
    let pos = p.shown.pos;
    match p.shown.kind {
        FindKind::Bench { style } => {
            let Some(img) = benches.art(style) else { return Ok(()) };
            let img = faded(img, alpha);
            p.back.present(&img, (pos.x - img.width as f32 * 0.5).round() as i32, (pos.y - img.height as f32).round() as i32)
        }
        FindKind::Deposit => {
            let Some(img) = &art.deposit else { return Ok(()) };
            let img = faded(img, alpha);
            let dx = if p.shake > 0.0 { (p.shake * 90.0).sin() * 3.0 * s } else { 0.0 };
            // Sunk a little into its ledge, like a rock that was always there.
            p.back.present(&img, (pos.x - img.width as f32 * 0.5 + dx).round() as i32, (pos.y - img.height as f32 + 3.0 * s).round() as i32)
        }
        FindKind::Spring => {
            let (back, front) = spring_layers(art, s, clock);
            let (back, front) = (faded(&back, alpha), faded(&front, alpha));
            let x = (pos.x - back.width as f32 * 0.5).round() as i32;
            p.back.present(&back, x, (pos.y - SPRING_DEPTH * s - 20.0 * s).round() as i32)?;
            if let Some(f) = &mut p.front {
                f.present(&front, x, (pos.y - (SPRING_DEPTH + STEAM_ROOM) * s).round() as i32)?;
            }
            Ok(())
        }
    }
}

/// The two spring layers: the basin behind the knight and the water / rim / steam in
/// front of him. Both are as wide as the footprint; the front one starts `STEAM_ROOM`
/// above the water line, the back one 20 px above it.
fn spring_layers(art: &Ready, s: f32, clock: f32) -> (Image, Image) {
    let px = |v: f32| (v * s).round() as i32;
    let w = px(SPRING_W + 2.0 * SPRING_PAD);
    let (pad, depth) = (px(SPRING_PAD), px(SPRING_DEPTH));
    let (x0, x1) = (pad + px(4.0), w - pad - px(4.0));

    // Back: the far rim at the water line and the dark pool interior.
    let mut back = Image::new(w as u32, (px(20.0) + depth + px(6.0)) as u32);
    let yw = px(20.0);
    fill(&mut back, x0, yw, x1, yw + depth, |_, _| premul(46, 78, 104, 245));
    if let Some(r) = &art.back_rim {
        back.blit_over(r, (w - r.width as i32) / 2, yw - px(17.0), false);
    }

    // Front: steam, translucent milky water, surface glow, ripples, near rim and rocks.
    let mut front = Image::new(w as u32, (px(STEAM_ROOM) + depth + px(8.0)) as u32);
    let yw = px(STEAM_ROOM);
    let yl = yw + depth;
    for i in 0..5 {
        let Some(puff) = art.steam.get(i % art.steam.len().max(1)) else { break };
        let t = (clock / 4.5 + i as f32 / 5.0).fract();
        let a = (std::f32::consts::PI * t).sin() * 0.55;
        let cx = pad as f32 + (SPRING_W * s) * (0.12 + 0.19 * i as f32) + 12.0 * s * (clock * 0.7 + i as f32 * 1.7).sin();
        let cy = yw as f32 - 6.0 * s - t * STEAM_ROOM * s * 0.8;
        let img = faded(puff, a);
        front.blit_over(&img, (cx - img.width as f32 * 0.5) as i32, (cy - img.height as f32 * 0.5) as i32, i % 2 == 1);
    }
    let d = depth.max(1) as f32;
    fill(&mut front, x0, yw, x1, yl, |_, y| {
        let deep = (y - yw) as f32 / d;
        premul(168, 214, 232, (95.0 + 70.0 * deep) as u8)
    });
    if let Some(g) = &art.glow {
        front.blit_over(g, (w - g.width as i32) / 2, yw - g.height as i32 / 2, false);
    }
    fill(&mut front, x0, yw - px(1.0).max(1), x1, yw + px(1.0).max(1), |_, _| premul(232, 244, 255, 190));
    for i in 0..6 {
        let fi = i as f32;
        let span = (x1 - x0 - px(24.0)).max(1) as f32;
        let x = x0 as f32 + ((fi * 0.17 + clock * (0.03 + 0.01 * fi)).fract()) * span;
        let y = yw + px(4.0 + 5.0 * (i % 3) as f32);
        let a = (0.25 + 0.2 * (clock * 2.0 + fi).sin()).max(0.0);
        fill(&mut front, x as i32, y, x as i32 + px(14.0 + 4.0 * (i % 2) as f32), y + px(1.5).max(1), |_, _| premul(240, 248, 255, (a * 255.0) as u8));
    }
    if let Some(r) = &art.front_rim {
        front.blit_over(r, (w - r.width as i32) / 2, yl + px(6.0) - r.height as i32, false);
    }
    if let (Some(l), Some(r)) = (&art.rock_l, &art.rock_r) {
        front.blit_over(l, pad - l.width as i32 / 2 + px(4.0), yl + px(6.0) - l.height as i32, false);
        front.blit_over(r, w - pad - r.width as i32 / 2 - px(4.0), yl + px(6.0) - r.height as i32, false);
    }
    (back, front)
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

fn mirrored(img: &Image) -> Image {
    let mut out = Image::new(img.width, img.height);
    out.blit_over(img, 0, 0, true);
    out
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
