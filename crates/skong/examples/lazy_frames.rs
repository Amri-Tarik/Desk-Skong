//! Knight + effect frames held eagerly (every frame unpacked, the old way) vs packed and
//! unpacked on first use (`FrameStore`): load time, process memory, first-play latency.
//! Each (mode, scale) runs in a fresh child process so heaps don't mix. Nothing is shown.
//!
//! cargo run -p skong --example lazy_frames --release -- [data/hk] [skin dir]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};
use skong_core::anim::ClipLibrary;
use skong_core::assets::{Atlas, CollectionJson, Frame, FrameStore, Skin, SpriteBank, SpriteKey, rasterize, read_json};
use skong_core::effects::EffectSet;
use skong_win::process::memory;

/// What a calm session plays most (idle, walk, run, jump / fall, land, dash, sit).
const COMMON: &[&str] = &["Idle", "Walk", "Run", "Airborne", "Fall", "Land", "Dash", "Turn", "Run To Idle", "Sit", "Sitting Asleep"];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        return child(&args[1], args[2].parse()?, Path::new(&args[3]), Path::new(&args[4]));
    }
    let data = PathBuf::from(args.first().cloned().unwrap_or_else(|| "data/hk".into()));
    let skin = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "E:/Data/Desk-Skong/skins/Vanilla".into()));
    raster_costs(&data, &skin)?;
    verify(&data, &skin)?;
    println!();
    println!("mode   scale  load ms  frames KiB (unpacked KiB)  WS / private MiB: base -> loaded -> common clips -> every clip  first play ms (median / max clip)");
    for scale in ["0.6", "1.0"] {
        for mode in ["eager", "lazy"] {
            let out = Command::new(std::env::current_exe()?).args(["--child", mode, scale]).arg(&data).arg(&skin).output()?;
            if !out.status.success() {
                anyhow::bail!("{mode} x{scale}: {}", String::from_utf8_lossy(&out.stderr));
            }
            print!("{}", String::from_utf8_lossy(&out.stdout));
        }
    }
    Ok(())
}

/// Why frames are not rasterized lazily from the atlas: decoding it again is slow, keeping
/// it (or its sprite pixels) costs more than every frame at the pet's scales (<= 1.0).
fn raster_costs(data: &Path, skin: &Path) -> Result<()> {
    let bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Knight.json"), &bank)?;
    let col: CollectionJson = read_json(&data.join("collections/Knight.json"))?;
    let ki = bank.collection_index("Knight").context("Knight")?;
    let t = Instant::now();
    let atlas = Atlas::load_png(&skin.join("Knight.png"))?;
    println!("Knight.png {}x{}: decode {:.0} ms, {} MiB decoded", atlas.width, atlas.height, ms(t), atlas.rgba.len() >> 20);
    for scale in [0.6f32, 1.0] {
        let mut times: Vec<(f64, &str)> = lib
            .iter()
            .map(|(_, c)| {
                let t = Instant::now();
                for k in c.frames.iter().filter_map(|f| f.sprite).filter(|k| k.collection == ki) {
                    std::hint::black_box(rasterize(&col.sprites[k.sprite as usize], &atlas, 64.0 * scale));
                }
                (ms(t), c.name.as_str())
            })
            .collect();
        times.sort_by(|a, b| a.0.total_cmp(&b.0));
        let q = |f: f64| times[((times.len() - 1) as f64 * f) as usize];
        println!(
            "rasterizing one clip from the decoded atlas at x{scale}: median {:.1} ms, p90 {:.1} ms, max {:.1} ms ({})",
            q(0.5).0,
            q(0.9).0,
            q(1.0).0,
            q(1.0).1
        );
    }
    Ok(())
}

