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
    rect(&mut r)?; // m_Rect
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
    r.byte_array()?; // m_IndexBuffer
    r.u32()?; // m_VertexCount
    skip_vec(&mut r, 4)?; // m_Channels
    r.byte_array()?; // m_DataSize
    skip_vec(&mut r, 64)?; // m_Bindpose
    let tr = rect(&mut r)?;
    if !(tr[2] > 0.0 && tr[3] > 0.0 && tr[2] <= 16384.0 && tr[3] <= 16384.0 && ppu > 0.0) {
        bail!("sprite '{name}': unrecognised Sprite layout (Unity {})", sf.unity_version);
    }
    Ok(Sprite { name, texture: (!texture.is_null()).then_some(texture), rect: round_rect(tr, pivot, ppu), atlas, render_key })
}

/// The texture and rectangle a `SpriteAtlas` stores for the sprite with `key`.
pub fn atlas_render_data(sf: &SerializedFile, bytes: &[u8], key: &[u8; 24]) -> Result<Option<(PPtr, SpriteRect)>> {
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
        r.skip(8 + 8 + 16 + 4 + 4)?; // textureRectOffset, atlasRectOffset, uvTransform, downscale, settings
        skip_secondary_textures(&mut r)?;
        if k == key {
            return Ok(Some((texture, round_rect(tr, [0.5, 0.5], 100.0))));
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
    }
}
