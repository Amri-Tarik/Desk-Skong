//! The Troupe Master's friendly visit: rare, never hostile. He appears beside the knight in
//! a teleport flash with a flourish of his cape, bows, says something theatrical, then
//! teleports around him a couple of times (each time striking a pose), says farewell, bows
//! and vanishes. Sometimes — only for a knight who could already find the secret — he
//! leaves his scarlet flame behind, a hint.
//!
//! The knight watches (the brain's `Scene` behaviour with a [`SceneCue::Watch`]): a startle
//! at the entrance, a bow for his bow, his gaze following each teleport, a nod goodbye.
use crate::ai::SceneCue;
use crate::ai::rng::Rng;
use crate::anim::{Animator, ClipId};
use crate::assets::SpriteKey;
use crate::geom::{RectF, Vec2};

use super::{GrimmEnv, GrimmSet, LineKind, ground_under};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// Tele In, fading in.
    Enter,
    /// The cape swirl after the teleport.
    Flourish,
    Bow,
    /// His line (a hand flourish, then standing while it's read).
    Taunt,
    /// Tele Out; `last`: off for good.
    Out { last: bool },
    Hidden,
    In,
    /// A pose after a teleport.
    Pose,
    Farewell,
    Gone,
}

/// What the cameo did this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameoTick {
    pub say: Option<LineKind>,
    /// Teleport flashes (body centres).
    pub flashes: Vec<Vec2>,
    /// He just left; `true`: his scarlet flame stays behind.
    pub left: Option<bool>,
}

/// The friendly visit (see the module docs).
pub struct Cameo {
    /// Feet point.
    pub pos: Vec2,
    pub facing_right: bool,
    pub alpha: f32,
    anim: Animator,
    stage: Stage,
    t: f32,
    len: f32,
    hops_left: u32,
    /// Side of the knight he stands on (±1).
    side: f32,
    /// Performance beats so far (the knight answers each once).
    beat: u32,
    react: Option<&'static str>,
    rng: Rng,
    /// He'll leave the scarlet flame behind.
    pub leaves_flame: bool,
    pub serial: u32,
    ppu: f32,
}

