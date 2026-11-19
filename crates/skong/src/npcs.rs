//! App side of NPC visitors: schedules visits, opens dream portals, renders each visitor in
//! its own sprite window (vanilla art), shows their lines in speech bubbles, starts
//! conversations when the brain asks, and turns the knight's dream nail into a thought
//! bubble about the window he struck.
//!
//! The logic lives in `skong_core::npc` / `skong_core::portal`; this module is rendering and
//! plumbing.
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use skong_core::ai::Visitor;
use skong_core::ai::rng::Rng;
use skong_core::assets::{Frame, Skin, SpriteBank, SpriteKey};
use skong_core::bubble::BubbleStyle;
use skong_core::controller::Character;
use skong_core::desktop::{DesktopSnapshot, WindowInfo};
use skong_core::geom::{RectF, RectI, Vec2};
use skong_core::nav::{NavGoal, NavGraph, TravelStyle};
use skong_core::npc::{Arrival, Npc, NpcEnv, NpcEvent, NpcPhase, NpcSet, Placement, TalkContext, VisitConditions, VisitScheduler, plan_placement};
use skong_core::portal::{Footing, Portal, PortalPhase, PortalSpot};
use skong_core::terrain::Terrain;
use skong_win::sprite_window::SpriteWindow;

use crate::bubble::{BubbleId, Bubbles, faded};
use crate::fx::Fx;

/// Per-definition art measurements (px at the current scale).
#[derive(Clone, Copy, Debug, Default)]
struct Metrics {
    /// Sprite origin above the feet / seat point.
    origin_above_feet: f32,
    /// Head top above the feet point (bubble anchor).
    head: f32,
}

struct LiveNpc {
    npc: Npc,
    win: SpriteWindow,
    drawn: Option<(SpriteKey, bool, u8)>,
    bubble: Option<BubbleId>,
}

/// A portal whose guest has not stepped out yet (`def` None: a departure portal).
struct OpenPortal {
    portal: Portal,
    guest: Option<(usize, Placement)>,
}

/// What the visitors need to know about the world this tick.
pub struct World<'a> {
    pub terrain: &'a Terrain,
    pub desk: &'a DesktopSnapshot,
    pub knight: &'a Character,
    pub graph: Option<&'a NavGraph>,
    pub user_idle_secs: f32,
    pub keys_per_sec: f32,
    pub fullscreen: bool,
    /// Asleep, hiding, held...: no visitors now.
    pub knight_busy: bool,
    pub hour: f32,
    /// Monitors with a fullscreen game: nothing of ours shows or arrives there.
    pub game_monitors: &'a [RectI],
}

pub struct Visitors {
    set: NpcSet,
    meta: SpriteBank,
    frames: Vec<HashMap<SpriteKey, Frame>>,
    metrics: Vec<Metrics>,
    live: Vec<LiveNpc>,
    portals: Vec<OpenPortal>,
    sched: VisitScheduler,
    rng: Rng,
    next_id: u32,
    last_def: Option<usize>,
    ppu: f32,
    pub bubbles: Bubbles,
    pub capturable: bool,
    /// A visit asked for from the menu / panel (definition name or any).
    invited: Option<Option<String>>,
    /// Knight's dream nail: the slash clip started this tick; thought shows after a beat.
    thought_in: Option<f32>,
    knight_clip_was_slash: bool,
    /// Departure portals to draw (opened while handling visitor events).
    pending_fx: Vec<Vec2>,
    /// Every window hidden (games cover every monitor).
    hidden: bool,
}

const SLASH_CLIP: &str = "DN Slash";

