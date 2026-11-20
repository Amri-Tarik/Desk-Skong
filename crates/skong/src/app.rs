//! The live pet: desktop → terrain → controller → locomotion/acts → sprite window,
//! controlled from a tray menu and global hotkeys (Ctrl+Alt+…): K manual control,
//! A random act, W wake, R respawn at cursor, S next skin, B bench at cursor, C click-through,
//! P pause, D debug overlay, Q quit. Control panels connect over a named pipe (`control`).
use std::time::Instant;

use anyhow::{Context, Result, bail};
use skong_core::acts::{ActId, ActLibrary, ActPlayer, ActStatus, Requirement};
use skong_core::anim::{AnimEvent, Animator, ClipId, ClipLibrary};
use std::collections::HashMap;

use skong_core::assets::{Frame, FrameStore, Skin, SpriteBank, SpriteKey, VANILLA};
use skong_core::effects::EffectSet;

use crate::fx::Fx;
use crate::gameguard::GameGuard;
use skong_core::controller::{Abilities, Character, CharacterParams, MoveEvent, MoveInput, MoveState, Side};
use skong_core::ai::{Navigator, NavStatus, ActCatalog, Activity, AttackDir, Brain, BrainConfig, Cursor, GraphNavigator, Intent, Motion, Percepts, Poke, RunningAct, SwitchReason, UserActivity};
use skong_win::input::{self, UserInput};

use crate::grab::Grab;
use skong_core::geom::{RectI, Vec2};
use skong_core::locomotion::Locomotion;
use skong_core::nav::{NavGoal, TravelStyle};
use skong_core::terrain::{Terrain, TerrainOptions};
use skong_win::capture::ContentFeed;
use skong_win::desktop::DesktopWatcher;
use skong_win::monitor;
use skong_win::runloop::{Flow, RunLoop};
use skong_win::selection::SelectionWatcher;
use skong_win::sprite_window::SpriteWindow;
use skong_win::tray::{MenuItem, Tray, TrayEvent};
use skong_win::vk;

use crate::Args;
use crate::keyboard::Keyboard;
use crate::settings::{BOTHER, SCALES, Settings};
use crate::emotes::{Emote, Emotes};
use skong_core::ai::BehaviourKind;
use crate::benches::{Benches, SEAT_BELOW_ORIGIN};
use crate::discoveries::Discoveries;
use crate::shinies::ShinyProps;
use crate::pastimes::PastimeProps;
use crate::navbuild::NavBuilder;
use crate::control::ControlServer;
use crate::overlay::PetOverlay;
use skong_ipc::Command;

mod achievements;
mod discover;
mod dreamgate;
mod enemy;
mod grimm;
mod panel;
mod pastime;
mod visit;
mod stakes;

const HOTKEYS: &[(i32, vk::VIRTUAL_KEY, Command)] = &[
    (1, vk::VK_K, Command::ToggleManual),
    (2, vk::VK_A, Command::RandomAct),
    (3, vk::VK_W, Command::Wake),
    (4, vk::VK_R, Command::RespawnAtCursor),
    (5, vk::VK_S, Command::NextSkin),
    (6, vk::VK_C, Command::ToggleClickThrough),
    (7, vk::VK_P, Command::TogglePause),
    (8, vk::VK_Q, Command::Quit),
    (9, vk::VK_B, Command::PlaceBenchAtCursor(0)),
    (10, vk::VK_D, Command::ToggleDebugOverlay),
    (11, vk::VK_G, Command::GoToCursor { hurry: false }),
    (12, vk::VK_H, Command::GoToCursor { hurry: true }),
];

/// Ctrl+Alt+M: the tray menu, at the cursor (not a [`Command`]: it's local UI).
const MENU_HOTKEY: i32 = 13;

// Tray menu ids.
const ID_MANUAL: u32 = 1;
const ID_CLICK: u32 = 2;
const ID_PAUSE: u32 = 3;
const ID_ACT: u32 = 4;
const ID_RESPAWN: u32 = 5;
const ID_CLEAR_BENCHES: u32 = 6;
const ID_VISION: u32 = 7;
const ID_OVERLAY: u32 = 8;
const ID_PANEL: u32 = 10;
const ID_INVITE: u32 = 11;
const ID_ENEMY: u32 = 21;
const ID_GAUNTLET: u32 = 22;
const ID_DREAM: u32 = 12;
const ID_SHARE: u32 = 14;
const ID_FIGHT: u32 = 15;
const ID_SHOWER: u32 = 20;
const ID_TEXT: u32 = 16;
const ID_QUIT: u32 = 9;
const ID_SKIN0: u32 = 1000;
const ID_SCALE0: u32 = 2000;
const ID_BOTHER0: u32 = 3000;
const ID_BENCH0: u32 = 4000;

/// Rasterized frames for one skin at one scale.
struct Look {
    bank: SpriteBank,
    /// Pixels from the sprite origin down to the feet (measured on Idle frame 0).
    feet_offset: f32,
    /// Seconds to the next eviction of idle unpacked frames.
    evict_in: f32,
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

    /// Rasterize the knight and every effect (each at its own scale) in one pass.
    fn load_look(&self, skin: usize, scale: f32, fx: &Fx) -> Result<(Look, Vec<HashMap<SpriteKey, Frame>>)> {
        let t = Instant::now();
        let ppu = 64.0 * scale;
        let keys = self.lib.sprite_keys();
        let fx_sets = fx.raster_sets();
        let mut sets: Vec<(&[SpriteKey], f32)> = vec![(&keys, ppu)];
        sets.extend(fx_sets.iter().map(|(k, s)| (k.as_slice(), ppu * s)));
        let mut bank = SpriteBank::load_metadata(&self.args_data)?;
        // Knight frames are packed as they come (most clips are rarely played and unpack
        // on first use); effect frames stay raw for `Fx` to bake.
        let mut knight = FrameStore::default();
        let mut fx_frames: Vec<HashMap<SpriteKey, Frame>> = vec![HashMap::new(); fx_sets.len()];
        bank.rasterize_each(&self.skins[skin], self.vanilla(), &sets, |set, k, f| match set {
            0 => knight.insert(k, &f),
            i => drop(fx_frames[i - 1].insert(k, f)),
        })?;
        bank.install_store(knight, ppu);
        let feet_offset = self.lib.get(self.idle).frames.first().and_then(|f| f.sprite).and_then(|k| bank.frame(k)).map_or(0.0, |f| f.image.height as f32 - f.anchor[1]);
        let store = bank.frames();
        log::info!(
            "skin '{}' at x{scale} ready in {:?} ({} KiB of packed knight frames, {} KiB unpacked)",
            self.skins[skin].name,
            t.elapsed(),
            store.bytes() / 1024,
            store.unpacked_bytes() / 1024
        );
        Ok((Look { bank, feet_offset, evict_in: EVICT_PERIOD }, fx_frames))
    }
}

/// Unpacked frames not drawn for this long are dropped (they stay packed).
const EVICT_PERIOD: f32 = 60.0;

