//! The live pet: desktop → terrain → controller → locomotion/acts → sprite window,
//! controlled from a tray menu and global hotkeys (Ctrl+Alt+…): K manual control,
//! A random act, W wake, R respawn at cursor, S next skin, C click-through,
//! P pause, Q quit.
use std::time::Instant;

use anyhow::{Context, Result, bail};
use skong_core::acts::{ActId, ActLibrary, ActPlayer, ActStatus};
use skong_core::anim::{AnimEvent, Animator, ClipId, ClipLibrary};
use skong_core::assets::{Skin, SpriteBank, SpriteKey, VANILLA};
use skong_core::controller::{Abilities, Character, CharacterParams, MoveEvent, MoveInput, Side};
use skong_core::geom::Vec2;
use skong_core::locomotion::Locomotion;
use skong_core::terrain::{Terrain, TerrainOptions};
use skong_win::desktop::DesktopWatcher;
use skong_win::monitor;
use skong_win::runloop::{Flow, RunLoop};
use skong_win::sprite_window::SpriteWindow;
use skong_win::tray::{MenuItem, Tray, TrayEvent};
use skong_win::vk;

use crate::Args;
use crate::keyboard::Keyboard;
use crate::settings::{BOTHER, SCALES, Settings};
use crate::wander::Wander;

/// Everything the user can ask for, from hotkeys or the tray menu.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    ToggleManual,
    RandomAct,
    Wake,
    RespawnAtCursor,
    NextSkin,
    SetSkin(usize),
    SetScale(f32),
    SetBother(f32),
    ToggleClickThrough,
    TogglePause,
    Quit,
}

const HOTKEYS: &[(i32, vk::VIRTUAL_KEY, Command)] = &[
    (1, vk::VK_K, Command::ToggleManual),
    (2, vk::VK_A, Command::RandomAct),
    (3, vk::VK_W, Command::Wake),
    (4, vk::VK_R, Command::RespawnAtCursor),
    (5, vk::VK_S, Command::NextSkin),
    (6, vk::VK_C, Command::ToggleClickThrough),
    (7, vk::VK_P, Command::TogglePause),
    (8, vk::VK_Q, Command::Quit),
];

// Tray menu ids.
const ID_MANUAL: u32 = 1;
const ID_CLICK: u32 = 2;
const ID_PAUSE: u32 = 3;
const ID_ACT: u32 = 4;
const ID_RESPAWN: u32 = 5;
const ID_QUIT: u32 = 9;
const ID_SKIN0: u32 = 1000;
const ID_SCALE0: u32 = 2000;
const ID_BOTHER0: u32 = 3000;

/// Rasterized frames for one skin at one scale.
struct Look {
    bank: SpriteBank,
    /// Pixels from the sprite origin down to the feet (measured on Idle frame 0).
    feet_offset: f32,
}

/// Static game data shared by everything.
struct Data {
    args_data: std::path::PathBuf,
    lib: ClipLibrary,
    acts: ActLibrary,
    skins: Vec<Skin>,
    idle: ClipId,
}

impl Data {
    fn vanilla(&self) -> Option<&Skin> {
        vanilla_of(&self.skins)
    }

    fn load_look(&self, skin: usize, scale: f32) -> Result<Look> {
        let t = Instant::now();
        let mut bank = SpriteBank::load_metadata(&self.args_data)?;
        bank.rasterize_skin(&self.skins[skin], self.vanilla(), &self.lib.sprite_keys(), 64.0 * scale)?;
        let feet_offset = self.lib.get(self.idle).frames.first().and_then(|f| f.sprite).and_then(|k| bank.frame(k)).map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
        log::info!("skin '{}' at x{scale} ready in {:?} ({} KiB of frames)", self.skins[skin].name, t.elapsed(), bank.frame_bytes() / 1024);
        Ok(Look { bank, feet_offset })
    }
}

struct Pet {
    data: Data,
    settings: Settings,
    skin: usize,
    look: Look,

    watcher: DesktopWatcher,
    topts: TerrainOptions,
    terrain: Terrain,

    ch: Character,
    loco: Locomotion,
    anim: Animator,
    act: Option<ActPlayer>,
    brain: Wander,
    keyboard: Keyboard,
    manual: bool,
    paused: bool,

    win: SpriteWindow,
    tray: Tray,
    drawn: Option<(SpriteKey, bool)>,

