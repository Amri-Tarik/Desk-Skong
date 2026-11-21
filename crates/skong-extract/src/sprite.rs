//! Unity `Sprite` (a rectangle of a texture, with pivot and pixels-per-unit) and the
//! `SpriteAtlas` render data that sprites packed into an atlas point to. Layout of
//! Unity 2020.2 … 6000.x release builds.
use anyhow::{Result, bail};

use crate::reader::{PPtr, Reader};
use crate::serialized::SerializedFile;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteRect {
    /// Texture pixels, origin bottom-left (Unity's).
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub pivot: [f32; 2],
    pub pixels_per_unit: f32,
}

pub struct Sprite {
    pub name: String,
    /// `None`: packed into [`Sprite::atlas`].
    pub texture: Option<PPtr>,
    pub rect: SpriteRect,
    pub atlas: PPtr,
    pub render_key: [u8; 24],
    /// Packing rotation of its own texture rectangle (`settingsRaw` bits 2-5; an atlas
    /// entry has its own, see [`atlas_render_data`]).
    pub rotation: u32,
    /// What of its rectangle is its own (tight packing shares rectangles).
    pub mesh: Option<Mesh>,
}

/// A sprite's mesh in its own space: positions in game units from the pivot (x right,
/// y up), triangles as index triples, and what places it on its texture rectangle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    /// `m_Rect` size (px).
    pub rect_size: [f32; 2],
    pub pivot: [f32; 2],
    pub ppu: f32,
    /// `textureRectOffset`: the texture rectangle's corner inside `m_Rect` (px).
    pub offset: [f32; 2],
}

/// The float position channel (stream 0) of a sprite's vertex data, as (x, y) pairs.
fn positions(count: usize, channels: &[[u8; 4]], data: &[u8]) -> Option<Vec<[f32; 2]>> {
    let pos = channels.first()?;
    if pos[0] != 0 || pos[2] != 0 || (pos[3] & 0xF) < 2 {
        return None;
    }
    let size = |format: u8| match format {
        0 | 10 | 11 => 4,
        1 | 4 | 5 | 8 | 9 => 2,
        _ => 1,
    };
    let stride = channels.iter().filter(|c| c[0] == 0 && c[3] & 0xF != 0).map(|c| c[1] as usize + size(c[2]) * (c[3] & 0xF) as usize).max()?;
    if count == 0 || data.len() < count * stride {
        return None;
    }
    let f = |o: usize| f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    Some((0..count).map(|i| i * stride + pos[1] as usize).map(|o| [f(o), f(o + 4)]).collect())
}

/// Undo a packer flip (`rotation` 1: horizontal, 2: vertical, 3: both) of a top-down
/// RGBA image in place. Rotated packing (4) is not supported.
pub fn unflip(px: &mut [u8], w: usize, h: usize, rotation: u32) -> Result<()> {
    let (fx, fy) = match rotation {
        0 => return Ok(()),
        1 => (true, false),
        2 => (false, true),
        3 => (true, true),
        r => bail!("packed rotated ({r}), not supported"),
    };
    let src = px.to_vec();
    for row in 0..h {
        for col in 0..w {
            let (sr, sc) = (if fy { h - 1 - row } else { row }, if fx { w - 1 - col } else { col });
            px[(row * w + col) * 4..][..4].copy_from_slice(&src[(sr * w + sc) * 4..][..4]);
        }
    }
    Ok(())
}

