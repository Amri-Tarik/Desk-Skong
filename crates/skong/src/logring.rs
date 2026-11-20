//! Logging: `env_logger` to stderr (honours `RUST_LOG`, default `info`) plus a bounded
//! in-memory ring of formatted records that the control server streams to panels. What
//! goes to stderr also goes to the opt-in log file ([`crate::logfile`]).
//!
//! Panels can raise the captured level for `skong*` targets ([`set_panel_level`]); such
//! extra records go to the ring only, so stderr keeps following `RUST_LOG`.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use log::{LevelFilter, Log, Metadata, Record};
use skong_ipc::{LogLevel, LogLine};

/// Records kept for panels that connect later.
pub const CAPACITY: usize = 2000;

/// Bounded log history with monotonically increasing sequence numbers.
pub struct LogRing {
    inner: Mutex<Inner>,
    start: Instant,
    capacity: usize,
}

struct Inner {
    lines: VecDeque<LogLine>,
    next_seq: u64,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self { inner: Mutex::new(Inner { lines: VecDeque::with_capacity(capacity.min(4096)), next_seq: 0 }), start: Instant::now(), capacity: capacity.max(1) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave `Inner` inconsistent: keep going.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, level: LogLevel, target: &str, msg: String) {
        let t = self.start.elapsed().as_secs_f64();
        let mut g = self.lock();
        let seq = g.next_seq;
        g.next_seq += 1;
        if g.lines.len() == self.capacity {
            g.lines.pop_front();
        }
        g.lines.push_back(LogLine { seq, t, level, target: target.to_string(), msg });
    }

    /// Sequence number the next record will get.
    pub fn next_seq(&self) -> u64 {
        self.lock().next_seq
    }

    /// Retained records with `seq >= from`, oldest first.
    pub fn since(&self, from: u64) -> Vec<LogLine> {
        let g = self.lock();
        let skip = g.lines.partition_point(|l| l.seq < from);
        g.lines.iter().skip(skip).cloned().collect()
    }
}

static RING: OnceLock<LogRing> = OnceLock::new();
/// Panel-requested level for `skong*` targets (`LevelFilter as usize`; 0 = off).
static PANEL_LEVEL: AtomicUsize = AtomicUsize::new(0);
static ENV_MAX: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);

/// The process-wide ring, if [`init`] ran.
pub fn ring() -> Option<&'static LogRing> {
    RING.get()
}

struct Logger {
    env: env_logger::Logger,
    ring: &'static LogRing,
}

fn panel_wants(meta: &Metadata) -> bool {
    meta.target().starts_with("skong") && (meta.level() as usize) <= PANEL_LEVEL.load(Ordering::Relaxed)
}

impl Log for Logger {
    fn enabled(&self, meta: &Metadata) -> bool {
        self.env.enabled(meta) || panel_wants(meta)
    }

    fn log(&self, record: &Record) {
        let to_stderr = self.env.matches(record);
        if !to_stderr && !panel_wants(record.metadata()) {
            return;
        }
        if to_stderr {
            self.env.log(record);
            crate::logfile::record(record.level(), record.target(), record.args());
        }
        self.ring.push(record.level().into(), record.target(), record.args().to_string());
    }

    fn flush(&self) {
        self.env.flush();
    }
}

/// Install the logger. Safe to call once; later calls are ignored.
pub fn init() {
    let env = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    let max = env.filter();
    let ring = RING.get_or_init(|| LogRing::new(CAPACITY));
    if log::set_boxed_logger(Box::new(Logger { env, ring })).is_ok() {
        ENV_MAX.store(max as usize, Ordering::Relaxed);
        log::set_max_level(max);
    }
}

/// Capture `skong*` records down to `level` for panels (never less than `RUST_LOG`).
pub fn set_panel_level(level: LogLevel) {
    let want = log::Level::from(level).to_level_filter();
    PANEL_LEVEL.store(want as usize, Ordering::Relaxed);
    let env = ENV_MAX.load(Ordering::Relaxed);
    log::set_max_level(if (want as usize) > env { want } else { LevelFilter::iter().nth(env).unwrap_or(LevelFilter::Info) });
    log::info!("panel log level: {}", level.name());
}

/// Most verbose level currently captured for panels.
pub fn panel_level() -> LogLevel {
    let l = PANEL_LEVEL.load(Ordering::Relaxed).max(ENV_MAX.load(Ordering::Relaxed));
    LevelFilter::iter().nth(l).and_then(|f| f.to_level()).map_or(LogLevel::Info, LogLevel::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_bounded_and_resumable() {
        let r = LogRing::new(3);
        for i in 0..5 {
            r.push(LogLevel::Info, "skong::t", format!("m{i}"));
        }
        assert_eq!(r.next_seq(), 5);
        let all = r.since(0);
        assert_eq!(all.iter().map(|l| l.msg.as_str()).collect::<Vec<_>>(), ["m2", "m3", "m4"]);
        assert_eq!(all.iter().map(|l| l.seq).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(r.since(4).len(), 1);
        assert!(r.since(5).is_empty());
        assert!(all.windows(2).all(|w| w[0].t <= w[1].t));
    }

    #[test]
    fn level_filters_index_like_log_crate() {
        // `panel_level` relies on `LevelFilter as usize` matching iteration order.
        for (i, f) in LevelFilter::iter().enumerate() {
            assert_eq!(f as usize, i);
        }
    }
}
