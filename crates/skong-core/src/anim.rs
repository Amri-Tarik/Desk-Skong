//! Clip player for exported tk2d sprite animation libraries.
//!
//! Wrap modes follow tk2d semantics:
//! * `Loop` — cycle all frames.
//! * `LoopSection` — play frames `0..loop_start` once, then cycle `loop_start..n`.
//! * `Once` — play through, hold the last frame, report [`AnimEvent::Finished`].
//! * `PingPong` — 0..n-1..1 repeating.
//! * `RandomFrame` — pick one random frame at start and hold it.
//! * `RandomLoop` — a random frame each tick.
//! * `Single` — frame 0 only.
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::assets::{AnimLibraryJson, EventJson, SpriteBank, SpriteKey, read_json};

#[derive(Deserialize)]
struct ExtraFile {
    clips: Vec<ExtraClip>,
    /// Clip -> sprite -> seat-line correction in px at 64 px/unit (+ = down).
    #[serde(default)]
    frame_offsets: HashMap<String, HashMap<String, f32>>,
}

#[derive(Deserialize)]
struct ExtraClip {
    name: String,
    #[serde(default = "knight")]
    collection: String,
    fps: f32,
    #[serde(default)]
    loop_start: u32,
    wrap_mode: String,
    sprites: Vec<String>,
}