impl Look {
    /// Every [`EVICT_PERIOD`], drop unpacked frames not drawn since the previous check.
    fn tick(&mut self, dt: f32) {
        self.evict_in -= dt;
        if self.evict_in <= 0.0 {
            self.evict_in = EVICT_PERIOD;
            let n = self.bank.evict_unused();
            if n > 0 {
                log::debug!("evicted {n} idle knight frames ({} KiB held)", self.bank.frame_bytes() / 1024);
            }
        }
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
    /// Live on-screen content surfaces (lines, borders...) as extra one-way ledges.
    content: ContentFeed,

    ch: Character,
    loco: Locomotion,
    anim: Animator,
    act: Option<ActPlayer>,
    /// Seat line (screen y) while a seated act plays.
    seat_y: Option<f32>,
    benches: Benches,
    /// Silent knight: emote icons above his head.
    emotes: Emotes,
    /// Last act already turned into an emote.
    emote_seen: Option<ActId>,
    emote_test: bool,
    /// Geo picked up lately: a big haul earns a sparkle.
    haul: skong_core::ai::GeoHaul,
    /// Things found in windows (benches, hot springs, geo deposits).
    discoveries: Discoveries,
    /// Shinies on the ground for the knight to pick up.
    shinies: ShinyProps,
    /// The training dummy and lumaflies (pastimes).
    pastimes: PastimeProps,
    /// Who stands where: props, NPCs, ... (nothing is placed inside something else).
    occupancy: skong_core::occupancy::Occupancy,
    fx: Fx,
    /// Keeps the knight off monitors running a fullscreen game.
    games: GameGuard,
    /// Hidden because games cover every monitor.
    game_hidden: bool,
    /// Warping off a game monitor: destination work area, once the gate act ends.
    leaving_for: Option<RectI>,
    /// A Dream Gate the brain planted at a prize no route reaches (glow + the jump).
    gate: skong_core::dreamgate::GateRuntime,
    /// Vision state to restore after a game.
    vision_before_game: bool,
    /// A user-requested trip (go to cursor) is being followed instead of the brain.
    forced_nav: bool,
    /// Seconds a walking act step has been pushing without moving.
    act_blocked: f32,
    /// Debug (`--fx-test`): seconds since start, drives a scripted superdash.
    fx_test: Option<f32>,
    /// Debug (`--move-test`): scripted dive + turn loop.
    move_test: Option<crate::movefx::MoveTest>,
    brain: Brain,
    nav: GraphNavigator,
    navb: NavBuilder,
    /// Ledge last found missing from the nav graph (rebuild requested for it once).
    nav_missing: Option<skong_core::controller::Support>,
    /// Dark surroundings (sampled behind the knight every 2 s) and seconds to the next sample.
    dark: (bool, f32),
    /// Seconds to the next update of the planner's keep-points.
    keep_check: f32,
    user_input: UserInput,
    /// Controller events not yet seen by the brain.
    brain_events: Vec<MoveEvent>,
    /// Input applied on the last physics step (the brain notices walking into walls).
    last_input: MoveInput,
    /// Screen activity for the brain, mirrored from the vision feed.
    activity: Activity,
    /// Where highlighted text is (opt-in `text_curiosity`; `None` while off).
    selection: Option<SelectionWatcher>,
    /// Last brain debug line (behaviour, mood, top utilities).
    brain_debug: String,
    keyboard: Keyboard,
    manual: bool,
    paused: bool,
    grab: Grab,
    /// Clicks / grabs / throws since the brain last looked.
    pokes: Vec<Poke>,

    win: SpriteWindow,
    tray: Tray,
    drawn: Option<(SpriteKey, bool)>,

    move_events: Vec<MoveEvent>,
    anim_events: Vec<AnimEvent>,

    /// Control panels (named pipe) and the terrain / nav debug overlay.
    control: ControlServer,
    overlay: PetOverlay,
    /// NPC visitors, speech bubbles, dream-nail thoughts (None if their data is missing).
    visitors: Option<crate::npcs::Visitors>,
    /// Death, the Shade, Geo and spell effects.
    stakes: stakes::Stakes,
    /// Enemies wandering in (None if their data is missing) and the bumps they gave him.
    enemies: Option<crate::enemies::Enemies>,
    enemy_hits: u32,
    /// The secret boss: his lantern, the show (None if his data is missing).
    troupe: Option<crate::grimm::Troupe>,
    /// Hidden achievements (all event mapping lives in `app/achievements.rs`).
    achievements: achievements::Achievements,
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
    let mut fx = Fx::new(EffectSet::load(&args.data.join("../effects.json"), &args.data.join("animations"), &meta)?);
    fx.capturable = args.capturable;
    let (look, fx_frames) = data.load_look(skin, settings.scale, &fx)?;
    fx.install_frames(fx_frames);

    let watcher = DesktopWatcher::new()?;
    let topts = TerrainOptions::default();
    let mut content = ContentFeed::new(args.vision, watcher.snapshot());
    let terrain = content.terrain(watcher.snapshot(), &topts);
    log::info!("terrain: {} ledges, {} walls, {} windows", terrain.ledges.len(), terrain.walls.len(), terrain.windows.len());

    let params = CharacterParams::hollow_knight(64.0 * settings.scale);
    let work = monitor::primary_work_area();
    let ch = Character::new(params, Abilities::default(), Vec2::new((work.left + work.right) as f32 * 0.5, work.bottom as f32));
    let loco = Locomotion::new(&data.lib, &params)?;
    let mut anim = Animator::new(idle);
    let act = data.acts.id("wake_up").map(|id| ActPlayer::start(&data.acts, &data.lib, &mut anim, id, seed()));

    let win = SpriteWindow::new()?;
    // Only the knight freezes the content under him (so his sprite never erases his own
    // ledge); coins, props and visitors must fall / vanish when their line goes away. With
    // --capturable every window of ours stays frozen (capture would see them).
    if !args.capturable {
        content.freeze_only(vec![win.hwnd()]);
    }
    if args.capturable {
        win.set_capturable(true);
        if args.vision {
            log::warn!("--capturable: the knight is now visible to content capture and may stand on himself");
        }
    }
    win.set_click_through(settings.click_through);
    let tray = Tray::new("Desk-Skong", &tray_icon(&data, &look))?;
    let benches = Benches::load(&args.data.join("../props.json"), data.vanilla().map(|s| s.dir.as_path()), settings.scale)?;
    let emotes = Emotes::new(&args.data, data.vanilla(), 64.0 * settings.scale, args.capturable)?;
    let discoveries = Discoveries::load(&args.data, &args.data.join("../props.json"), data.vanilla().map(|s| s.dir.as_path()), settings.scale);
    let shinies = ShinyProps::load(&args.data.join("../props.json"), settings.scale, seed() as u64);
    let pastimes = PastimeProps::load(&args.data, data.vanilla(), settings.scale, seed() as u64);

    let mut brain = Brain::new(BrainConfig::default(), ActCatalog::from_library(&data.acts), seed());
    brain.set_bother_level(settings.bother);
    let navb = NavBuilder::new();
    navb.reset(&terrain, &ch);

    let mut pet = Pet {
        stakes: stakes::Stakes::new(&args.data, settings.stakes.clone(), args.capturable, &brain.config().stakes),
        data,
        settings,
        skin,
        look,
        watcher,
        topts,
        terrain,
        content,
        ch,
        loco,
        anim,
        act,
        seat_y: None,
        benches,
        emotes,
        emote_seen: None,
        emote_test: args.emote_test,
        haul: Default::default(),
        discoveries,
        shinies,
        pastimes,
        occupancy: Default::default(),
        fx,
        games: GameGuard::new(),
        game_hidden: false,
        leaving_for: None,
        gate: Default::default(),
        vision_before_game: false,
        forced_nav: false,
        act_blocked: 0.0,
        fx_test: args.fx_test.then_some(0.0),
        move_test: args.move_test.then(crate::movefx::MoveTest::new),
        brain,
        nav: GraphNavigator::new(),
        navb,
        nav_missing: None,
        dark: (false, 0.0),
        keep_check: 0.0,
        user_input: UserInput::new()?,
        brain_events: Vec::new(),
        last_input: MoveInput::default(),
        activity: Activity::default(),
        selection: None,
        brain_debug: String::new(),
        keyboard: Keyboard::default(),
        manual: false,
        paused: false,
        grab: Grab::default(),
        pokes: Vec::new(),
        win,
        tray,
        drawn: None,
        move_events: Vec::new(),
        anim_events: Vec::new(),
        control: ControlServer::start(),
        overlay: PetOverlay::default(),
        visitors: None,
        enemies: None,
        enemy_hits: 0,
        troupe: None,
        achievements: achievements::Achievements::load(&args.data),
    };
    pet.visitors = pet.load_visitors(args.capturable);
    pet.enemies = pet.load_enemies(args.capturable);
    pet.troupe = pet.load_troupe(args.capturable);
    pet.achievements_setup();
    if let (Some(v), Some(name)) = (pet.visitors.as_mut(), &args.visit) {
        v.invite((name != "any").then(|| name.clone()));
    }
    if let Some(v) = pet.visitors.as_mut().filter(|_| args.dream_test) {
        v.dream_test = Some(4.0);
    }
    pet.control.set_hello(pet.hello());

    pet.benches.capturable = args.capturable;
    pet.discoveries.capturable = args.capturable;
    pet.shinies.capturable = args.capturable;
    pet.pastimes.capturable = args.capturable;
    pet.apply_share_visible();
    pet.apply_text_curiosity();
    if args.panel.unwrap_or(pet.settings.open_panel) {
        pet.apply(Command::OpenPanel)?;
    }
    if args.sit_test {
        pet.sit_test();
    }

    let mut rl = RunLoop::new(120.0)?;
    for &(id, key, _) in HOTKEYS {
        if let Err(e) = rl.register_hotkey(id, key) {
            log::warn!("{e:#}");
        }
    }
    if let Err(e) = rl.register_hotkey(MENU_HOTKEY, vk::VK_M) {
        log::warn!("{e:#}");
    }
    log::info!("tray menu ready; hotkeys Ctrl+Alt + M menu, K manual, A act, W wake, R respawn at cursor, G walk / H hurry to cursor, S skin, C click-through, P pause, D debug overlay, Q quit");

    let mut last_housekeeping = Instant::now();
    let mut goto_test_at = args.goto_test.then(Instant::now);
    rl.run(|tick| {
        let mut cmds: Vec<Command> = tick.hotkeys.iter().filter_map(|h| HOTKEYS.iter().find(|k| k.0 == *h).map(|k| k.2)).collect();
        // Ctrl+Alt+M opens the tray menu at the cursor, from anywhere.
        let menu_key = tick.hotkeys.contains(&MENU_HOTKEY);
        for ev in pet.tray.take_events() {
            if ev == TrayEvent::MenuRequested
                && let Some(id) = pet.tray.show_menu(&pet.menu())
            {
                cmds.extend(pet.menu_command(id));
            }
        }
        if menu_key && let Some(id) = pet.tray.show_menu(&pet.menu()) {
            cmds.extend(pet.menu_command(id));
        }
        cmds.extend(pet.service_panel());
        if let Some(t) = goto_test_at.as_mut()
            && t.elapsed().as_secs() >= 8
        {
            // Simulates pressing H with the control panel focused (manual mode on).
            *t = Instant::now();
            cmds.push(Command::SetManual(true));
            cmds.push(Command::GoToCursor { hurry: true });
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
            pet.benches.raise_all();
            pet.raise_visitors();
            pet.raise_enemies();
            pet.raise_troupe();
            pet.discoveries.raise_all();
            pet.shinies.raise_all();
            pet.pastimes.raise_all();
            pet.win.raise();
            pet.emotes.raise();
            log::debug!(
                "knight at ({:.0},{:.0}) {:?} clip '{}' act {:?} | brain: {}",
                pet.ch.pos.x,
                pet.ch.pos.y,
                pet.ch.state,
                pet.data.lib.get(pet.anim.clip()).name,
                pet.act.as_ref().map(|p| pet.data.acts.get(p.act()).name.as_str()),
                pet.brain_debug
            );
        }
        Ok(Flow::Continue)
    })
}

impl Pet {
    fn tick(&mut self, dt: f32) -> Result<()> {
        self.handle_mouse();
        self.look.tick(dt);
        self.move_events.clear();
        if let Some(g) = self.navb.poll() {
            self.nav.set_graph(g);
        }
        // Ledges carrying something he wants (deposits, shinies, resting Geo) stay in the
        // planner's terrain, or short ones were unreachable and he ignored the prize.
        self.keep_check -= dt;
        if self.keep_check <= 0.0 {
            self.keep_check = 0.5;
            let mut keep: Vec<Vec2> = self.discoveries.places().iter().flat_map(|p| [p.at, p.center]).collect();
            keep.extend(self.shinies.percepts().iter().map(|s| s.pos));
            keep.extend(self.pastimes.dummy_foot());
            keep.extend(self.stakes.resting_coins());
            if self.navb.set_keep(keep) {
                self.navb.request(&self.terrain, &self.ch, true);
            }
        }
        // Standing on a ledge the graph doesn't have (a small content ledge trimmed from the
        // planner's terrain): no route can start there, so rebuild with it, once per ledge.
        if let Some(s) = self.ch.support
            && self.nav_missing != Some(s)
            && self.nav.graph().is_some_and(|g| g.surface_at(&self.ch).is_none())
        {
            self.nav_missing = Some(s);
            log::debug!("nav: standing on a ledge the planner doesn't know; rebuilding with it");
            self.navb.request(&self.terrain, &self.ch, false);
        }

        // 1. World: rebuild terrain only when the desktop or the on-screen content changed.
        let desk_changed = self.watcher.poll();
        let games_changed = self.games.update(self.watcher.snapshot());
        if games_changed {
            self.benches.hide_on(&self.games.game_monitors);
            self.stakes.hide_on(&self.games.game_monitors);
            self.discoveries.hide_on(&self.games.game_monitors);
            self.shinies.hide_on(&self.games.game_monitors);
            self.pastimes.hide_on(&self.games.game_monitors);
        }
        if self.content.poll(self.watcher.snapshot(), desk_changed) || desk_changed || games_changed {
            let mut new = self.content.terrain(self.watcher.snapshot(), &self.topts);
            self.games.filter_terrain(&mut new, self.watcher.snapshot());
            // Content-only changes keep the desktop generation, but the nav graph built from
            // them has different surfaces: give every rebuild its own generation so routes
            // planned on an older graph are re-planned.
            new.generation = self.terrain.generation.max(new.generation) + 1;
            self.ch.on_terrain_rebuilt(&self.terrain, &new, &mut self.move_events);
            self.benches.follow(&self.terrain, &new);
            self.terrain = new;
            self.navb.request(&self.terrain, &self.ch, !desk_changed && !games_changed);
        }
        if self.game_step() {
            self.hide_visitors();
            self.hide_enemies();
            self.hide_troupe();
            self.achievements_tick(dt); // he hid from a game: that counts
            return Ok(()); // hidden while games cover every monitor
        }
        // Screen activity for the brain (the feed updates it at <= 10 Hz).
        if *self.content.activity() != self.activity {
            self.activity.clone_from(self.content.activity());
        }

        // 2. Intent: keyboard in manual mode, otherwise the brain.
        // A requested trip (go to cursor) beats manual mode — the panel switches manual on
        // while focused — until the user actually presses a movement key.
        let manual_input = self.manual.then(|| self.keyboard.read(self.control.manual_keys()));
        let user_moving = manual_input.is_some_and(|i| i.x != 0.0 || i.jump_held || i.dash_pressed || i.down || i.superdash_held);
        if self.forced_nav && user_moving {
            log::info!("go-to-cursor: cancelled by manual input");
            self.forced_nav = false;
            self.nav.stop();
        }
        let mut input = if self.forced_nav {
            self.follow_forced(dt)
        } else if let Some(i) = manual_input {
            i
        } else {
            self.think(dt)
        };
        if let Some(t) = self.fx_test.as_mut() {
            // Scripted: idle 3 s, charge a superdash for 1.2 s, release, repeat every 6 s.
            *t += dt;
            let phase = *t % 6.0;
            input = MoveInput { superdash_held: (3.0..4.2).contains(&phase), ..MoveInput::default() };
            self.act = None;
        }
        if let Some(mt) = self.move_test.as_mut() {
            let w = monitor::primary_work_area();
            input = mt.input(dt, &mut self.ch, Vec2::new((w.left + w.right) as f32 * 0.5, w.top as f32 + 120.0));
            self.act = None;
        } else if manual_input.is_none()
            // In a fight the combat kit picks its own dives: a quiet fall there is a pogo
            // bounce's apex or a jump over spikes, not a drop to shorten (a dive here cut
            // pogo chains to one bounce and cost Grimm fights). Fatal falls are still caught
            // by the hard-landing rule below.
            && !self.brain_steers_jumps()
            // Never mid-jump or mid-climb of a planned route: only where it plans a drop.
            && self.nav.follower().is_none_or(|f| f.dive_ok(&self.ch))
            && let Some(dark) = self.ch.auto_dive(&self.terrain, &input, self.brain.mind().arousal() > 0.6)
        {
            // Far straight drops: Desolate Dive (Descending Dark when excited / very deep).
            input.dive_pressed = true;
            input.dive_dark = dark;
        } else if manual_input.is_none() && self.move_test.is_none() && self.ch.hard_landing_ahead(&self.terrain) {
            // A fatal landing coming: dive, even off the planned line (the follower
            // re-plans from wherever he lands; alive beats precise).
            input.dive_pressed = true;
            input.dive_dark = true;
        }
        let d = &self.data;
        if let Some(p) = &self.act {
            let wants_to_move = input.x != 0.0 || input.jump_pressed || input.dash_pressed;
            if self.manual && wants_to_move && p.interruptible(&d.acts) {
                self.act = None;
            } else if !self.manual {
                input = MoveInput::default();
            }
        }
        if let Some(g) = self.act.as_ref().and_then(|p| p.gait(&d.acts)) {
            // Walking acts (map stroll, lantern patrol) pace their ledge: turn around at its
            // ends and when blocked, instead of pushing into walls or walking off.
            let near_end = self.ch.support.as_ref().is_some_and(|s| {
                let margin = self.ch.size.x;
                (self.ch.facing == Side::Left && self.ch.pos.x - s.x0 < margin) || (self.ch.facing == Side::Right && s.x1 - self.ch.pos.x < margin)
            });
            self.act_blocked = if self.ch.is_grounded() && self.ch.vel.x.abs() < 5.0 { self.act_blocked + dt } else { 0.0 };
            if near_end || self.act_blocked > 0.25 {
                self.ch.facing = self.ch.facing.opposite();
                self.act_blocked = 0.0;
                if let Some(p) = self.act.as_mut() {
                    p.turned(&d.acts, &d.lib, &mut self.anim);
                }
            }
            input.gait = g;
            input.x = self.ch.facing.sign();
        } else {
            self.act_blocked = 0.0;
        }

        // 3. Physics (suspended while the user holds the knight).
        if let Some(p) = self.grab.held_pos() {
            self.ch.pos = p;
            self.ch.vel = Vec2::ZERO;
            self.ch.support = None;
            self.ch.state = MoveState::Airborne;
        } else {
            self.ch.step(&input, &self.terrain, dt, &mut self.move_events);
            self.last_input = input;
        }
        if self.act.is_some() && !self.ch.is_grounded() {
            self.act = None; // knocked off / fell: acts are grounded choreography
        }

        // 4. Animation: acts own the animator while running; otherwise locomotion.
        let req = self.loco.update(&self.ch, &self.move_events, &self.anim_events, dt);
        if self.act.is_none() {
            match self.grab.held_pos() {
                Some(_) => self.anim.play(&d.lib, d.lib.id("Fall").unwrap_or(d.idle), false),
                None => self.anim.play(&d.lib, req.clip, req.restart),
            }
        }
        self.anim_events.clear();
        self.anim.update(&d.lib, dt, &mut self.anim_events);
        if let Some(p) = self.act.as_mut()
            && p.update(&d.acts, &d.lib, &mut self.anim, dt, &self.anim_events) == ActStatus::Done
        {
            self.act = None;
            // `…ToIdle` endings keep looping idle instead of restarting it.
            self.anim.play(&d.lib, self.loco.after_act(self.anim.clip()), false);
        }

        // The brain sees this tick's controller events on its next tick.
        self.brain_events.extend_from_slice(&self.move_events);
        if self.brain_events.len() > 256 {
            self.brain_events.drain(..128);
        }

        self.stakes_tick(dt)?;
        self.dream_gate_tick(dt);
        self.draw()?;
        self.trigger_fx();
        self.discoveries_tick(dt);
        let origin = self.knight_origin();
        self.tick_visitors(dt);
        self.tick_enemies(dt);
        self.tick_troupe(dt);
        self.achievements_tick(dt);
        self.emote_tick(dt, origin)?;
        self.fx.update(dt, origin, self.ch.facing == Side::Right, self.ch.params.px_per_unit)
    }

