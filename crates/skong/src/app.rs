//! The live pet: desktop → terrain → controller → locomotion/acts → sprite window.
//!
//! Hotkeys (Ctrl+Alt+…): K manual control, A random act, W wake, R respawn at cursor,
//! S next skin, C click-through, Q quit.
use std::time::Instant;

use anyhow::{Context, Result, bail};
use skong_core::acts::{ActLibrary, ActPlayer, ActStatus};
use skong_core::anim::{AnimEvent, Animator, ClipLibrary};
use skong_core::assets::{Skin, SpriteBank, SpriteKey};
use skong_core::controller::{Abilities, Character, CharacterParams, MoveEvent, MoveInput, Side};
use skong_core::geom::Vec2;
use skong_core::locomotion::Locomotion;
use skong_core::terrain::{Terrain, TerrainOptions};
use skong_win::desktop::DesktopWatcher;
use skong_win::monitor;
use skong_win::runloop::{Flow, RunLoop};
use skong_win::sprite_window::SpriteWindow;
use skong_win::vk;

use crate::Args;
use crate::keyboard::Keyboard;
use crate::wander::Wander;

const HK_MANUAL: i32 = 1;
const HK_ACT: i32 = 2;
const HK_WAKE: i32 = 3;
const HK_RESPAWN: i32 = 4;
const HK_SKIN: i32 = 5;
const HK_CLICK_THROUGH: i32 = 6;
const HK_QUIT: i32 = 7;

/// Everything the knight is drawn from, for one skin at one scale.
struct Look {
    bank: SpriteBank,
    /// Pixels from the sprite origin down to the feet (measured on Idle frame 0).
    feet_offset: f32,
}

fn load_look(mut bank: SpriteBank, skin: &Skin, lib: &ClipLibrary, ppu: f32) -> Result<Look> {
    let t = Instant::now();
    bank.rasterize_skin(skin, &lib.sprite_keys(), ppu)?;
    let feet_offset = lib
        .id("Idle")
        .and_then(|id| lib.get(id).frames.first()?.sprite)
        .and_then(|k| bank.frame(k))
        .map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
    log::info!("skin '{}' ready in {:?} ({} KiB of frames)", skin.name, t.elapsed(), bank.frame_bytes() / 1024);
    Ok(Look { bank, feet_offset })
}

