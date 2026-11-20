//! `Texture2D`: the header fields we need, the pixel data (inline or streamed from a
//! `.resS` file), and decoding to straight RGBA8, rows top-down.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result, bail, ensure};

use crate::decode;
use crate::serialized::{ObjectInfo, SerializedFile};

#[derive(Clone, Debug, PartialEq)]
pub struct TextureInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: i32,
    pub mip_count: i32,
    /// Bytes of the first mip level.
    pub image_size: usize,
    pub data: TextureData,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextureData {
    /// Bytes stored inside the object (offset into the object's bytes, length).
    Inline { offset: usize, len: usize },
    /// Bytes in a `.resS` file next to the serialized file.
    Stream { path: String, offset: u64, len: u64 },
}

/// Unity `TextureFormat` names, for messages.
pub fn format_name(f: i32) -> String {
    match f {
        1 => "Alpha8".into(),
        3 => "RGB24".into(),
        4 => "RGBA32".into(),
        5 => "ARGB32".into(),
        10 => "DXT1".into(),
        12 => "DXT5".into(),
        14 => "BGRA32".into(),
        25 => "BC7".into(),
        28 => "DXT1Crunched".into(),
        29 => "DXT5Crunched".into(),
        _ => format!("format {f}"),
    }
}

/// Parse a Texture2D's header. Release builds carry no type trees, so the layout is
/// Unity's (2019.3 … 6000.x): the gap between `m_Name` and `m_Width` changed across
/// versions (fallback format / flags), so the plausible one is picked; the stream info
/// is read from the end, where it has always been.
pub fn parse(sf: &SerializedFile, bytes: &[u8]) -> Result<TextureInfo> {
    let mut r = sf.reader(bytes);
    let name = r.string()?;
    let after_name = r.pos();
    let major: u32 = sf.unity_version.split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mips_stripped = major >= 2020; // `m_MipsStripped` since 2020.1
    let mut head = None;
    for gap in [4usize, 8, 0, 12] {
        let mut r = sf.reader(bytes);
        r.skip(after_name + gap)?;
        let (width, height, complete) = (r.i32()?, r.i32()?, r.u32()?);
        if mips_stripped {
            r.i32()?;
        }
        let (format, mip_count) = (r.i32()?, r.i32()?);
        let plausible = (1..=16384).contains(&width)
            && (1..=16384).contains(&height)
            && complete > 0
            && (1..=80).contains(&format)
            && (1..=15).contains(&mip_count);
        if plausible {
            head = Some((width as u32, height as u32, format, mip_count));
            break;
        }
    }
    let Some((width, height, format, mip_count)) = head else {
        bail!("texture '{name}': unrecognised Texture2D layout (Unity {})", sf.unity_version)
    };
    let image_size = decode::level_size(format, width, height).with_context(|| format!("texture '{name}': unsupported {}", format_name(format)))?;
    let data = stream_info(sf, bytes, image_size).with_context(|| format!("texture '{name}': could not locate its pixels"))?;
    Ok(TextureInfo { name, width, height, format, mip_count, image_size, data })
}

/// The object ends with `image data (vector<UInt8>)` then `m_StreamData {offset u64, size
/// u32, path string}`. Find the path string by its length prefix, working back from the end.
fn stream_info(sf: &SerializedFile, bytes: &[u8], image_size: usize) -> Result<TextureData> {
    let n = bytes.len();
    for len in 0..=255usize {
        let field = 4 + len.next_multiple_of(4);
        let Some(start) = n.checked_sub(field) else { break };
        if start < 12 + 4 || start % 4 != 0 {
            continue;
        }
        let mut r = sf.reader(&bytes[start..]);
        if r.i32()? as usize != len {
            continue;
        }
        let path = std::str::from_utf8(r.bytes(len)?).ok();
        let Some(path) = path.filter(|p| p.chars().all(|c| !c.is_control())) else { continue };
        let mut r = sf.reader(&bytes[start - 12..]);
        let (offset, size) = (r.u64()?, r.u32()? as u64);
        if !path.is_empty() && size > 0 {
            ensure!(size as usize >= image_size, "streamed data ({size} bytes) smaller than one level ({image_size})");
            return Ok(TextureData::Stream { path: path.to_string(), offset, len: size });
        }
        // Inline: `vector<UInt8>` right before the stream info.
        let before = start - 12;
        // The vector's length isn't known up front (mips); scan back for its length prefix.
        let mut p = before;
        while p >= 4 {
            p -= 4;
            let l = sf.reader(&bytes[p..]).i32()?;
            if l >= image_size as i32 && p + 4 + (l as usize).next_multiple_of(4) == before {
                return Ok(TextureData::Inline { offset: p + 4, len: l as usize });
            }
            if before - p > image_size * 2 + 64 {
                break;
            }
        }
        bail!("no inline image data and no stream file");
    }
    bail!("no stream info at the end of the object")
}