    /// Silent emotes besides the brain's expressions (`Intent::emote`, shown as each brain
    /// tick ends): zzz while asleep, "!" on the startled act.
    fn emote_tick(&mut self, dt: f32, origin: Vec2) -> Result<()> {
        let act = self.act.as_ref().map(|p| p.act());
        if act != self.emote_seen {
            self.emote_seen = act;
            if act.is_some_and(|id| self.data.acts.get(id).name == "startled") {
                self.emotes.show(Emote::Bang);
            }
        }
        if self.emote_test {
            // Debug (`--emote-test`): cycle every emote.
            let n = (self.emotes.clock() / 2.5) as usize % Emote::ALL.len();
            self.emotes.show(Emote::ALL[n]);
        }
        let clip = self.data.lib.get(self.anim.clip()).name.as_str();
        self.emotes.sleeping(matches!(clip, "Sitting Asleep" | "Sitting Asleep Breathe" | "Prostrate" | "Prostrate Breathe"));
        // Just above his head: the top of the frame being drawn (works lying, sitting, standing).
        let top = self.anim.current(&self.data.lib).and_then(|k| self.look.bank.frame(k)).map_or(origin.y - 0.85 * self.ch.params.px_per_unit, |f| origin.y - f.anchor[1]);
        let head = Vec2::new(origin.x, top - 0.1 * self.ch.params.px_per_unit);
        self.emotes.update(dt, head)
    }

