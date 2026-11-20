//! Pastime props on screen (see `skong_core::pastime`; the brain side is
//! `skong_core::ai::pastime`):
//!
//! * the **training dummy**: the game's `Training Dummy Real Anim` (exported into the vanilla
//!   skin as `Training_Dummy_Anim.png`), or a drawn stand-in when that art is missing. Idle,
//!   then a light / heavy hit swing per blow, with the game's hit marker and a short white
//!   hit flash. The user's click on it is a blow too (no drag: `app/drag.rs`). Worn out it
//!   breaks: its library has no break clip, so it fades fast while the app throws wood
//!   splinters (`particles::DUMMY_SPLINTERS`, [`PastimeNews::broken`]).
//! * **lumaflies**: a procedural glow (`pastime::glow`), bobbing as it drifts.
//!
//! One small capture-excluded sprite window each (lumaflies' are click-through; the
//! dummy's opaque pixels catch the mouse: the user can drag it or strike it, see
//! `app/drag.rs`; its window is kept hidden for the next dummy). Nothing is saved.
use std::collections::HashMap;
use std::path::Path;

use skong_core::ai::{KnownDummy, KnownLumafly};
use skong_core::anim::{Animator, ClipId, ClipLibrary};
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::desktop::DesktopSnapshot;
use skong_core::discovery::Footprint;
use skong_core::geom::{RectF, RectI, Vec2};
use skong_core::lasting::{LedgeAges, SETTLED_SECS};
use skong_core::occupancy::{ClaimKey, ClaimKind, Occupancy};
use skong_core::pastime::{self, DummyBreak, DummyHit, DummyMiss, DummyRules, LumaflyRules, ShownDummy, ShownFly, Sky, Swipe, Yard};
use skong_core::shiny::{self, LASTING_SECS, ShinySite};
use skong_core::terrain::{Ledge, Terrain};
use skong_win::sprite_window::SpriteWindow;

use crate::bubble::faded;
use crate::enemies::flashed;

/// The white hit flash on a blow (s; drawn in 4 steps, as the enemies').
const HIT_FLASH: f32 = 0.12;

/// The dummy's art, relative to the game's size (its sprite is 7.4 units tall: too tall
/// beside the knight on a desktop). About 1.8 knights tall.
const DUMMY_ART_SCALE: f32 = 0.34;
/// The stand-in's size (px at scale 1).
const STAND_IN: (f32, f32) = (64.0, 160.0);
/// The lumafly's glow square (px at scale 1) and redraw rate.
const GLOW: f32 = 40.0;
const FLY_FPS: f32 = 20.0;
/// The dummy's claim in the shared occupancy.
const DUMMY_CLAIM: ClaimKey = ClaimKey::new(ClaimKind::Other, 0x6475_6d79);

struct DummyArt {
    lib: ClipLibrary,
    frames: HashMap<SpriteKey, Frame>,
    idle: ClipId,
}

impl DummyArt {
    fn load(data: &Path, vanilla: Option<&Skin>, ppu: f32) -> anyhow::Result<DummyArt> {
        let skin = vanilla.ok_or_else(|| anyhow::anyhow!("no vanilla skin export"))?;
        let bank = SpriteBank::load_metadata(data)?;
        let lib = ClipLibrary::load(&data.join("animations/Training_Dummy_Real_Anim.json"), &bank)?;
        let idle = lib.id("Idle").ok_or_else(|| anyhow::anyhow!("no Idle clip"))?;
        let keys = lib.sprite_keys();
        let frames = bank.rasterize_sets(skin, None, &[(&keys, ppu)])?.pop().unwrap_or_default();
        if frames.is_empty() {
            anyhow::bail!("no frames (Training_Dummy_Anim.png missing from the skin?)");
        }
        Ok(DummyArt { lib, frames, idle })
    }
}

struct FlyWin {
    id: u64,
    win: SpriteWindow,
    drawn_at: f64,
}

