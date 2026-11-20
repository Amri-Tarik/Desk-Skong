# Desk-Skong architecture

A Rust workspace. Everything that can be pure is pure (`skong-core`, unit-tested without
Windows); the Win32 surface is isolated in `skong-win`; `skong` glues them into the pet.

Coordinates everywhere: **virtual-desktop pixels, x right, y down** (Win32 convention).
Game data is y-up world units; conversion happens once at load. The Knight's sprites are
64 px per game unit; render scale (tray *Size*) multiplies that (`px_per_unit = 64 × scale`,
default scale 0.6).

## Crates

| Crate | Role |
|---|---|
| `skong-core` | Simulation. No platform code. |
| `skong-win` | Win32: windows, capture, input, tray, overlay. |
| `skong` | The pet binary (`skong.exe`). |
| `skong-ipc` | Panel ⇄ pet protocol types (`Command`, `PanelMsg`, `PetMsg`, `Status`). |
| `skong-panel` | Control panel binary (`skong-panel.exe`, eframe/egui); also the first-run art setup window (`--setup`). Its look lives in `ui/style.rs` (Hallownest at dusk: near-black blue-grey, bone-white text, muted silver lines, square frames with corner ticks, drawn ornamental dividers, Cinzel headings, Crimson Pro body at 17 px / subtext 15 px, a muted ember only for the sandbox warning). `geo.rs` rasterizes the wallet's Geo icon at runtime from the player's extracted art (the pet's own `Geo`/`coin0000` sprite; a drawn pale stone without it). |
| `skong-extract` | Reads the atlases and prop sprites from the player's Hollow Knight install (Unity serialized files, BC7/DXT/RGBA decoding) into `%APPDATA%\Desk-Skong\skins\Vanilla`; finds the install. See [DEVELOPING.md](DEVELOPING.md). |

### `skong-core` modules

| Module | What it does |
|---|---|
| `assets` | Loads exported tk2d collections; rasterizes each sprite's quad from its UVs (trim, pivot and atlas rotation exact), supersampled + alpha-trimmed. `SpriteBank::rasterize_sets` / `rasterize_each` decode each atlas once for many scaled key sets. `FrameStore` keeps frames PNG-packed (lossless, ~1/4–1/6 the size) and unpacks each on first draw (≤ 1.3 ms, usually µs), evicting frames idle for a minute: knight, effects and stakes use it. Skin discovery + Custom Knight file mapping, vanilla fallback. |
| `anim` | `ClipLibrary` + deterministic `Animator` (all tk2d wrap modes, frame events). |
| `acts` | Data-driven choreographies (`data/acts.json` + `data/acts/*.json`): steps with hold/repeat/until-signal/move-gait/turn; `Requirement::{Grounded, Any, Seat}`. |
| `locomotion` | Controller state + events → Knight clip (accent one-shots, gait cycles, `…ToIdle` hand-overs after acts). A carried item (`set_carry`, from `Intent::carry`) swaps the ground idle / walk for `Map Idle` / `Map Walk` (+ `Map Turn`) or `Lantern Idle` / `Lantern Run`. |
| `controller` | The movement kit with the game's own constants (read from the HeroController prefab): walk/run/sprint, variable jump, double jump, wall slide/jump, dash, down-dash, shadow dash, superdash (ground/wall charge), Desolate Dive/Descending Dark, hard/super-hard landing, drop-through (hold down or down+jump), pogo (`MoveInput::pogo` → `MoveEvent::Pogo`: a mid-air down slash that hits bounces him and refills double jump + air dash). Swept collision; deterministic (the planner forward-simulates it). |
| `terrain` | Collision model: every window is a platform box (top = one-way ledge, sides = walls blocking from outside, bottom passable), occlusion-clipped, work-area clipped; monitor ground + outer walls; no window walls on monitor edges (seams stay open). Content ledges/walls from vision (`LedgeKind::Content`, `WallKind::Content`) stay solid while the knight is inside their window. |
| `vision` | Pure detection on luma: top surfaces and vertical content walls (adaptive contrast, works on dark themes), persistent `SurfaceMap` with dirty-rect rescans, hysteresis, freeze-under-our-windows; screen-activity heat map (`Activity` spots). |
| `nav` | Surface graph built by simulating the controller (cached per surface pair, incremental rebuilds), A* with `TravelStyle` (Stroll / Normal / Hurry), `PathFollower` replaying validated input scripts with re-planning; `nav::debug` for the overlay. |
| `ai` | The brain: percepts → needs/emotions/personality → interest map → utility-scored behaviours with commitment/hysteresis (priority tiers in flight) → `Intent` (motion, acts, signals, attacks, facing, reason). Behaviours: companion, watch activity, explore, rest/nap/visit, battle, shade hunt, loot, shiny, flee, hide, ride, recover, death, greet, train and company (NPC visitors, their lessons and calm scenes: `npc::company`), scene (`ai::scene`, cued by `Brain::see_scene`: Hornet's race on a Hurry route, watching Grimm's friendly visit, lingering by a scarlet flame as a pastime)… Carried items (`ai::carry`): the map (`take_out_map`, 15–40 s, `put_map_away`) and the lantern (while it's dark) only change his look via `Intent::carry` while idling / keeping company / exploring; anything needing his hands (another behaviour, a non-glance act, hurrying, leaving the ground) puts it away. Handling (`ai::handling`): clicks try his patience (a look, an anger mark, then a provoked cursor battle that still waits for anything more important), a shake while held leaves him dizzy (`dizzy`, urgent, the Stun clip), repeated throws make him wary of the cursor, a gentle carry is fine; clicks no longer make him flee. Expressions (`ai::expression`): rare emote bubbles from behaviour switches, endings and need thresholds with hysteresis, each with a cooldown, sent as `Intent::emote` (drawn by `skong/src/emotes.rs`). Fights use the whole kit (`ai::combat`: pogo chains, Desolate Dive onto foes below, shadow-dash dodges through lunges, crystal-dash openers, wall leaps; spells and nail arts by situation via `SpellCues`; Focus after a hurt fight), each move with a reason and a cooldown. Anti-ping-pong: reachable targets only (`Navigator::reach`), failure cooldowns, oscillation guard. |
| `effects` | Hero effect definitions (`data/effects.json`, offsets/scales/rotations from the Knight prefab) + placement/mirroring. |
| `props` | Benches: placement on ledges, riding windows, dropping when the window goes. |
| `discovery` | Things found inside windows (benches, geo deposits) per app, deterministic rolls, window-relative placement, persistence. A find the user dragged (`World::move_find`) moves into the app room of the window it landed on (a deposit takes that app's region look), or lies on a monitor floor (`Find::floor`, still its own room's); visits and hits go with it. |
| `drag` | The user's hand on props: what is draggable (`DragKind`: the training dummy, visitors, Geo deposits, placed and found benches; not enemies, Grimm or his flames, the Shade, coins, shinies, lumaflies — easy to extend), and the drop (`Fall`: a falling body under the visitors' gravity, keeping a little of the hand's swing, landing on the first ledge below that is wide enough: a title bar / the floor / a *settled* content line; let go over a fullscreen game's monitor it drops from the nearest point of the nearest other monitor; nothing below: the work-area floor). `sitting_on` (a seated knight's bench). Visitors' side: `Npc::grab` / `hold_at` / `set_down` (a talk pauses and resumes from its line; lessons, spars, calm scenes, Hornet's challenges end and they take their leave; their `handled` lines in `npcs.json`). The dummy's: `Yard::moved` pins it (never relocated by itself). |
| `particles` | The game's Unity particle bits recreated procedurally (run / land / jump / dash / wall dust, crystal dash shards, Geo sparkles): seeded bursts and streams with velocity, gravity, drag and fade, drawn into their bounding box (one small click-through window per burst in `fx.rs`, ≤ 8 bursts, 60 fps). |
| `shiny` | Shinies: short-lived glints on lasting ledges / the floor (shared occupancy, never on game monitors, reachable), spawn timer and rewards, the procedural twinkle (`glint`). Not persisted. |
| `pastime` | Pastime props: the training dummy (`Yard`: placed like shinies on settled ledges, never on a game monitor; stays minutes, fades; blows land after a delay and pick its `Light/Heavy Hit L/R` clip; a drawn stand-in) and lumaflies (`Sky`: drift from title bar to title bar on his monitor, bob, dart off when swiped, sometimes caught; the procedural `glow`). The brain side is `ai::pastime` (the `Pastime` behaviour: dummy drills, the lumafly chase, the view from a high window edge, stargazing at night; Ambient. A dummy or lumafly there beats plain company at once (0.75–0.79, below the summoned flame's 0.8); the view and the stars are driven by boredom and time in plain company). |
| `placement` | The Test tab's "📍 Place…": `snap_below` takes a clicked point to the nearest ledge at or below it (a few px of slack above), on the click's monitor, never a game's, kept off the ledge's ends. Forced visitor scenes and their "can't …" reasons live in `npc::forced`. |
| `dreamgate` | Dream Gate fallback travel: gate spots (settled ledge, headroom) and the app-side `GateRuntime` (gate glow, the jump as the warp act ends). The brain side (`ai::dream_gate`) picks an unreachable engagement prize, with a cooldown. |
| `stakes` | Shade simulation, Geo coins/physics/wallet, spellbook (soul, spells, nail arts), sources (battles, shattered windows, showers). |
| `enemy` | Enemies (`data/enemies.json`): arrival planning (walk in from a monitor edge, drop onto a settled ledge, fly in), the sim (ride windows, patrol / hover, notice, chase, wind-up + lunge, recoil + flash, death clip, corpse fade), contact bumps; seen by the brain as `Foe`s (`ai::fight`, the `Fight` behaviour). Zote's spar lives in `npc::spar` + the visitor actor; Hornet's race and duel in `npc::challenge` / `npc::race` / `npc::duel` (app side: `npcs/challenge.rs`). `enemy::gauntlet`: the gauntlet director (triggers, start rules, waves, reward, aborts; pure, driven by `enemies.rs`), seen by the brain as a `GauntletCue` (`ai::gauntlet`, the `Gauntlet` ceremony behaviour). `enemy::grimm`: the secret boss (`data/grimm.json`): `GrimmDirector` (the scarlet lantern once enough achievements are unlocked — the user clicks it, it waits ~30 min —, his rare friendly visit `Cameo`, debug summons, start rules, aborts, endings) and `Grimm` (teleports, dive / ground dash, slashes + uppercut, firebats, flame pillars, spikes, the balloon at each phase; every spot relative to the knight's ledge, flames ignore windows). Seen by the brain as a `SceneCue` (the flame, the cameo), `FoeKind::Boss` (fought at 0.98, lethal: `fight.boss_masks`) and `FoeKind::Hazard` (his flames: `Fight::evade` dodges, jumps, steps off pillars, keeps clear of the balloon). |
| `achievements` | Hidden achievements (`data/achievements.json`): the `Tracker` consumes a small `AchievementEvent` stream and holds every criterion (counts, "every region", "never touched the floor"); progress saves as `AchievementSave`. |
| `banner` | The achievement banner's pure side: the queue and timing (`Banners`: fade in 0.6 s, hold 4 s, fade out 1 s, 0.5 s between queued ones, ≤ 12 waiting), the "never over a fullscreen game or app" rule (`blocked_monitors`: a game guard monitor, or one whose frontmost window is fullscreen; a blocked banner vanishes at once and replays later unless it was fading out), `pick_monitor` (the knight's, else the nearest free one, else wait), `origin` (centred, 7 % below the work area's top), description wrapping, and the drawn mist + ornamental divider. |
| `fonts` | The two bundled fonts (SIL OFL, `crates/skong-core/fonts/`, licence in `OFL.txt`, shipped as `FONTS-OFL.txt`): Cinzel (engraved capitals, standing in for the game's Trajan) and Crimson Pro (body), embedded with `include_bytes!`; `family_name` reads a TTF's name table. Not game art. |
| `canvas`, `geom`, `desktop` | Premultiplied BGRA images, rects/vectors, desktop snapshot types. |

### `skong-win` modules

| Module | What it does |
|---|---|
| `sprite_window` | Small topmost per-pixel-alpha layered window (`UpdateLayeredWindow`), capture-excluded (`WDA_EXCLUDEFROMCAPTURE`), flip without allocation, mouse events with capture (grab/throw). One per knight / bench / effect / prop. Layered windows hit-test per pixel (fully transparent pixels pass clicks through); `set_click_through` (`WS_EX_TRANSPARENT`) makes the whole window pass them. `set_opacity` fades the whole window (`SourceConstantAlpha`) without re-uploading pixels. Click-through: effects, bubbles, emotes, the achievement banner, enemies, Grimm, coins, shinies, lumaflies. Taking the mouse: the knight, the scarlet flame, and the draggable props (dummy, visitors, finds, benches). |
| `runloop` | High-resolution waitable timer at 120 Hz + `MsgWaitForMultipleObjectsEx` message pump (never spins), global hotkeys. |
| `desktop` | `DesktopWatcher`: event-driven (`SetWinEventHook`) window/monitor enumeration → `DesktopSnapshot`; per-monitor-v2 DPI. |
| `capture` | DXGI Desktop Duplication, one thread per output, blocks until the screen changes, reads back only dirty rects, ≤30 Hz, self-exclusion, rate limiting of animated regions; feeds vision + activity (`ContentFeed`). |
| `tray` | Notify icon (the knight's own idle frame) + data-driven popup menu. |
| `debug_overlay` | Full-desktop click-through overlay drawing terrain, nav surfaces/edges, route, legend; redraws only on change. |
| `input` | Idle time, typing *rate* via Raw Input (counts only), mouse buttons. |
| `click_capture` | Only while the Test tab places something: low-level mouse + keyboard hooks on their own thread take the next left click anywhere (swallowed, down and up), right-click / Esc cancel; clicks on the control panel pass through. Never moves the cursor. Unhooked on drop; never installed by tests. |
| `games` | Fullscreen-game detection (Windows GameConfigStore registry, game-library paths, exclusive fullscreen). |
| `monitor` | Cursor position, primary work area, single-instance mutex, local time. |
| `text` | GDI text into per-pixel-alpha images (`TextPainter`: speech bubbles, emotes, the place hint, the banner); `register_font` makes an embedded font selectable by name in this process only (`AddFontMemResourceEx`). |
| `selection` | On by default (tray / panel toggle): where highlighted text is (rectangle only, never the text). Background MTA thread: WinEvent `EVENT_OBJECT_TEXTSELECTIONCHANGED` / foreground hooks → debounced UI Automation query (focused element's TextPattern selection bounds). |

### `skong` (the pet) — one tick (120 Hz)

```
mouse (grab/throw/clicks) → the hand on props (drag, falls) → desktop watcher poll → game guard → content feed poll
  → terrain rebuild if anything changed (window boxes + content, game monitors cut out,
    new generation) → controller.on_terrain_rebuilt, benches/props follow, nav rebuild request
  → intent: manual keys | forced trip (go-to-cursor) | brain.think(percepts)
  → controller.step (or held by the user) → locomotion / acts → animator
  → draw knight (present only when the frame changes, else move) → effects → stakes/discoveries
```

Background threads: nav graph builder (coalesces bursts, incremental), one capture thread
per monitor, pipe server threads for panels. The tick only does non-blocking hand-offs.

Key app files: `app.rs` (Pet + tick + commands + tray), `app/panel.rs` (status/IPC glue),
`app/stakes.rs`, `app/discover.rs` (discoveries + shinies), `pastimes.rs` + `app/pastime.rs` (the training dummy and lumaflies), `app/achievements.rs` (the one place
events become achievement events; announce, save, panel copy), `banner.rs` (the achievement "area title" banner: its window and GDI text with the bundled fonts; logic in `skong_core::banner`; ticked every frame, also while paused — hidden then, the queue held), `enemies.rs` + `app/enemy.rs` (enemies, strikes, spar glue), `grimm.rs` + `app/grimm.rs` (the secret boss: art loaded on a background thread when a show draws near, his windows, lines in speech bubbles; world glue), `benches.rs`, `shinies.rs`, `fx.rs` (pooled effect windows),
`app/test.rs` + `placehint.rs` (the panel's Test tab: forced visitors / scenes / enemies / pastimes, placing where the user clicks, the feedback notes in `Status::test`),
`app/drag.rs` (the user's hand on props and visitors: press / drag / let go on their windows via `grab.rs`'s `Grab`, the fall, the consequences),
`gameguard.rs`, `navbuild.rs`, `grab.rs`, `control.rs` (pipe server), `logring.rs`
(logger + ring buffer for panels), `logfile.rs` (opt-in log file, crash reports via a panic
hook), `settings.rs`, `keyboard.rs`, `browser.rs` (`--browse`). Saves go through
`skong_core::persist` (atomic writes, malformed files kept as `.bak`); see
[DEVELOPING.md](DEVELOPING.md#saved-files).

**The panel is the app's window.** The pet launches `skong-panel.exe` on start (setting
`open_panel`; `--no-panel` skips it). Closing the panel sends `Quit`; its **Hide to tray**
just ends the panel process. A tray left-click (or launching `skong.exe` again) raises a
connected panel (`PetMsg::Raise`) or launches a fresh one, which also recovers from a
crashed / killed panel (`control::Reopen`). A quitting pet broadcasts `PetMsg::Bye` and the
panels close with it.

**Sandbox.** "Make him do X" commands and debug tools (`Command::needs_sandbox`) are
refused by `Pet::apply` (whoever sends them: hotkey, tray, panel) until
`Command::EnableSandbox`, which lasts until restart: `Tracker::set_paused` makes the
achievement tracker ignore every event, the Geo wallet stops saving, the tray shows its
Sandbox submenu, `Status::sandbox` turns on. Forcing debug flags (`--visit`, `--goto-test`,
…) and `--sandbox` start with it on.

## Data files (`data/`)

| File | Content |
|---|---|
| `hk/collections/*.json`, `hk/animations/*.json` | Exported tk2d metadata (geometry, clips). Regenerate with the exporter. |
| `acts.json`, `acts/*.json` | Behaviour choreographies (any `data/acts/*.json` is merged in). |
| `npcs.json` | NPC visitors: art clips, arrival / leave, lines, demos, lessons, spars and calm scenes (`company`; the knight's part in `acts/company.json`) (schema in its `$comment`). |
| `enemies.json` | Enemies: clips, movement, hp, speeds, aggro, size, Geo, schedule and group rules (schema in its `$comment`). |
| `effects.json` | Hero effects with prefab offsets/scales/rotations. |
| `props.json` | Bench styles (sprite, seat height), discovery art, and shiny rates (`shinies`). |
| `achievements.json` | Hidden achievements: id, title + description (shown once unlocked), vague hint, `goal` (schema in its `$comment`). |
| `grimm.json` | The secret boss: art libraries and scale, hp, phases, tempo, reward, aborts, the lantern's rules, his (original) lines (schema in its `$comment`). |

Local, git-ignored: `skins/Vanilla/` (exported atlases, `props/` sprites), community skins
under `skins/`, `%APPDATA%\Desk-Skong\{settings,world,achievements}.json` and
`%APPDATA%\Desk-Skong\logs\` (log and crash files).

## Debug flags & tools

`skong.exe` flags: `--skin`, `--scale`, `--data`, `--skins`, `--no-vision`, `--panel` /
`--no-panel`, `--capturable` (let screenshots see our windows), `--capture-stats`,
`--browse` (clip/act/skin browser), `--sandbox`, and (each turning the sandbox on)
`--sit-test`, `--fx-test`, `--goto-test`, `--move-test`, `--emote-test`, `--visit`, `--dream-test`.
Examples: `skong-core`: `contact_sheet`, `fx_sheet`, `particles_bench`, `nav_bench`, `vision_bench`, `vision_png`;
`skong-win`: `terrain_debug`, `capture_probe`. `skong-panel --self-test` inspects a running pet
(`--self-test-overlay` needs the sandbox on).
The panel's **Sandbox** tab (after its consent) triggers each interaction on demand; the
manual checklist is [TESTING.md](TESTING.md).

## Performance (release, 2560×1440 + 1920×1080)

Pet idle ≈ 0–1 % of a core without vision, ≈ 1–3 % with live content capture on a busy
desktop; ~20–60 MB working set depending on scale/skin. Nav graph builds 1–60 ms on a
background thread (incremental cache). Panel idle ≈ 2 %, 77–139 MB (egui/glow).

## Conventions

Branches `<scope>_<kebab-topic>`, PR titles `feat(<scope>): …`, squash-merge. Scopes:
ext, anim, phys, path, vis, rend, core, ai, npc, docs, build. Wide hand formatting — don't
run `cargo fmt` on the whole tree. `cargo clippy --workspace --all-targets` and
`cargo test --workspace` must be clean.