    /// Screen position of the knight sprite's origin (seated acts put the sitting pose's
    /// seat line on the seat; otherwise feet on the ground).
    fn knight_origin(&self) -> Vec2 {
        let y = match self.seat_y.filter(|_| self.act.is_some()) {
            Some(seat) => seat - (SEAT_BELOW_ORIGIN - self.anim.seat_dy(&self.data.lib)) * self.ch.params.px_per_unit,
            None => self.ch.pos.y - self.look.feet_offset,
        };
        Vec2::new(self.ch.pos.x, y)
    }

    /// Spawn / stop effects for this tick's movement events (game offsets from the prefab).
    fn trigger_fx(&mut self) {
        let origin = self.knight_origin();
        let right = self.ch.facing == Side::Right;
        for e in &self.move_events {
            match e {
                MoveEvent::SuperdashCharging => {
                    let on_wall = matches!(self.ch.state, MoveState::SuperdashCharge { wall: Some(_) });
                    self.fx.spawn(if on_wall { "sd_charge_wall" } else { "sd_charge" }, origin, right);
                }
                MoveEvent::SuperdashCharged => self.fx.spawn("sd_bling", origin, right),
                MoveEvent::SuperdashCancelled => {
                    self.fx.stop("sd_charge");
                    self.fx.stop("sd_charge_wall");
                }
                MoveEvent::SuperdashLaunched => {
                    self.fx.stop("sd_charge");
                    self.fx.stop("sd_charge_wall");
                    self.fx.spawn("sd_burst", origin, right);
                    self.fx.spawn("sd_trail", origin, right);
                }
                MoveEvent::SuperdashHitWall => {
                    self.fx.stop("sd_trail");
                    self.fx.spawn("sd_break", origin, right);
                    self.fx.spawn("sd_trail_end", origin, right);
                }
                MoveEvent::SuperdashEnded if self.fx.is_live("sd_trail") => {
                    self.fx.stop("sd_trail");
                    self.fx.spawn("sd_trail_end", origin, right);
                }
                MoveEvent::DoubleJumped => self.fx.spawn("double_jump", origin, right),
                MoveEvent::WallJumped { .. } => self.fx.spawn("wall_puff", origin, right),
                MoveEvent::DashStarted { shadow: true } if !self.ch.dashing_down() => self.fx.spawn("shadow_burst", origin, right),
                _ => {}
            }
        }
        crate::movefx::spawn(&mut self.fx, &self.move_events, &self.ch, origin);
    }

