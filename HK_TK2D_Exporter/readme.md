# HK_TK2D_Exporter

A one-shot tool that extracts **sprite bounds and UVs** from Hollow Knight’s
**tk2d sprite collections** and saves them as a clean JSON you can ship with your own PNGs.

- ✅ Reads `tk2dSpriteCollectionData` (MonoBehaviour) directly from Unity `.assets`
- ✅ Exports per-sprite:
  - `name`
  - `uvRect` (u/v min/max)
  - `pixelRect` (x/y/w/h in atlas pixels)
  - `bounds` (center/extents)
  - `untrimmedBounds` (center/extents)
  - `pivot` (derived—bounds center)
  - `indices` (if present)
- ✅ Includes atlas width/height and collection `texelSize`
- ❌ Does **not** ship any game files. **You must provide your own files** from your legal copy.

---

## Why this exists

Hollow Knight mods/skins often reuse **big atlas images** with per-sprite metadata stored in Unity assets (tk2d).
This tool extracts that metadata once, so the overlay can use a simple **PNG + JSON** at runtime.

---

## Prerequisites

- **.NET SDK** 8 or 9 (either is fine)
- A legal copy of **Hollow Knight**
- Your game’s:
  - `hollow_knight_Data/resources.assets`
  - `hollow_knight_Data/globalgamemanagers.assets` (required to resolve scripts)
  - `hollow_knight_Data/Managed/` folder (contains `Assembly-CSharp.dll` and others)
- The provided **`classdata.tpk`** (used to parse built-in Unity types like `Texture2D`)

> If you need a different `classdata.tpk` (for other Unity versions), grab the uncompressed version from the AssetRipper **Tpk** releases and replace the file in the repo root.

---

## Install & build

```bash
# From the repo root:
cd HK_TK2D_Exporter
dotnet restore
dotnet build -c Release
dotnet run -c Release
```