/// The first mip level's raw bytes.
pub fn read_pixels(sf: &SerializedFile, obj: &ObjectInfo, tex: &TextureInfo) -> Result<Vec<u8>> {
    match &tex.data {
        TextureData::Inline { offset, .. } => {
            let bytes = sf.read(obj)?;
            Ok(bytes[*offset..*offset + tex.image_size].to_vec())
        }
        TextureData::Stream { path, offset, .. } => {
            let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
            let file = sf.path.parent().map(|d| d.join(base)).context("no folder")?;
            let mut f = File::open(&file).with_context(|| format!("open {}", file.display()))?;
            f.seek(SeekFrom::Start(*offset))?;
            let mut buf = vec![0u8; tex.image_size];
            f.read_exact(&mut buf).with_context(|| format!("{}: texture '{}' truncated", file.display(), tex.name))?;
            Ok(buf)
        }
    }
}

/// Decode to straight RGBA8 with rows top-down (Unity stores them bottom-up).
pub fn decode_top_down(tex: &TextureInfo, raw: &[u8]) -> Result<Vec<u8>> {
    let mut rgba = decode::decode(tex.format, tex.width, tex.height, raw).with_context(|| format!("texture '{}'", tex.name))?;
    flip_rows(&mut rgba, tex.width as usize * 4);
    Ok(rgba)
}

pub fn flip_rows(px: &mut [u8], stride: usize) {
    let rows = px.len() / stride;
    for y in 0..rows / 2 {
        let (top, bottom) = px.split_at_mut((rows - 1 - y) * stride);
        top[y * stride..(y + 1) * stride].swap_with_slice(&mut bottom[..stride]);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::serialized::build::{self, Obj};
    use crate::serialized::CLASS_TEXTURE2D;
    use std::path::Path;

    /// A Texture2D body laid out like Unity 6000.0's, with either inline pixels or a
    /// stream reference.
    pub(crate) fn texture_body(name: &str, w: i32, h: i32, format: i32, inline: Option<&[u8]>, stream: Option<(&str, u64, u32)>) -> Vec<u8> {
        let mut v = build::string(name);
        v.extend([0u8; 4]); // flags
        let size = decode::level_size(format, w as u32, h as u32).unwrap() as u32;
        for x in [w, h, size as i32, 0, format, 1] {
            v.extend(x.to_le_bytes());
        }
        v.extend([0u8; 4]); // readable…, aligned
        v.extend(build::string("")); // mipmap limit group
        v.extend([0u8; 4]); // streaming mipmaps
        for x in [0i32, 1, 2, 1, 1, 0, 1, 1, 1, 0, 1, 0] {
            v.extend(x.to_le_bytes()); // priority, image count, dimension, settings (6), lightmap, colour space, platform blob
        }
        match inline {
            Some(px) => {
                v.extend((px.len() as i32).to_le_bytes());
                v.extend(px);
                while !v.len().is_multiple_of(4) {
                    v.push(0);
                }
            }
            None => v.extend(0i32.to_le_bytes()),
        }
        let (path, off, len) = stream.unwrap_or(("", 0, 0));
        v.extend(off.to_le_bytes());
        v.extend(len.to_le_bytes());
        v.extend(build::string(path));
        v
    }

    fn file_with(body: Vec<u8>) -> SerializedFile {
        let bytes = build::file("6000.0.61f1", &[Obj { path_id: 5, class_id: CLASS_TEXTURE2D, script: None, data: body }], &[], &[]);
        SerializedFile::from_bytes(bytes, Path::new("mem.assets")).unwrap()
    }

    #[test]
    fn parses_a_streamed_bc7_texture_header() {
        let sf = file_with(texture_body("atlas0", 2048, 2048, 25, None, Some(("resources.assets.resS", 97357168, 4194304))));
        let obj = sf.object(5).unwrap();
        let tex = parse(&sf, &sf.read(obj).unwrap()).unwrap();
        assert_eq!((tex.name.as_str(), tex.width, tex.height, tex.format, tex.mip_count), ("atlas0", 2048, 2048, 25, 1));
        assert_eq!(tex.data, TextureData::Stream { path: "resources.assets.resS".into(), offset: 97357168, len: 4194304 });
    }

    #[test]
    fn reads_inline_rgba32_pixels_and_flips_them_top_down() {
        // 1x2: bottom row red, top row blue (Unity rows are bottom-up).
        let px = [255, 0, 0, 255, 0, 0, 255, 255];
        let sf = file_with(texture_body("tiny", 1, 2, 4, Some(&px), None));
        let obj = sf.object(5).unwrap();
        let tex = parse(&sf, &sf.read(obj).unwrap()).unwrap();
        assert_eq!(tex.data, TextureData::Inline { offset: tex_offset(&sf, obj, &px), len: 8 });
        let raw = read_pixels(&sf, obj, &tex).unwrap();
        assert_eq!(decode_top_down(&tex, &raw).unwrap(), vec![0, 0, 255, 255, 255, 0, 0, 255]);
    }

    fn tex_offset(sf: &SerializedFile, obj: &ObjectInfo, px: &[u8]) -> usize {
        let b = sf.read(obj).unwrap();
        b.windows(px.len()).position(|w| w == px).unwrap()
    }

    #[test]
    fn garbage_is_an_error() {
        let sf = file_with(build::string("not a texture"));
        let obj = sf.object(5).unwrap();
        assert!(parse(&sf, &sf.read(obj).unwrap()).is_err());
    }

    #[test]
    fn flips_rows() {
        let mut px = vec![1, 2, 3, 4, 5, 6];
        flip_rows(&mut px, 2);
        assert_eq!(px, vec![5, 6, 3, 4, 1, 2]);
    }
}