    /// Present only when the frame changed; otherwise just move the window.
    fn draw(&mut self) -> Result<()> {
        let flip = self.ch.facing == Side::Right; // HK sprites face left
        let Some(key) = self.anim.current(&self.data.lib) else { return Ok(()) };
        let Some(frame) = self.look.bank.frame(key) else { return Ok(()) };
        let origin = self.knight_origin();
        let x = (origin.x - frame.anchor_x(flip)).round() as i32;
        let y = (origin.y - frame.anchor[1]).round() as i32;
        if self.drawn != Some((key, flip)) {
            self.win.present_flipped(&frame.image, x, y, flip)?;
            self.drawn = Some((key, flip));
        } else {
            self.win.move_to(x, y)?;
        }
        Ok(())
    }

    /// One brain tick: gather percepts, apply the intent's acts/signals/attacks, and
    /// return this tick's movement input.
    /// Keep away from games: warp to a free monitor, or vanish when there is none.
    /// Returns true while hidden (the rest of the tick is skipped).
    fn game_step(&mut self) -> bool {
        let desk = self.watcher.snapshot();
        let all_taken = !desk.monitors.is_empty() && desk.monitors.iter().all(|m| self.games.is_game_monitor(&m.rect));
        if self.game_hidden {
            if all_taken {
                return true;
            }
            if let Some(w) = self.games.free_work_area(desk, self.ch.pos) {
                log::info!("a monitor is free again: the knight comes back");
                self.game_hidden = false;
                self.win.set_visible(true);
                if self.vision_before_game {
                    let desk = desk.clone();
                    self.content.set_enabled(true, &desk);
                }
                self.warp_to(w);
            }
            return false;
        }
        if all_taken {
            log::info!("games cover every monitor: the knight hides until one is free");
            self.act = None;
            self.forced_nav = false;
            self.nav.stop();
            self.leaving_for = None;
            self.emotes.hide();
            self.win.set_visible(false);
            self.vision_before_game = self.content.is_enabled();
            let desk = desk.clone();
            self.content.set_enabled(false, &desk);
            self.game_hidden = true;
            return true;
        }
        let on_game = GameGuard::monitor_of(desk, self.ch.pos).is_some_and(|m| self.games.is_game_monitor(&m));
        if !on_game {
            self.leaving_for = None;
            return false;
        }
        let Some(dest) = self.games.free_work_area(desk, self.ch.pos) else { return false };
        match self.leaving_for {
            // Dream-gate out (when standing), then reappear on the free monitor.
            None if self.ch.is_grounded() && self.data.acts.id("dreamgate_out").is_some() => {
                log::info!("game on the knight's monitor: warping to {dest:?}");
                self.forced_nav = false;
                self.nav.stop();
                if let Some(id) = self.data.acts.id("dreamgate_out") {
                    self.start_act(id);
                }
                self.leaving_for = Some(dest);
            }
            Some(_) if self.act.is_some() && self.ch.is_grounded() => {}
            _ => {
                self.leaving_for = None;
                self.warp_to(dest);
            }
        }
        false
    }

    /// Reappear on the floor of `work` (centre) with the dream-gate arrival.
    fn warp_to(&mut self, work: RectI) {
        let pos = Vec2::new((work.left + work.right) as f32 * 0.5, work.bottom as f32);
        self.ch = Character::new(self.ch.params, self.ch.abilities, pos);
        self.act = None;
        self.forced_nav = false;
        self.nav.stop();
        if let Some(id) = self.data.acts.id("dreamgate_in") {
            self.start_act(id);
        }
    }

    /// Follow a user-requested route; hand control back to the brain when it ends.
    fn follow_forced(&mut self, dt: f32) -> MoveInput {
        let input = self.nav.next_input(&self.ch, &self.terrain, dt);
        let status = self.nav.status();
        log::trace!("forced: pos ({:.0},{:.0}) {:?} input x={:.2} jump={} dash={} status {:?}", self.ch.pos.x, self.ch.pos.y, self.ch.state, input.x, input.jump_pressed, input.dash_pressed, status);
        if status != NavStatus::Moving {
            log::info!("go-to-cursor: {status:?}");
            self.forced_nav = false;
        }
        input
    }

    /// Path-find to the cursor (drops any act; the brain resumes on arrival).
    fn go_to_cursor(&mut self, hurry: bool) {
        let Some((x, y)) = monitor::cursor_pos() else { return };
        let style = if hurry { TravelStyle::Hurry } else { TravelStyle::Stroll };
        if self.nav.go(&self.ch, NavGoal::Point(Vec2::new(x as f32, y as f32)), style) {
            let moves = self.nav.follower().map(|f| f.route().moves.clone()).unwrap_or_default();
            log::info!("go-to-cursor ({style:?}) toward ({x},{y}) via {moves:?}");
            // The planner heads for the closest reachable point; say so when that is not the cursor.
            if let Some(end) = self.nav.follower().map(|f| f.route().end) {
                // Ending below a cursor that hovers in mid-air is normal; only a horizontal miss
                // means the spot itself is out of reach.
                let miss = (end.x - x as f32).abs();
                if miss > 120.0 {
                    log::warn!("go-to-cursor: can't get under the cursor {style:?}; heading to the closest point ({:.0},{:.0}), {miss:.0} px to the side", end.x, end.y);
                }
            }
            self.act = None;
            self.forced_nav = true;
        } else {
            log::warn!("go-to-cursor ({style:?}): no route to ({x},{y}){}", if hurry { "" } else { " — try hurrying (Ctrl+Alt+H)" });
        }
    }