impl Visitors {
    pub fn load(data_dir: &Path, vanilla: Option<&Skin>, scale: f32, seed: u32) -> Result<Visitors> {
        let meta = SpriteBank::load_metadata(data_dir)?;
        let set = NpcSet::load(&data_dir.join("../npcs.json"), &data_dir.join("animations"), &meta)?;
        let mut rng = Rng::new(seed);
        let sched = VisitScheduler::new(set.visits, &mut rng);
        log::info!(
            "visitors: {} ({}), first visit in ~{:.0} min of presence",
            set.defs.len(),
            set.defs.iter().map(|d| d.display.as_str()).collect::<Vec<_>>().join(", "),
            sched.due_in() / 60.0
        );
        let bubbles = Bubbles::new(data_dir, vanilla, scale);
        let mut v = Visitors {
            frames: Vec::new(),
            metrics: Vec::new(),
            set,
            meta,
            live: Vec::new(),
            portals: Vec::new(),
            sched,
            rng,
            next_id: 1,
            last_def: None,
            ppu: 64.0 * scale,
            bubbles,
            capturable: false,
            invited: None,
            thought_in: None,
            knight_clip_was_slash: false,
            pending_fx: Vec::new(),
            hidden: false,
        };
        v.rescale(vanilla, scale)?;
        Ok(v)
    }

    /// Rasterize every visitor's clips at the pet's scale (vanilla art only).
    pub fn rescale(&mut self, vanilla: Option<&Skin>, scale: f32) -> Result<()> {
        self.ppu = 64.0 * scale;
        let Some(skin) = vanilla else {
            log::warn!("visitors: no vanilla skin export; visitors stay invisible");
            return Ok(());
        };
        let keys: Vec<Vec<SpriteKey>> = (0..self.set.defs.len())
            .map(|i| {
                let lib = self.set.lib(i);
                let mut k: Vec<SpriteKey> = lib.iter().flat_map(|(_, c)| c.frames.iter().filter_map(|f| f.sprite)).collect();
                k.sort_by_key(|k| (k.collection, k.sprite));
                k.dedup();
                k
            })
            .collect();
        let sets: Vec<(&[SpriteKey], f32)> = keys.iter().map(|k| (k.as_slice(), self.ppu)).collect();
        let t = std::time::Instant::now();
        self.frames = self.meta.rasterize_sets(skin, None, &sets)?;
        self.metrics = (0..self.set.defs.len())
            .map(|i| {
                let idle = self.set.lib(i).get(self.set.clips[i].idle);
                let f = idle.frames.first().and_then(|f| f.sprite).and_then(|k| self.frames[i].get(&k));
                let below = f.map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
                let above = f.map_or(2.0 * self.ppu, |f| f.anchor[1]);
                let origin_above_feet = self.set.defs[i].origin_above_feet.map_or(below, |u| u * self.ppu);
                Metrics { origin_above_feet, head: origin_above_feet + above }
            })
            .collect();
        for l in &mut self.live {
            l.drawn = None;
        }
        self.bubbles.rescale(scale);
        log::info!("visitors: art at x{scale} in {:?}", t.elapsed());
        Ok(())
    }

    /// Ask for a visit now (debug / menu). `name` picks the visitor.
    pub fn invite(&mut self, name: Option<String>) {
        self.invited = Some(name);
    }

    /// For the brain.
    pub fn percepts(&self) -> Vec<Visitor> {
        self.live.iter().filter(|l| l.npc.phase() != NpcPhase::Gone).map(|l| l.npc.percept(&self.set, self.ppu)).collect()
    }

    pub fn present(&self) -> bool {
        !self.live.is_empty() || self.portals.iter().any(|p| p.guest.is_some())
    }

    /// The brain stands beside visitor `id`: start the conversation.
    pub fn greet(&mut self, id: u32, w: &World) {
        let Some(i) = self.live.iter().position(|l| l.npc.id == id) else { return };
        if !self.live[i].npc.can_talk() {
            return;
        }
        let ctx = TalkContext::from_desktop(w.desk, Some(w.hour));
        let def = self.live[i].npc.def;
        let Some(lines) = ctx.pick(&self.set.defs[def].conversations, &mut self.rng) else { return };
        let mut events = Vec::new();
        if self.live[i].npc.begin_talk(lines, w.knight.pos.x, &self.set, &mut events) {
            log::info!("visitor: {} talks with the knight", self.set.defs[def].display);
            self.handle_events(i, events, w);
        }
    }

