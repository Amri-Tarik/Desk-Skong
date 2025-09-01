# Desk-Skong

**Desk-Skong** is a Windows overlay “pet” that walks, jumps, and reacts to what’s on your screen — powered by community-made Hollow Knight skins, any skin that includes the `Knight.png` file will be supported.  

> ⚠️ Legal: This repository **does not** include Hollow Knight assets. Users must provide their own files. Only `classdata.tpk` (a Unity class database) is kept in version control.

---

## What lives in this repo

- `HK_TK2D_Exporter/` — a small one-time tool that converts Hollow Knight’s tk2d sprite collections into our PNG+JSON format.  
  (See its own README inside that folder for usage.)
- Overlay code (planned): renders sprites, reads the screen, runs basic pathfinding/AI.

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

## Data format (what the overlay consumes)

Each skin is represented by:
1) An **atlas PNG** (e.g., `Knight.png`)
2) A **JSON** file (e.g., `Knight_tk2d.json`) describing every sprite/frame

### JSON shape

```json
{
  "atlasWidth": 4096,
  "atlasHeight": 4096,
  "collectionName": "",
  "texelSize": [0.01, 0.01],
  "sprites": [
    {
      "name": "idle_still_020007",
      "uvRect": { "uMin": 0.2731936, "vMin": 0.463135, "uMax": 0.3049314, "vMax": 0.4780271 },
      "pixelRect": { "x": 1119, "y": 1897, "w": 130, "h": 61 },
      "bounds": {
        "center": [0.054687425, -0.39062503, 0],
        "extents": [0.9531249, 2.03125, 0]
      },
      "untrimmedBounds": {
        "center": [0.0078125, 0, 0],
        "extents": [5.453125, 2.90625, 0]
      },
      "pivot": [0.054687425, -0.39062503],
      "indices": [0, 3, 1, 2, 3, 0]
    }
  ]
}
```

## Field Meanings

### `atlasWidth`, `atlasHeight`
- **Description:** Pixel size of the atlas PNG.

### `texelSize` (world units per pixel)
- **Description:** Conversion factor from tk2d/Unity world units to pixels.
- **Example:**  
  ```
  pixelWidth = (bounds.extents.x * 2) / texelSize.x
  ```

### `sprites[]`
Per-frame descriptors:

- **`name`**  
  Sprite/frame identifier (useful for animation lists).

- **`uvRect`** (normalized)  
  Sub-rectangle on the atlas in UV space [0,1], origin bottom-left (Unity).  
  _Convert to pixels:_  
  ```
  x = uMin * atlasWidth
  y = vMin * atlasHeight
  w = (uMax - uMin) * atlasWidth
  h = (vMax - vMin) * atlasHeight
  ```

- **`pixelRect`** (pixels)  
  Same rect in pixels. `x`, `y` are bottom-left.  
  _If your image API is top-left:_  
  ```
  yTopLeft = atlasHeight - (y + h)
  ```

- **`bounds`** (trimmed, world units)  
  Local bounding box for the trimmed sprite geometry.
  - `center`: (x, y, z)
  - `extents`: half-size (w/2, h/2, d/2)

- **`untrimmedBounds`** (world units)  
  Bounding box for the original, untrimmed sprite before the atlas packer removed empty margins.  
  Use this when you need the full source canvas/padding.

- **`pivot`**  
  Exported as `bounds.center` (world-space anchor).  
  You can map this to your renderer’s anchor convention.

- **`indices`**  
  Triangle order for the sprite’s quad (only needed if you rebuild meshes; simple blits can ignore).

---

### Coordinate Notes

- **UV and `pixelRect`** use bottom-left origin (Unity/tk2d).
- Many image libraries use top-left; remember to flip `y` when sampling/cropping.
- `bounds`/`untrimmedBounds` are in world units; use `texelSize` to convert to pixels.