    /// Is it dark where he is? Every 2 s, the mean luminance of the screen around him (a
    /// dark-themed editor, a night photo, a dimmed screen). Hysteresis keeps it steady.
    /// Apply "visible in screen shares": every window of ours shows in capture (or not),
    /// and while they show, vision freezes under all of them so it never sees our own
    /// sprites as content (otherwise only the knight freezes the content under him).
    fn apply_share_visible(&mut self) {
        let on = self.settings.share_visible;
        skong_win::sprite_window::set_share_visible(on);
        if on || self.fx.capturable {
            self.content.freeze_all();
        } else {
            self.content.freeze_only(vec![self.win.hwnd()]);
        }
    }

    /// Apply "curious about highlighted text": start or stop the selection watcher (its
    /// thread and hooks exist only while the setting is on).
    fn apply_text_curiosity(&mut self) {
        match (self.settings.text_curiosity, self.selection.is_some()) {
            (true, false) => match SelectionWatcher::start() {
                Ok(w) => self.selection = Some(w),
                Err(e) => log::warn!("text selection watcher unavailable: {e:#}"),
            },
            (false, true) => self.selection = None,
            _ => {}
        }
    }

    fn sense_darkness(&mut self, dt: f32) {
        self.dark.1 -= dt;
        if self.dark.1 > 0.0 {
            return;
        }
        self.dark.1 = 2.0;
        let (p, s) = (self.ch.pos, self.ch.size);
        // Around and above him, not the ledge he stands on.
        let r = RectI::new((p.x - 2.0 * s.x) as i32, (p.y - 2.2 * s.y) as i32, (p.x + 2.0 * s.x) as i32, (p.y - 2.0) as i32);
        if let Some(l) = monitor::mean_luma(r) {
            let was = self.dark.0;
            self.dark.0 = if was { l < 0.26 } else { l < 0.18 };
            if self.dark.0 != was {
                log::debug!("surroundings {} (luma {l:.2})", if self.dark.0 { "dark" } else { "light" });
            }
        }
    }

    /// The brain is steering his jumps itself (fights, battles, pester, the Shade): no
    /// automatic dives on top of them.
    fn brain_steers_jumps(&self) -> bool {
        matches!(
            self.brain.behaviour(),
            BehaviourKind::Fight | BehaviourKind::Battle | BehaviourKind::Pester | BehaviourKind::ShadeHunt | BehaviourKind::Gauntlet | BehaviourKind::Train
        )
    }

    fn think(&mut self, dt: f32) -> MoveInput {
        self.stakes_sense();
        self.see_foes();
        let mut benches: Vec<Vec2> = self.benches.placed.iter().map(|(b, _)| b.pos).collect();
        let seat = self.discovery_percepts(&mut benches).or_else(|| self.benches.seat_here(&self.ch, &self.terrain));
        let (left, right) = input::mouse_buttons();
        let cursor = monitor::cursor_pos().map(|(x, y)| Cursor { pos: Vec2::new(x as f32, y as f32), left, right });
        let user = UserActivity { idle_secs: input::idle_secs(), keys_per_sec: self.user_input.keys_per_sec() };
        self.sense_darkness(dt);
        // Non-blocking: the watcher's thread does the UI Automation work.
        let selection = self.selection.as_mut().and_then(|w| {
            w.poll();
            w.rect()
        });
        // A lingering pose already counts as finished for the brain (so it chooses what comes
        // next); its ending after a linger is not cut again.
        let act = self.act.as_ref().filter(|p| !p.is_lingering()).map(|p| RunningAct { id: p.act(), interruptible: p.interruptible(&self.data.acts) && !p.is_released() });
        let percepts = Percepts {
            dt,
            knight: &self.ch,
            terrain: &self.terrain,
            desktop: self.watcher.snapshot(),
            cursor,
            user,
            fullscreen_front: false,
            pokes: &self.pokes,
            act,
            benches: &benches,
            perch: seat.is_some(),
            move_events: &self.brain_events,
            activity: &self.activity,
            last_input: self.last_input,
            local_hour: Some(monitor::local_hour()),
            dark: self.dark.0,
            selection,
        };
        let visitors = self.visitor_percepts();
        self.brain.see_visitors(visitors);
        let intent = self.brain.tick(&percepts, &mut self.nav);
        if let Some(e) = intent.emote {
            log::debug!("emote: {e:?}");
            self.emotes.show(e.into());
        }
        self.apply_strikes(&intent.strikes);
        let quiet = lingering_fits(&intent, self.ch.facing);
        if let Some(id) = intent.greet {
            self.greet_visitor(id);
        }
        if let Some(id) = intent.dream_visitor {
            self.aim_dream_at_visitor(id);
        }
        if let Some(r) = intent.lesson {
            self.lesson_signal(r);
        }
        if let Some(id) = intent.part {
            self.part_from_visitor(id);
        }
        if let Some(ev) = intent.discovery {
            self.on_discovery(ev);
        }
        self.pastime_intent();
        if let Some(ev) = intent.request {
            self.on_world_request(ev);
        }
        if let Some(ev) = intent.gate {
            self.on_gate_event(ev);
        }
        self.pokes.clear();
        self.brain_events.clear();
        self.stakes_events(&intent.stakes);
        self.achievements_intent(&intent);
        if intent.debug != self.brain_debug {
            self.brain_debug = intent.debug;
        }

        let d = &self.data;
        if let Some(p) = self.act.as_mut() {
            // Seated life: tired → nods and dozes, bored → map checks, mischief → fidgets.
            let needs = &self.brain.mind().needs;
            p.set_mood(skong_core::life::Mood { tired: 1.0 - needs.energy, curious: needs.boredom, lively: needs.mischief });
        }
        if intent.cancel_act
            && let Some(p) = self.act.as_mut().filter(|p| p.interruptible(&d.acts))
            && !p.wrap_up(&d.acts, &d.lib, &mut self.anim, false)
        {
            self.act = None; // seated lives wrap up instead: finish the state, then get off
        }
        // Only urgent / higher-tier switches cut a seated state short (via its exit clip);
        // being outscored lets the current state (map, lean, doze) finish first.
        let urgent_switch = self.brain.last_switch().is_some_and(|s| matches!(s.reason, SwitchReason::Urgent | SwitchReason::Preempted));
        // A higher-priority behaviour took over: finish the pose gracefully (get up from the
        // seat, lower the map…) instead of snapping out of the act. Seated exits keep the seat.
        if let Some(id) = intent.exit_act
            && let Some(p) = self.act.as_mut().filter(|p| p.interruptible(&d.acts))
            && !p.wrap_up(&d.acts, &d.lib, &mut self.anim, urgent_switch)
        {
            self.act = Some(ActPlayer::start(&d.acts, &d.lib, &mut self.anim, id, seed()));
        }
        if let Some(sig) = intent.signal
            && let Some(p) = self.act.as_mut()
        {
            p.signal(&d.acts, &d.lib, &mut self.anim, sig);
        }
        // A pose held on after its act (look down, look up, gaze): the same pose chosen again
        // carries on (no ending + start stutter); anything else lets it end first.
        if let Some(p) = self.act.as_mut().filter(|p| p.is_lingering()) {
            if let Some(id) = intent.act
                && p.fuse(&d.acts, &d.lib, &mut self.anim, id)
            {
                log::info!("act '{}' (holding the pose)", d.acts.get(id).name);
            } else if !quiet {
                p.release(&d.acts, &d.lib, &mut self.anim);
            }
        }
        let can_start = self.act.is_none() && self.ch.is_grounded() && !self.loco.busy();
        if let Some(id) = intent.act.filter(|_| can_start) {
            let seated = d.acts.get(id).requires == Requirement::Seat;
            if !seated || seat.is_some() {
                log::info!("act '{}'{}", d.acts.get(id).name, if seated { " (seated)" } else { "" });
                self.seat_y = if seated { seat } else { None };
                self.act = Some(ActPlayer::start(&d.acts, &d.lib, &mut self.anim, id, seed()).allow_linger(LINGER_SECS));
            }
        }
        // A slash that connects with the cursor: a hit marker right on it (the game's slash
        // impact, along the swing), so every landed blow shows before any Geo spills.
        if let Some(a) = intent.attack.filter(|a| a.hit)
            && intent.behaviour == Some(BehaviourKind::Battle)
            && let Some((x, y)) = monitor::cursor_pos()
        {
            self.fx.spawn("cursor_hit", Vec2::new(x as f32, y as f32), a.side == Side::Right);
        }
        if let Some(a) = intent.attack
            && self.act.is_none()
            && self.ch.is_grounded()
            && let Some(id) = d.acts.id(a.dir.act_name())
        {
            self.ch.facing = a.side;
            let slash = match a.dir {
                AttackDir::Forward if seed().is_multiple_of(2) => "slash_alt",
                AttackDir::Forward => "slash",
                AttackDir::Up => "up_slash",
                AttackDir::Down => "down_slash",
            };
            let origin = Vec2::new(self.ch.pos.x, self.ch.pos.y - self.look.feet_offset);
            self.fx.spawn(slash, origin, a.side == Side::Right);
            self.seat_y = None;
            self.act = Some(ActPlayer::start(&d.acts, &d.lib, &mut self.anim, id, seed()));
        }
        if let Some(a) = intent.attack
            && self.act.is_none()
            && matches!(self.ch.state, MoveState::Airborne)
        {
            // Mid-air: the slash plays over the jump (down slashes in drills, flicks).
            if a.dir == AttackDir::Forward {
                self.ch.facing = a.side;
            }
            self.loco.air_slash(a.dir);
            let slash = match a.dir {
                AttackDir::Forward => "slash",
                AttackDir::Up => "up_slash",
                AttackDir::Down => "down_slash",
            };
            self.fx.spawn(slash, self.knight_origin(), self.ch.facing == Side::Right);
        }
        if intent.attack.is_some() && self.act.is_none() && matches!(self.ch.state, MoveState::WallSlide { .. }) {
            // Slashing off a wall: away from it (the knight faces away while sliding).
            self.loco.wall_slash();
            self.fx.spawn("wall_slash", self.knight_origin(), self.ch.facing == Side::Right);
        }
        if let Some(f) = intent.face
            && self.act.is_none()
            && self.ch.is_grounded()
            && self.ch.vel.x.abs() < 1.0
        {
            if f != self.ch.facing {
                self.loco.turned_in_place();
            }
            self.ch.facing = f;
        }
        match intent.motion {
            Motion::Hold => MoveInput::default(),
            Motion::Navigate { .. } => self.nav.next_input(&self.ch, &self.terrain, dt),
            Motion::Direct(i) => i,
        }
    }

