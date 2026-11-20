# Developing Desk-Skong

Player-facing information lives in the [README](../readme.md). This page is for building,
hacking on and releasing the pet.

- **Architecture & internals:** [ARCHITECTURE.md](ARCHITECTURE.md)
- **Feature status, open work & backlog:** [STATUS.md](STATUS.md), [ROADMAP.md](ROADMAP.md)
- **Manual test checklist:** [TESTING.md](TESTING.md)
- **Asset exporter:** [HK_TK2D_Exporter/readme.md](../HK_TK2D_Exporter/readme.md)

> ⚠️ Legal: this repository **does not** include Hollow Knight art, and must never. You
> export it from your own copy of the game (see *Exporting the art* below); it stays in
> the git-ignored `skins/` folder. Only metadata JSON (sprite geometry, animation timings)
> and `classdata.tpk` are versioned. The release zip is checked for art by
> `scripts/package.ps1`.

---

## What lives in this repo

| Path | What |
|---|---|
| `crates/skong-core` | Platform-independent simulation: assets/animation, acts, controller (movement kit), terrain, vision detection, navigation, AI brain, effects, props, discoveries, stakes (Shade/Geo), persistence helpers |
| `crates/skong-win` | Win32 layer: sprite windows, desktop watcher, DXGI screen capture, tray, debug overlay, input sensing, game detection |
| `crates/skong` | The pet app (`skong.exe`) |
| `crates/skong-ipc` | Pet ⇄ control-panel protocol (named pipe, JSON lines) |
| `crates/skong-panel` | The control panel (`skong-panel.exe`, egui) |
| `data/` | Versioned metadata: exported tk2d collections/animations (`data/hk`), acts, effects, props |
| `HK_TK2D_Exporter/` | C# tool that reads the game's Unity assets and exports metadata + (locally) art |
| `scripts/package.ps1` | Builds the release zip (and guards it against game art) |
| `.github/workflows/` | CI (build, clippy, tests on Windows) and the tag-triggered draft release |
| `desk-skong/` | Legacy Python prototype (superseded by the Rust crates; kept for history) |

---

## Building and running

Build and run the pet (Windows, Rust stable, MSVC):

```text
cargo build --release -p skong -p skong-panel
target\release\skong.exe [--skin NAME] [--scale F] [--no-vision] [--data DIR] [--skins DIR]
```

Release builds of `skong.exe` open no console window. Debug builds (`cargo run -p skong`)
keep it, with the log on stderr (`RUST_LOG=debug` for more). In release, read the log in
the control panel, or turn on *Write a log file* (see *Logs and crash reports*).

The knight lives in the notification area: right-click the tray icon for skins, size,
mischief, benches, manual control, pause, vision, the debug overlay, the log file and
**Open control panel**. Only one pet runs at a time.

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

## Game art: the Rust extractor (players) and the .NET exporter (metadata)

**Players** never run a tool: when no Vanilla art is found, `skong.exe` starts
`skong-panel.exe --setup` and waits for it. That window finds the install
(`skong_extract::install`: Steam registry keys → `libraryfolders.vdf` → every library's
`steamapps/common/<installdir>`, then GOG, Epic manifests, `X:\XboxGames`), or asks for
`hollow_knight.exe`; then `skong_extract::extract` writes the atlases and prop sprites into
`%APPDATA%\Desk-Skong\skins\Vanilla` (through `Vanilla.partial`, swapped in at the end,
with an `extracted.json` record: game folder, Unity version, files). Skins are looked up in
the dev `skins/` folder (or `--skins`) first, then there. Developers can run the same
window by hand: `skong-panel --setup --data data/hk --out <somewhere>\Vanilla`.

**`crates/skong-extract`** (no dependencies beyond `png`, `serde` and `windows` for the registry; ~2500 lines with tests):

- `serialized`: Unity serialized files, formats 17–22 (the game is Unity 6000.0.61f1,
  format 22, **no type trees**); reads only the metadata, objects on demand.
- `texture`: `Texture2D` header (the gap before `m_Width` is picked by plausibility; the
  `m_StreamData` tail is read from the end), pixels inline or from `.resS`; `decode` /
  `bc7`: RGBA32, ARGB32, BGRA32, RGB24, Alpha8, DXT1, DXT5, BC7 (the game's atlases are
  BC7, DXT5 and RGBA32; crunched formats are refused with a clear error).
- `catalog`: a tk2d collection is found by its GameObject's name (`name` in
  `data/hk/collections/*.json`) on a `tk2dSpriteCollectionData` MonoBehaviour; its atlas is
  the `PPtr<Texture2D>` array in the MonoBehaviour's bytes whose texture has the expected
  name and size (from the same JSON), so the script layout never needs decoding. Props are
  Unity `Sprite`s by name (benches and discovery art listed in `data/props.json`), cropped
  from their texture or their `SpriteAtlas` render data.
- What gets extracted follows the committed metadata: **a new collection in `data/hk`
  (or a new prop in `props.json`) is picked up with no code change.**

