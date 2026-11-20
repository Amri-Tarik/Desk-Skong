//! Log and crash files for bug reports, kept local in `%APPDATA%\Desk-Skong\logs`:
//!
//! - **Log file** (opt-in, the "Write a log file (for bug reports)" setting): what goes to
//!   stderr (`RUST_LOG`, default `info`) also goes to `skong-<date>.log` (UTC). A file that
//!   reaches [`MAX_BYTES`] continues in `skong-<date>-2.log`, … ; only the newest
//!   [`KEEP_LOGS`] log files are kept.
//! - **Crash file** (always, even with the setting off): a panic writes its message, place,
//!   thread and a backtrace to `crash-<date>_<time>.log` (newest [`KEEP_CRASHES`] kept), and
//!   to the log file when it is on. Nothing is ever sent anywhere.
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Size at which a log file continues in the next part (bytes).
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Log files kept (oldest deleted first).
pub const KEEP_LOGS: usize = 5;
/// Crash files kept.
pub const KEEP_CRASHES: usize = 10;

const LOG_PREFIX: &str = "skong-";
const CRASH_PREFIX: &str = "crash-";

/// Civil (proleptic Gregorian) date of a day count since 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DD` (UTC) of a unix time.
pub fn date(unix: u64) -> String {
    let (y, m, d) = civil((unix / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `HH:MM:SS` (UTC) of a unix time.
fn time_of_day(unix: u64) -> String {
    let s = unix % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn now() -> (u64, u32) {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs(), d.subsec_millis())
}

/// File name of part `part` (1-based) of the log of `date`.
pub fn log_name(date: &str, part: u32) -> String {
    if part <= 1 { format!("{LOG_PREFIX}{date}.log") } else { format!("{LOG_PREFIX}{date}-{part}.log") }
}

/// File name of a crash at unix time `unix`.
pub fn crash_name(unix: u64) -> String {
    format!("{CRASH_PREFIX}{}_{}.log", date(unix), time_of_day(unix).replace(':', "-"))
}

/// Age order of a log or crash file's name (without prefix and `.log`): its date-time
/// text, then its part (`2026-11-20` is part 1, `2026-11-20-2` part 2).
fn age_key(stem: &str) -> (&str, u32) {
    match stem.rsplit_once('-') {
        Some((day, part)) if day.len() == 10 && !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) => (day, part.parse().unwrap_or(u32::MAX)),
        _ => (stem, 1),
    }
}

/// Delete the oldest `<prefix>*.log` files in `dir` (by the date, time and part in their
/// names) so that at most `keep` remain. Other files are never touched.
pub fn prune(dir: &Path, prefix: &str, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_prefix(prefix)?.strip_suffix(".log")?.to_string();
            Some((stem, e.path()))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by(|a, b| age_key(&a.0).cmp(&age_key(&b.0)));
    for (_, p) in &files[..files.len() - keep] {
        let _ = std::fs::remove_file(p);
    }
}

/// The open log file: which day and part, how much is in it.
struct Sink {
    dir: PathBuf,
    date: String,
    part: u32,
    file: File,
    written: u64,
    max_bytes: u64,
    keep: usize,
}

impl Sink {
    /// Open (append) the newest part of `date`'s log, or the next one if it is full.
    fn open(dir: &Path, date: &str, max_bytes: u64, keep: usize) -> std::io::Result<Sink> {
        std::fs::create_dir_all(dir)?;
        let newest = std::fs::read_dir(dir)?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let (day, part) = age_key(name.strip_prefix(LOG_PREFIX)?.strip_suffix(".log")?);
                (day == date).then_some(part)
            })
            .max();
        let part = match newest {
            Some(n) if std::fs::metadata(dir.join(log_name(date, n))).is_ok_and(|m| m.len() >= max_bytes) => n + 1,
            Some(n) => n,
            None => 1,
        };
        let path = dir.join(log_name(date, part));
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map_or(0, |m| m.len());
        prune(dir, LOG_PREFIX, keep);
        Ok(Sink { dir: dir.to_path_buf(), date: date.to_string(), part, file, written, max_bytes, keep })
    }

    /// Append one line, moving to a new file on a new day or when this one is full.
    fn write(&mut self, today: &str, line: &str) {
        if today != self.date || self.written >= self.max_bytes {
            let next = if today != self.date { Sink::open(&self.dir, today, self.max_bytes, self.keep) } else { self.next_part() };
            match next {
                Ok(s) => *self = s,
                Err(_) => return,
            }
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.written += line.len() as u64;
        }
    }

    fn next_part(&self) -> std::io::Result<Sink> {
        let path = self.dir.join(log_name(&self.date, self.part + 1));
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map_or(0, |m| m.len());
        prune(&self.dir, LOG_PREFIX, self.keep);
        Ok(Sink { dir: self.dir.clone(), date: self.date.clone(), part: self.part + 1, file, written, max_bytes: self.max_bytes, keep: self.keep })
    }
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

fn sink() -> std::sync::MutexGuard<'static, Option<Sink>> {
    SINK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The logs folder (`%APPDATA%\Desk-Skong\logs`), once [`install`] ran.
pub fn dir() -> Option<&'static Path> {
    LOG_DIR.get().map(PathBuf::as_path)
}