    /// Clicks on the knight poke him; press-and-drag picks him up; release throws him.
    fn handle_mouse(&mut self) {
        for ev in self.win.take_mouse() {
            let poke = self.grab.on_event(ev, self.ch.pos);
            if let Some(p) = &poke {
                self.achievements_poke(p);
            }
            match poke {
                Some(Poke::Grab) => {
                    self.act = None;
                    self.pokes.push(Poke::Grab);
                }
                Some(Poke::Release { vel }) => {
                    self.ch.vel = vel;
                    self.pokes.push(Poke::Release { vel });
                }
                Some(Poke::Click { pos }) => {
                    self.pokes.push(Poke::Click { pos });
                }
                None => {}
            }
        }
        self.pokes.truncate(32);
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
                    let pool: Vec<ActId> = self.data.acts.iter().filter(|(_, a)| a.weight > 0.0 && a.requires != Requirement::Seat).map(|(id, _)| id).collect();
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
                self.benches.rescale(s);
                self.discoveries.rescale(s);
                self.shinies.rescale(s);
                self.pastimes.rescale(&self.data.args_data, self.data.vanilla(), s);
                self.emotes.rescale(&self.data.args_data, self.data.vanilla(), 64.0 * s);
                self.navb.reset(&self.terrain, &self.ch);
                self.settings.save();
            }
            Command::SetBother(b) => {
                self.brain.set_bother_level(b);
                self.settings.bother = b;
                self.settings.save();
            }
            Command::ToggleClickThrough => {
                self.settings.click_through = !self.settings.click_through;
                self.win.set_click_through(self.settings.click_through);
                self.settings.save();
            }
            Command::ToggleFightMode => {
                let on = !self.brain.fight_mode();
                self.brain.set_fight_mode(on);
                log::info!("fight mode: {on}");
            }
            Command::ToggleShareVisible => {
                self.settings.share_visible = !self.settings.share_visible;
                self.apply_share_visible();
                self.settings.save();
                log::info!("visible in screen shares: {}", self.settings.share_visible);
            }
            Command::ToggleTextCuriosity => {
                self.settings.text_curiosity = !self.settings.text_curiosity;
                self.apply_text_curiosity();
                self.settings.save();
                log::info!("curious about highlighted text: {}", self.settings.text_curiosity);
            }
            Command::TogglePause => {
                self.paused = !self.paused;
                self.win.set_visible(!self.paused);
                self.emotes.hide();
            }
            Command::GoToCursor { hurry } => self.go_to_cursor(hurry),
            Command::PlaceBenchAtCursor(style) => {
                if let Some((x, y)) = monitor::cursor_pos() {
                    self.place_bench(style, Vec2::new(x as f32, y as f32));
                }
            }
            Command::PlaceBenchNearKnight(style) => {
                let at = Vec2::new(self.ch.pos.x + self.ch.facing.sign() * 90.0 * self.settings.scale, self.ch.pos.y - 4.0);
                self.place_bench(style, at);
            }
            Command::ClearBenches => self.benches.clear(),
            Command::InviteVisitor | Command::DreamNail => return self.apply_visit(c),
            Command::SendEnemy => match self.enemies.as_mut() {
                Some(e) => {
                    e.send();
                    log::info!("enemies: one invited (arrives at the next good spot)");
                }
                None => log::warn!("enemies: not loaded"),
            },
            Command::StartGauntlet => self.start_gauntlet(),
            Command::SummonGrimm { lantern } => self.summon_grimm(lantern),
            Command::Discover(_) | Command::ForgetDiscoveries => return self.apply_discovery(c),
            Command::Quit => return Ok(Flow::Quit),
            Command::Stakes(c) => self.apply_stakes(c),
            Command::ResetAchievements => self.reset_achievements(),
            Command::SetManual(_) | Command::ToggleVision | Command::ToggleDebugOverlay | Command::SetOverlayLayers(_) | Command::OpenPanel => return self.apply_panel(c),
        }
        Ok(Flow::Continue)
    }

    /// Debug (`--sit-test`): bench under the knight, then read the map on it.
    fn sit_test(&mut self) {
        self.place_bench(0, self.ch.pos - Vec2::new(0.0, 4.0));
        let id = self.data.acts.id("read_map");
        if let (Some(i), Some(id)) = (self.benches.placed.len().checked_sub(1), id) {
            self.ch.pos.x = self.benches.placed[i].0.pos.x;
            self.seat_y = Some(self.benches.seat_y(i));
            self.act = Some(ActPlayer::start(&self.data.acts, &self.data.lib, &mut self.anim, id, seed()));
        }
    }

    fn place_bench(&mut self, style: usize, at: Vec2) {
        match self.benches.place(style, at, &self.terrain, &self.occupancy) {
            Ok(()) => self.win.raise(), // keep the knight in front of his furniture
            Err(e) => log::warn!("bench: {e:#}"),
        }
    }

    fn reload_look(&mut self) -> Result<()> {
        let (look, fx_frames) = self.data.load_look(self.skin, self.settings.scale, &self.fx)?;
        self.look = look;
        self.fx.install_frames(fx_frames);
        self.drawn = None;
        self.rescale_visitors();
        self.rescale_enemies();
        self.rescale_troupe();
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
            MenuItem::sub("Place bench", self.benches.styles.iter().enumerate().map(|(i, s)| MenuItem::Action { id: ID_BENCH0 + i as u32, label: s.name.clone(), checked: false, enabled: self.benches.available(i) }).collect()),
            MenuItem::action(ID_CLEAR_BENCHES, "Remove benches"),
            discover::menu(),
            MenuItem::Separator,
            MenuItem::action(ID_ACT, "Do something!"),
            MenuItem::check(ID_FIGHT, "Fight the cursor", self.brain.fight_mode()),
            MenuItem::action(ID_INVITE, "Invite a visitor"),
            MenuItem::action(ID_ENEMY, "Send an enemy"),
            MenuItem::action(ID_GAUNTLET, "Start a gauntlet"),
            MenuItem::action(ID_SHOWER, "Geo shower"),
            MenuItem::action(ID_DREAM, "Dream-nail a window"),
            MenuItem::action(ID_RESPAWN, "Come here (respawn at cursor)"),
            MenuItem::check(ID_MANUAL, "Manual control", self.manual),
            MenuItem::check(ID_CLICK, "Click-through", self.settings.click_through),
            MenuItem::check(ID_SHARE, "Visible in screen shares", self.settings.share_visible),
            MenuItem::check(ID_TEXT, "Curious about highlighted text", self.settings.text_curiosity),
            MenuItem::check(ID_PAUSE, "Pause", self.paused),
            MenuItem::Separator,
            MenuItem::check(ID_VISION, "Vision (content terrain)", self.content.is_enabled()),
            MenuItem::check(ID_OVERLAY, "Debug overlay\tCtrl+Alt+D", self.overlay.is_on()),
            MenuItem::action(ID_PANEL, "Open control panel"),
            MenuItem::Separator,
            MenuItem::action(ID_QUIT, "Quit"),
        ]
    }

    fn menu_command(&self, id: u32) -> Option<Command> {
        Some(match id {
            ID_MANUAL => Command::ToggleManual,
            ID_CLICK => Command::ToggleClickThrough,
            ID_SHARE => Command::ToggleShareVisible,
            ID_FIGHT => Command::ToggleFightMode,
            ID_TEXT => Command::ToggleTextCuriosity,
            ID_PAUSE => Command::TogglePause,
            ID_ACT => Command::RandomAct,
            ID_RESPAWN => Command::RespawnAtCursor,
            ID_QUIT => Command::Quit,
            ID_CLEAR_BENCHES => Command::ClearBenches,
            ID_VISION => Command::ToggleVision,
            ID_OVERLAY => Command::ToggleDebugOverlay,
            ID_PANEL => Command::OpenPanel,
            ID_INVITE => Command::InviteVisitor,
            ID_ENEMY => Command::SendEnemy,
            ID_GAUNTLET => Command::StartGauntlet,
            ID_SHOWER => Command::Stakes(skong_ipc::StakesCommand::SpillGeo),
            ID_DREAM => Command::DreamNail,
            i if (ID_BENCH0..ID_BENCH0 + self.benches.styles.len() as u32).contains(&i) => Command::PlaceBenchNearKnight((i - ID_BENCH0) as usize),
            i if (ID_SKIN0..ID_SKIN0 + self.data.skins.len() as u32).contains(&i) => Command::SetSkin((i - ID_SKIN0) as usize),
            i if (ID_SCALE0..ID_SCALE0 + SCALES.len() as u32).contains(&i) => Command::SetScale(SCALES[(i - ID_SCALE0) as usize].1),
            i if (ID_BOTHER0..ID_BOTHER0 + BOTHER.len() as u32).contains(&i) => Command::SetBother(BOTHER[(i - ID_BOTHER0) as usize].1),
            _ => return discover::menu_command(id),
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

/// How long a pose act (look down, look up, gaze) may be held on after it ends, waiting for
/// the brain to choose the same pose again (its gaps between acts are 0.3–3 s).
const LINGER_SECS: f32 = 3.0;

/// The brain only waits (no new act, no move, no attack, same facing): a lingering pose may
/// stay up. Anything else lets it play its ending.
fn lingering_fits(intent: &Intent, facing: Side) -> bool {
    let still = match &intent.motion {
        Motion::Hold => true,
        Motion::Navigate { .. } => false,
        Motion::Direct(i) => i.x == 0.0 && !i.jump_pressed && !i.jump_held && !i.dash_pressed && !i.superdash_held && !i.down && !i.dive_pressed,
    };
    still && intent.act.is_none() && intent.attack.is_none() && intent.exit_act.is_none() && !intent.cancel_act && intent.face.is_none_or(|f| f == facing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skong_core::acts::ActId;
    use skong_core::ai::Attack;

    #[test]
    fn a_lingering_pose_stays_only_while_the_brain_waits() {
        let wait = Intent::default();
        assert!(lingering_fits(&wait, Side::Left));
        assert!(lingering_fits(&Intent { face: Some(Side::Left), ..Intent::default() }, Side::Left));
        assert!(lingering_fits(&Intent { motion: Motion::Direct(MoveInput::default()), ..Intent::default() }, Side::Left));
        let walk = MoveInput { x: 1.0, ..MoveInput::default() };
        for busy in [
            Intent { act: Some(ActId(0)), ..Intent::default() },
            Intent { face: Some(Side::Right), ..Intent::default() },
            Intent { motion: Motion::Direct(walk), ..Intent::default() },
            Intent { motion: Motion::Navigate { goal: NavGoal::Point(Vec2::ZERO), style: TravelStyle::Stroll }, ..Intent::default() },
            Intent { attack: Some(Attack { dir: AttackDir::Forward, side: Side::Left, hit: false }), ..Intent::default() },
            Intent { exit_act: Some(ActId(1)), ..Intent::default() },
        ] {
            assert!(!lingering_fits(&busy, Side::Left), "{busy:?}");
        }
    }
}
