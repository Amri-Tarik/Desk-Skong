# Desk-Skong

**Desk-Skong** is a Windows desktop pet: the Knight from Hollow Knight lives on your
desktop. Window tops, the taskbar and even lines drawn on screen (table borders, Paint
strokes) are his platforms. He walks, runs, wall-jumps, dashes and crystal-dashes between
them with the game's own movement constants and animations, decides what to do with a
mood-driven brain (keep you company, watch what changes on screen, gather Geo, rest on
benches, fight your cursor…), and stays out of the way of fullscreen games.
Any Custom-Knight-style skin (a folder with `Knight.png`) works.

> ⚠️ Legal: this repository **does not** include Hollow Knight art. You export it from
> your own copy of the game (see *First-time setup*); it stays in the git-ignored `skins/`
> folder. Only metadata JSON (sprite geometry, animation timings) and `classdata.tpk` are
> versioned.

- **Architecture & internals:** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Feature status, open work & backlog:** [docs/STATUS.md](docs/STATUS.md)
- **Asset exporter:** [HK_TK2D_Exporter/readme.md](HK_TK2D_Exporter/readme.md)

---

## What lives in this repo

| Path | What |
|---|---|
| `crates/skong-core` | Platform-independent simulation: assets/animation, acts, controller (movement kit), terrain, vision detection, navigation, AI brain, effects, props, discoveries, stakes (Shade/Geo) |
| `crates/skong-win` | Win32 layer: sprite windows, desktop watcher, DXGI screen capture, tray, debug overlay, input sensing, game detection |
| `crates/skong` | The pet app (`skong.exe`) |
| `crates/skong-ipc` | Pet ⇄ control-panel protocol (named pipe, JSON lines) |
| `crates/skong-panel` | The control panel (`skong-panel.exe`, egui) |
| `data/` | Versioned metadata: exported tk2d collections/animations (`data/hk`), acts, effects, props |
| `HK_TK2D_Exporter/` | C# tool that reads the game's Unity assets and exports metadata + (locally) art |
| `desk-skong/` | Legacy Python prototype (superseded by the Rust crates; kept for history) |

---

## Usage

Build and run the pet (Windows, Rust stable):

```text
cargo build --release -p skong -p skong-panel
target\release\skong.exe [--skin NAME] [--scale F] [--no-vision] [--data DIR] [--skins DIR]
```

The knight lives in the notification area: right-click the tray icon for skins, size,
mischief, benches, manual control, pause, vision, the debug overlay and
**Open control panel**. Only one pet runs at a time. Logs go to stderr (`RUST_LOG=debug`
for more).

**Global hotkeys** (Ctrl+Alt + …): `K` manual control · `A` do something · `W` wake ·
`R` respawn at cursor · `G` walk to the cursor · `H` hurry to the cursor (path-finding with the full move kit) · `S` next skin · `B` bench at cursor ·
`P` pause · `D` debug overlay · `Q` quit. Manual control: arrows move, Up jumps, LShift
dashes, hold LCtrl for a superdash, Z walks, X sprints.

**Control panel** (opens with the pet by default — `--no-panel` / `--panel` override the
setting; also `skong-panel.exe` or tray → *Open control panel*; it must sit next to `skong.exe`): every tray action, live status (knight state, brain, terrain and nav-graph
counts, vision stats, CPU/RAM) and the pet's log stream with level/text filters, copy and
clear. It connects to the running pet over `\\.\pipe\desk-skong` and reconnects by itself
if the pet restarts; several panels can be open at once. While the panel window is
focused and manual control is on (K — a toggle, off by default), the arrows / Up / Shift /
Ctrl / Z / X drive the knight without leaking into other apps, and the plain letters
`K A W R G H S B C P D` do what Ctrl+Alt+letter does (not while typing in a text field).
`skong-panel --self-test` checks a running pet from the command line.

**Debug overlay** (`Ctrl+Alt+D`, tray or panel): draws over the whole desktop — ground
ledges blue, window tops green, detected content ledges magenta, walls orange, nav
surfaces cyan and every nav edge coloured by move kind (legend top-right), with the
knight's current route thick on top. The panel chooses the layers. It is click-through,
invisible to screenshots/vision, and redraws only when terrain, nav graph or route
change. `cargo run -p skong-win --example terrain_debug --release` shows the terrain
layer without the pet.

---

## First-time setup

