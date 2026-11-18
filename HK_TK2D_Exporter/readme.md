# HK_TK2D_Exporter

Reads Hollow Knight's **tk2d sprite collections** and **sprite animation libraries**
straight from your game install and writes plain JSON. No game art is copied — the
overlay pairs this metadata with a `Knight.png` (vanilla or any community skin).

Verified against the Unity 6 build of Hollow Knight (`6000.0.61f1`). Community skins
made for older versions still line up: the Knight atlas layout is unchanged.

## Prerequisites

- **.NET SDK 9**
- A legal copy of **Hollow Knight**
- `classdata.tpk` (kept in this folder; replace it from AssetRipper's Tpk releases if Unity changes)

## Usage

```bash
cd HK_TK2D_Exporter
dotnet build -c Release

# what's in the game?
dotnet run -c Release -- --data "<Steam>/steamapps/common/Hollow Knight/hollow_knight_Data" --list

# export animation libraries + every collection their frames reference
dotnet run -c Release -- --data "<...>/hollow_knight_Data" --out ../data/hk \
    --animations "Knight,Knight Effects Anim,Strikes,Spells Anim,Dust,Hollow Shade,Grimmchild Anim,Weaverling Anim,Knight Hatchling Anim"
```

Add `--textures ../skins/Vanilla` to also decode each exported collection's atlas (BC7 etc.)
to PNG. Atlases get Custom Knight names (`Knight.png`, `Sprint.png`, `Unn.png`, …; others
`<Collection_Name>.png`), so the folder is a complete skin and the runtime's fallback for
anything a community skin lacks. **This is game art: keep it in the git-ignored `skins/`
folder and never redistribute it.**

Props: `--find bench` lists matching Sprite/Texture2D assets across all asset files;
`--export-sprites town_bench,guardian_bench --textures ../skins/Vanilla/props` crops those sprites
(including ones packed in SpriteAtlases) to PNG plus a `.json` with pivot and pixels-per-unit.

`--dump-mono HeroController --hierarchy` prints the hero prefab's object tree (local position,
scale, components, tk2d collection / animation library + default clip) — the source of the effect
offsets in `data/effects.json`.

Options: `--collections A,B` exports collections by name instead of by reference;
`--files resources.assets,level1` scans other asset files (default `resources.assets`).

### Dumping serialized tuning (`--dump-mono`)

```bash
# HeroController fields + the BoxCollider2D / Rigidbody2D / ... on the Knight GameObject
dotnet run -c Release -- --data "<...>/hollow_knight_Data" --dump-mono HeroController > hero.json

# built-in Unity classes work too (here: project gravity and fixed timestep)
dotnet run -c Release -- --data "<...>/hollow_knight_Data" --files globalgamemanagers --dump-mono Physics2DSettings
dotnet run -c Release -- --data "<...>/hollow_knight_Data" --files globalgamemanagers --dump-mono TimeManager
```

Prints a JSON array to stdout, one entry per matching object: its serialized fields and,
for components, every component on the same GameObject (built-in ones with their fields,
MonoBehaviours by class name). Arrays longer than `--max-array N` (default 64) are
summarised by length. Only serialized data is read; no game code is decompiled.
The movement constants in `crates/skong-core/src/controller/params.rs` come from this.

## Output

`<out>/collections/<Name>.json`

```jsonc
{
  "name": "Knight", "atlas_name": "atlas0", "atlas_width": 4096, "atlas_height": 4096,
  "sprites": [{
    "id": 0, "name": "idle_still_020003",
    "positions": [[x,y,z] x4],   // quad corners, local world units, origin = sprite anchor
    "uvs": [[u,v] x4],           // matching atlas UVs (bottom-left origin) — rotation baked in
    "indices": [0,3,1,2,3,0],
    "flipped": 1,                // tk2d FlipMode (informational)
    "texel_size": [0.015625, 0.015625],   // world units per pixel (Knight = 64 px/unit)
    "bounds": [[cx,cy,cz],[ex,ey,ez]], "untrimmed_bounds": [...], "material_id": 0
  }]
}
```

Render a sprite by drawing `positions` (scaled by `1 / texel_size`) textured with `uvs`;
trim, pivot and atlas rotation are all already encoded.

`<out>/animations/<Library>.json`

```jsonc
{ "name": "Knight", "clips": [{
    "name": "Run", "fps": 12, "loop_start": 3,
    "wrap_mode": "LoopSection",   // Loop | LoopSection | Once | PingPong | RandomFrame | RandomLoop | Single
    "frames": [{ "collection": "Knight", "sprite_id": 412, "sprite": "run0001",
                 "event": { "info": "", "int": 0, "float": 0 } }]   // event only on trigger frames
}]}
```
