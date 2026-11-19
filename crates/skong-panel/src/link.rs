//! Connection to the pet: a background thread that (re)connects to the control pipe,
//! reads [`PetMsg`]s into shared state and forwards queued [`PanelMsg`]s.
//!
//! The UI never blocks on the pipe: it reads [`Shared`] under a short lock and sends with
//! [`Link::send`] (dropped while disconnected).
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use skong_ipc::{Hello, LogLine, PanelMsg, PetMsg, Status, decode, encode};

/// Log lines kept by the panel.
pub const MAX_LOGS: usize = 5000;
const RETRY: Duration = Duration::from_millis(500);

/// What the panel knows about the pet.
#[derive(Default)]
pub struct Shared {
    pub connected: bool,
    pub hello: Option<Hello>,
    pub status: Option<Status>,
    pub logs: VecDeque<LogLine>,
    /// Highest log sequence number seen in this connection (dedups backlog vs stream).
    pub last_seq: Option<u64>,
    /// Status messages received since connecting.
    pub statuses: u64,
    /// Bumped on every connect, so the UI can resend its preferences.
    pub session: u64,
    /// Pet pid + last log seq of the previous connection (a reconnect to the same pet
    /// must not duplicate its backlog; a restarted pet starts over at 0).
    prev: Option<(u32, Option<u64>)>,
}

impl Shared {
    fn ingest(&mut self, msg: PetMsg) {
        match msg {
            PetMsg::Hello(h) => {
                if let Some((pid, seq)) = self.prev.take()
                    && pid == h.pid
                {
                    self.last_seq = seq;
                }
                self.hello = Some(h);
            }
            PetMsg::Status(s) => {
                self.status = Some(*s);
                self.statuses += 1;
            }
            PetMsg::Logs(lines) => {
                for l in lines {
                    if self.last_seq.is_some_and(|s| l.seq <= s) {
                        continue;
                    }
                    self.last_seq = Some(l.seq);
                    if self.logs.len() == MAX_LOGS {
                        self.logs.pop_front();
                    }
                    self.logs.push_back(l);
                }
            }
        }
    }
}

pub struct Link {
    shared: Arc<Mutex<Shared>>,
    out: Sender<PanelMsg>,
}

impl Link {
    /// Start connecting to `pipe`; `wake` is called whenever new data arrives.
    pub fn start(pipe: String, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (out, rx) = channel();
        let s = shared.clone();
        let spawned = std::thread::Builder::new().name("pet-link".into()).spawn(move || manage(&pipe, &s, &rx, Arc::new(wake)));
        if let Err(e) = spawned {
            eprintln!("cannot start the connection thread: {e}");
        }
        Self { shared, out }
    }

    pub fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn send(&self, msg: PanelMsg) {
        let _ = self.out.send(msg);
    }
}

fn lock(s: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// Connection manager: connect, pump until the pipe breaks, repeat.
fn manage(pipe: &str, shared: &Arc<Mutex<Shared>>, out: &Receiver<PanelMsg>, wake: Arc<dyn Fn() + Send + Sync>) {
    loop {
        let conn = match skong_ipc::connect(pipe) {
            Ok(c) => c,
            Err(_) => {
                // Not running (yet): drop anything queued meanwhile and retry.
                while out.try_recv().is_ok() {}
                match out.recv_timeout(RETRY) {
                    Err(RecvTimeoutError::Disconnected) => return,
                    _ => continue,
                }
            }
        };
        {
            let mut s = lock(shared);
            let prev = s.hello.as_ref().map(|h| (h.pid, s.last_seq));
            *s = Shared { connected: true, session: s.session + 1, logs: std::mem::take(&mut s.logs), prev, ..Shared::default() };
        }
        wake();

        let alive = Arc::new(AtomicBool::new(true));
        let (reader, mut writer) = (conn.reader, conn.writer);
        let (s2, w2, a2) = (shared.clone(), wake.clone(), alive.clone());
        let read = std::thread::Builder::new().name("pet-link-read".into()).spawn(move || {
            let mut lines = BufReader::new(reader);
            let mut line = String::new();
            loop {
                line.clear();
                match lines.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(msg) = decode::<PetMsg>(&line) {
                            lock(&s2).ingest(msg);
                            w2();
                        }
                    }
                }
            }
            a2.store(false, Ordering::Relaxed);
            w2();
        });
        if read.is_err() {
            alive.store(false, Ordering::Relaxed);
        }

        while alive.load(Ordering::Relaxed) {
            match out.recv_timeout(Duration::from_millis(100)) {
                Ok(msg) => {
                    if writer.write_all(encode(&msg).as_bytes()).is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return, // UI gone
            }
        }
        drop(writer);
        if let Ok(h) = read {
            let _ = h.join();
        }
        lock(shared).connected = false;
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skong_ipc::LogLevel;

    fn line(seq: u64) -> LogLine {
        LogLine { seq, t: seq as f64, level: LogLevel::Info, target: "skong".into(), msg: format!("m{seq}") }
    }

    #[test]
    fn logs_dedup_by_sequence() {
        let mut s = Shared::default();
        s.ingest(PetMsg::Logs(vec![line(0), line(1), line(2)]));
        s.ingest(PetMsg::Logs(vec![line(2), line(3)])); // overlap between backlog and stream
        assert_eq!(s.logs.iter().map(|l| l.seq).collect::<Vec<_>>(), [0, 1, 2, 3]);
        s.ingest(PetMsg::Status(Box::default()));
        assert_eq!(s.statuses, 1);
    }
}
