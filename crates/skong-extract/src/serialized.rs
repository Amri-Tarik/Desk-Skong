//! Unity serialized files (`*.assets`, `levelN`): the header, the type list and the
//! object table, read without type trees (release builds strip them). Only the metadata
//! is read up front; object bodies are read on demand.
//!
//! Supported: serialized-file format 17 (Unity 2017) to 22 (Unity 2020.1 … Unity 6).
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};

use crate::reader::{PPtr, Reader};

pub const CLASS_GAME_OBJECT: i32 = 1;
pub const CLASS_TEXTURE2D: i32 = 28;
pub const CLASS_MONO_BEHAVIOUR: i32 = 114;
pub const CLASS_MONO_SCRIPT: i32 = 115;
pub const CLASS_SPRITE: i32 = 213;
pub const CLASS_SPRITE_ATLAS: i32 = 687078895;

const MIN_VERSION: u32 = 17;
const MAX_VERSION: u32 = 22;

#[derive(Clone, Debug)]
pub struct ObjectInfo {
    pub path_id: i64,
    /// Absolute offset of the object's data in the file.
    pub offset: u64,
    pub size: u32,
    pub class_id: i32,
    /// MonoBehaviours: index into [`SerializedFile::scripts`].
    pub script: Option<usize>,
}

enum Source {
    File(Mutex<File>),
    Mem(Vec<u8>),
}

pub struct SerializedFile {
    pub path: PathBuf,
    /// Serialized-file format version (22 for Unity 2020.1+).
    pub format: u32,
    pub unity_version: String,
    pub big_endian: bool,
    pub objects: Vec<ObjectInfo>,
    /// MonoScript references used by this file's MonoBehaviour types.
    pub scripts: Vec<PPtr>,
    /// External files, by `PPtr::file - 1`.
    pub externals: Vec<String>,
    source: Source,
}

impl SerializedFile {
    /// Open a file and read its metadata (not its objects).
    pub fn open(path: &Path) -> Result<SerializedFile> {
        let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut head = [0u8; 48];
        f.read_exact(&mut head).with_context(|| format!("{}: too short for a Unity file", path.display()))?;
        let (header_len, meta_len) = header_and_metadata_len(&head)?;
        let mut buf = vec![0u8; header_len + meta_len];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf).with_context(|| format!("{}: truncated metadata", path.display()))?;
        let mut sf = Self::parse_metadata(&buf, path)?;
        sf.source = Source::File(Mutex::new(f));
        Ok(sf)
    }

    /// A whole serialized file held in memory (tests).
    pub fn from_bytes(bytes: Vec<u8>, path: &Path) -> Result<SerializedFile> {
        let mut sf = Self::parse_metadata(&bytes, path)?;
        sf.source = Source::Mem(bytes);
        Ok(sf)
    }

    fn parse_metadata(buf: &[u8], path: &Path) -> Result<SerializedFile> {
        parse(buf, path).with_context(|| format!("{}: not a readable Unity serialized file", path.display()))
    }

    pub fn object(&self, path_id: i64) -> Option<&ObjectInfo> {
        // Object tables are sorted by path id in practice; fall back to a scan otherwise.
        match self.objects.binary_search_by_key(&path_id, |o| o.path_id) {
            Ok(i) => Some(&self.objects[i]),
            Err(_) => self.objects.iter().find(|o| o.path_id == path_id),
        }
    }

    pub fn of_class(&self, class_id: i32) -> impl Iterator<Item = &ObjectInfo> {
        self.objects.iter().filter(move |o| o.class_id == class_id)
    }

    /// The object's serialized bytes.
    pub fn read(&self, obj: &ObjectInfo) -> Result<Vec<u8>> {
        self.read_prefix(obj, obj.size as usize)
    }

    /// The first `n` bytes of the object (at most its size): enough for a name.
    pub fn read_prefix(&self, obj: &ObjectInfo, n: usize) -> Result<Vec<u8>> {
        let n = n.min(obj.size as usize);
        match &self.source {
            Source::File(f) => {
                let mut f = f.lock().unwrap_or_else(|e| e.into_inner());
                f.seek(SeekFrom::Start(obj.offset))?;
                let mut buf = vec![0u8; n];
                f.read_exact(&mut buf).with_context(|| format!("{}: object {} truncated", self.path.display(), obj.path_id))?;
                Ok(buf)
            }
            Source::Mem(m) => {
                let start = usize::try_from(obj.offset)?;
                m.get(start..start + n).map(<[u8]>::to_vec).with_context(|| format!("object {} out of bounds", obj.path_id))
            }
        }
    }

    pub fn reader<'a>(&self, bytes: &'a [u8]) -> Reader<'a> {
        Reader::new(bytes, self.big_endian)
    }

    /// The name every named object starts with (`m_Name`), read from the object's head.
    pub fn object_name(&self, obj: &ObjectInfo) -> Result<String> {
        let head = self.read_prefix(obj, 260)?;
        let mut r = self.reader(&head);
        let n = r.i32()?;
        if !(0..=256).contains(&n) {
            bail!("object {} has no plausible name", obj.path_id);
        }
        if n as usize + 4 > head.len() {
            let full = self.read_prefix(obj, n as usize + 4)?;
            return self.reader(&full).string();
        }
        let mut r = self.reader(&head);
        r.string()
    }

    /// Path of the file a non-local `PPtr` points into, next to this one.
    pub fn external_path(&self, file: i32) -> Option<PathBuf> {
        let name = self.externals.get(usize::try_from(file).ok()?.checked_sub(1)?)?;
        // Externals look like "library/unity default resources" or "sharedassets3.assets".
        let base = name.rsplit(['/', '\\']).next()?;
        Some(self.path.parent()?.join(base))
    }
}