/// Every real Knight frame comes back from the packed store pixel-identical.
fn verify(data: &Path, skin_dir: &Path) -> Result<()> {
    let skin = Skin::from_dir(skin_dir).context("not a skin dir")?;
    let bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Knight.json"), &bank)?;
    let keys = lib.sprite_keys();
    for scale in [0.6f32, 1.0] {
        let eager = bank.rasterize_sets(&skin, None, &[(&keys, 64.0 * scale)])?.pop().unwrap_or_default();
        let store = FrameStore::pack(eager.clone());
        let same = eager.iter().all(|(k, f)| store.get(k).is_some_and(|g| g.image.px == f.image.px && g.anchor == f.anchor && g.image.width == f.image.width));
        println!("x{scale}: {} Knight frames, packed == eager: {same}", eager.len());
        anyhow::ensure!(same, "packed frames differ");
    }
    Ok(())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn mib(b: u64) -> f64 {
    b as f64 / (1 << 20) as f64
}

enum Frames {
    Eager(Vec<HashMap<SpriteKey, Frame>>),
    Lazy(Vec<FrameStore>),
}

impl Frames {
    fn get(&self, set: usize, k: &SpriteKey) -> Option<&Frame> {
        match self {
            Frames::Eager(m) => m[set].get(k),
            Frames::Lazy(s) => s[set].get(k),
        }
    }

    fn bytes(&self) -> (usize, usize) {
        match self {
            Frames::Eager(m) => {
                let b = m.iter().flat_map(|m| m.values()).map(|f| f.image.px.len() * 4).sum();
                (b, b)
            }
            Frames::Lazy(s) => (s.iter().map(FrameStore::bytes).sum(), s.iter().map(FrameStore::unpacked_bytes).sum()),
        }
    }
}

fn child(mode: &str, scale: f32, data: &Path, skin_dir: &Path) -> Result<()> {
    let skin = Skin::from_dir(skin_dir).context("not a skin dir")?;
    let bank = SpriteBank::load_metadata(data)?;
    let lib = ClipLibrary::load(&data.join("animations/Knight.json"), &bank)?;
    let fx = EffectSet::load(&data.join("../effects.json"), &data.join("animations"), &bank)?;
    let ppu = 64.0 * scale;
    let keys = lib.sprite_keys();
    let fx_keys: Vec<(Vec<SpriteKey>, f32)> = (0..fx.defs.len()).map(|i| (fx.sprite_keys(i), fx.defs[i].raster_scale())).collect();
    let mut sets: Vec<(&[SpriteKey], f32)> = vec![(&keys, ppu)];
    sets.extend(fx_keys.iter().map(|(k, s)| (k.as_slice(), ppu * s)));
    let base = memory();

    // Load: same single pass as the app's `load_look`.
    let t = Instant::now();
    let frames = if mode == "eager" {
        Frames::Eager(bank.rasterize_sets(&skin, None, &sets)?)
    } else {
        let mut stores: Vec<FrameStore> = sets.iter().map(|_| FrameStore::default()).collect();
        bank.rasterize_each(&skin, None, &sets, |set, k, f| stores[set].insert(k, &f))?;
        Frames::Lazy(stores)
    };
    let load = ms(t);
    let loaded = memory();
    let (held, unpacked) = frames.bytes();
    let knight_held = match &frames {
        Frames::Eager(m) => m[0].values().map(|f| f.image.px.len() * 4).sum(),
        Frames::Lazy(s) => s[0].bytes(),
    };
    println!("{mode} x{scale}: knight frames hold {} KiB right after load", knight_held / 1024);

    // First play of a clip = getting each of its frames once (the animator asks for one
    // frame per tick, so the worst single frame is what a tick can feel).
    let worst_frame = std::cell::Cell::new(0.0f64);
    let play = |name: &str| -> f64 {
        let Some(id) = lib.id(name) else { return 0.0 };
        let t = Instant::now();
        for k in lib.get(id).frames.iter().filter_map(|f| f.sprite) {
            let tf = Instant::now();
            std::hint::black_box(frames.get(0, &k));
            worst_frame.set(worst_frame.get().max(ms(tf)));
        }
        ms(t)
    };
    for c in COMMON {
        play(c);
    }
    let common = memory();
    let mut firsts: Vec<(f64, String)> = lib.iter().map(|(_, c)| c.name.clone()).filter(|n| !COMMON.contains(&n.as_str())).map(|n| (play(&n), n)).collect();
    for (set, (ks, _)) in fx_keys.iter().enumerate() {
        for k in ks {
            std::hint::black_box(frames.get(set + 1, k));
        }
    }
    let every = memory();
    firsts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (med, max) = (&firsts[firsts.len() / 2], &firsts[firsts.len() - 1]);
    let wp = |m: skong_win::process::Memory| format!("{:.0}/{:.0}", mib(m.working_set), mib(m.private));
    println!(
        "{mode:<6} x{scale:<4} {load:>7.0}  {:>10} ({:>6})  {} -> {} -> {} -> {} (peak WS {:.0})  {:.3} / {:.3} ({}), worst frame {:.3}",
        held / 1024,
        unpacked / 1024,
        wp(base),
        wp(loaded),
        wp(common),
        wp(every),
        mib(every.peak_working_set),
        med.0,
        max.0,
        max.1,
        worst_frame.get()
    );
    Ok(())
}