/// Clear every pixel of a top-down RGBA image (`w` x `h`, the sprite's texture rectangle
/// in its own orientation) whose centre lies outside all of `mesh`'s triangles, after
/// growing them by one pixel (8-neighbourhood) so antialiased edges stay. Same rule and
/// arithmetic as the .NET exporter's `ApplyMeshMask` (the two must match pixel for pixel).
pub fn mask(px: &mut [u8], w: usize, h: usize, mesh: &Mesh) {
    let ppu = mesh.ppu as f64;
    let bx = mesh.pivot[0] as f64 * mesh.rect_size[0] as f64 - mesh.offset[0] as f64;
    let by = mesh.pivot[1] as f64 * mesh.rect_size[1] as f64 - mesh.offset[1] as f64;
    let v: Vec<(f64, f64)> = mesh.positions.iter().map(|p| (p[0] as f64 * ppu + bx, p[1] as f64 * ppu + by)).collect();
    if mesh.indices.len() < 3 || mesh.indices.iter().any(|&i| i as usize >= v.len()) {
        return;
    }
    let mut inside = vec![false; w * h];
    for t in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
        for row in 0..h {
            for col in 0..w {
                if inside[row * w + col] {
                    continue;
                }
                let (qx, qy) = (col as f64 + 0.5, h as f64 - row as f64 - 0.5);
                let e0 = (b.0 - a.0) * (qy - a.1) - (b.1 - a.1) * (qx - a.0);
                let e1 = (c.0 - b.0) * (qy - b.1) - (c.1 - b.1) * (qx - b.0);
                let e2 = (a.0 - c.0) * (qy - c.1) - (a.1 - c.1) * (qx - c.0);
                if (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0) {
                    inside[row * w + col] = true;
                }
            }
        }
    }
    for row in 0..h {
        for col in 0..w {
            let keep = (row.saturating_sub(1)..=(row + 1).min(h - 1)).any(|r| (col.saturating_sub(1)..=(col + 1).min(w - 1)).any(|c| inside[r * w + c]));
            if !keep {
                px[(row * w + col) * 4..][..4].fill(0);
            }
        }
    }
}

fn rect(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?])
}

fn round_rect(f: [f32; 4], pivot: [f32; 2], ppu: f32) -> SpriteRect {
    SpriteRect { x: f[0].round() as i32, y: f[1].round() as i32, width: f[2].round() as i32, height: f[3].round() as i32, pivot, pixels_per_unit: ppu }
}

fn skip_vec(r: &mut Reader, elem: usize) -> Result<()> {
    let n = r.len(elem)?;
    r.skip(n * elem)
}

fn skip_secondary_textures(r: &mut Reader) -> Result<()> {
    let n = r.len(16)?;
    for _ in 0..n {
        r.pptr()?;
        r.string()?;
    }
    Ok(())
}

pub fn parse(sf: &SerializedFile, bytes: &[u8]) -> Result<Sprite> {
    let mut r = sf.reader(bytes);
    let name = r.string()?;
    let m_rect = rect(&mut r)?;
    r.skip(8 + 16)?; // m_Offset, m_Border
    let ppu = r.f32()?;
    let pivot = [r.f32()?, r.f32()?];
    r.u32()?; // m_Extrude
    r.bool()?; // m_IsPolygon
    r.align()?;
    let render_key: [u8; 24] = r.bytes(24)?.try_into()?;
    let tags = r.len(4)?;
    for _ in 0..tags {
        r.string()?;
    }
    let atlas = r.pptr()?;
    // m_RD
    let texture = r.pptr()?;
    r.pptr()?; // alphaTexture
    skip_secondary_textures(&mut r)?;
    skip_vec(&mut r, 48)?; // m_SubMeshes
    let index_bytes = r.byte_array()?; // m_IndexBuffer
    let count = r.u32()? as usize; // m_VertexCount
    let n = r.len(4)?;
    let channels: Vec<[u8; 4]> = (0..n).map(|_| r.bytes(4).map(|b| [b[0], b[1], b[2], b[3]])).collect::<Result<_>>()?;
    let data = r.byte_array()?; // m_DataSize
    skip_vec(&mut r, 64)?; // m_Bindpose
    let tr = rect(&mut r)?;
    if !(tr[2] > 0.0 && tr[3] > 0.0 && tr[2] <= 16384.0 && tr[3] <= 16384.0 && ppu > 0.0) {
        bail!("sprite '{name}': unrecognised Sprite layout (Unity {})", sf.unity_version);
    }
    let offset = [r.f32()?, r.f32()?]; // textureRectOffset
    r.skip(8)?; // atlasRectOffset
    let settings = r.u32()?;
    let indices: Vec<u16> = index_bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    let mesh = positions(count, &channels, data).map(|positions| Mesh { positions, indices, rect_size: [m_rect[2], m_rect[3]], pivot, ppu, offset });
    Ok(Sprite { name, texture: (!texture.is_null()).then_some(texture), rect: round_rect(tr, pivot, ppu), atlas, render_key, rotation: (settings >> 2) & 0xF, mesh })
}