/// Remember the logs folder and install the panic hook (crash files). Call once, early.
pub fn install(logs: Option<PathBuf>) {
    if let Some(d) = logs {
        let _ = LOG_DIR.set(d);
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let (unix, _) = now();
        let report = format!(
            "Desk-Skong {} crashed at {} {} UTC\nthread '{}' {info}\n\nbacktrace:\n{}\n",
            env!("CARGO_PKG_VERSION"),
            date(unix),
            time_of_day(unix),
            std::thread::current().name().unwrap_or("<unnamed>"),
            std::backtrace::Backtrace::force_capture(),
        );
        if let Some(d) = dir() {
            let written = write_crash(d, unix, &report);
            // Into the log file too, if it's on (never wait on a lock held by the panicking code).
            if let Ok(mut g) = SINK.try_lock()
                && let Some(s) = g.as_mut()
            {
                s.write(&date(unix), &format!("{report}crash file: {}\n", written.as_deref().map_or("(could not be written)".into(), |p| p.display().to_string())));
            }
        }
        default(info);
    }));
}

/// Write a crash report into `dir`; returns its path.
pub fn write_crash(dir: &Path, unix: u64, report: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(crash_name(unix));
    std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(report.as_bytes())).ok()?;
    prune(dir, CRASH_PREFIX, KEEP_CRASHES);
    Some(path)
}

/// Turn the log file on or off (the setting). On: opens today's file.
pub fn set_enabled(on: bool) {
    let mut g = sink();
    match (on, g.is_some(), dir()) {
        (true, false, Some(d)) => match Sink::open(d, &date(now().0), MAX_BYTES, KEEP_LOGS) {
            Ok(s) => {
                *g = Some(s);
                drop(g);
                log::info!("writing a log file in {} (Desk-Skong {})", d.display(), env!("CARGO_PKG_VERSION"));
            }
            Err(e) => {
                drop(g);
                log::warn!("could not open a log file in {}: {e}", d.display());
            }
        },
        (false, true, _) => {
            *g = None;
            drop(g);
            log::info!("log file off");
        }
        _ => {}
    }
}

/// Show the logs folder in Explorer (created if needed, so it opens even before a log).
pub fn open_folder() {
    let Some(d) = dir() else {
        log::warn!("no logs folder (APPDATA is not set)");
        return;
    };
    if let Err(e) = std::fs::create_dir_all(d).and_then(|()| std::process::Command::new("explorer.exe").arg(d).spawn().map(drop)) {
        log::warn!("could not open {}: {e}", d.display());
    }
}

/// Append a formatted record to the log file, if it's on (called by the logger).
pub fn record(level: log::Level, target: &str, msg: &std::fmt::Arguments) {
    let mut g = sink();
    let Some(s) = g.as_mut() else { return };
    let (unix, ms) = now();
    let line = format!("{} {}.{ms:03} {level:<5} {target}: {msg}\n", date(unix), time_of_day(unix));
    s.write(&date(unix), &line);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("skong-logs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn dates_and_names() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_795_132_800 + 86_399), "2026-11-20");
        assert_eq!(time_of_day(3_723), "01:02:03");
        assert_eq!(log_name("2026-11-20", 1), "skong-2026-11-20.log");
        assert_eq!(log_name("2026-11-20", 3), "skong-2026-11-20-3.log");
        assert_eq!(crash_name(1_795_132_800 + 3_723), "crash-2026-11-20_01-02-03.log");
        // Parts sort after their day's first file, days by date, crash times by time.
        assert!(age_key("2026-11-20") < age_key("2026-11-20-2"));
        assert!(age_key("2026-11-20-2") < age_key("2026-11-20-10"));
        assert!(age_key("2026-11-20-10") < age_key("2026-11-21"));
        assert!(age_key("2026-11-20_01-02-03") < age_key("2026-11-20_01-02-04"));
    }

    #[test]
    fn full_files_continue_in_parts_and_only_the_newest_are_kept() {
        let dir = temp("rotate");
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        std::fs::write(dir.join("crash-2026-01-01_00-00-00.log"), "boom").unwrap();
        let mut s = Sink::open(&dir, "2026-11-20", 100, 3).unwrap();
        let line = format!("{}\n", "x".repeat(39));
        for _ in 0..12 {
            s.write("2026-11-20", &line);
        }
        // 40-byte lines, 100-byte cap: 3 lines a part, 4 parts; only 3 kept.
        assert_eq!(s.part, 4);
        let logs: Vec<_> = names(&dir).into_iter().filter(|n| n.starts_with(LOG_PREFIX)).collect();
        assert_eq!(logs.len(), 3, "{logs:?}");
        assert!(!logs.contains(&log_name("2026-11-20", 1)), "the oldest part went first: {logs:?}");
        // A new day starts a new file; other files are never touched.
        s.write("2026-11-21", &line);
        assert_eq!((s.date.as_str(), s.part), ("2026-11-21", 1));
        let all = names(&dir);
        assert!(all.contains(&"skong-2026-11-21.log".to_string()) && all.contains(&"settings.json".to_string()));
        assert!(all.contains(&"crash-2026-01-01_00-00-00.log".to_string()));
        assert_eq!(all.iter().filter(|n| n.starts_with(LOG_PREFIX)).count(), 3);
        // Reopening continues after the newest part (part 4 is full; 1 and 2 were pruned).
        drop(s);
        let mut s = Sink::open(&dir, "2026-11-20", 100, 10).unwrap();
        assert_eq!(s.part, 5);
        // ... and appends to it while it has room.
        s.write("2026-11-20", "short\n");
        drop(s);
        assert_eq!(Sink::open(&dir, "2026-11-20", 100, 10).unwrap().part, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_files_are_written_and_capped() {
        let dir = temp("crash");
        for i in 0..(KEEP_CRASHES as u64 + 3) {
            let p = write_crash(&dir, 1_795_132_800 + i, "panicked at 'x'").expect("written");
            assert!(std::fs::read_to_string(p).unwrap().contains("panicked"));
        }
        assert_eq!(names(&dir).len(), KEEP_CRASHES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