/// (header length, metadata length) from the first 48 bytes.
fn header_and_metadata_len(head: &[u8]) -> Result<(usize, usize)> {
    let mut r = Reader::new(head, true);
    let meta_old = r.u32()?;
    r.u32()?;
    let format = r.u32()?;
    if !(MIN_VERSION..=MAX_VERSION + 8).contains(&format) {
        bail!("unsupported serialized-file format {format} (supported: {MIN_VERSION}–{MAX_VERSION})");
    }
    if format >= 22 {
        r.skip(4 + 4)?; // data offset (legacy), endianness + reserved
        Ok((48, r.u32()? as usize))
    } else {
        Ok((20, meta_old as usize))
    }
}

fn parse(buf: &[u8], path: &Path) -> Result<SerializedFile> {
    let mut r = Reader::new(buf, true);
    r.skip(8)?;
    let format = r.u32()?;
    if !(MIN_VERSION..=MAX_VERSION).contains(&format) {
        bail!("unsupported serialized-file format {format} (supported: {MIN_VERSION}–{MAX_VERSION})");
    }
    let mut data_offset = r.u32()? as u64;
    let big_endian = r.u8()? != 0;
    r.skip(3)?;
    if format >= 22 {
        r.u32()?; // metadata size
        r.u64()?; // file size
        data_offset = r.u64()?;
        r.u64()?;
    }
    r.set_big_endian(big_endian);

    let unity_version = r.cstr()?;
    r.i32()?; // target platform
    let type_trees = r.bool()?;

    // Types: class ids (and, for MonoBehaviours, the script) by index.
    let type_count = r.len(1)?;
    let mut types = Vec::with_capacity(type_count);
    for _ in 0..type_count {
        let class_id = r.i32()?;
        r.u8()?; // stripped
        let script_index = r.i16()?;
        if class_id == CLASS_MONO_BEHAVIOUR {
            r.skip(16)?; // script id
        }
        r.skip(16)?; // old type hash
        if type_trees {
            let nodes = r.len(1)?;
            let strings = r.len(1)?;
            r.skip(nodes * if format >= 19 { 32 } else { 24 } + strings)?;
            if format >= 21 {
                let deps = r.len(4)?;
                r.skip(deps * 4)?;
            }
        }
        types.push((class_id, script_index));
    }

    let object_count = r.len(1)?;
    let mut objects = Vec::with_capacity(object_count);
    for _ in 0..object_count {
        r.align()?;
        let path_id = r.i64()?;
        let start = if format >= 22 { r.i64()? as u64 } else { r.u32()? as u64 };
        let size = r.u32()?;
        let type_index = r.i32()?;
        let &(class_id, script_index) = usize::try_from(type_index)
            .ok()
            .and_then(|i| types.get(i))
            .with_context(|| format!("object {path_id}: bad type index {type_index}"))?;
        let script = (class_id == CLASS_MONO_BEHAVIOUR && script_index >= 0).then_some(script_index as usize);
        objects.push(ObjectInfo { path_id, offset: data_offset + start, size, class_id, script });
    }

    let script_count = r.len(1)?;
    let mut scripts = Vec::with_capacity(script_count);
    for _ in 0..script_count {
        let file = r.i32()?;
        r.align()?;
        scripts.push(PPtr { file, path_id: r.i64()? });
    }

    let ext_count = r.len(1)?;
    let mut externals = Vec::with_capacity(ext_count);
    for _ in 0..ext_count {
        r.cstr()?;
        r.skip(16 + 4)?; // guid, type
        externals.push(r.cstr()?);
    }

    Ok(SerializedFile { path: path.to_path_buf(), format, unity_version, big_endian, objects, scripts, externals, source: Source::Mem(Vec::new()) })
}

