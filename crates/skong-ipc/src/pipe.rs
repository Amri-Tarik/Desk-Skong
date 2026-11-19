//! Named-pipe transport: a non-blocking [`Server`] for the pet's tick thread and a
//! blocking [`connect`] for panels.
//!
//! Every pipe handle is opened `FILE_FLAG_OVERLAPPED` so that one thread can block in a
//! read while another writes the same handle (synchronous pipe handles serialize the two
//! and deadlock). Each [`PipeReader`] / [`PipeWriter`] owns its own event and waits for its
//! own operation, giving plain blocking `Read` / `Write` on top.
//!
//! Server threads: one listener (blocked in `ConnectNamedPipe` — no CPU while nobody
//! connects) plus a reader and a writer per connected client. The owner only ever does
//! `try_recv` / `try_send`, so it never blocks.
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError, channel, sync_channel};
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{HSTRING, PCWSTR};

/// The pet's control pipe.
pub const PIPE_NAME: &str = r"\\.\pipe\desk-skong";
/// Lines queued per client before it is considered stuck and dropped.
const CLIENT_QUEUE: usize = 4096;
const BUFFER: u32 = 64 * 1024;

fn win_err(e: windows::core::Error) -> io::Error {
    // Win32 errors arrive as HRESULT_FROM_WIN32(code): the low 16 bits are the code.
    io::Error::from_raw_os_error(e.code().0 & 0xffff)
}

fn is(e: &windows::core::Error, code: windows::Win32::Foundation::WIN32_ERROR) -> bool {
    e.code() == code.to_hresult()
}

/// An owned kernel handle.
struct Owned(HANDLE);

// SAFETY: kernel handles are process-wide values usable from any thread; all I/O on the
// pipe handle is overlapped with per-operation OVERLAPPED structures, so concurrent use
// from a reader and a writer thread is sound.
unsafe impl Send for Owned {}
// SAFETY: see `Send`; `&Owned` only exposes the raw handle value.
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// One direction of an overlapped pipe handle, with its own completion event.
struct Io {
    pipe: Arc<Owned>,
    event: Owned,
}

impl Io {
    fn new(pipe: Arc<Owned>) -> io::Result<Self> {
        // SAFETY: anonymous manual-reset event, owned by the returned value.
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(win_err)?;
        Ok(Self { pipe, event: Owned(event) })
    }

    /// Start an overlapped operation and wait for it. Returns bytes transferred.
    fn run(&self, start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> windows::core::Result<()>) -> windows::core::Result<u32> {
        let mut ov = OVERLAPPED { hEvent: self.event.0, ..Default::default() };
        match start(self.pipe.0, &mut ov) {
            Ok(()) => {}
            Err(e) if is(&e, ERROR_IO_PENDING) => {}
            Err(e) => return Err(e), // nothing in flight: `ov` may go out of scope
        }
        let mut n = 0;
        // SAFETY: `ov` stays alive until the operation completes (bWait = true).
        unsafe { GetOverlappedResult(self.pipe.0, &ov, &mut n, true) }?;
        Ok(n)
    }
}