Checks: `cargo test -p skong-extract` (synthetic files); with the game installed,
`cargo test -p skong-extract --release -- --ignored --nocapture` extracts into a temp folder
and compares every image with `skins/Vanilla` (`SKONG_HK`, `SKONG_REF` override the paths).
At the time of writing: 44 images (38 atlases, 6 props), pixel-identical, ~3 s cold.

**The .NET exporter is dev-only**: it regenerates the metadata in `data/hk` (it decodes the
game's scripts to read tk2d geometry and clips) and can still write a dev `skins/Vanilla`:

1. **Tools:** .NET 9 SDK, a legal copy of Hollow Knight.
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

---

## Saved files

Everything lives in `%APPDATA%\Desk-Skong`:

| File | What |
|---|---|
| `settings.json` | Settings, the wallet and the Shade (`crates/skong/src/settings.rs`) |
| `world.json` | Things found in windows (`skong_core::discovery::World`) |
| `achievements.json` | Achievement progress (`skong_core::achievements`) |
| `logs\skong-<date>.log` | Opt-in log file (`crates/skong/src/logfile.rs`) |
| `logs\crash-<date>_<time>.log` | Crash reports (always written on a panic) |

**Surviving updates and crashes** (`skong_core::persist`): every save is atomic (a
`<file>.tmp` beside it, synced, renamed over the old one), and a file that doesn't parse
is copied to `<file>.bak` (or `.bak-<unix secs>` if that exists) before starting fresh.
Settings carry a `version` (see `Settings::migrate`); a missing field takes its default
(`#[serde(default)]`), and a field the running version doesn't know (written by a newer
one) is kept and written back (`Settings::extra`), so going back a release and forward
again loses nothing. When adding a setting: give it a sensible `Default`, and bump
`VERSION` + add a step to `migrate` only if old files must be *reinterpreted*.

---

## Logs and crash reports

- **Log file** (off by default; tray / panel *Write a log file (for bug reports)*, saved
  in settings): what goes to stderr (`RUST_LOG`, default `info`) also goes to
  `logs\skong-<date>.log` (UTC dates). A file over 4 MiB continues in
  `skong-<date>-2.log`, …; only the newest 5 log files are kept.
- **Crash reports** (always): a panic writes its message, location, thread and a
  backtrace to `logs\crash-<date>_<time>.log` (newest 10 kept) and into the log file if
  it's on. A startup error (e.g. missing data) is kept the same way, since release builds
  have no console. Release builds ship without debug info, so backtraces show few symbol
  names; build with `debug = "line-tables-only"` locally to get file:line frames.
- Nothing is ever uploaded. Tray → *Open log folder* (or the panel's *Open folder*).

---

## Tests and CI

```text
cargo clippy --workspace --all-targets   # 0 warnings
cargo test --workspace
powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -SelfTest
```

CI (`.github/workflows/ci.yml`) runs build, clippy (`-D warnings`), the tests and the
package guard self-test on `windows-latest` for every PR and every push to `master`.
**The runner has no game art** (`skins/` is never committed, nor is the game installed):
tests must only use the committed metadata in `data/`, or images they make themselves
in a temp folder. A test that genuinely needs exported art must look for
`skins/Vanilla` and, when it's absent, print why (`eprintln!("skipped: …")`) and return
early instead of failing. Tests never write to the real `%APPDATA%\Desk-Skong` (use
`std::env::temp_dir()`).

**README GIFs (placeholders).** The `readme_gifs` example renders three scripted scenes
offline from the clips on a drawn desktop: the knight exploring, a fight and a visitor.
It needs the extracted art, and its output is game art, so it writes into git-ignored
`docs/media/` (`STRIP=1` also writes a contact strip of each scene):

```
cargo run -p skong-core --example readme_gifs --release -- data/hk skins/Vanilla docs/media
```

---

## Releasing

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`), update
   `Cargo.lock` (`cargo build`), merge to `master`.
2. Locally, `powershell -ExecutionPolicy Bypass -File scripts\package.ps1` builds
   `dist\Desk-Skong-<version>-windows-x64.zip` to try out: `skong.exe`,
   `skong-panel.exe`, `data\` (metadata JSON tracked by git, nothing else), `README.md`
   and `LICENSE.txt` (the MIT `LICENSE`). The script fails if
   anything in it looks like game art (images, audio, Unity assets, embedded PNG data,
   non-JSON files in `data\`) and checks the finished zip entry by entry.
3. Tag `v<version>` and push the tag: `.github/workflows/release.yml` runs the tests,
   checks the tag matches the Cargo version, packages, and attaches the zip to a **draft**
   GitHub Release with generated notes. Review it, then publish.

---

## Contributing

- **Don’t commit game assets.** Keep `assets/`, `Managed/` and `skins/` out of git (they’re ignored).
- Improvements to the data format, overlay, or exporter welcome via PR.
- File issues with a short repro (OS, steps, and a redacted JSON snippet if relevant);
  attach a log file / crash report from `%APPDATA%\Desk-Skong\logs` if you have one.

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
[HK_TK2D_Exporter/readme.md](../HK_TK2D_Exporter/readme.md). Acts, effects and props formats
are described at the top of their files in `data/` and in [ARCHITECTURE.md](ARCHITECTURE.md).