fn knight() -> String {
    "Knight".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapMode {
    Loop,
    LoopSection,
    Once,
    PingPong,
    RandomFrame,
    RandomLoop,
    Single,
}

impl WrapMode {
    fn parse(s: &str) -> WrapMode {
        match s {
            "Loop" => WrapMode::Loop,
            "LoopSection" => WrapMode::LoopSection,
            "PingPong" => WrapMode::PingPong,
            "RandomFrame" => WrapMode::RandomFrame,
            "RandomLoop" => WrapMode::RandomLoop,
            "Single" => WrapMode::Single,
            _ => WrapMode::Once,
        }
    }

    pub fn loops(self) -> bool {
        !matches!(self, WrapMode::Once | WrapMode::Single | WrapMode::RandomFrame)
    }
}

#[derive(Clone, Debug)]
pub struct ClipFrame {
    /// `None` when the frame's collection wasn't exported.
    pub sprite: Option<SpriteKey>,
    pub event: Option<EventJson>,
    /// Vertical correction in game units (y down) applied while the knight is seated, so
    /// every seated pose meets the seat at the same line (see `frame_offsets`).
    pub dy: f32,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub fps: f32,
    pub loop_start: usize,
    pub wrap: WrapMode,
    pub frames: Vec<ClipFrame>,
}

impl Clip {
    /// Duration of one pass in seconds (intro + one loop for LoopSection).
    pub fn duration(&self) -> f32 {
        self.frames.len() as f32 / self.fps.max(0.001)
    }
}

/// Handle to a clip inside a [`ClipLibrary`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipId(pub u16);

#[derive(Debug, Default)]
pub struct ClipLibrary {
    pub name: String,
    clips: Vec<Clip>,
    by_name: HashMap<String, ClipId>,
}

impl ClipLibrary {
    /// Load `<data>/animations/<file>`, resolving collections through `bank`.
    pub fn load(path: &Path, bank: &SpriteBank) -> Result<ClipLibrary> {
        let raw: AnimLibraryJson = read_json(path)?;
        let mut lib = ClipLibrary { name: raw.name, ..Default::default() };
        for c in raw.clips {
            let frames = c
                .frames
                .into_iter()
                .map(|f| ClipFrame {
                    sprite: bank.collection_index(&f.collection).map(|collection| SpriteKey { collection, sprite: f.sprite_id }),
                    event: f.event,
                    dy: 0.0,
                })
                .collect();
            let id = ClipId(lib.clips.len() as u16);
            lib.by_name.entry(c.name.clone()).or_insert(id);
            lib.clips.push(Clip {
                name: c.name,
                fps: c.fps.max(0.001),
                loop_start: c.loop_start as usize,
                wrap: WrapMode::parse(&c.wrap_mode),
                frames,
            });
        }
        let extra = path.with_extension("extra.json");
        if extra.exists() {
            lib.load_extra(&extra, bank).with_context(|| format!("{}", extra.display()))?;
        }
        Ok(lib)
    }

    /// Hand-authored clips (`<Library>.extra.json` next to the exported library) built
    /// from exported sprites by name: poses the game draws but never sequences into a
    /// clip of its own (e.g. a seated breath or head turn).
    fn load_extra(&mut self, path: &Path, bank: &SpriteBank) -> Result<()> {
        let raw: ExtraFile = read_json(path)?;
        for c in raw.clips {
            if self.by_name.contains_key(&c.name) {
                bail!("extra clip '{}' already exists", c.name);
            }
            let frames = c
                .sprites
                .iter()
                .map(|s| {
                    let key = bank.sprite_key(&c.collection, s).with_context(|| format!("clip '{}': no sprite '{s}' in {}", c.name, c.collection))?;
                    Ok(ClipFrame { sprite: Some(key), event: None, dy: 0.0 })
                })
                .collect::<Result<Vec<_>>>()?;
            self.by_name.insert(c.name.clone(), ClipId(self.clips.len() as u16));
            self.clips.push(Clip { name: c.name, fps: c.fps.max(0.001), loop_start: c.loop_start as usize, wrap: WrapMode::parse(&c.wrap_mode), frames });
        }
        for (clip, sprites) in raw.frame_offsets {
            let id = self.id(&clip).with_context(|| format!("frame_offsets: no clip '{clip}'"))?;
            for (sprite, px) in &sprites {
                let mut hit = false;
                for f in &mut self.clips[id.0 as usize].frames {
                    if f.sprite.and_then(|k| bank.sprite_name(k)) == Some(sprite.as_str()) {
                        f.dy = px / 64.0;
                        hit = true;
                    }
                }
                if !hit {
                    bail!("frame_offsets: clip '{clip}' has no frame '{sprite}'");
                }
            }
        }
        Ok(())
    }

    pub fn id(&self, name: &str) -> Option<ClipId> {
        self.by_name.get(name).copied()
    }

    pub fn get(&self, id: ClipId) -> &Clip {
        &self.clips[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.clips.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ClipId, &Clip)> {
        self.clips.iter().enumerate().map(|(i, c)| (ClipId(i as u16), c))
    }

    /// Append every clip of `other` under `prefix` + its name (a character's second
    /// library, e.g. Zote's fight clips next to his visitor clips). Names already taken
    /// keep the existing clip.
    pub fn merge(&mut self, other: ClipLibrary, prefix: &str) {
        for mut c in other.clips {
            c.name = format!("{prefix}{}", c.name);
            if self.by_name.contains_key(&c.name) {
                continue;
            }
            self.by_name.insert(c.name.clone(), ClipId(self.clips.len() as u16));
            self.clips.push(c);
        }
    }

    /// Every sprite referenced by any clip (for rasterizing up front).
    pub fn sprite_keys(&self) -> Vec<SpriteKey> {
        let mut v: Vec<SpriteKey> = self.clips.iter().flat_map(|c| c.frames.iter().filter_map(|f| f.sprite)).collect();
        v.sort_by_key(|k| (k.collection, k.sprite));
        v.dedup();
        v
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnimEvent {
    /// A frame flagged `triggerEvent` in the game data was entered.
    Frame { clip: ClipId, frame: usize, event: EventJson },
    /// A non-looping clip reached its end.
    Finished(ClipId),
}

/// Plays one clip at a time. Deterministic (own xorshift RNG for random wrap modes).
#[derive(Clone, Debug)]
pub struct Animator {
    clip: ClipId,
    /// Frames elapsed since the clip started (fractional).
    t: f32,
    frame: usize,
    finished: bool,
    speed: f32,
    rng: u32,
}

impl Animator {
    pub fn new(clip: ClipId) -> Self {
        Self { clip, t: 0.0, frame: 0, finished: false, speed: 1.0, rng: 0x9e37_79b9 }
    }

    pub fn clip(&self) -> ClipId {
        self.clip
    }

    pub fn frame_index(&self) -> usize {
        self.frame
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Playback speed multiplier (1 = clip fps).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
    }

    /// Switch clip. No-op if already playing it, unless `restart`.
    pub fn play(&mut self, lib: &ClipLibrary, clip: ClipId, restart: bool) {
        if clip == self.clip && !restart {
            return;
        }
        self.clip = clip;
        self.t = 0.0;
        self.finished = false;
        let c = lib.get(clip);
        self.frame = match c.wrap {
            WrapMode::RandomFrame | WrapMode::RandomLoop => self.random(c.frames.len()),
            _ => 0,
        };
    }

    pub fn current(&self, lib: &ClipLibrary) -> Option<SpriteKey> {
        lib.get(self.clip).frames.get(self.frame).and_then(|f| f.sprite)
    }

    /// Seat-line correction of the current frame (game units, y down; see [`ClipFrame::dy`]).
    pub fn seat_dy(&self, lib: &ClipLibrary) -> f32 {
        lib.get(self.clip).frames.get(self.frame).map_or(0.0, |f| f.dy)
    }

    /// Advance by `dt` seconds, pushing events. Returns true if the displayed frame changed.
    pub fn update(&mut self, lib: &ClipLibrary, dt: f32, events: &mut Vec<AnimEvent>) -> bool {
        let c = lib.get(self.clip);
        let n = c.frames.len();
        if n == 0 || self.finished {
            return false;
        }
        let prev_whole = self.t as usize;
        self.t += dt * c.fps * self.speed;
        let whole = self.t as usize;
        if whole == prev_whole {
            return false;
        }
        let old = self.frame;
        // Visit every crossed frame so events on skipped frames still fire.
        for step in prev_whole + 1..=whole {
            let Some(f) = self.frame_at(c, step) else {
                self.finished = true;
                events.push(AnimEvent::Finished(self.clip));
                break;
            };
            self.frame = f;
            if let Some(ev) = &c.frames[f].event {
                events.push(AnimEvent::Frame { clip: self.clip, frame: f, event: ev.clone() });
            }
        }
        self.frame != old
    }

    /// Frame for absolute step `k`, or `None` once a non-looping clip has ended.
    fn frame_at(&mut self, c: &Clip, k: usize) -> Option<usize> {
        let n = c.frames.len();
        match c.wrap {
            WrapMode::Loop => Some(k % n),
            WrapMode::LoopSection => {
                let ls = c.loop_start.min(n - 1);
                Some(if k < ls { k } else { ls + (k - ls) % (n - ls) })
            }
            WrapMode::Once => (k < n).then_some(k),
            WrapMode::PingPong => {
                if n == 1 {
                    return Some(0);
                }
                let p = k % (2 * n - 2);
                Some(if p < n { p } else { 2 * n - 2 - p })
            }
            WrapMode::RandomLoop => Some(self.random(n)),
            WrapMode::RandomFrame | WrapMode::Single => None,
        }
    }

    fn random(&mut self, n: usize) -> usize {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as usize) % n.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(wrap: WrapMode, n: usize, loop_start: usize) -> ClipLibrary {
        let frames = (0..n)
            .map(|i| ClipFrame {
                sprite: Some(SpriteKey { collection: 0, sprite: i as u32 }),
                event: (i == 2).then(EventJson::default),
                dy: 0.0,
            })
            .collect();
        let mut l = ClipLibrary::default();
        l.clips.push(Clip { name: "c".into(), fps: 10.0, loop_start, wrap, frames });
        l.by_name.insert("c".into(), ClipId(0));
        l
    }

    fn run(l: &ClipLibrary, ticks: usize) -> (Vec<usize>, Vec<AnimEvent>) {
        let mut a = Animator::new(ClipId(0));
        let mut ev = Vec::new();
        let mut seen = vec![a.frame_index()];
        for _ in 0..ticks {
            a.update(l, 0.1, &mut ev);
            seen.push(a.frame_index());
        }
        (seen, ev)
    }

    #[test]
    fn loop_section_plays_intro_once() {
        let (seen, _) = run(&lib(WrapMode::LoopSection, 5, 2), 8);
        assert_eq!(seen, vec![0, 1, 2, 3, 4, 2, 3, 4, 2]);
    }

    #[test]
    fn once_holds_last_and_finishes() {
        let (seen, ev) = run(&lib(WrapMode::Once, 3, 0), 5);
        assert_eq!(seen, vec![0, 1, 2, 2, 2, 2]);
        assert!(ev.contains(&AnimEvent::Finished(ClipId(0))));
        assert_eq!(ev.iter().filter(|e| matches!(e, AnimEvent::Finished(_))).count(), 1);
    }

    #[test]
    fn ping_pong() {
        let (seen, _) = run(&lib(WrapMode::PingPong, 3, 0), 6);
        assert_eq!(seen, vec![0, 1, 2, 1, 0, 1, 2]);
    }

    #[test]
    fn events_fire_even_when_frames_skipped() {
        let l = lib(WrapMode::Once, 5, 0);
        let mut a = Animator::new(ClipId(0));
        let mut ev = Vec::new();
        a.update(&l, 0.35, &mut ev); // jumps from frame 0 to 3
        assert_eq!(a.frame_index(), 3);
        assert!(matches!(ev[0], AnimEvent::Frame { frame: 2, .. }));
    }

    #[test]
    fn real_knight_library_loads() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/hk");
        let bank = SpriteBank::load_metadata(&data).expect("bank");
        let l = ClipLibrary::load(&data.join("animations/Knight.json"), &bank).expect("lib");
        let run = l.get(l.id("Run").expect("Run clip"));
        assert_eq!(run.wrap, WrapMode::LoopSection);
        assert!(l.id("Sit Map Open").is_some());
        assert!(l.sprite_keys().len() > 500);
    }
}
