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
| `skong-panel` | Control panel binary (`skong-panel.exe`, eframe/egui). |

### `skong-core` modules

| Module | What it does |
|---|---|
| `assets` | Loads exported tk2d collections; rasterizes each sprite's quad from its UVs (trim, pivot and atlas rotation exact), supersampled + alpha-trimmed. `SpriteBank::rasterize_sets` / `rasterize_each` decode each atlas once for many scaled key sets. `FrameStore` keeps frames PNG-packed (lossless, ~1/4–1/6 the size) and unpacks each on first draw (≤ 1.3 ms, usually µs), evicting frames idle for a minute: knight, effects and stakes use it. Skin discovery + Custom Knight file mapping, vanilla fallback. |
| `anim` | `ClipLibrary` + deterministic `Animator` (all tk2d wrap modes, frame events). |
| `acts` | Data-driven choreographies (`data/acts.json` + `data/acts/*.json`): steps with hold/repeat/until-signal/move-gait/turn; `Requirement::{Grounded, Any, Seat}`. |
| `locomotion` | Controller state + events → Knight clip (accent one-shots, gait cycles, `…ToIdle` hand-overs after acts). |
| `controller` | The movement kit with the game's own constants (read from the HeroController prefab): walk/run/sprint, variable jump, double jump, wall slide/jump, dash, down-dash, shadow dash, superdash (ground/wall charge), Desolate Dive/Descending Dark, hard/super-hard landing, drop-through (hold down or down+jump), pogo (`MoveInput::pogo` → `MoveEvent::Pogo`: a mid-air down slash that hits bounces him and refills double jump + air dash). Swept collision; deterministic (the planner forward-simulates it). |
| `terrain` | Collision model: every window is a platform box (top = one-way ledge, sides = walls blocking from outside, bottom passable), occlusion-clipped, work-area clipped; monitor ground + outer walls; no window walls on monitor edges (seams stay open). Content ledges/walls from vision (`LedgeKind::Content`, `WallKind::Content`) stay solid while the knight is inside their window. |
| `vision` | Pure detection on luma: top surfaces and vertical content walls (adaptive contrast, works on dark themes), persistent `SurfaceMap` with dirty-rect rescans, hysteresis, freeze-under-our-windows; screen-activity heat map (`Activity` spots). |
| `nav` | Surface graph built by simulating the controller (cached per surface pair, incremental rebuilds), A* with `TravelStyle` (Stroll / Normal / Hurry), `PathFollower` replaying validated input scripts with re-planning; `nav::debug` for the overlay. |
| `ai` | The brain: percepts → needs/emotions/personality → interest map → utility-scored behaviours with commitment/hysteresis (priority tiers in flight) → `Intent` (motion, acts, signals, attacks, facing, reason). Behaviours: companion, watch activity, explore, rest/nap/visit, battle, shade hunt, loot, shiny, flee, hide, ride, recover, death, greet, train and company (NPC visitors, their lessons and calm scenes: `npc::company`), scene (`ai::scene`, cued by `Brain::see_scene`: Hornet's race on a Hurry route, watching Grimm's friendly visit, lingering by a scarlet flame as a pastime)… Expressions (`ai::expression`): rare emote bubbles from behaviour switches, endings and need thresholds with hysteresis, each with a cooldown, sent as `Intent::emote` (drawn by `skong/src/emotes.rs`). Fights use the whole kit (`ai::combat`: pogo chains, Desolate Dive onto foes below, shadow-dash dodges through lunges, crystal-dash openers, wall leaps; spells and nail arts by situation via `SpellCues`; Focus after a hurt fight), each move with a reason and a cooldown. Anti-ping-pong: reachable targets only (`Navigator::reach`), failure cooldowns, oscillation guard. |
| `effects` | Hero effect definitions (`data/effects.json`, offsets/scales/rotations from the Knight prefab) + placement/mirroring. |
| `props` | Benches: placement on ledges, riding windows, dropping when the window goes. |
| `discovery` | Things found inside windows (benches, geo deposits) per app, deterministic rolls, window-relative placement, persistence. |
| `particles` | The game's Unity particle bits recreated procedurally (run / land / jump / dash / wall dust, crystal dash shards, Geo sparkles): seeded bursts and streams with velocity, gravity, drag and fade, drawn into their bounding box (one small click-through window per burst in `fx.rs`, ≤ 8 bursts, 60 fps). |
| `shiny` | Shinies: short-lived glints on lasting ledges / the floor (shared occupancy, never on game monitors, reachable), spawn timer and rewards, the procedural twinkle (`glint`). Not persisted. |
| `pastime` | Pastime props: the training dummy (`Yard`: placed like shinies on settled ledges, never on a game monitor; stays minutes, fades; blows land after a delay and pick its `Light/Heavy Hit L/R` clip; a drawn stand-in) and lumaflies (`Sky`: drift from title bar to title bar on his monitor, bob, dart off when swiped, sometimes caught; the procedural `glow`). The brain side is `ai::pastime` (the `Pastime` behaviour: dummy drills, the lumafly chase, the view from a high window edge, stargazing at night; Ambient, driven by boredom and time in plain company). |
| `dreamgate` | Dream Gate fallback travel: gate spots (settled ledge, headroom) and the app-side `GateRuntime` (gate glow, the jump as the warp act ends). The brain side (`ai::dream_gate`) picks an unreachable engagement prize, with a cooldown. |
| `stakes` | Shade simulation, Geo coins/physics/wallet, spellbook (soul, spells, nail arts), sources (battles, shattered windows, showers). |
| `enemy` | Enemies (`data/enemies.json`): arrival planning (walk in from a monitor edge, drop onto a settled ledge, fly in), the sim (ride windows, patrol / hover, notice, chase, wind-up + lunge, recoil + flash, death clip, corpse fade), contact bumps; seen by the brain as `Foe`s (`ai::fight`, the `Fight` behaviour). Zote's spar lives in `npc::spar` + the visitor actor; Hornet's race and duel in `npc::challenge` / `npc::race` / `npc::duel` (app side: `npcs/challenge.rs`). `enemy::gauntlet`: the gauntlet director (triggers, start rules, waves, reward, aborts; pure, driven by `enemies.rs`), seen by the brain as a `GauntletCue` (`ai::gauntlet`, the `Gauntlet` ceremony behaviour). `enemy::grimm`: the secret boss (`data/grimm.json`): `GrimmDirector` (the scarlet lantern once enough achievements are unlocked — the user clicks it, it waits ~30 min —, his rare friendly visit `Cameo`, debug summons, start rules, aborts, endings) and `Grimm` (teleports, dive / ground dash, slashes + uppercut, firebats, flame pillars, spikes, the balloon at each phase; every spot relative to the knight's ledge, flames ignore windows). Seen by the brain as a `SceneCue` (the flame, the cameo), `FoeKind::Boss` (fought at 0.98, lethal: `fight.boss_masks`) and `FoeKind::Hazard` (his flames: `Fight::evade` dodges, jumps, steps off pillars, keeps clear of the balloon). |
| `achievements` | Hidden achievements (`data/achievements.json`): the `Tracker` consumes a small `AchievementEvent` stream and holds every criterion (counts, "every region", "never touched the floor"); progress saves as `AchievementSave`. |
| `canvas`, `geom`, `desktop` | Premultiplied BGRA images, rects/vectors, desktop snapshot types. |

### `skong-win` modules

| Module | What it does |
|---|---|
| `sprite_window` | Small topmost per-pixel-alpha layered window (`UpdateLayeredWindow`), capture-excluded (`WDA_EXCLUDEFROMCAPTURE`), flip without allocation, mouse events with capture (grab/throw). One per knight / bench / effect / prop. |
| `runloop` | High-resolution waitable timer at 120 Hz + `MsgWaitForMultipleObjectsEx` message pump (never spins), global hotkeys. |
| `desktop` | `DesktopWatcher`: event-driven (`SetWinEventHook`) window/monitor enumeration → `DesktopSnapshot`; per-monitor-v2 DPI. |
| `capture` | DXGI Desktop Duplication, one thread per output, blocks until the screen changes, reads back only dirty rects, ≤30 Hz, self-exclusion, rate limiting of animated regions; feeds vision + activity (`ContentFeed`). |
| `tray` | Notify icon (the knight's own idle frame) + data-driven popup menu. |
| `debug_overlay` | Full-desktop click-through overlay drawing terrain, nav surfaces/edges, route, legend; redraws only on change. |
| `input` | Idle time, typing *rate* via Raw Input (counts only), mouse buttons. |
| `games` | Fullscreen-game detection (Windows GameConfigStore registry, game-library paths, exclusive fullscreen). |
| `monitor` | Cursor position, primary work area, single-instance mutex, local time. |
| `selection` | On by default (tray / panel toggle): where highlighted text is (rectangle only, never the text). Background MTA thread: WinEvent `EVENT_OBJECT_TEXTSELECTIONCHANGED` / foreground hooks → debounced UI Automation query (focused element's TextPattern selection bounds). |

### `skong` (the pet) — one tick (120 Hz)

```
mouse (grab/throw/clicks) → desktop watcher poll → game guard → content feed poll
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
events become achievement events; announce, save, panel copy), `enemies.rs` + `app/enemy.rs` (enemies, strikes, spar glue), `grimm.rs` + `app/grimm.rs` (the secret boss: art loaded on a background thread when a show draws near, his windows, lines in speech bubbles; world glue), `benches.rs`, `shinies.rs`, `fx.rs` (pooled effect windows),
`gameguard.rs`, `navbuild.rs`, `grab.rs`, `control.rs` (pipe server), `logring.rs`
(logger + ring buffer for panels), `settings.rs`, `keyboard.rs`, `browser.rs` (`--browse`).

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
under `skins/`, `%APPDATA%\Desk-Skong\{settings,world,achievements}.json`.

## Debug flags & tools

`skong.exe` flags: `--skin`, `--scale`, `--data`, `--skins`, `--no-vision`, `--panel` /
`--no-panel`, `--capturable` (let screenshots see our windows), `--capture-stats`,
`--browse` (clip/act/skin browser), `--sit-test`, `--fx-test`, `--goto-test`, `--move-test`.
Examples: `skong-core`: `contact_sheet`, `fx_sheet`, `particles_bench`, `nav_bench`, `vision_bench`, `vision_png`;
`skong-win`: `terrain_debug`, `capture_probe`. `skong-panel --self-test` inspects a running pet.

## Performance (release, 2560×1440 + 1920×1080)

Pet idle ≈ 0–1 % of a core without vision, ≈ 1–3 % with live content capture on a busy
desktop; ~20–60 MB working set depending on scale/skin. Nav graph builds 1–60 ms on a
background thread (incremental cache). Panel idle ≈ 2 %, 77–139 MB (egui/glow).

## Conventions

Branches `<scope>_<kebab-topic>`, PR titles `feat(<scope>): …`, squash-merge. Scopes:
ext, anim, phys, path, vis, rend, core, ai, npc, docs, build. Wide hand formatting — don't
run `cargo fmt` on the whole tree. `cargo clippy --workspace --all-targets` and
`cargo test --workspace` must be clean.