    move_events: Vec<MoveEvent>,
    anim_events: Vec<AnimEvent>,
}

pub fn run(args: &Args) -> Result<()> {
    let mut settings = Settings::load();
    if let Some(s) = &args.skin {
        settings.skin = Some(s.clone());
    }
    if let Some(s) = args.scale {
        settings.scale = s;
    }

    let skins = Skin::discover(&args.skins);
    if skins.is_empty() {
        bail!("no skins (folders containing Knight.png) under {}", args.skins.display());
    }
    let skin = settings.skin.as_deref().and_then(|n| skins.iter().position(|s| s.name.eq_ignore_ascii_case(n))).unwrap_or(0);
    let meta = SpriteBank::load_metadata(&args.data)?;
    let lib = ClipLibrary::load(&args.data.join("animations/Knight.json"), &meta)?;
    let acts = ActLibrary::load(&args.data.join("../acts.json"), &lib)?;
    let idle = lib.id("Idle").context("Idle clip")?;
    let data = Data { args_data: args.data.clone(), lib, acts, skins, idle };
    let look = data.load_look(skin, settings.scale)?;

    let watcher = DesktopWatcher::new()?;
    let topts = TerrainOptions::default();
    let terrain = Terrain::from_desktop(watcher.snapshot(), &topts);
    log::info!("terrain: {} ledges, {} walls, {} windows", terrain.ledges.len(), terrain.walls.len(), terrain.windows.len());

    let params = CharacterParams::hollow_knight(64.0 * settings.scale);
    let work = monitor::primary_work_area();
    let ch = Character::new(params, Abilities::default(), Vec2::new((work.left + work.right) as f32 * 0.5, work.bottom as f32));
    let loco = Locomotion::new(&data.lib, &params)?;
    let mut anim = Animator::new(idle);
    let act = data.acts.id("wake_up").map(|id| ActPlayer::start(&data.acts, &data.lib, &mut anim, id, seed()));

    let win = SpriteWindow::new()?;
    if args.capturable {
        win.set_capturable(true);
    }
    win.set_click_through(settings.click_through);
    let tray = Tray::new("Desk-Skong", &tray_icon(&data, &look))?;

    let mut pet = Pet {
        data,
        settings,
        skin,
        look,
        watcher,
        topts,
        terrain,
        ch,
        loco,
        anim,
        act,
        brain: Wander::new(seed()),
        keyboard: Keyboard::default(),
        manual: false,
        paused: false,
        win,
        tray,
        drawn: None,
        move_events: Vec::new(),
        anim_events: Vec::new(),
    };

    let mut rl = RunLoop::new(120.0)?;
    for &(id, key, _) in HOTKEYS {
        if let Err(e) = rl.register_hotkey(id, key) {
            log::warn!("{e:#}");
        }
    }
    log::info!("tray menu ready; hotkeys Ctrl+Alt + K manual, A act, W wake, R respawn at cursor, S skin, C click-through, P pause, Q quit");

    let mut last_housekeeping = Instant::now();
    rl.run(|tick| {
        let mut cmds: Vec<Command> = tick.hotkeys.iter().filter_map(|h| HOTKEYS.iter().find(|k| k.0 == *h).map(|k| k.2)).collect();
        for ev in pet.tray.take_events() {
            if ev == TrayEvent::MenuRequested
                && let Some(id) = pet.tray.show_menu(&pet.menu())
            {
                cmds.extend(pet.menu_command(id));
            }
        }
        for c in cmds {
            if pet.apply(c)? == Flow::Quit {
                return Ok(Flow::Quit);
            }
        }
        if !pet.paused {
            pet.tick(tick.dt)?;
        }
        if last_housekeeping.elapsed().as_secs() >= 2 {
            last_housekeeping = Instant::now();
            pet.win.raise();
            log::debug!(
                "knight at ({:.0},{:.0}) {:?} clip '{}' act {:?}",
                pet.ch.pos.x,
                pet.ch.pos.y,
                pet.ch.state,
                pet.data.lib.get(pet.anim.clip()).name,
                pet.act.as_ref().map(|p| pet.data.acts.get(p.act()).name.as_str())
            );
        }
        Ok(Flow::Continue)
    })
}

