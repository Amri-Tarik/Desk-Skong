//! Saving and loading the pet's files in `%APPDATA%\Desk-Skong` (settings, the world,
//! achievements) so that they survive crashes, full disks and updates:
//!
//! - [`write_atomic`]: a temporary file beside the target, flushed to disk, then renamed
//!   over it. A crash mid-write leaves the old file whole, never a truncated one.
//! - [`load_or_fresh`]: a missing file is a fresh start; so is a malformed one, but it is
//!   kept beside it first ([`backup_path`]), since the next save would overwrite it.
use std::io::Write;
use std::path::{Path, PathBuf};

/// `path` with `suffix` appended to its file name (`world.json` → `world.json.tmp`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Where a malformed file is kept: `<file>.bak`, or `<file>.bak-<unix secs>` if that one is
/// taken (an earlier one is never overwritten).
pub fn backup_path(path: &Path) -> PathBuf {
    let bak = with_suffix(path, ".bak");
    if !bak.exists() {
        return bak;
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    with_suffix(path, &format!(".bak-{secs}"))
}

/// Write `bytes` to `path` atomically (creating its folder): to `<file>.tmp`, synced, then
/// renamed over `path`. On failure the temporary file is removed and `path` is untouched.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Read `path` with `parse`. Missing (or unreadable) → `fresh()`. Malformed → the file is
/// copied to [`backup_path`] (logged), then `fresh()`.
pub fn load_or_fresh<T, E: std::fmt::Display>(path: &Path, parse: impl FnOnce(&str) -> Result<T, E>, fresh: impl FnOnce() -> T) -> T {
    let Ok(text) = std::fs::read_to_string(path) else { return fresh() };
    match parse(&text) {
        Ok(v) => v,
        Err(e) => {
            let bak = backup_path(path);
            match std::fs::copy(path, &bak) {
                Ok(_) => log::warn!("malformed {} ({e}); kept as {}, starting fresh", path.display(), bak.display()),
                Err(c) => log::warn!("malformed {} ({e}); could not keep a copy ({c}), starting fresh", path.display()),
            }
            fresh()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("skong-persist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).expect("dir").map(|e| e.expect("entry").file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn atomic_writes_replace_the_file_and_leave_nothing_beside_it() {
        let dir = temp("atomic");
        let path = dir.join("nested").join("world.json");
        write_atomic(&path, b"{\"a\":1}").expect("first write creates the folder");
        write_atomic(&path, b"{\"a\":2}").expect("rewrite");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":2}");
        assert_eq!(names(&dir.join("nested")), ["world.json"]);
        // A failed write (the target is a folder: rename fails) leaves no temporary file.
        let blocked = dir.join("blocked");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_atomic(&blocked, b"x").is_err());
        assert!(!with_suffix(&blocked, ".tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_files_are_backed_up_once_each_and_start_fresh() {
        let dir = temp("backup");
        let path = dir.join("settings.json");
        let parse = |s: &str| serde_json::from_str::<u32>(s);
        // Missing: fresh, nothing written.
        assert_eq!(load_or_fresh(&path, parse, || 7), 7);
        assert!(!dir.exists());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "42").unwrap();
        assert_eq!(load_or_fresh(&path, parse, || 7), 42);
        assert_eq!(names(&dir), ["settings.json"]);
        // Malformed: kept as .bak, fresh value.
        std::fs::write(&path, "{ half").unwrap();
        assert_eq!(load_or_fresh(&path, parse, || 7), 7);
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.bak")).unwrap(), "{ half");
        // Again: the first backup stays, a second one goes beside it.
        std::fs::write(&path, "nope").unwrap();
        assert_eq!(load_or_fresh(&path, parse, || 7), 7);
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.bak")).unwrap(), "{ half");
        assert_eq!(names(&dir).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