pub struct PastimeProps {
    yard: Yard,
    drules: DummyRules,
    sky: Sky,
    frules: LumaflyRules,
    scale: f32,
    art: Option<DummyArt>,
    anim: Option<Animator>,
    /// Stand-in swing (−1..1) and its decay, when there's no art.
    swing: f32,
    dummy_win: Option<SpriteWindow>,
    /// The last dummy's window, hidden, for the next one.
    spare_win: Option<SpriteWindow>,
    /// (frame or none for the stand-in, alpha, stand-in tilt, flash step) last drawn.
    dummy_drawn: Option<(Option<SpriteKey>, u8, i32, u8)>,
    /// Hit flash left (s).
    flash: f32,
    /// The user's blows since the last update: where they clicked, the blow.
    clicks: Vec<(Vec2, DummyHit)>,
    shown_dummy: Option<ShownDummy>,
    /// In the user's hand (or falling from it): its foot. The brain doesn't see it meanwhile.
    held: Option<Vec2>,
    flies: Vec<FlyWin>,
    shown_flies: Vec<ShownFly>,
    game_monitors: Vec<RectI>,
    pub capturable: bool,
}

/// The surroundings a new dummy is placed in (see `shinies::Where`).
pub struct Where<'a> {
    pub desk: &'a DesktopSnapshot,
    pub terrain: &'a Terrain,
    pub ages: &'a LedgeAges,
    pub now: f64,
    pub occupancy: &'a Occupancy,
    pub knight: Vec2,
    /// Would the brain walk over and practise on a dummy standing there (its own test,
    /// `skong_core::ai::dummy_stand`)?
    pub practisable: &'a dyn Fn(&KnownDummy) -> bool,
}

/// What happened this tick, for the pet (hit markers).
#[derive(Default)]
pub struct PastimeNews {
    /// Blows that landed on the dummy: where (its figure's centre, or the user's click) and
    /// from which side.
    pub hits: Vec<(Vec2, DummyHit)>,
    /// The dummy broke: what broke it, its figure's centre and its foot (the splinters).
    pub broken: Vec<(DummyBreak, Vec2, Vec2)>,
}

