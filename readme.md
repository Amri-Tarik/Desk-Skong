# Desk-Skong

**Desk-Skong** is a Windows overlay “pet” that walks, jumps, and reacts to what’s on your screen — powered by computer vision and community-made Hollow Knight skins, any skin that includes the `Knight.png` file will be supported.  

> ⚠️ Legal: This repository **does not** include Hollow Knight assets. Users must provide their own files. Only `classdata.tpk` (a Unity class database) is kept in version control.

---

## What lives in this repo

- `HK_TK2D_Exporter/` — a small one-time tool that converts Hollow Knight’s tk2d sprite collections into our PNG+JSON format.  
  (See its own README inside that folder for usage.)
- Overlay code : renders sprites, reads the screen, runs basic pathfinding/AI.

---

## Usage

Build and run the pet (Windows, Rust stable):

```text
cargo build --release -p skong -p skong-panel
target\release\skong.exe [--skin NAME] [--scale F] [--no-vision] [--data DIR] [--skins DIR]
```

The knight lives in the notification area: right-click the tray icon for skins, size,
mischief, benches, manual control, click-through, pause, vision, the debug overlay and
**Open control panel**. Only one pet runs at a time. Logs go to stderr (`RUST_LOG=debug`
for more).

**Global hotkeys** (Ctrl+Alt + …): `K` manual control · `A` do something · `W` wake ·
`R` respawn at cursor · `G` walk to the cursor · `H` hurry to the cursor (path-finding with the full move kit) · `S` next skin · `B` bench at cursor · `C` click-through ·
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

## Project Goals

- **Zero art pipeline:** Reuse community Hollow Knight skins by reading existing atlases/metadata once.
- **Simple runtime:** The overlay only needs PNG+JSON to animate a character.
- **Screen awareness:** Periodically read screen captures and classify “walkable” surfaces (CNN).
- **Tiny pathfinding/AI:** Keep the character grounded, bounce off walls, climb UI edges, etc.
- **Windows-first overlay:** Always-on-top sprites with smooth 60fps-ish motion.

---

## Roadmap (High Level)

- Overlay renderer (Rust/WGPU) + sprite animator
- Walkable-surface classifier (lightweight CNN) + heuristics
- Pathfinding/physics micro-engine for UI terrain
- Input hooks for playful reactions (optional)
- Packaging and config (choose skin, animation sets)

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

# Setup Python env

install uv (https://docs.astral.sh/uv/)

run 
```bash
cd desk-skong
uv run
```

---

## Data format (what the overlay consumes)

Each skin is represented by:
1) An **atlas PNG** (e.g., `Knight.png`)
2) A **JSON** file (e.g., `Knight_tk2d.json`) describing every sprite/frame

### JSON shape

```json
{
  "AtlasWidth": 4096,
  "AtlasHeight": 4096,
  "Sprites": [
    {
      "Name": "idle_still_020007",
      "PixelRect": { "X": 1119, "Y": 1897, "W": 130, "H": 61 },  // atlas pixels, bottom-left origin
      "Pivot": [0.054687425, -0.39062503],                        // world-units anchor (from tk2d bounds.center)
      "Flip": "TPackerCW",                                        // None | Tk2d | TPackerCW (90° CW)
      "SourceSize": { "W": 350, "H": 186 },                        // original (untrimmed) size in pixels
      "SpriteSourceOffset": { "X": 24, "Y": 18 }                   // bottom-left offset of trimmed rect within the untrimmed
    }
  ]
}
```

### Field Descriptions

| Field                | Description                                                                                                                                                                                                                 |
|----------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `atlasWidth`, `atlasHeight` | Dimensions of the atlas PNG in pixels.                                                                                                                                                                         |
| `sprites[]`          | Array of sprite/frame objects.                                                                                                                                                                                             |
| `name`               | Frame identifier (used for animation sequences).                                                                                                                                                                           |
| `pixelRect`          | Sub-rectangle of the atlas in pixels, using bottom-left origin.<br/>**Note:** If your image API uses top-left origin, convert with:<br/>`yTopLeft = atlasHeight - (y + h)`                                                |
| `pivot`              | World-space anchor point from tk2d (`bounds.center`). Map this to your renderer’s anchor system as needed.                                                                          |
| `flip`               | Atlas rotation flag from tk2d:<br/>- `None` (not rotated)<br/>- `Tk2d` (90° clockwise + horizontal flip)<br/>- `TPackerCW` (90° counter-clockwise)<br/>Respect this when sampling the atlas (rotate UVs or the quad accordingly).                      |
| `sourceSize`         | Untrimmed/original size of the sprite in pixels.                                                                                                                                                                           |
| `spriteSourceOffset` | Offset (in pixels, bottom-left origin) of the trimmed rectangle within the original untrimmed image.<br/>This allows consistent placement even when aggressive packing/trimming is used.                                   |

---

### Additional Notes

- All coordinates in the JSON use **bottom-left origin** (Unity/tk2d convention).
- `pivot` is in **world units** (tk2d/Unity space). If you need pixel units, divide by the collection’s `texelSize` during export (the exporter already does this for trim offsets; `texelSize` is not included in the JSON).
- The combination of `pixelRect`, `sourceSize`, and `spriteSourceOffset` matches [TexturePacker](https://www.codeandweb.com/texturepacker/documentation#json-array) semantics, and is sufficient to reconstruct the full, untrimmed canvas position for each frame.