/// The texture and rectangle a `SpriteAtlas` stores for the sprite with `key`.
/// Also returns the entry's packing rotation (`settingsRaw` bits 2-5).
pub fn atlas_render_data(sf: &SerializedFile, bytes: &[u8], key: &[u8; 24]) -> Result<Option<(PPtr, SpriteRect, u32)>> {
    let mut r = sf.reader(bytes);
    r.string()?; // m_Name
    skip_vec(&mut r, 12)?; // m_PackedSprites
    let names = r.len(4)?;
    for _ in 0..names {
        r.string()?;
    }
    let entries = r.len(24)?;
    for _ in 0..entries {
        let k = r.bytes(24)?;
        let texture = r.pptr()?;
        r.pptr()?; // alphaTexture
        let tr = rect(&mut r)?;
        r.skip(8 + 8 + 16 + 4)?; // textureRectOffset, atlasRectOffset, uvTransform, downscale
        let settings = r.u32()?;
        skip_secondary_textures(&mut r)?;
        if k == key {
            return Ok(Some((texture, round_rect(tr, [0.5, 0.5], 100.0), (settings >> 2) & 0xF)));
        }
    }
    Ok(None)
}

/// Crop a sprite out of a decoded texture (`rgba` top-down), output top-down.
pub fn crop(rgba: &[u8], tex_w: u32, tex_h: u32, r: &SpriteRect) -> Result<Vec<u8>> {
    let (tw, th) = (tex_w as i32, tex_h as i32);
    if r.x < 0 || r.y < 0 || r.width <= 0 || r.height <= 0 || r.x + r.width > tw || r.y + r.height > th {
        bail!("sprite rectangle {r:?} outside its {tw}x{th} texture");
    }
    let (w, h) = (r.width as usize, r.height as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    // Rect origin is bottom-left; the top row of the sprite is texture row y+h-1 from the
    // bottom, i.e. th-(y+h) from the top.
    let top = (th - r.y - r.height) as usize;
    for row in top..top + h {
        let o = (row * tw as usize + r.x as usize) * 4;
        out.extend_from_slice(&rgba[o..o + w * 4]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialized::build::{self, Obj};
    use std::path::Path;

    fn f32s(v: &mut Vec<u8>, xs: &[f32]) {
        for x in xs {
            v.extend(x.to_le_bytes());
        }
    }

    pub(crate) fn sprite_body(name: &str, tex: (i32, i64), rect_px: [f32; 4], pivot: [f32; 2]) -> Vec<u8> {
        let mut v = build::string(name);
        f32s(&mut v, &[0.0, 0.0, rect_px[2], rect_px[3], 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 64.0, pivot[0], pivot[1]]);
        v.extend(1u32.to_le_bytes()); // extrude
        v.extend([0, 0, 0, 0]); // polygon, aligned
        v.extend([3u8; 24]); // render key
        v.extend(0i32.to_le_bytes()); // atlas tags
        v.extend(build::pptr(0, 0)); // atlas
        v.extend(build::pptr(tex.0, tex.1));
        v.extend(build::pptr(0, 0));
        v.extend(0i32.to_le_bytes()); // secondary textures
        v.extend(1i32.to_le_bytes());
        v.extend([0u8; 48]); // one submesh
        v.extend(6i32.to_le_bytes());
        v.extend([0u8; 8]); // index buffer (6, aligned to 8)
        v.extend(4u32.to_le_bytes());
        v.extend(1i32.to_le_bytes());
        v.extend([0u8; 4]); // one channel
        v.extend(3i32.to_le_bytes());
        v.extend([1, 2, 3, 0]); // vertex data, aligned
        v.extend(0i32.to_le_bytes()); // bindpose
        f32s(&mut v, &rect_px);
        f32s(&mut v, &[0.0; 4 + 4]);
        v.extend(0u32.to_le_bytes());
        v.extend(1.0f32.to_le_bytes());
        v
    }

    #[test]
    fn parses_a_sprite_and_crops_it() {
        let body = sprite_body("town_bench", (0, 9), [1.0, 0.0, 2.0, 1.0], [0.5, 0.25]);
        let bytes = build::file("6000.0.61f1", &[Obj { path_id: 4, class_id: 213, script: None, data: body }], &[], &[]);
        let sf = SerializedFile::from_bytes(bytes, Path::new("m")).unwrap();
        let s = parse(&sf, &sf.read(sf.object(4).unwrap()).unwrap()).unwrap();
        assert_eq!(s.name, "town_bench");
        assert_eq!(s.texture, Some(PPtr { file: 0, path_id: 9 }));
        assert_eq!(s.rect, SpriteRect { x: 1, y: 0, width: 2, height: 1, pivot: [0.5, 0.25], pixels_per_unit: 64.0 });

        // 3x2 texture, top-down: row 0 = a b c, row 1 = d e f. The rect (1,0) 2x1 is the
        // bottom row's right two pixels: e f.
        let px: Vec<u8> = (0..6u8).flat_map(|i| [i, i, i, 255]).collect();
        assert_eq!(crop(&px, 3, 2, &s.rect).unwrap(), [4, 4, 4, 255, 5, 5, 5, 255]);
        assert!(crop(&px, 2, 2, &s.rect).is_err());
        assert!(s.mesh.is_none(), "no float position channel: nothing to mask with");
    }

    #[test]
    fn a_flipped_packing_is_undone() {
        // 2x2 top-down: a b / c d.
        let img = |v: [u8; 4]| -> Vec<u8> { v.iter().flat_map(|&i| [i, i, i, 255]).collect() };
        let mut px = img([1, 2, 3, 4]);
        unflip(&mut px, 2, 2, 2).unwrap();
        assert_eq!(px, img([3, 4, 1, 2]), "vertical");
        let mut px = img([1, 2, 3, 4]);
        unflip(&mut px, 2, 2, 3).unwrap();
        assert_eq!(px, img([4, 3, 2, 1]), "both");
        assert!(unflip(&mut img([1, 2, 3, 4]), 2, 2, 4).is_err(), "rotated packing");
    }

    #[test]
    fn the_mesh_keeps_its_own_pixels_and_a_one_pixel_rim() {
        // An 8x4 rectangle whose mesh is its left 2 columns (1 px per unit, pivot at the
        // bottom-left corner): columns 0-2 stay (the mesh plus a pixel), the rest is a
        // neighbour's art and goes.
        let mesh = Mesh { positions: vec![[0.0, 0.0], [2.0, 0.0], [0.0, 4.0], [2.0, 4.0]], indices: vec![0, 1, 2, 2, 1, 3], rect_size: [8.0, 4.0], pivot: [0.0, 0.0], ppu: 1.0, offset: [0.0, 0.0] };
        let mut px = vec![255u8; 8 * 4 * 4];
        mask(&mut px, 8, 4, &mesh);
        for row in 0..4 {
            let kept: Vec<bool> = (0..8).map(|c| px[(row * 8 + c) * 4 + 3] == 255).collect();
            assert_eq!(kept, [true, true, true, false, false, false, false, false], "row {row}");
        }
    }
}