/// Builds small serialized files in memory (format 22, little-endian metadata), for tests.
#[cfg(test)]
pub mod build {
    use super::*;

    pub struct Obj {
        pub path_id: i64,
        pub class_id: i32,
        pub script: Option<i16>,
        pub data: Vec<u8>,
    }

    pub fn file(unity: &str, objs: &[Obj], scripts: &[PPtr], externals: &[&str]) -> Vec<u8> {
        let mut types: Vec<(i32, i16)> = Vec::new();
        let mut type_of = Vec::new();
        for o in objs {
            let key = (o.class_id, o.script.unwrap_or(-1));
            let i = types.iter().position(|t| *t == key).unwrap_or_else(|| {
                types.push(key);
                types.len() - 1
            });
            type_of.push(i as i32);
        }
        let mut m = Vec::new();
        m.extend(unity.as_bytes());
        m.push(0);
        m.extend(19i32.to_le_bytes());
        m.push(0); // no type trees
        m.extend((types.len() as i32).to_le_bytes());
        for (class_id, script) in &types {
            m.extend(class_id.to_le_bytes());
            m.push(0);
            m.extend(script.to_le_bytes());
            if *class_id == CLASS_MONO_BEHAVIOUR {
                m.extend([0u8; 16]);
            }
            m.extend([0u8; 16]);
        }
        m.extend((objs.len() as i32).to_le_bytes());
        let mut data = Vec::new();
        let mut table = Vec::new();
        for (o, t) in objs.iter().zip(&type_of) {
            while data.len() % 8 != 0 {
                data.push(0);
            }
            table.push((o.path_id, data.len() as i64, o.data.len() as u32, *t));
            data.extend(&o.data);
        }
        for (pid, start, size, t) in table {
            while (48 + m.len()) % 4 != 0 {
                m.push(0);
            }
            m.extend(pid.to_le_bytes());
            m.extend(start.to_le_bytes());
            m.extend(size.to_le_bytes());
            m.extend(t.to_le_bytes());
        }
        m.extend((scripts.len() as i32).to_le_bytes());
        for s in scripts {
            m.extend(s.file.to_le_bytes());
            while (48 + m.len()) % 4 != 0 {
                m.push(0);
            }
            m.extend(s.path_id.to_le_bytes());
        }
        m.extend((externals.len() as i32).to_le_bytes());
        for e in externals {
            m.push(0);
            m.extend([0u8; 20]);
            m.extend(e.as_bytes());
            m.push(0);
        }
        m.extend(0i32.to_le_bytes()); // ref types
        m.push(0); // user info
        let data_offset = (48 + m.len()).next_multiple_of(16) as u64;
        let mut out = Vec::new();
        out.extend([0u8; 8]);
        out.extend(22u32.to_be_bytes());
        out.extend([0u8; 4]);
        out.extend([0u8; 4]); // little-endian
        out.extend((m.len() as u32).to_be_bytes());
        out.extend((data_offset + data.len() as u64).to_be_bytes());
        out.extend(data_offset.to_be_bytes());
        out.extend([0u8; 8]);
        out.extend(m);
        out.resize(data_offset as usize, 0);
        out.extend(data);
        out
    }