/// Blocking reader half; EOF when the other end closes.
pub struct PipeReader(Io);
/// Blocking writer half.
pub struct PipeWriter(Io);

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize);
        // SAFETY: `buf` outlives the operation, which `run` waits for.
        match self.0.run(|h, ov| unsafe { ReadFile(h, Some(&mut buf[..len]), None, Some(ov)) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if is(&e, ERROR_BROKEN_PIPE) || is(&e, ERROR_PIPE_NOT_CONNECTED) || is(&e, ERROR_OPERATION_ABORTED) => Ok(0),
            Err(e) => Err(win_err(e)),
        }
    }
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize);
        // SAFETY: `buf` outlives the operation, which `run` waits for.
        match self.0.run(|h, ov| unsafe { WriteFile(h, Some(&buf[..len]), None, Some(ov)) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if is(&e, ERROR_BROKEN_PIPE) || is(&e, ERROR_NO_DATA) || is(&e, ERROR_PIPE_NOT_CONNECTED) => {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"))
            }
            Err(e) => Err(win_err(e)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl PipeWriter {
    /// Abort pending I/O and disconnect (server side): the peer and our reader see EOF.
    fn hang_up(&self) {
        // SAFETY: plain calls on a handle we keep alive via `Arc`.
        unsafe {
            let _ = CancelIoEx(self.0.pipe.0, None);
            let _ = DisconnectNamedPipe(self.0.pipe.0);
        }
    }
}

fn split(pipe: Owned) -> io::Result<(PipeReader, PipeWriter)> {
    let pipe = Arc::new(pipe);
    Ok((PipeReader(Io::new(pipe.clone())?), PipeWriter(Io::new(pipe)?)))
}

/// A panel's connection to the pet.
pub struct Connection {
    pub reader: PipeReader,
    pub writer: PipeWriter,
}

/// Connect to a pipe server (waits up to ~2 s if all instances are busy).
pub fn connect(name: &str) -> io::Result<Connection> {
    let wide = HSTRING::from(name);
    for attempt in 0..2 {
        // SAFETY: opening a pipe client handle that `split` takes ownership of.
        let r = unsafe {
            CreateFileW(&wide, (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
        };
        match r {
            Ok(h) => {
                let (reader, writer) = split(Owned(h))?;
                return Ok(Connection { reader, writer });
            }
            Err(e) if is(&e, ERROR_PIPE_BUSY) && attempt == 0 => {
                // SAFETY: plain wait on a pipe name.
                let _ = unsafe { WaitNamedPipeW(&wide, 2000) };
            }
            Err(e) => return Err(win_err(e)),
        }
    }
    Err(io::Error::new(io::ErrorKind::WouldBlock, "all pipe instances busy"))
}

pub type ClientId = u64;

/// What happened since the last [`Server::poll`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    Connected(ClientId),
    /// One line received (without the trailing newline).
    Line(ClientId, String),
    Disconnected(ClientId),
}

enum Internal {
    Connected(ClientId, SyncSender<Arc<str>>),
    Line(ClientId, String),
    Disconnected(ClientId),
}

/// Pipe server driven by non-blocking polling.
pub struct Server {
    events: Receiver<Internal>,
    clients: HashMap<ClientId, SyncSender<Arc<str>>>,
}

impl Server {
    /// Create the pipe and start accepting. Fails if another process already serves `name`.
    pub fn start(name: &str) -> io::Result<Self> {
        let wide = HSTRING::from(name);
        let first = Owned(create_instance(&wide, true)?);
        let (tx, events) = channel();
        std::thread::Builder::new().name("ipc-listener".into()).spawn(move || listen(wide, first, tx))?;
        Ok(Self { events, clients: HashMap::new() })
    }

    /// Drain connection events and received lines. Never blocks.
    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(Internal::Connected(id, tx)) => {
                    self.clients.insert(id, tx);
                    out.push(ServerEvent::Connected(id));
                }
                Ok(Internal::Line(id, l)) => out.push(ServerEvent::Line(id, l)),
                Ok(Internal::Disconnected(id)) => {
                    if self.clients.remove(&id).is_some() {
                        out.push(ServerEvent::Disconnected(id));
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        out
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Queue `line` (newline-terminated) for one client. A client whose queue is full is
    /// hung up on (it will see EOF and may reconnect).
    pub fn send(&mut self, id: ClientId, line: Arc<str>) {
        if let Some(tx) = self.clients.get(&id)
            && let Err(e) = tx.try_send(line)
        {
            if matches!(e, TrySendError::Full(_)) {
                log::warn!("control client {id} is not reading; disconnecting it");
            }
            self.clients.remove(&id); // dropping the sender ends its writer thread
        }
    }

    pub fn broadcast(&mut self, line: &str) {
        if self.clients.is_empty() {
            return;
        }
        let line: Arc<str> = Arc::from(line);
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            self.send(id, line.clone());
        }
    }
}

fn create_instance(name: &HSTRING, first: bool) -> io::Result<HANDLE> {
    let mut open = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    // SAFETY: creates a pipe instance whose handle the caller owns.
    let h = unsafe { CreateNamedPipeW(name, open, mode, PIPE_UNLIMITED_INSTANCES, BUFFER, BUFFER, 0, None) };
    if h.is_invalid() { Err(io::Error::last_os_error()) } else { Ok(h) }
}

/// Listener thread: wait for a client on `pending`, hand it off, create the next instance.
fn listen(name: HSTRING, mut pending: Owned, events: Sender<Internal>) {
    let mut next_id: ClientId = 1;
    loop {
        match accept(pending.0) {
            Ok(()) => {
                if spawn_client(next_id, pending, events.clone()).is_err() {
                    return;
                }
                next_id += 1;
            }
            Err(e) => {
                log::warn!("control pipe: accept failed: {e}");
                drop(pending);
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        pending = loop {
            match create_instance(&name, false) {
                Ok(h) => break Owned(h),
                Err(e) => {
                    log::warn!("control pipe: cannot create instance: {e}");
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        };
    }
}

/// Block (overlapped wait, no CPU) until a client connects to `pipe`.
fn accept(pipe: HANDLE) -> io::Result<()> {
    // SAFETY: temporary event closed by `Owned`; `ov` outlives the wait below.
    unsafe {
        let event = Owned(CreateEventW(None, true, false, PCWSTR::null()).map_err(win_err)?);
        let mut ov = OVERLAPPED { hEvent: event.0, ..Default::default() };
        match ConnectNamedPipe(pipe, Some(&mut ov)) {
            Ok(()) => Ok(()),
            Err(e) if is(&e, ERROR_PIPE_CONNECTED) => Ok(()), // connected between create and connect
            Err(e) if is(&e, ERROR_IO_PENDING) => {
                let mut n = 0;
                GetOverlappedResult(pipe, &ov, &mut n, true).map_err(win_err)
            }
            Err(e) => Err(win_err(e)),
        }
    }
}

/// Reader + writer threads for one connected client. Errs only when the owner is gone.
fn spawn_client(id: ClientId, pipe: Owned, events: Sender<Internal>) -> Result<(), ()> {
    let (reader, writer) = match split(pipe) {
        Ok(rw) => rw,
        Err(e) => {
            log::warn!("control pipe: {e}");
            return Ok(());
        }
    };
    let (tx, rx) = sync_channel::<Arc<str>>(CLIENT_QUEUE);
    events.send(Internal::Connected(id, tx)).map_err(|_| ())?;

    let spawned = std::thread::Builder::new().name(format!("ipc-write-{id}")).spawn(move || {
        let mut w = writer;
        while let Ok(line) = rx.recv() {
            if w.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
        w.hang_up();
    });
    if let Err(e) = spawned {
        log::warn!("control pipe: {e}");
    }
    let events2 = events.clone();
    let spawned = std::thread::Builder::new().name(format!("ipc-read-{id}")).spawn(move || {
        let mut lines = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match lines.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let l = line.trim_end();
                    if !l.is_empty() && events2.send(Internal::Line(id, l.to_string())).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = events2.send(Internal::Disconnected(id));
    });
    if let Err(e) = spawned {
        log::warn!("control pipe: {e}");
        let _ = events.send(Internal::Disconnected(id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn wait_for(server: &mut Server, pred: impl Fn(&ServerEvent) -> bool) -> Vec<ServerEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut all = Vec::new();
        while Instant::now() < deadline {
            let evs = server.poll();
            let done = evs.iter().any(&pred);
            all.extend(evs);
            if done {
                return all;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out; got {all:?}");
    }

    #[test]
    fn duplex_lines_multiple_clients_and_reconnect() {
        let name = format!(r"\\.\pipe\desk-skong-test-{}", std::process::id());
        let mut server = Server::start(&name).unwrap();
        assert!(Server::start(&name).is_err(), "second server on the same name must fail");

        let a = connect(&name).unwrap();
        let b = connect(&name).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.client_count() < 2 {
            assert!(Instant::now() < deadline, "clients never connected");
            server.poll();
            std::thread::sleep(Duration::from_millis(5));
        }

        let (mut ar, mut aw) = (BufReader::new(a.reader), a.writer);
        aw.write_all(b"hello pet\n").unwrap();
        wait_for(&mut server, |e| matches!(e, ServerEvent::Line(_, l) if l == "hello pet"));
        server.broadcast("hi panels\n");
        let mut got = String::new();
        ar.read_line(&mut got).unwrap();
        assert_eq!(got, "hi panels\n");
        let mut br = BufReader::new(b.reader);
        got.clear();
        br.read_line(&mut got).unwrap();
        assert_eq!(got, "hi panels\n");

        // Panel A goes away: the server notices, B still works, a new client can join.
        drop(ar);
        drop(aw);
        wait_for(&mut server, |e| matches!(e, ServerEvent::Disconnected(_)));
        assert_eq!(server.client_count(), 1);
        let c = connect(&name).unwrap();
        wait_for(&mut server, |e| matches!(e, ServerEvent::Connected(_)));
        server.broadcast("again\n");
        got.clear();
        br.read_line(&mut got).unwrap();
        assert_eq!(got, "again\n");
        got.clear();
        BufReader::new(c.reader).read_line(&mut got).unwrap();
        assert_eq!(got, "again\n");
    }
}
