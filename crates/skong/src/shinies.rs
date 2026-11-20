//! Shinies on screen: little glints on the ground for the knight to find and pick up
//! (see `skong_core::shiny`). One small capture-excluded, click-through sprite window per
//! shiny, drawn procedurally (`shiny::glint`: a soft pale point with a thin breathing star
//! and two winking sparks), fading in when it turns up and out when it goes.
//!
//! Rates and rewards come from `data/props.json` → `shinies` ([`ShinyRules`]). Nothing is
//! saved: shinies are short-lived.
use std::path::Path;

use serde::Deserialize;
use skong_core::ai::KnownShiny;
use skong_core::assets::read_json;
use skong_core::desktop::DesktopSnapshot;
use skong_core::discovery::Footprint;
use skong_core::geom::{RectF, RectI, Vec2};
use skong_core::lasting::LedgeAges;
use skong_core::occupancy::Occupancy;
use skong_core::shiny::{self, LASTING_SECS, Shinies, ShinyId, ShinyRules, ShinySite, ShownShiny};
use skong_core::terrain::Terrain;
use skong_win::sprite_window::SpriteWindow;

/// Fade in / out (s).
const FADE_IN: f32 = 1.5;
const FADE_OUT: f32 = 4.0;
/// Picked up: gone this fast (s).
const VANISH: f32 = 0.25;
/// Redraw rate of the twinkle (frames per second).
const FPS: f32 = 20.0;
/// A pick-up that never reaches "Collect Normal 2" is forgotten after this long (s).
const PENDING_SECS: f32 = 4.0;

// Sizes in px at scale 1 (the knight is ~88 px tall).
/// The glint's square.
const GLINT: f32 = 30.0;
/// Its footprint on the ledge (what it claims in the shared occupancy).
const FOOT_W: f32 = 20.0;
const FOOT_H: f32 = 18.0;
/// Where the knight stands to pick it up, from its centre.
const REACH: f32 = 20.0;

#[derive(Deserialize, Default)]
struct PropsFile {
    #[serde(default)]
    shinies: ShinyRules,
}

struct Prop {
    id: ShinyId,
    pos: Vec2,
    win: SpriteWindow,
    /// Twinkle phase (0..1): neighbours don't pulse in step.
    phase: f32,
    drawn_at: f64,
    /// Clock time it was picked up (fading away fast).
    vanishing: Option<f64>,
}

pub struct ShinyProps {
    shinies: Shinies,
    rules: ShinyRules,
    scale: f32,
    props: Vec<Prop>,
    shown: Vec<ShownShiny>,
    /// The knight is picking this one up (since this clock time).
    pending: Option<(ShinyId, f64)>,
    game_monitors: Vec<RectI>,
    pub capturable: bool,
}

/// The surroundings a new shiny is placed in.
pub struct Where<'a> {
    pub desk: &'a DesktopSnapshot,
    pub terrain: &'a Terrain,
    pub ages: &'a LedgeAges,
    /// Clock of `ages`.
    pub now: f64,
    pub occupancy: &'a Occupancy,
    pub knight: Vec2,
    pub reachable: &'a dyn Fn(Vec2) -> bool,
}

impl ShinyProps {
    pub fn load(props_json: &Path, scale: f32, seed: u64) -> ShinyProps {
        let rules = read_json::<PropsFile>(props_json).map(|p| p.shinies).unwrap_or_default();
        log::info!("shinies: one every {:.0}-{:.0} s, at most {}", rules.every_secs[0], rules.every_secs[1], rules.max);
        ShinyProps { shinies: Shinies::new(seed, &rules), rules, scale, props: Vec::new(), shown: Vec::new(), pending: None, game_monitors: Vec::new(), capturable: false }
    }

    pub fn rescale(&mut self, scale: f32) {
        self.scale = scale;
        for p in &mut self.props {
            p.drawn_at = f64::NEG_INFINITY;
        }
    }

    fn footprint(&self) -> Footprint {
        let s = self.scale;
        Footprint { w: FOOT_W * s, h: FOOT_H * s, room: FOOT_W * s + 2.0 * (REACH + 24.0) * s }
    }

    /// Advance the spawn timer; `true` when one should turn up by itself now.
    pub fn due(&mut self, dt: f32, allowed: bool) -> bool {
        self.shinies.due(dt, allowed, &self.rules)
    }

