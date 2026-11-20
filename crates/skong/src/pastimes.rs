//! Pastime props on screen (see `skong_core::pastime`; the brain side is
//! `skong_core::ai::pastime`):
//!
//! * the **training dummy**: the game's `Training Dummy Real Anim` (exported into the vanilla
//!   skin as `Training_Dummy_Anim.png`), or a drawn stand-in when that art is missing. Idle,
//!   then a light / heavy hit swing per blow, with the game's hit marker.
//! * **lumaflies**: a procedural glow (`pastime::glow`), bobbing as it drifts.
//!
//! One small click-through, capture-excluded sprite window each. Nothing is saved.
use std::collections::HashMap;
use std::path::Path;

use skong_core::ai::{KnownDummy, KnownLumafly};
use skong_core::anim::{Animator, ClipId, ClipLibrary};
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::desktop::DesktopSnapshot;
use skong_core::discovery::Footprint;
use skong_core::geom::{RectF, RectI, Vec2};
use skong_core::lasting::LedgeAges;
use skong_core::occupancy::{ClaimKey, ClaimKind, Occupancy};
use skong_core::pastime::{self, DummyHit, DummyRules, LumaflyRules, ShownDummy, ShownFly, Sky, Swipe, Yard};
use skong_core::shiny::{self, LASTING_SECS, ShinySite};
use skong_core::terrain::{Ledge, Terrain};
use skong_win::sprite_window::SpriteWindow;

use crate::bubble::faded;

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
    dummy_drawn: Option<(Option<SpriteKey>, u8, i32)>,
    shown_dummy: Option<ShownDummy>,
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
    pub reachable: &'a dyn Fn(Vec2) -> bool,
}

/// What happened this tick, for the pet (hit markers).
#[derive(Default)]
pub struct PastimeNews {
    /// Blows that landed on the dummy: where (its figure's centre) and from which side.
    pub hits: Vec<(Vec2, DummyHit)>,
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
            dummy_drawn: None,
            shown_dummy: None,
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

    /// Put the dummy on a settled ledge he can get to on his monitor (`debug`: any surface).
    pub fn spawn_dummy(&mut self, w: &Where, debug: bool) -> Option<Vec2> {
        let fp = self.footprint();
        let area = w.desk.monitor_at(w.knight.x as i32, w.knight.y as i32 - 1).or(w.desk.primary()).map(|m| m.work.to_f());
        let site = ShinySite {
            terrain: w.terrain,
            ages: w.ages,
            now: w.now,
            min_age: if debug { 0.0 } else { LASTING_SECS },
            occupancy: w.occupancy,
            footprint: fp,
            knight: w.knight,
            min_distance: self.drules.min_distance * self.scale,
            area,
            avoid: &self.game_monitors,
            reachable: w.reachable,
        };
        let salt = (self.yard.clock() * 1000.0) as u64 ^ w.knight.x.to_bits() as u64;
        match shiny::place(&site, &[], salt) {
            Some((spot, pos)) => {
                let id = self.yard.add(spot, &self.drules);
                log::info!("a training dummy stands at ({:.0},{:.0}) (#{id})", pos.x, pos.y);
                Some(pos)
            }
            None => {
                self.yard.retry_soon();
                log::debug!("training dummy: no room for one right now");
                None
            }
        }
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
        shown = shown.filter(|d| !self.game_monitors.iter().any(|m| m.contains(d.pos.x as i32, d.pos.y as i32 - 1)));
        occ.release(DUMMY_CLAIM);
        if let Some(d) = shown {
            occ.claim(DUMMY_CLAIM, self.footprint().rect(d.pos), 20);
        }
        // Blows that landed: swing, hit marker.
        for hit in self.yard.take_landed() {
            if let Some(d) = shown {
                news.hits.push((self.body(d.pos).center(), hit));
            }
            self.swing = if hit.from_right { -1.0 } else { 1.0 } * if hit.heavy { 1.0 } else { 0.6 };
            if let (Some(art), Some(anim)) = (&self.art, self.anim.as_mut())
                && let Some(clip) = art.lib.id(hit.clip())
            {
                anim.play(&art.lib, clip, true);
            }
        }
        self.swing *= (-dt * 5.0).exp();
        match shown {
            None => {
                if let Some(win) = self.dummy_win.take() {
                    win.set_visible(false);
                }
                self.anim = None;
                self.dummy_drawn = None;
            }
            Some(d) => {
                if self.dummy_win.is_none()
                    && let Ok(win) = SpriteWindow::new()
                {
                    win.set_capturable(self.capturable);
                    win.set_click_through(true);
                    self.dummy_win = Some(win);
                    created = true;
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
                        if self.dummy_drawn != Some((Some(k), alpha, 0)) {
                            let img = if alpha < 255 { faded(&f.image, alpha) } else { f.image.clone() };
                            if let Err(e) = win.present(&img, x, y) {
                                log::warn!("dummy: {e:#}");
                            }
                            self.dummy_drawn = Some((Some(k), alpha, 0));
                        } else if let Err(e) = win.move_to(x, y) {
                            log::warn!("dummy: {e:#}");
                        }
                    }
                } else if let Some(win) = self.dummy_win.as_mut() {
                    let tilt = (self.swing * 20.0).round() as i32;
                    let img = pastime::dummy_stand_in(w.round() as u32, h.round() as u32, self.swing, d.alpha);
                    let (x, y) = ((d.pos.x - w * 0.5).round() as i32, (d.pos.y - h).round() as i32);
                    if self.dummy_drawn != Some((None, alpha, tilt)) {
                        if let Err(e) = win.present(&img, x, y) {
                            log::warn!("dummy: {e:#}");
                        }
                        self.dummy_drawn = Some((None, alpha, tilt));
                    } else if let Err(e) = win.move_to(x, y) {
                        log::warn!("dummy: {e:#}");
                    }
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
        let dummy = self.shown_dummy.filter(|d| d.alpha > 0.5).map(|d| KnownDummy { id: d.id, pos: d.pos, span: d.span, body: self.body(d.pos) });
        let flies = self.shown_flies.iter().map(|f| KnownLumafly { id: f.id, pos: f.pos, leaving: f.leaving || f.alpha < 0.3 }).collect();
        (dummy, flies)
    }

    /// The dummy's foot, if one stands (its ledge stays in the planner).
    pub fn dummy_foot(&self) -> Option<Vec2> {
        self.shown_dummy.map(|d| d.pos)
    }

    /// Nothing on monitors running a fullscreen game: the dummy leaves, lumaflies drift off.
    pub fn hide_on(&mut self, game_monitors: &[RectI]) {
        self.game_monitors = game_monitors.to_vec();
        if self.shown_dummy.is_some_and(|d| game_monitors.iter().any(|m| m.contains(d.pos.x as i32, d.pos.y as i32 - 1))) {
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