1. **Tools:** Rust stable (MSVC), .NET 9 SDK (for the exporter), a legal copy of Hollow Knight.
2. **Export the vanilla art** (metadata is already in `data/hk`; this writes atlases into the
   git-ignored `skins/Vanilla`, which is also the fallback for anything a skin lacks):
   ```text
   cd HK_TK2D_Exporter
   dotnet run -c Release -- --data "<Steam>/steamapps/common/Hollow Knight/hollow_knight_Data" ^
       --out ../data/hk --textures ../skins/Vanilla ^
       --animations "Knight,Knight Effects Anim,Strikes,Spells Anim,Dust,Hollow Shade,Grimmchild Anim,Weaverling Anim,Knight Hatchling Anim,Geo"
   dotnet run -c Release -- --data "<…>/hollow_knight_Data" --textures ../skins/Vanilla/props ^
       --export-sprites town_bench,town_bench_lit,guardian_bench,fung_bench_new,tram_interior_bench,mines_small_rock
   ```
   (Re-running `--out ../data/hk` regenerates the committed metadata; it should come out identical.)
   Scene-only art lives in `sharedassets*.assets` files: `--list --files <a,b,…>` shows which
   file holds a library. The secret boss, for example (file numbers of the Unity 6 build):
   ```text
   dotnet run -c Release -- --data "<…>/hollow_knight_Data" --out ../data/hk --textures ../skins/Vanilla ^
       --files sharedassets392.assets,sharedassets6.assets,resources.assets ^
       --animations "Grimm Anim,Grimm Flash Anim,Flameball Anim"
   ```
   Without it he still shows up, drawn as plain shapes.
3. **Optional community skins:** drop skin folders (each containing `Knight.png`, optionally
   `Sprint.png`, `Unn.png`, …) anywhere under `skins/` (up to two levels deep).
4. **Build & run:** see *Usage* above.

Settings persist in `%APPDATA%\Desk-Skong\settings.json`; the discovered-places world in
`%APPDATA%\Desk-Skong\world.json`.

---

## Contributing

- **Don’t commit game assets.** Keep `assets/` and `Managed/` out of git (they’re ignored).
- Improvements to the data format, overlay, or exporter welcome via PR.
- File issues with a short repro (OS, steps, and a redacted JSON snippet if relevant).

---

## Scopes (prefixes for PRs, commits, and branches)

Use these short, consistent scopes. Examples:
- **Branch**: `<scope>_<short-topic>` → `anim_walk_cycle_player`
- **PR title**: `feat(<scope>): <message>` → `feat(path): add A* with turn cost`

### Core game scopes

- **ext** — Extractor (tk2d → PNG+JSON tools, exporters, data prep)
  - *Examples*: `ext_index_picker`, `ext_json_include_fields`, `ext_list_collections`

- **anim** — Sprite Animator (frame sequencing, timing curves, blending, state machines)
  - *Examples*: `anim_stategraph_idle_run`, `anim_event_callbacks`, `anim_lerp_pivots`

- **phys** — Physics and collision Engine (walkable detection, contacts, ledges, edges, slope/stairs)
  - *Examples*: `coll_aabb_resolve`, `coll_ui_edge_probes`, `coll_ceil_floor_snap`

- **path** — Pathfinding (nav extraction, graph building, A*/D*, steering, avoidance)
  - *Examples*: `path_a_star`, `path_waypoint_smoothing`, `path_ledge_chooser`

### Supporting scopes (use when relevant)

- **vis** — Vision / Screen Capture / CNN (grab frames, classify walkables, OCR if any)
  - *Examples*: `vis_win_capture`, `vis_unet_walkable`, `vis_textbox_detector`

- **rend** — Rendering & Overlay (always-on-top window, wgpu, batching, atlases)
  - *Examples*: `rend_win_overlay`, `rend_atlas_blit`, `rend_vsync_timing`

- **core** — Shared utilities (math, time, config, logging, serialization)
  - *Examples*: `core_toml_config`, `core_vec2_math`, `core_logger`

- **docs** — Documentation, READMEs, design notes
  - *Examples*: `docs_data_format`, `docs_anim_spec`

- **build** — Tooling, CI, packaging, scripts
  - *Examples*: `build_ci_cache`, `build_release_bundle`

### Rules

- **Lowercase** scope, **kebab-case** topic: `scope_my-feature`
- One primary scope per PR. If truly cross-cutting, pick the dominant one.
- Prefer small focused branches/PRs; keep titles imperative and specific.

---

## Data format

The exporter's JSON formats (tk2d collections as raw quad geometry + UVs, animation
libraries with fps/wrap mode/loop start/events) are documented in
[HK_TK2D_Exporter/readme.md](HK_TK2D_Exporter/readme.md). Acts, effects and props formats
are described at the top of their files in `data/` and in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