    /// Put a new shiny somewhere on the knight's monitor (lasting surfaces only, unless
    /// `debug`). Returns where, if there was room.
    pub fn spawn(&mut self, w: &Where, debug: bool) -> Option<Vec2> {
        if self.shinies.len() >= self.rules.max && !debug {
            return None;
        }
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
            min_distance: self.rules.min_distance * self.scale,
            area,
            avoid: &self.game_monitors,
            reachable: w.reachable,
        };
        let taken: Vec<RectF> = self.shinies.rects(w.terrain, fp);
        let salt = (self.shinies.clock() * 1000.0) as u64 ^ w.knight.x.to_bits() as u64;
        match shiny::place(&site, &taken, salt) {
            Some((spot, pos)) => {
                let id = self.shinies.add(spot);
                log::info!("a shiny turns up at ({:.0},{:.0}) (#{id})", pos.x, pos.y);
                Some(pos)
            }
            None => {
                self.shinies.retry_soon();
                log::debug!("shinies: no room for one right now");
                None
            }
        }
    }

    /// Resolve, settle against the other props, and draw. `true` when a new window was
    /// created (raise the knight above it).
    pub fn update(&mut self, desk: &DesktopSnapshot, terrain: &Terrain, occ: &mut Occupancy, settled: &dyn Fn(&skong_core::terrain::Ledge) -> bool) -> bool {
        let clock = self.shinies.clock();
        let fp = self.footprint();
        let mut raw = self.shinies.shown(desk, terrain, &self.rules, fp.w, settled);
        raw.retain(|s| !self.game_monitors.iter().any(|m| m.contains(s.pos.x as i32, s.pos.y as i32 - 1)));
        let shown = shiny::settle(raw, occ, fp);
        let mut created = false;
        if self.pending.is_some_and(|(id, t)| (clock - t) as f32 > PENDING_SECS || self.shinies.get(id).is_none()) {
            self.pending = None;
        }

        // Windows: new ones, gone ones (picked up ones fade fast), moves.
        for p in &mut self.props {
            if p.vanishing.is_none() && !shown.iter().any(|s| s.id == p.id) {
                p.win.set_visible(false);
                p.vanishing = Some(f64::NEG_INFINITY); // hidden / expired: drop now
            }
        }
        self.props.retain(|p| p.vanishing.is_none_or(|t| ((clock - t) as f32) < VANISH));
        for s in &shown {
            match self.props.iter_mut().find(|p| p.id == s.id) {
                Some(p) => p.pos = s.pos,
                None => {
                    let Ok(win) = SpriteWindow::new() else { continue };
                    win.set_capturable(self.capturable);
                    win.set_click_through(true);
                    let phase = (s.id as f32 * 0.618).fract();
                    self.props.push(Prop { id: s.id, pos: s.pos, win, phase, drawn_at: f64::NEG_INFINITY, vanishing: None });
                    created = true;
                }
            }
        }
        let size = ((GLINT * self.scale).round() as u32).max(9);
        for p in &mut self.props {
            if ((clock - p.drawn_at) as f32) < 1.0 / FPS {
                continue;
            }
            p.drawn_at = clock;
            let age = shown.iter().find(|s| s.id == p.id).map_or(0.0, |s| s.age);
            let left = self.rules.lifetime_secs - age;
            let alpha = match p.vanishing {
                Some(t) => 1.0 - (clock - t) as f32 / VANISH,
                None => (age / FADE_IN).min(left / FADE_OUT).clamp(0.0, 1.0),
            };
            let img = shiny::glint(size, clock, p.phase, alpha);
            // Resting on the ledge: the lower arm just touches it.
            let (x, y) = ((p.pos.x - size as f32 * 0.5).round() as i32, (p.pos.y - size as f32 * 0.62).round() as i32);
            if let Err(e) = p.win.present(&img, x, y) {
                log::warn!("shiny: {e:#}");
            }
        }
        self.shown = shown;
        created
    }

    /// What the brain knows about the shinies on screen.
    pub fn percepts(&self) -> Vec<KnownShiny> {
        let reach = REACH * self.scale;
        self.shown.iter().map(|s| KnownShiny { id: s.id, pos: s.pos, span: s.span, reach }).collect()
    }

    /// The brain is picking `id` up (its act was requested).
    pub fn picking(&mut self, id: ShinyId) {
        if self.pending.is_none_or(|p| p.0 != id) {
            self.pending = Some((id, self.shinies.clock()));
        }
    }

    /// The pick-up act reached "Collect Normal 2": the pending shiny is gone. Returns
    /// where it was and the Geo that pops out of it (usually none).
    pub fn picked(&mut self) -> Option<(Vec2, u32)> {
        let (id, _) = self.pending.take()?;
        let pos = self.shown.iter().find(|s| s.id == id).map(|s| s.pos)?;
        self.shinies.take(id)?;
        let clock = self.shinies.clock();
        if let Some(p) = self.props.iter_mut().find(|p| p.id == id) {
            p.vanishing = Some(clock);
            p.drawn_at = f64::NEG_INFINITY;
        }
        self.shown.retain(|s| s.id != id);
        let geo = self.shinies.reward(&self.rules);
        log::info!("the knight picked up a shiny at ({:.0},{:.0}){}", pos.x, pos.y, if geo > 0 { format!(": {geo} geo inside!") } else { String::new() });
        Some((pos, geo))
    }

    /// Hide everything on monitors running a fullscreen game.
    pub fn hide_on(&mut self, game_monitors: &[RectI]) {
        self.game_monitors = game_monitors.to_vec();
        let on_game = |p: Vec2| game_monitors.iter().any(|m| m.contains(p.x as i32, p.y as i32 - 1));
        self.props.retain(|p| !on_game(p.pos));
        self.shown.retain(|s| !on_game(s.pos));
    }

    pub fn raise_all(&self) {
        for p in &self.props {
            p.win.raise();
        }
    }

    pub fn clear(&mut self) {
        self.shinies.clear();
        self.props.clear();
        self.shown.clear();
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_load_from_props_json() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/props.json");
        let file: PropsFile = read_json(&path).expect("props.json");
        assert_eq!(file.shinies.max, 2);
        assert!(file.shinies.every_secs[0] >= 60.0 && file.shinies.every_secs[0] <= file.shinies.every_secs[1], "every few minutes");
    }
}