pub fn run(args: &Args) -> Result<()> {
    let skins = Skin::discover(&args.skins);
    if skins.is_empty() {
        bail!("no skins (folders containing Knight.png) under {}", args.skins.display());
    }
    let mut skin_idx = args.skin.as_deref().and_then(|n| skins.iter().position(|s| s.name.eq_ignore_ascii_case(n))).unwrap_or(0);

    let meta = SpriteBank::load_metadata(&args.data)?;
    let lib = ClipLibrary::load(&args.data.join("animations/Knight.json"), &meta)?;
    let acts = ActLibrary::load(&args.data.join("../acts.json"), &lib)?;
    let ppu = 64.0 * args.scale;
    let mut look = load_look(meta, &skins[skin_idx], &lib, ppu)?;

    let mut watcher = DesktopWatcher::new()?;
    let topts = TerrainOptions::default();
    let mut terrain = Terrain::from_desktop(watcher.snapshot(), &topts);
    log::info!("terrain: {} ledges, {} walls, {} windows", terrain.ledges.len(), terrain.walls.len(), terrain.windows.len());

    let params = CharacterParams::hollow_knight(ppu);
    let work = monitor::primary_work_area();
    let mut ch = Character::new(params, Abilities::default(), Vec2::new((work.left + work.right) as f32 * 0.5, work.bottom as f32));
    let mut loco = Locomotion::new(&lib, &params)?;
    let idle = lib.id("Idle").context("Idle clip")?;
    let mut anim = Animator::new(idle);
    let mut act = acts.id("wake_up").map(|id| ActPlayer::start(&acts, &lib, &mut anim, id, seed()));
    let mut brain = Wander::new(seed());
    let mut keyboard = Keyboard::default();
    let mut manual = false;

    let mut win = SpriteWindow::new()?;
    if args.capturable {
        win.set_capturable(true);
    }
    let mut rl = RunLoop::new(120.0)?;
    for (id, key) in [
        (HK_MANUAL, vk::VK_K),
        (HK_ACT, vk::VK_A),
        (HK_WAKE, vk::VK_W),
        (HK_RESPAWN, vk::VK_R),
        (HK_SKIN, vk::VK_S),
        (HK_CLICK_THROUGH, vk::VK_C),
        (HK_QUIT, vk::VK_Q),
    ] {
        rl.register_hotkey(id, key)?;
    }
    log::info!("hotkeys: Ctrl+Alt + K manual control, A act, W wake, R respawn at cursor, S next skin, C click-through, Q quit");

    let mut move_events: Vec<MoveEvent> = Vec::new();
    let mut anim_events: Vec<AnimEvent> = Vec::new();
    let mut click_through = false;
    let mut drawn: Option<(SpriteKey, bool)> = None;
    let mut last_raise = Instant::now();
    let mut stats = Stats::default();

    rl.run(|tick| {
        let dt = tick.dt;
        move_events.clear();

        for &hk in &tick.hotkeys {
            match hk {
                HK_MANUAL => {
                    manual = !manual;
                    log::info!("manual control {}", if manual { "ON (arrows, Up jump, LShift dash, LCtrl superdash, Z walk, X sprint)" } else { "off" });
                }
                HK_ACT => {
                    if ch.is_grounded()
                        && let Some(id) = brain_pick(&acts)
                    {
                        log::info!("act '{}'", acts.get(id).name);
                        act = Some(ActPlayer::start(&acts, &lib, &mut anim, id, seed()));
                    }
                }
                HK_WAKE => {
                    if let Some(p) = act.as_mut() {
                        p.signal(&acts, &lib, &mut anim, "woken");
                    }
                }
                HK_RESPAWN => {
                    if let Some((x, y)) = monitor::cursor_pos() {
                        ch = Character::new(params, Abilities::default(), Vec2::new(x as f32, y as f32));
                        act = None;
                    }
                }
                HK_SKIN => {
                    skin_idx = (skin_idx + 1) % skins.len();
                    let bank = SpriteBank::load_metadata(&args.data)?;
                    look = load_look(bank, &skins[skin_idx], &lib, ppu)?;
                    drawn = None;
                }
                HK_CLICK_THROUGH => {
                    click_through = !click_through;
                    win.set_click_through(click_through);
                }
                HK_QUIT => return Ok(Flow::Quit),
                _ => {}
            }
        }

        // 1. World: rebuild terrain only when the desktop actually changed.
        if watcher.poll() {
            let t = Instant::now();
            let new = Terrain::from_desktop(watcher.snapshot(), &topts);
            ch.on_terrain_rebuilt(&terrain, &new, &mut move_events);
            terrain = new;
            stats.terrain_rebuilds += 1;
            log::debug!("terrain rebuilt in {:?}: {} ledges, {} walls", t.elapsed(), terrain.ledges.len(), terrain.walls.len());
        }

        // 2. Intent.
        let act_running = act.is_some();
        let mut input = if manual {
            keyboard.read()
        } else {
            let d = brain.decide(&ch, &acts, act_running, dt);
            if let Some(id) = d.act
                && ch.is_grounded()
                && !loco.busy()
            {
                log::info!("act '{}'", acts.get(id).name);
                act = Some(ActPlayer::start(&acts, &lib, &mut anim, id, seed()));
            }
            d.input
        };
        if let Some(p) = &act {
            let wants_to_move = input.x != 0.0 || input.jump_pressed || input.dash_pressed;
            if manual && wants_to_move && p.interruptible(&acts) {
                act = None;
            } else if !manual {
                input = MoveInput::default();
            }
        }

        // 3. Physics.
        if let Some(g) = act.as_ref().and_then(|p| p.gait(&acts)) {
            input.gait = g;
            input.x = ch.facing.sign();
        }
        ch.step(&input, &terrain, dt, &mut move_events);
        if act.is_some() && !ch.is_grounded() {
            act = None; // knocked off / fell: acts are grounded choreography
        }

        // 4. Animation: acts own the animator while running; otherwise locomotion.
        let req = loco.update(&ch, &move_events, &anim_events, dt);
        if act.is_none() {
            anim.play(&lib, req.clip, req.restart);
        }
        anim_events.clear();
        anim.update(&lib, dt, &mut anim_events);
        if let Some(p) = act.as_mut()
            && p.update(&acts, &lib, &mut anim, dt, &anim_events) == ActStatus::Done
        {
            act = None;
            anim.play(&lib, idle, true);
        }

        // 5. Draw (only when something visible changed).
        let flip = ch.facing == Side::Right; // HK sprites face left
        if let Some(key) = anim.current(&lib)
            && let Some(frame) = look.bank.frame(key)
        {
            let x = (ch.pos.x - frame.anchor_x(flip)).round() as i32;
            let y = (ch.pos.y - look.feet_offset - frame.anchor[1]).round() as i32;
            if drawn != Some((key, flip)) {
                win.present_flipped(&frame.image, x, y, flip)?;
                drawn = Some((key, flip));
                stats.uploads += 1;
            } else {
                win.move_to(x, y)?;
            }
        }

        if last_raise.elapsed().as_secs() >= 2 {
            win.raise();
            last_raise = Instant::now();
            stats.report();
            log::debug!("knight at ({:.0},{:.0}) {:?} clip '{}' act {:?}", ch.pos.x, ch.pos.y, ch.state, lib.get(anim.clip()).name, act.as_ref().map(|p| acts.get(p.act()).name.as_str()));
        }
        Ok(Flow::Continue)
    })
}

fn brain_pick(acts: &ActLibrary) -> Option<skong_core::acts::ActId> {
    let pool: Vec<_> = acts.iter().filter(|(_, a)| a.weight > 0.0).map(|(id, _)| id).collect();
    pool.get(seed() as usize % pool.len().max(1)).copied()
}

fn seed() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.subsec_nanos() ^ d.as_secs() as u32)
}

#[derive(Default)]
struct Stats {
    uploads: u32,
    terrain_rebuilds: u32,
}

impl Stats {
    fn report(&mut self) {
        log::debug!("last 2s: {} sprite uploads, {} terrain rebuilds", self.uploads, self.terrain_rebuilds);
        *self = Stats::default();
    }
}
