//! Desk-Skong — currently a clip/skin browser for the new renderer.
//!
//! Global hotkeys (Ctrl+Alt+…): ←/→ clip, ↑/↓ skin, F flip, R restart clip,
//! C click-through, Q quit.
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use skong_core::anim::{AnimEvent, Animator, ClipLibrary};
use skong_core::assets::{Skin, SpriteBank};
use skong_win::monitor;
use skong_win::runloop::{Flow, RunLoop};
use skong_win::sprite_window::SpriteWindow;
use skong_win::vk;

struct Args {
    data: PathBuf,
    skins: PathBuf,
    skin: Option<String>,
    scale: f32,
    clip: String,
}

fn parse_args() -> Result<Args> {
    let mut a = Args { data: "data/hk".into(), skins: "skins/extracted".into(), skin: None, scale: 0.6, clip: "Idle".into() };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().with_context(|| format!("missing value for {k}"));
        match k.as_str() {
            "--data" => a.data = v()?.into(),
            "--skins" => a.skins = v()?.into(),
            "--skin" => a.skin = Some(v()?),
            "--scale" => a.scale = v()?.parse()?,
            "--clip" => a.clip = v()?,
            _ => bail!("unknown argument {k}\nusage: skong [--data DIR] [--skins DIR] [--skin NAME] [--scale F] [--clip NAME]"),
        }
    }
    Ok(a)
}

const HK_NEXT_CLIP: i32 = 1;
const HK_PREV_CLIP: i32 = 2;
const HK_NEXT_SKIN: i32 = 3;
const HK_PREV_SKIN: i32 = 4;
const HK_FLIP: i32 = 5;
const HK_RESTART: i32 = 6;
const HK_CLICK_THROUGH: i32 = 7;
const HK_QUIT: i32 = 8;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    monitor::enable_dpi_awareness();
    let args = parse_args()?;

    let skins = Skin::discover(&args.skins);
    if skins.is_empty() {
        bail!("no skins (folders containing Knight.png) under {}", args.skins.display());
    }
    let mut skin_idx = args
        .skin
        .as_deref()
        .and_then(|n| skins.iter().position(|s| s.name.eq_ignore_ascii_case(n)))
        .unwrap_or(0);

    let mut bank = SpriteBank::load_metadata(&args.data)?;
    let lib = ClipLibrary::load(&args.data.join("animations/Knight.json"), &bank)?;
    let keys = lib.sprite_keys();
    let ppu = 64.0 * args.scale;
    let load_skin = |bank: &mut SpriteBank, i: usize| -> Result<()> {
        let t = Instant::now();
        bank.rasterize_skin(&skins[i], &keys, ppu)?;
        log::info!("skin '{}' ({}/{}) ready in {:?}, frames {} KiB", skins[i].name, i + 1, skins.len(), t.elapsed(), bank.frame_bytes() / 1024);
        Ok(())
    };
    load_skin(&mut bank, skin_idx)?;

    let clips: Vec<_> = lib.iter().map(|(id, _)| id).collect();
    let mut clip_idx = lib.id(&args.clip).and_then(|id| clips.iter().position(|c| *c == id)).unwrap_or(0);
    let mut anim = Animator::new(clips[clip_idx]);
    let describe = |i: usize| {
        let c = lib.get(clips[i]);
        log::info!("clip {:>3}/{}: '{}' - {} frames @ {} fps, {:?}", i + 1, clips.len(), c.name, c.frames.len(), c.fps, c.wrap);
    };
    describe(clip_idx);

    let work = monitor::primary_work_area();
    // Feet stay on the taskbar: offset from sprite origin to feet measured on Idle frame 0.
    let feet_offset = |bank: &SpriteBank| {
        lib.id("Idle")
            .and_then(|id| lib.get(id).frames.first()?.sprite)
            .and_then(|k| bank.frame(k))
            .map_or(0.0, |f| f.image.height as f32 - f.anchor[1])
    };
    let mut origin_y = work.bottom as f32 - feet_offset(&bank);
    let origin_x = (work.left + work.right) as f32 * 0.5;

    let mut win = SpriteWindow::new()?;
    let mut rl = RunLoop::new(120.0)?;
    for (id, key) in [
        (HK_NEXT_CLIP, vk::VK_RIGHT),
        (HK_PREV_CLIP, vk::VK_LEFT),
        (HK_NEXT_SKIN, vk::VK_UP),
        (HK_PREV_SKIN, vk::VK_DOWN),
        (HK_FLIP, vk::VK_F),
        (HK_RESTART, vk::VK_R),
        (HK_CLICK_THROUGH, vk::VK_C),
        (HK_QUIT, vk::VK_Q),
    ] {
        rl.register_hotkey(id, key)?;
    }
    log::info!("hotkeys: Ctrl+Alt + Left/Right clip, Up/Down skin, F flip, R restart, C click-through, Q quit");

    let mut facing_right = false;
    let mut click_through = false;
    let mut dirty = true;
    let mut events = Vec::new();
    let mut last_raise = Instant::now();

    rl.run(|tick| {
        for &hk in &tick.hotkeys {
            match hk {
                HK_NEXT_CLIP | HK_PREV_CLIP => {
                    clip_idx = if hk == HK_NEXT_CLIP { (clip_idx + 1) % clips.len() } else { (clip_idx + clips.len() - 1) % clips.len() };
                    anim.play(&lib, clips[clip_idx], true);
                    describe(clip_idx);
                }
                HK_NEXT_SKIN | HK_PREV_SKIN => {
                    skin_idx = if hk == HK_NEXT_SKIN { (skin_idx + 1) % skins.len() } else { (skin_idx + skins.len() - 1) % skins.len() };
                    load_skin(&mut bank, skin_idx)?;
                    origin_y = work.bottom as f32 - feet_offset(&bank);
                }
                HK_FLIP => facing_right = !facing_right,
                HK_RESTART => anim.play(&lib, clips[clip_idx], true),
                HK_CLICK_THROUGH => {
                    click_through = !click_through;
                    win.set_click_through(click_through);
                    log::info!("click-through {click_through}");
                }
                HK_QUIT => return Ok(Flow::Quit),
                _ => {}
            }
            dirty = true;
        }

        events.clear();
        dirty |= anim.update(&lib, tick.dt, &mut events);
        for e in &events {
            match e {
                AnimEvent::Finished(_) => {
                    // Loop one-shot clips in the browser after a short beat.
                    anim.play(&lib, clips[clip_idx], true);
                    dirty = true;
                }
                AnimEvent::Frame { frame, event, .. } => log::info!("frame event at {frame}: {event:?}"),
            }
        }

        if dirty {
            dirty = false;
            if let Some(frame) = anim.current(&lib).and_then(|k| bank.frame(k)) {
                // HK sprites face left; mirror for right.
                let mut img = frame.image.clone();
                if facing_right {
                    for row in img.px.chunks_exact_mut(img.width as usize) {
                        row.reverse();
                    }
                }
                let x = (origin_x - frame.anchor_x(facing_right)).round() as i32;
                let y = (origin_y - frame.anchor[1]).round() as i32;
                win.present(&img, x, y)?;
            }
        }
        if last_raise.elapsed().as_secs() >= 2 {
            win.raise();
            last_raise = Instant::now();
        }
        Ok(Flow::Continue)
    })
}