    /// Hide everything (games on every monitor); the next tick shows it again.
    pub fn hide_all(&mut self) {
        if !self.hidden {
            self.hidden = true;
            for l in &self.live {
                l.win.set_visible(false);
            }
            self.bubbles.set_visible(false);
        }
    }

    pub fn raise_all(&self) {
        for l in &self.live {
            l.win.raise();
        }
        self.bubbles.raise_all();
    }

    pub fn tick(&mut self, dt: f32, w: &World, fx: &mut Fx) {
        if self.hidden {
            self.hidden = false;
            for l in &self.live {
                l.win.set_visible(true);
            }
            self.bubbles.set_visible(true);
        }
        // Games: nothing of ours on their monitors.
        let on_game = |p: Vec2| w.game_monitors.iter().any(|r| r.contains(p.x as i32, (p.y - 1.0) as i32));
        let before = self.live.len();
        for l in self.live.iter().filter(|l| on_game(l.npc.pos)) {
            if let Some(b) = l.bubble {
                self.bubbles.close(b);
            }
        }
        self.live.retain(|l| !on_game(l.npc.pos));
        if self.live.len() != before {
            log::info!("visitor: a game took their monitor; they left");
        }
        self.portals.retain(|p| !on_game(p.portal.spot.at));
        self.bubbles.drop_on(w.game_monitors);

        // 1. Schedule.
        let cond = VisitConditions {
            user_idle_secs: w.user_idle_secs,
            keys_per_sec: w.keys_per_sec,
            fullscreen: w.fullscreen,
            visitor_present: self.present(),
            knight_busy: w.knight_busy,
        };
        let due = self.sched.tick(dt, &cond, &mut self.rng);
        // Invitations wait for the nav graph (placement must be reachable).
        if w.graph.is_some()
            && let Some(name) = self.invited.take()
        {
            if !self.present() {
                self.spawn(name.as_deref(), w, fx);
            }
            self.sched.schedule_next(&mut self.rng);
        } else if due {
            self.spawn(None, w, fx);
        }

        // 2. Portals: glow, ripple, guest steps out.
        let mut emerged = Vec::new();
        for p in &mut self.portals {
            let phase = p.portal.tick(dt);
            if p.portal.take_emerge()
                && let Some(g) = p.guest.take()
            {
                emerged.push(g);
            }
            if phase == PortalPhase::Closed {
                fx.stop("portal_glow");
            }
        }
        self.portals.retain(|p| p.portal.phase() != PortalPhase::Closed);
        for (def, at) in emerged {
            self.add_npc(def, at, w);
        }

        // 3. Visitors.
        for i in 0..self.live.len() {
            let mut events = Vec::new();
            let area = self.area_at(self.live[i].npc.pos, w.desk);
            let env = NpcEnv { terrain: w.terrain, knight: w.knight.pos, ppu: self.ppu, area };
            self.live[i].npc.tick(dt, &self.set, &env, &mut events);
            self.handle_events(i, events, w);
            if let Some(b) = self.live[i].bubble {
                let anchor = self.head(&self.live[i].npc);
                self.bubbles.follow(b, anchor);
            }
        }
        for p in self.live.iter().filter(|l| l.npc.phase() == NpcPhase::Gone) {
            log::info!("visitor: {} left", self.set.defs[p.npc.def].display);
        }
        self.live.retain(|l| l.npc.phase() != NpcPhase::Gone);
        for at in std::mem::take(&mut self.pending_fx) {
            fx.spawn("portal_glow", at, false);
            fx.spawn("portal_ripple", at, false);
        }
        if let Err(e) = self.draw() {
            log::warn!("visitors: {e:#}");
        }

        // 4. Dream-nail thoughts.
        if let Some(t) = self.thought_in.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.thought_in = None;
                self.dream_thought(w);
            }
        }
        self.bubbles.update(dt);
    }

    /// Watch the knight's clip: the dream nail's slash spawns its effect, then a thought.
    pub fn knight_clip(&mut self, clip: &str, origin: Vec2, facing_right: bool, fx: &mut Fx) {
        let slash = clip == SLASH_CLIP;
        if slash && !self.knight_clip_was_slash {
            fx.spawn("dn_slash", origin, facing_right);
            self.thought_in = Some(0.45);
        }
        self.knight_clip_was_slash = slash;
    }

    /// Pick a visitor and where they appear, then start their arrival.
    fn spawn(&mut self, name: Option<&str>, w: &World, fx: &mut Fx) {
        let def = match name.and_then(|n| self.set.id(n)) {
            Some(d) => d,
            None => {
                let items: Vec<(usize, f32)> =
                    self.set.defs.iter().enumerate().map(|(i, d)| (i, if Some(i) == self.last_def && self.set.defs.len() > 1 { 0.0 } else { d.weight })).collect();
                match self.rng.pick_weighted(&items) {
                    Some(d) => d,
                    None => return,
                }
            }
        };
        let footings: Vec<Footing> = match w.graph {
            Some(g) => g.surfaces().iter().map(|s| Footing { x0: s.x0, x1: s.x1, y: s.y, owner: s.owner }).collect(),
            None => w.terrain.ledges.iter().map(|l| Footing { x0: l.x0, x1: l.x1, y: l.y, owner: l.owner }).collect(),
        };
        let knight = w.knight;
        let mut reachable = |p: Vec2| w.graph.is_none_or(|g| g.plan(knight, NavGoal::Point(p), TravelStyle::Hurry).is_some());
        let arrive = self.set.defs[def].arrive;
        let ledges: Vec<Footing> = w.terrain.ledges.iter().map(|l| Footing { x0: l.x0, x1: l.x1, y: l.y, owner: l.owner }).collect();
        // Nav surfaces first (reachable by construction), raw ledges as a fallback.
        let monitors: Vec<_> = w.desk.monitors.iter().filter(|m| !w.game_monitors.contains(&m.rect)).cloned().collect();
        let at = plan_placement(arrive, &footings, knight.pos, &monitors, &mut self.rng, &mut reachable)
            .or_else(|| plan_placement(arrive, &ledges, knight.pos, &monitors, &mut self.rng, &mut reachable));
        let Some(at) = at else {
            log::info!("visitor: no good spot for {} right now", self.set.defs[def].display);
            return;
        };
        self.last_def = Some(def);
        log::info!("visitor: {} arrives ({arrive:?}) at ({:.0},{:.0})", self.set.defs[def].display, at.feet.x, at.feet.y);
        match arrive {
            Arrival::Portal | Arrival::Drop => {
                let lift = if arrive == Arrival::Drop { 2.5 * self.ppu } else { 0.0 };
                let base = at.feet - Vec2::new(0.0, lift);
                fx.spawn("portal_glow", base, false);
                fx.spawn("portal_ripple", base, false);
                self.portals.push(OpenPortal { portal: Portal::open(PortalSpot { at: base, owner: at.owner }), guest: Some((def, at)) });
            }
            Arrival::Perch | Arrival::Walk => self.add_npc(def, at, w),
        }
    }

    fn add_npc(&mut self, def: usize, at: Placement, w: &World) {
        let win = match SpriteWindow::new() {
            Ok(win) => win,
            Err(e) => {
                log::warn!("visitor window: {e:#}");
                return;
            }
        };
        win.set_click_through(true);
        win.set_capturable(self.capturable);
        let id = self.next_id;
        self.next_id += 1;
        let mut npc = Npc::new(id, def, &self.set, at, self.rng.next_u32(), self.ppu);
        let area = self.area_at(at.feet, w.desk);
        npc.attach(&NpcEnv { terrain: w.terrain, knight: w.knight.pos, ppu: self.ppu, area });
        self.live.push(LiveNpc { npc, win, drawn: None, bubble: None });
    }

    fn handle_events(&mut self, i: usize, events: Vec<NpcEvent>, w: &World) {
        for e in events {
            match e {
                NpcEvent::Say { text, secs } => {
                    if let Some(b) = self.live[i].bubble.take() {
                        self.bubbles.close(b);
                    }
                    let npc = &self.live[i].npc;
                    let anchor = self.head(npc);
                    let area = self.work_at(npc.pos, w.desk);
                    self.live[i].bubble = self.bubbles.say(anchor, &text, BubbleStyle::Speech, secs + 0.3, area);
                }
                NpcEvent::LineDone => {
                    if let Some(b) = self.live[i].bubble.take() {
                        self.bubbles.close(b);
                    }
                }
                NpcEvent::ConversationOver => log::info!("visitor: conversation with {} over", self.set.defs[self.live[i].npc.def].display),
                NpcEvent::OpenPortal(at) => {
                    self.portals.push(OpenPortal { portal: Portal::open(PortalSpot { at, owner: None }), guest: None });
                    self.pending_fx.push(at);
                }
                NpcEvent::Gone => {
                    if let Some(b) = self.live[i].bubble.take() {
                        self.bubbles.close(b);
                    }
                }
            }
        }
    }

    fn head(&self, npc: &Npc) -> Vec2 {
        let m = self.metrics.get(npc.def).copied().unwrap_or_default();
        Vec2::new(npc.pos.x, npc.pos.y - m.head)
    }

    fn area_at(&self, p: Vec2, desk: &DesktopSnapshot) -> RectF {
        self.work_at(p, desk).to_f()
    }

    fn work_at(&self, p: Vec2, desk: &DesktopSnapshot) -> RectI {
        desk.monitor_at(p.x as i32, (p.y - 1.0) as i32).or(desk.primary()).map_or(RectI::new(0, 0, 1920, 1080), |m| m.work)
    }

    fn draw(&mut self) -> Result<()> {
        for l in &mut self.live {
            let def = l.npc.def;
            let Some(key) = l.npc.sprite(&self.set) else { continue };
            let Some(frame) = self.frames.get(def).and_then(|f| f.get(&key)) else { continue };
            let flip = l.npc.flipped(&self.set);
            let m = self.metrics[def];
            let origin = Vec2::new(l.npc.pos.x, l.npc.pos.y - m.origin_above_feet);
            let x = (origin.x - frame.anchor_x(flip)).round() as i32;
            let y = (origin.y - frame.anchor[1]).round() as i32;
            let alpha = (l.npc.alpha.clamp(0.0, 1.0) * 255.0) as u8;
            if l.drawn != Some((key, flip, alpha)) {
                if alpha == 255 {
                    l.win.present_flipped(&frame.image, x, y, flip)?;
                } else {
                    l.win.present_flipped(&faded(&frame.image, alpha), x, y, flip)?;
                }
                l.drawn = Some((key, flip, alpha));
            } else {
                l.win.move_to(x, y)?;
            }
        }
        Ok(())
    }

    /// Thought bubble for the window the knight just dream-nailed (the one under him, else
    /// the nearest one around him).
    fn dream_thought(&mut self, w: &World) {
        let k = w.knight;
        let owner = k.support.and_then(|s| s.owner);
        let near = |win: &&WindowInfo| {
            let r = win.rect.to_f();
            let dx = (k.pos.x - r.left.max(k.pos.x.min(r.right))).abs();
            let dy = (k.pos.y - r.top.max(k.pos.y.min(r.bottom))).abs();
            dx < 200.0 && dy < 200.0
        };
        let target = owner.and_then(|id| w.desk.window(id)).or_else(|| w.desk.windows.iter().filter(|w| !w.title.trim().is_empty()).find(near));
        let Some(win) = target else {
            log::info!("dream nail: nothing to dream about here");
            return;
        };
        let Some(text) = self.set.dream.thought(win, Some(w.hour), &mut self.rng) else { return };
        log::info!("dream nail: '{}' ({}) thinks \"{text}\"", win.title, win.process);
        let head = Vec2::new(k.pos.x, k.pos.y - 2.4 * self.ppu);
        let area = self.work_at(k.pos, w.desk);
        let secs = skong_core::bubble::read_secs(&text) + 1.0;
        self.bubbles.say(head, &text, BubbleStyle::Thought, secs, area);
    }
}