impl Cameo {
    /// He appears at `at` (feet), facing `toward` (x), for `hops` teleports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(set: &GrimmSet, at: Vec2, toward: f32, hops: u32, leaves_flame: bool, serial: u32, seed: u32, ppu: f32) -> Self {
        let clip = set.clips.tele_in;
        let mut anim = Animator::new(clip);
        anim.play(&set.lib, clip, true);
        Self {
            pos: at,
            facing_right: toward > at.x,
            alpha: 0.0,
            anim,
            stage: Stage::Enter,
            t: 0.0,
            len: 0.3,
            hops_left: hops,
            side: if at.x >= toward { 1.0 } else { -1.0 },
            beat: 1,
            react: Some("startled|look_up"),
            rng: Rng::new(seed),
            leaves_flame,
            serial,
            ppu,
        }
    }

    pub fn sprite(&self, set: &GrimmSet) -> Option<SpriteKey> {
        self.anim.current(&set.lib)
    }

    pub fn clip(&self) -> ClipId {
        self.anim.clip()
    }

    pub fn gone(&self) -> bool {
        self.stage == Stage::Gone
    }

    /// Leaving (or gone): no more beats.
    pub fn leaving(&self) -> bool {
        matches!(self.stage, Stage::Out { last: true } | Stage::Gone)
    }

    pub fn body(&self, set: &GrimmSet, ppu: f32) -> RectF {
        RectF::from_feet(self.pos, Vec2::new(set.cfg.size[0], set.cfg.size[1]) * ppu)
    }

    /// The knight's part: watch him, answer each beat.
    pub fn cue(&self, set: &GrimmSet) -> SceneCue {
        SceneCue::Watch { serial: self.serial, at: self.body(set, self.ppu).center(), beat: self.beat, react: self.react.map(str::to_string) }
    }

    /// Off at once (a fullscreen app, a visitor, enemies...): a quick teleport out, no flame.
    pub fn bow_out(&mut self, set: &GrimmSet) {
        if !self.leaving() {
            self.leaves_flame = false;
            self.go(set, Stage::Out { last: true }, set.clips.tele_out, 0.3);
        }
    }

    fn go(&mut self, set: &GrimmSet, stage: Stage, clip: ClipId, len: f32) {
        self.stage = stage;
        self.t = 0.0;
        self.len = len;
        self.anim.play(&set.lib, clip, true);
    }

    fn face(&mut self, x: f32) {
        if (x - self.pos.x).abs() > 2.0 {
            self.facing_right = x > self.pos.x;
        }
    }

    fn beat(&mut self, react: &'static str) {
        self.beat += 1;
        self.react = Some(react);
    }

    /// The next spot: on the knight's ground line, the other side of him, 3–6 units away
    /// (inside the stage).
    fn hop_spot(&mut self, env: &GrimmEnv, knight: Vec2) -> Vec2 {
        let u = env.ppu;
        let (gy, x0, x1) = ground_under(env.terrain, knight, env.area);
        self.side = -self.side;
        let off = self.rng.range(3.0, 6.0) * u;
        let mut x = knight.x + self.side * off;
        let (lo, hi) = (x0.max(env.area.left) + u, x1.min(env.area.right) - u);
        if !(lo..=hi).contains(&x) {
            self.side = -self.side;
            x = knight.x + self.side * off;
        }
        Vec2::new(x.clamp(lo, hi.max(lo)), gy)
    }

    pub fn tick(&mut self, dt: f32, set: &GrimmSet, env: &GrimmEnv) -> CameoTick {
        let mut out = CameoTick::default();
        self.ppu = env.ppu;
        let mut ev = Vec::new();
        self.anim.update(&set.lib, dt, &mut ev);
        self.t += dt;
        let c = set.clips.clone();
        let u = env.ppu;
        let knight = env.knight.map_or(self.pos, |k| Vec2::new(k.center().x, k.bottom));
        if self.stage == Stage::Enter && self.t <= dt + 1e-6 {
            out.flashes.push(self.body(set, u).center());
        }
        match self.stage {
            Stage::Enter => {
                self.alpha = (self.t / 0.25).min(1.0);
                self.face(knight.x);
                if self.t >= self.len {
                    self.alpha = 1.0;
                    self.go(set, Stage::Flourish, c.tele_to_idle, 1.5);
                }
            }
            Stage::Flourish => {
                if self.t >= self.len {
                    self.face(knight.x);
                    self.beat("challenge");
                    self.go(set, Stage::Bow, c.bow, 0.9);
                }
            }
            Stage::Bow => {
                if self.t >= self.len {
                    out.say = Some(LineKind::Cameo);
                    self.go(set, Stage::Taunt, c.hand_out, 3.6);
                }
            }
            Stage::Taunt => {
                if self.t >= 0.7 && self.anim.clip() == c.hand_out {
                    self.anim.play(&set.lib, c.idle, false);
                }
                if self.t >= self.len {
                    self.go(set, Stage::Out { last: false }, c.tele_out, 0.3);
                }
            }
            Stage::Out { last } => {
                self.alpha = (1.0 - self.t / 0.3).max(0.0);
                if self.t >= self.len {
                    self.alpha = 0.0;
                    if last {
                        out.left = Some(self.leaves_flame);
                        self.stage = Stage::Gone;
                    } else {
                        self.stage = Stage::Hidden;
                        self.t = 0.0;
                        self.len = self.rng.range(0.35, 0.7);
                    }
                }
            }
            Stage::Hidden => {
                if self.t >= self.len {
                    self.pos = self.hop_spot(env, knight);
                    self.face(knight.x);
                    self.hops_left = self.hops_left.saturating_sub(1);
                    self.beat("glance|look_up");
                    self.go(set, Stage::In, c.tele_in, 0.25);
                    out.flashes.push(self.body(set, u).center());
                }
            }
            Stage::In => {
                self.alpha = (self.t / 0.25).min(1.0);
                if self.t >= self.len {
                    self.alpha = 1.0;
                    let (clip, len) = if self.rng.chance(0.5) { (c.look_back, 1.4) } else { (c.hand_click, 1.1) };
                    self.go(set, Stage::Pose, clip, len);
                }
            }
            Stage::Pose => {
                if self.t >= 0.6 && self.anim.is_finished() {
                    self.anim.play(&set.lib, c.idle, false);
                }
                if self.t >= self.len {
                    if self.hops_left > 0 {
                        self.go(set, Stage::Out { last: false }, c.tele_out, 0.3);
                    } else {
                        self.face(knight.x);
                        out.say = Some(LineKind::CameoBye);
                        self.beat("nod|challenge");
                        self.go(set, Stage::Farewell, c.bow, 3.0);
                    }
                }
            }
            Stage::Farewell => {
                if self.t >= 0.9 && self.anim.clip() == c.bow {
                    self.anim.play(&set.lib, c.bow_return, true);
                }
                if self.t >= self.len {
                    self.go(set, Stage::Out { last: true }, c.tele_out, 0.3);
                }
            }
            Stage::Gone => {}
        }
        out
    }
}