impl PastimeProps {
    pub fn load(data: &Path, vanilla: Option<&Skin>, scale: f32, seed: u64) -> PastimeProps {
        let (drules, frules) = (DummyRules::default(), LumaflyRules::default());
        let art = match DummyArt::load(data, vanilla, 64.0 * scale * DUMMY_ART_SCALE) {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("training dummy: using the drawn stand-in ({e:#})");
                None
            }
        };
        log::info!(
            "pastimes: a training dummy every {:.0}-{:.0} min, a lumafly every {:.0}-{:.0} min",
            drules.every_secs[0] / 60.0,
            drules.every_secs[1] / 60.0,
            frules.every_secs[0] / 60.0,
            frules.every_secs[1] / 60.0
        );
        PastimeProps {
            yard: Yard::new(seed, &drules),
            drules,
            sky: Sky::new(seed, &frules),
            frules,
            scale,
            art,
            anim: None,
            swing: 0.0,
            dummy_win: None,
            spare_win: None,
            dummy_drawn: None,
            flash: 0.0,
            clicks: Vec::new(),
            shown_dummy: None,
            held: None,
            flies: Vec::new(),
            shown_flies: Vec::new(),
            game_monitors: Vec::new(),
            capturable: false,
        }
    }

    pub fn rescale(&mut self, data: &Path, vanilla: Option<&Skin>, scale: f32) {
        self.scale = scale;
        if self.art.is_some() {
            self.art = DummyArt::load(data, vanilla, 64.0 * scale * DUMMY_ART_SCALE).ok();
        }
        self.dummy_drawn = None;
        for f in &mut self.flies {
            f.drawn_at = f64::NEG_INFINITY;
        }
    }

    /// The dummy's drawn size (w, h) in px.
    fn dummy_size(&self) -> (f32, f32) {
        match &self.art {
            Some(a) => {
                let f = a.lib.get(a.idle).frames.iter().find_map(|f| f.sprite).and_then(|k| a.frames.get(&k));
                f.map_or((STAND_IN.0 * self.scale, STAND_IN.1 * self.scale), |f| (f.image.width as f32 * 0.6, f.image.height as f32))
            }
            None => (STAND_IN.0 * self.scale, STAND_IN.1 * self.scale),
        }
    }

    fn footprint(&self) -> Footprint {
        let (w, h) = self.dummy_size();
        let reach = 0.55 * 64.0 * self.scale;
        Footprint { w: w * 0.5, h, room: w + 2.0 * (reach + 30.0 * self.scale) }
    }

    /// Timers: `true` when a dummy should turn up now (then [`Self::spawn_dummy`]); a
    /// lumafly drifts in by itself on the knight's monitor (`area`, its window ledges `bars`).
    pub fn due(&mut self, dt: f32, allowed: bool, area: Option<RectF>, bars: &[(f32, f32, f32)], knight: Vec2) -> bool {
        let dummy = self.yard.due(dt, allowed, &self.drules);
        if self.sky.due(dt, allowed && area.is_some(), &self.frules)
            && let Some(area) = area
        {
            self.sky.spawn(area, bars, knight, &self.frules, self.scale);
        }
        self.sky.step(dt, bars, &self.frules, self.scale);
        dummy
    }

    /// Put the dummy (a new one, or the one standing moved) on a lasting ledge of his monitor
    /// where the brain's own test says he'd walk over and practise: near him, on his level
    /// or the floor first (`debug`: any settled surface, nearer still). Why not, if not.
    pub fn spawn_dummy(&mut self, w: &Where, debug: bool) -> Result<Vec2, DummyMiss> {
        let fp = self.footprint();
        let area = w.desk.monitor_at(w.knight.x as i32, w.knight.y as i32 - 1).or(w.desk.primary()).map(|m| m.work.to_f());
        let no_check = |_: Vec2| true;
        let site = ShinySite {
            terrain: w.terrain,
            ages: w.ages,
            now: w.now,
            // Settled at least: a dummy on a younger ledge isn't drawn (or seen) until it is.
            min_age: if debug { SETTLED_SECS } else { LASTING_SECS },
            occupancy: w.occupancy,
            footprint: fp,
            knight: w.knight,
            min_distance: self.drules.min_distance * self.scale * if debug { 0.6 } else { 1.0 },
            area,
            avoid: &self.game_monitors,
            reachable: &no_check,
        };
        let salt = (self.yard.clock() * 1000.0) as u64 ^ w.knight.x.to_bits() as u64;
        let mut practisable = |foot: Vec2, span: (f32, f32)| (w.practisable)(&self.known(0, foot, span));
        match pastime::place_dummy(&site, salt, &mut practisable) {
            Ok((spot, pos)) => {
                let id = self.yard.add(spot, &self.drules);
                self.held = None;
                log::info!("a training dummy stands at ({:.0},{:.0}) (#{id}; the knight at ({:.0},{:.0}))", pos.x, pos.y, w.knight.x, w.knight.y);
                Ok(pos)
            }
            Err(miss) => {
                self.yard.retry_soon();
                Err(miss)
            }
        }
    }

    /// Test panel: the dummy right at `at` on window `owner`'s ledge (`None`: the floor),
    /// replacing one standing elsewhere.
    pub fn place_dummy_at(&mut self, terrain: &Terrain, at: Vec2, owner: Option<skong_core::desktop::WindowId>) -> bool {
        let Some(spot) = shiny::spot_at(terrain, at, owner) else { return false };
        let id = self.yard.add(spot, &self.drules);
        self.yard.pin(id);
        self.held = None;
        log::info!("a training dummy placed at ({:.0},{:.0}) (#{id})", at.x, at.y);
        true
    }

    /// May dummy `id` be moved elsewhere when he can't get to it? Not one the user put
    /// where it is (the Test tab placed it, or it was dragged there).
    pub fn relocatable(&self, id: u64) -> bool {
        self.yard.relocatable(id)
    }

    /// Mouse events on the dummy's window since the last call, with its id (the user's
    /// hand, `app/drag.rs`).
    pub fn take_mouse(&self) -> Vec<(u64, skong_win::sprite_window::MouseEvent)> {
        let events = self.dummy_win.as_ref().map(|w| w.take_mouse()).unwrap_or_default();
        match self.yard.dummy() {
            // A broken one (fading out) is past handling.
            Some(d) if !d.is_broken() => events.into_iter().map(|e| (d.id, e)).collect(),
            _ => Vec::new(),
        }
    }

    /// Where dummy `id` stands (or hangs in the user's hand), while it's whole.
    pub fn dummy_at(&self, id: u64) -> Option<Vec2> {
        self.yard.dummy().filter(|d| d.id == id && !d.is_broken())?;
        self.held.or(self.shown_dummy.map(|d| d.pos))
    }

    /// The user clicked dummy `id` at `at` (a press without a drag): a blow, as the knight's
    /// (the hit clip from that side, the flash, the hit marker where they clicked). It wears
    /// it: about five break a fresh one. `Some(wear)` after the blow (1: it broke).
    pub fn click(&mut self, id: u64, at: Vec2) -> Option<f32> {
        let foot = self.dummy_at(id).filter(|_| self.held.is_none())?;
        let hit = DummyHit::clicked(at, foot);
        self.yard.click(id, &self.drules)?;
        self.clicks.push((at, hit));
        self.wobble(hit);
        self.flash = HIT_FLASH;
        Some(self.yard.wear().unwrap_or(1.0))
    }

    /// The user picked dummy `id` up: it wobbles; the brain loses sight of it.
    pub fn pick_up(&mut self, id: u64) -> bool {
        let Some(at) = self.dummy_at(id) else { return false };
        self.held = Some(at);
        self.wobble(DummyHit { heavy: false, from_right: false });
        true
    }

    /// Carried / falling: its foot is at `foot`.
    pub fn hold(&mut self, foot: Vec2) {
        if self.held.is_some() {
            self.held = Some(foot);
        }
    }

    /// Dummy `id` landed at `at` on window `owner`'s edge (`None`: the floor): it stays there
    /// (pinned: never moved away by itself), with a thud.
    pub fn set_down(&mut self, id: u64, terrain: &Terrain, at: Vec2, owner: Option<skong_core::desktop::WindowId>) -> bool {
        self.held = None;
        let spot = shiny::spot_at(terrain, at, owner).unwrap_or(shiny::Spot::Ground { pos: at });
        if !self.yard.moved(id, spot) {
            return false;
        }
        self.wobble(DummyHit { heavy: true, from_right: true });
        true
    }

    /// Swing as if struck (`hit`'s clip, or the stand-in's tilt).
    fn wobble(&mut self, hit: DummyHit) {
        self.swing = if hit.from_right { -1.0 } else { 1.0 } * if hit.heavy { 1.0 } else { 0.6 };
        if let (Some(art), Some(anim)) = (&self.art, self.anim.as_mut())
            && let Some(clip) = art.lib.id(hit.clip())
        {
            anim.play(&art.lib, clip, true);
        }
    }

    /// The brain's view of a dummy that would stand at `foot` (a ledge spanning `span`).
    pub fn known_at(&self, foot: Vec2, span: (f32, f32)) -> KnownDummy {
        self.known(0, foot, span)
    }

    /// The dummy's half width (px), for placement.
    pub fn dummy_half_width(&self) -> f32 {
        0.5 * self.dummy_size().0
    }

    /// The dummy standing, if one is up and not on its way out (its id).
    pub fn dummy_id(&self) -> Option<u64> {
        self.yard.dummy().filter(|d| d.leave_at > self.yard.clock() && !d.is_broken()).map(|d| d.id)
    }

    /// Fade the dummy out (one he can't get to and no better place for it).
    pub fn retire_dummy(&mut self) {
        self.yard.dismiss();
    }

    /// The brain's view of a dummy standing at `foot` on a ledge spanning `span`.
    fn known(&self, id: u64, foot: Vec2, span: (f32, f32)) -> KnownDummy {
        KnownDummy { id, pos: foot, span, body: self.body(foot) }
    }

    /// A lumafly now (debug), on `area`.
    pub fn spawn_lumafly(&mut self, area: RectF, bars: &[(f32, f32, f32)], knight: Vec2) {
        self.sky.clear();
        self.sky.spawn(area, bars, knight, &self.frules, self.scale);
    }

    /// The brain's blow / swipe.
    pub fn strike(&mut self, hit: DummyHit, delay: f32) {
        self.yard.strike(hit, delay);
    }

    pub fn swipe(&mut self, fly: u64, from: Vec2) -> Swipe {
        self.sky.swipe(fly, from, &self.frules, self.scale)
    }

    /// Resolve, claim and draw. `true` when a new window was created (raise the knight).
    pub fn update(&mut self, dt: f32, desk: &DesktopSnapshot, terrain: &Terrain, occ: &mut Occupancy, settled: &dyn Fn(&Ledge) -> bool) -> (bool, PastimeNews) {
        let mut created = false;
        let mut news = PastimeNews::default();
        let (w, h) = self.dummy_size();
        let mut shown = self.yard.shown(desk, terrain, self.footprint().w, settled);
        let broke = self.yard.take_broken();
        match self.held {
            // Worn out in the user's hand (or gone): it comes apart right there.
            Some(p) if !broke.is_empty() || self.yard.dummy().is_none_or(|d| d.is_broken()) => {
                news.broken.extend(broke.iter().map(|b| (*b, self.body(p).center(), p)));
                self.held = None;
                shown = None;
            }
            // In the user's hand: drawn where the hand is (it still wears).
            Some(p) => {
                shown = self.yard.dummy().map(|d| ShownDummy { id: d.id, pos: p, span: (p.x, p.x), alpha: shown.map_or(1.0, |s| s.alpha), broken: false });
            }
            None => {
                shown = shown.filter(|d| !self.game_monitors.iter().any(|m| m.contains(d.pos.x as i32, d.pos.y as i32 - 1)));
                // Splinters from where it stood (last seen, if its ledge is covered now).
                if let Some(d) = shown.or(self.shown_dummy) {
                    news.broken.extend(broke.iter().map(|b| (*b, self.body(d.pos).center(), d.pos)));
                }
            }
        }
        occ.release(DUMMY_CLAIM);
        if let Some(d) = shown.filter(|_| self.held.is_none()) {
            occ.claim(DUMMY_CLAIM, self.footprint().rect(d.pos), 20);
        }
        // Blows that landed: swing, flash, hit marker (the user's where they clicked).
        for hit in self.yard.take_landed() {
            if let Some(d) = shown {
                news.hits.push((self.body(d.pos).center(), hit));
            }
            self.wobble(hit);
            self.flash = HIT_FLASH;
        }
        news.hits.append(&mut self.clicks);
        self.swing *= (-dt * 5.0).exp();
        let flash = ((self.flash / HIT_FLASH) * 4.0).ceil().clamp(0.0, 4.0) as u8;
        self.flash = (self.flash - dt).max(0.0);
        let mut reshow = false;
        match shown {
            None => {
                // Hidden, kept for the next dummy.
                if let Some(win) = self.dummy_win.take() {
                    win.set_visible(false);
                    self.spare_win = Some(win);
                }
                self.anim = None;
                self.dummy_drawn = None;
                self.flash = 0.0;
            }
            Some(d) => {
                if self.dummy_win.is_none() {
                    if let Some(win) = self.spare_win.take() {
                        let _ = win.take_mouse(); // nothing from its last dummy
                        win.set_capturable(self.capturable);
                        self.dummy_win = Some(win);
                        reshow = true;
                        created = true;
                    } else if let Ok(win) = SpriteWindow::new() {
                        win.set_capturable(self.capturable);
                        // Its opaque pixels catch the mouse: the user can pick it up or strike it.
                        self.dummy_win = Some(win);
                        created = true;
                    }
                }
                let alpha = (d.alpha * 255.0) as u8;
                if let Some(art) = &self.art {
                    let anim = self.anim.get_or_insert_with(|| Animator::new(art.idle));
                    let mut events = Vec::new();
                    anim.update(&art.lib, dt, &mut events);
                    if anim.is_finished() && anim.clip() != art.idle {
                        anim.play(&art.lib, art.idle, true);
                    }
                    let key = anim.current(&art.lib);
                    if let (Some(k), Some(win)) = (key, self.dummy_win.as_mut())
                        && let Some(f) = art.frames.get(&k)
                    {
                        // The pole's foot on the ledge (sunk a hair into it).
                        let x = (d.pos.x - f.anchor_x(false)).round() as i32;
                        let y = (d.pos.y + 2.0 * self.scale - f.image.height as f32).round() as i32;
                        if self.dummy_drawn != Some((Some(k), alpha, 0, flash)) {
                            let lit = if flash > 0 { flashed(&f.image, flash as f32 / 4.0) } else { f.image.clone() };
                            let img = if alpha < 255 { faded(&lit, alpha) } else { lit };
                            if let Err(e) = win.present(&img, x, y) {
                                log::warn!("dummy: {e:#}");
                            }
                            self.dummy_drawn = Some((Some(k), alpha, 0, flash));
                        } else if let Err(e) = win.move_to(x, y) {
                            log::warn!("dummy: {e:#}");
                        }
                    }
                } else if let Some(win) = self.dummy_win.as_mut() {
                    let tilt = (self.swing * 20.0).round() as i32;
                    let (x, y) = ((d.pos.x - w * 0.5).round() as i32, (d.pos.y - h).round() as i32);
                    if self.dummy_drawn != Some((None, alpha, tilt, flash)) {
                        let mut img = pastime::dummy_stand_in(w.round() as u32, h.round() as u32, self.swing, d.alpha);
                        if flash > 0 {
                            img = flashed(&img, flash as f32 / 4.0);
                        }
                        if let Err(e) = win.present(&img, x, y) {
                            log::warn!("dummy: {e:#}");
                        }
                        self.dummy_drawn = Some((None, alpha, tilt, flash));
                    } else if let Err(e) = win.move_to(x, y) {
                        log::warn!("dummy: {e:#}");
                    }
                }
                if reshow && let Some(win) = &self.dummy_win {
                    win.set_visible(true);
                }
            }
        }
        self.shown_dummy = shown;

        // Lumaflies.
        let flies: Vec<ShownFly> = self.sky.shown(self.scale).into_iter().filter(|f| !self.game_monitors.iter().any(|m| m.contains(f.pos.x as i32, f.pos.y as i32))).collect();
        self.flies.retain(|w| {
            let keep = flies.iter().any(|f| f.id == w.id);
            if !keep {
                w.win.set_visible(false);
            }
            keep
        });
        let clock = self.sky.clock();
        let size = ((GLOW * self.scale).round() as u32).max(9);
        for f in &flies {
            let fw = match self.flies.iter_mut().position(|w| w.id == f.id) {
                Some(i) => &mut self.flies[i],
                None => {
                    let Ok(win) = SpriteWindow::new() else { continue };
                    win.set_capturable(self.capturable);
                    win.set_click_through(true);
                    self.flies.push(FlyWin { id: f.id, win, drawn_at: f64::NEG_INFINITY });
                    created = true;
                    self.flies.last_mut().expect("just pushed")
                }
            };
            let (x, y) = ((f.pos.x - size as f32 * 0.5).round() as i32, (f.pos.y - size as f32 * 0.5).round() as i32);
            if (clock - fw.drawn_at) as f32 >= 1.0 / FLY_FPS {
                fw.drawn_at = clock;
                let img = pastime::glow(size, clock, f.phase, f.glow, f.alpha);
                if let Err(e) = fw.win.present(&img, x, y) {
                    log::warn!("lumafly: {e:#}");
                }
            } else if let Err(e) = fw.win.move_to(x, y) {
                log::warn!("lumafly: {e:#}");
            }
        }
        self.shown_flies = flies;
        (created, news)
    }

    /// The dummy's figure (what blows hit), from its foot.
    fn body(&self, foot: Vec2) -> RectF {
        let (w, h) = self.dummy_size();
        let half = (0.4 * w).max(12.0 * self.scale);
        RectF::new(foot.x - half, foot.y - 0.97 * h, foot.x + half, foot.y - 0.42 * h)
    }

    /// What the brain knows.
    pub fn percepts(&self) -> (Option<KnownDummy>, Vec<KnownLumafly>) {
        let dummy = self.shown_dummy.filter(|d| d.alpha > 0.5 && !d.broken && self.held.is_none()).map(|d| self.known(d.id, d.pos, d.span));
        let flies = self.shown_flies.iter().map(|f| KnownLumafly { id: f.id, pos: f.pos, leaving: f.leaving || f.alpha < 0.3 }).collect();
        (dummy, flies)
    }

    /// The dummy's foot, if one stands (its ledge stays in the planner).
    pub fn dummy_foot(&self) -> Option<Vec2> {
        self.shown_dummy.filter(|d| !d.broken && self.held.is_none()).map(|d| d.pos)
    }

    /// Nothing on monitors running a fullscreen game: the dummy leaves, lumaflies drift off.
    pub fn hide_on(&mut self, game_monitors: &[RectI]) {
        self.game_monitors = game_monitors.to_vec();
        if self.held.is_none() && self.shown_dummy.is_some_and(|d| game_monitors.iter().any(|m| m.contains(d.pos.x as i32, d.pos.y as i32 - 1))) {
            self.yard.dismiss();
        }
        if self.sky.flies().iter().any(|f| game_monitors.iter().any(|m| m.contains(f.pos.x as i32, f.pos.y as i32))) {
            self.sky.dismiss();
        }
    }

    pub fn raise_all(&self) {
        if let Some(w) = &self.dummy_win {
            w.raise();
        }
        for f in &self.flies {
            f.win.raise();
        }
    }

    pub fn clear(&mut self) {
        self.yard.clear();
        self.sky.clear();
        self.held = None;
        self.clicks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dummys_clips_are_in_the_exported_metadata() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = SpriteBank::load_metadata(&data).expect("metadata");
        let lib = ClipLibrary::load(&data.join("animations/Training_Dummy_Real_Anim.json"), &bank).expect("dummy clips");
        assert!(lib.id("Idle").is_some());
        for heavy in [false, true] {
            for from_right in [false, true] {
                let clip = DummyHit { heavy, from_right }.clip();
                assert!(lib.id(clip).is_some(), "{clip}");
            }
        }
    }

    #[test]
    fn without_art_the_stand_in_is_used() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let p = PastimeProps::load(&data, None, 0.6, 1);
        assert!(p.art.is_none());
        let (w, h) = p.dummy_size();
        assert!(h > 2.0 * w && h > 60.0, "{w}x{h}");
        // Blows hit its stuffed figure, above the pole.
        let body = p.body(Vec2::new(500.0, 1000.0));
        assert!(body.bottom < 1000.0 - 0.3 * h && body.top > 1000.0 - h - 1.0);
    }
}