impl Pet {
    fn tick(&mut self, dt: f32) -> Result<()> {
        let d = &self.data;
        self.move_events.clear();

        // 1. World: rebuild terrain only when the desktop actually changed.
        if self.watcher.poll() {
            let new = Terrain::from_desktop(self.watcher.snapshot(), &self.topts);
            self.ch.on_terrain_rebuilt(&self.terrain, &new, &mut self.move_events);
            self.terrain = new;
        }

        // 2. Intent.
        let mut input = if self.manual {
            self.keyboard.read()
        } else {
            let dec = self.brain.decide(&self.ch, &d.acts, self.act.is_some(), dt);
            if let Some(id) = dec.act
                && self.ch.is_grounded()
                && !self.loco.busy()
            {
                log::info!("act '{}'", d.acts.get(id).name);
                self.act = Some(ActPlayer::start(&d.acts, &d.lib, &mut self.anim, id, seed()));
            }
            dec.input
        };
        if let Some(p) = &self.act {
            let wants_to_move = input.x != 0.0 || input.jump_pressed || input.dash_pressed;
            if self.manual && wants_to_move && p.interruptible(&d.acts) {
                self.act = None;
            } else if !self.manual {
                input = MoveInput::default();
            }
        }
        if let Some(g) = self.act.as_ref().and_then(|p| p.gait(&d.acts)) {
            input.gait = g;
            input.x = self.ch.facing.sign();
        }

        // 3. Physics.
        self.ch.step(&input, &self.terrain, dt, &mut self.move_events);
        if self.act.is_some() && !self.ch.is_grounded() {
            self.act = None; // knocked off / fell: acts are grounded choreography
        }

        // 4. Animation: acts own the animator while running; otherwise locomotion.
        let req = self.loco.update(&self.ch, &self.move_events, &self.anim_events, dt);
        if self.act.is_none() {
            self.anim.play(&d.lib, req.clip, req.restart);
        }
        self.anim_events.clear();
        self.anim.update(&d.lib, dt, &mut self.anim_events);
        if let Some(p) = self.act.as_mut()
            && p.update(&d.acts, &d.lib, &mut self.anim, dt, &self.anim_events) == ActStatus::Done
        {
            self.act = None;
            self.anim.play(&d.lib, d.idle, true);
        }

        self.draw()
    }

    /// Present only when the frame changed; otherwise just move the window.
    fn draw(&mut self) -> Result<()> {
        let flip = self.ch.facing == Side::Right; // HK sprites face left
        let Some(key) = self.anim.current(&self.data.lib) else { return Ok(()) };
        let Some(frame) = self.look.bank.frame(key) else { return Ok(()) };
        let x = (self.ch.pos.x - frame.anchor_x(flip)).round() as i32;
        let y = (self.ch.pos.y - self.look.feet_offset - frame.anchor[1]).round() as i32;
        if self.drawn != Some((key, flip)) {
            self.win.present_flipped(&frame.image, x, y, flip)?;
            self.drawn = Some((key, flip));
        } else {
            self.win.move_to(x, y)?;
        }
        Ok(())
    }

    fn start_act(&mut self, id: ActId) {
        log::info!("act '{}'", self.data.acts.get(id).name);
        self.act = Some(ActPlayer::start(&self.data.acts, &self.data.lib, &mut self.anim, id, seed()));
    }

    fn apply(&mut self, c: Command) -> Result<Flow> {
        match c {
            Command::ToggleManual => {
                self.manual = !self.manual;
                log::info!("manual control {}", if self.manual { "ON (arrows, Up jump, LShift dash, LCtrl superdash, Z walk, X sprint)" } else { "off" });
            }
            Command::RandomAct => {
                if self.ch.is_grounded() {
                    let pool: Vec<ActId> = self.data.acts.iter().filter(|(_, a)| a.weight > 0.0).map(|(id, _)| id).collect();
                    if let Some(&id) = pool.get(seed() as usize % pool.len().max(1)) {
                        self.start_act(id);
                    }
                }
            }
            Command::Wake => {
                if let Some(p) = self.act.as_mut() {
                    p.signal(&self.data.acts, &self.data.lib, &mut self.anim, "woken");
                }
            }
            Command::RespawnAtCursor => {
                if let Some((x, y)) = monitor::cursor_pos() {
                    self.ch = Character::new(self.ch.params, self.ch.abilities, Vec2::new(x as f32, y as f32));
                    self.act = None;
                }
            }
            Command::NextSkin => return self.apply(Command::SetSkin((self.skin + 1) % self.data.skins.len())),
            Command::SetSkin(i) => {
                self.skin = i;
                self.reload_look()?;
                self.settings.skin = Some(self.data.skins[i].name.clone());
                self.settings.save();
            }
            Command::SetScale(s) => {
                self.settings.scale = s;
                self.reload_look()?;
                // Same feet position, new size and speeds.
                let params = CharacterParams::hollow_knight(64.0 * s);
                self.ch = Character::new(params, self.ch.abilities, self.ch.pos);
                self.loco = Locomotion::new(&self.data.lib, &params)?;
                self.settings.save();
            }
            Command::SetBother(b) => {
                self.settings.bother = b;
                self.settings.save();
            }
            Command::ToggleClickThrough => {
                self.settings.click_through = !self.settings.click_through;
                self.win.set_click_through(self.settings.click_through);
                self.settings.save();
            }
            Command::TogglePause => {
                self.paused = !self.paused;
                self.win.set_visible(!self.paused);
            }
            Command::Quit => return Ok(Flow::Quit),
        }
        Ok(Flow::Continue)
    }