    /// `m_Name` as Unity writes it.
    pub fn string(s: &str) -> Vec<u8> {
        let mut v = (s.len() as i32).to_le_bytes().to_vec();
        v.extend(s.as_bytes());
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
        v
    }

    pub fn pptr(file: i32, path_id: i64) -> Vec<u8> {
        let mut v = file.to_le_bytes().to_vec();
        v.extend(path_id.to_le_bytes());
        v
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn parses_header_types_objects_scripts_and_externals() {
        let objs = [
            Obj { path_id: 1, class_id: CLASS_GAME_OBJECT, script: None, data: string("Knight") },
            Obj { path_id: 2, class_id: CLASS_TEXTURE2D, script: None, data: string("atlas0") },
            Obj { path_id: 7, class_id: CLASS_MONO_BEHAVIOUR, script: Some(0), data: pptr(0, 1) },
        ];
        let bytes = file("6000.0.61f1", &objs, &[PPtr { file: 1, path_id: 99 }], &["globalgamemanagers.assets"]);
        let sf = SerializedFile::from_bytes(bytes, Path::new("dir/sharedassets1.assets")).unwrap();
        assert_eq!(sf.format, 22);
        assert_eq!(sf.unity_version, "6000.0.61f1");
        assert_eq!(sf.objects.len(), 3);
        assert_eq!(sf.of_class(CLASS_TEXTURE2D).count(), 1);
        let tex = sf.object(2).unwrap();
        assert_eq!(sf.object_name(tex).unwrap(), "atlas0");
        let mb = sf.object(7).unwrap();
        assert_eq!(mb.script, Some(0));
        assert_eq!(sf.scripts[0], PPtr { file: 1, path_id: 99 });
        let body = sf.read(mb).unwrap();
        assert_eq!(sf.reader(&body).pptr().unwrap(), PPtr { file: 0, path_id: 1 });
        assert_eq!(sf.external_path(1).unwrap(), Path::new("dir").join("globalgamemanagers.assets"));
        assert!(sf.external_path(0).is_none() && sf.external_path(2).is_none());
        assert!(sf.object(3).is_none());
    }

    #[test]
    fn rejects_garbage_and_unknown_formats_without_panicking() {
        assert!(SerializedFile::from_bytes(vec![0; 10], Path::new("x")).is_err());
        let mut bytes = file("6000.0.61f1", &[], &[], &[]);
        bytes[8..12].copy_from_slice(&9u32.to_be_bytes());
        let err = SerializedFile::from_bytes(bytes, Path::new("x")).err().unwrap();
        assert!(format!("{err:#}").contains("unsupported serialized-file format 9"), "{err:#}");
        // Truncated metadata.
        let bytes = file("6000.0.61f1", &[Obj { path_id: 1, class_id: 28, script: None, data: string("a") }], &[], &[]);
        assert!(SerializedFile::from_bytes(bytes[..70].to_vec(), Path::new("x")).is_err());
    }
}