    fn reload_look(&mut self) -> Result<()> {
        self.look = self.data.load_look(self.skin, self.settings.scale)?;
        self.drawn = None;
        if let Err(e) = self.tray.set_icon(&tray_icon(&self.data, &self.look)) {
            log::warn!("tray icon: {e:#}");
        }
        Ok(())
    }

    fn menu(&self) -> Vec<MenuItem> {
        let skins = self
            .data
            .skins
            .iter()
            .enumerate()
            .map(|(i, s)| MenuItem::check(ID_SKIN0 + i as u32, if s.name == VANILLA { "Vanilla (game)".to_string() } else { s.name.clone() }, i == self.skin))
            .collect();
        let scales = SCALES.iter().enumerate().map(|(i, (n, v))| MenuItem::check(ID_SCALE0 + i as u32, *n, (self.settings.scale - v).abs() < 1e-3)).collect();
        let bother = BOTHER.iter().enumerate().map(|(i, (n, v))| MenuItem::check(ID_BOTHER0 + i as u32, *n, (self.settings.bother - v).abs() < 1e-3)).collect();
        vec![
            MenuItem::label("Desk-Skong"),
            MenuItem::Separator,
            MenuItem::sub(format!("Skin ({})", self.data.skins.len()), skins),
            MenuItem::sub("Size", scales),
            MenuItem::sub("Mischief", bother),
            MenuItem::Separator,
            MenuItem::action(ID_ACT, "Do something!"),
            MenuItem::action(ID_RESPAWN, "Come here (respawn at cursor)"),
            MenuItem::check(ID_MANUAL, "Manual control", self.manual),
            MenuItem::check(ID_CLICK, "Click-through", self.settings.click_through),
            MenuItem::check(ID_PAUSE, "Pause", self.paused),
            MenuItem::Separator,
            MenuItem::action(ID_QUIT, "Quit"),
        ]
    }

    fn menu_command(&self, id: u32) -> Option<Command> {
        Some(match id {
            ID_MANUAL => Command::ToggleManual,
            ID_CLICK => Command::ToggleClickThrough,
            ID_PAUSE => Command::TogglePause,
            ID_ACT => Command::RandomAct,
            ID_RESPAWN => Command::RespawnAtCursor,
            ID_QUIT => Command::Quit,
            i if (ID_SKIN0..ID_SKIN0 + self.data.skins.len() as u32).contains(&i) => Command::SetSkin((i - ID_SKIN0) as usize),
            i if (ID_SCALE0..ID_SCALE0 + SCALES.len() as u32).contains(&i) => Command::SetScale(SCALES[(i - ID_SCALE0) as usize].1),
            i if (ID_BOTHER0..ID_BOTHER0 + BOTHER.len() as u32).contains(&i) => Command::SetBother(BOTHER[(i - ID_BOTHER0) as usize].1),
            _ => return None,
        })
    }
}

/// The knight's first Idle frame, used as the tray icon.
fn tray_icon(data: &Data, look: &Look) -> skong_core::canvas::Image {
    data.lib.get(data.idle).frames.first().and_then(|f| f.sprite).and_then(|k| look.bank.frame(k)).map(|f| f.image.clone()).unwrap_or_default()
}

pub fn vanilla_of(skins: &[Skin]) -> Option<&Skin> {
    skins.iter().find(|s| s.name.eq_ignore_ascii_case(VANILLA))
}

fn seed() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.subsec_nanos() ^ d.as_secs() as u32)
}
