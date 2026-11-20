//! Where the user's highlighted text is, via UI Automation — **rectangles only, never the
//! text** (on by default; the pet's "Curious about highlighted text" toggle).
//!
//! One background thread (COM in the MTA) does everything; the pet's 120 Hz loop only
//! drains a channel ([`SelectionWatcher::poll`]):
//!
//! * **Event-driven.** An out-of-context WinEvent hook for `EVENT_OBJECT_TEXTSELECTIONCHANGED`
//!   (what UIA's `TextSelectionChanged` is bridged to, and what MSAA / IA2 apps raise) plus
//!   `EVENT_SYSTEM_FOREGROUND` (switching apps hides the old selection). The callback only
//!   timestamps; no UIA work happens in it, and nothing polls.
//! * **Throttled.** A burst (dragging a selection fires dozens) is coalesced: the thread
//!   queries once the events pause for [`QUIET`], or every [`MAX_DELAY`] during a long drag.
//! * **Thin query.** `GetFocusedElement` → `TextPattern` → `GetSelection` →
//!   `GetBoundingRectangles` (capped), unioned and clipped to the element. Elements without
//!   a TextPattern, password fields and failures simply mean "no selection". UIA timeouts
//!   are shortened so a hung app can't stall the thread for long.
//!
//! `IUIAutomationTextRange::GetText` is never called: the text never enters this process.
use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use skong_core::geom::RectI;
use windows::Win32::Foundation::{HWND, LPARAM, WAIT_TIMEOUT, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId, INFINITE};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, HWINEVENTHOOK, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern, SetWinEventHook, UIA_TextPatternId, UnhookWinEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_TEXTSELECTIONCHANGED, EVENT_SYSTEM_FOREGROUND, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_NOREMOVE,
    PM_REMOVE, PeekMessageW, PostThreadMessageW, QS_ALLINPUT, TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};
use windows::core::Interface;

/// Query once the selection events have paused this long ...
pub const QUIET: Duration = Duration::from_millis(150);
/// ... or at the latest this long after the first event of a burst (long drags).
pub const MAX_DELAY: Duration = Duration::from_millis(500);
/// Never query more often than this.
const MIN_GAP: Duration = Duration::from_millis(100);
/// UIA connection / transaction timeouts (ms): a hung app can't hold the thread for long.
const UIA_TIMEOUT_MS: u32 = 1000;
/// At most this many selected ranges (multi-selection) and rectangles (lines) are read.
const MAX_RANGES: i32 = 8;
const MAX_RECTS: usize = 256;

/// Watches the user's text selection on a background thread. Dropping it unhooks and stops
/// the thread.
pub struct SelectionWatcher {
    rx: Receiver<Option<RectI>>,
    thread_id: u32,
    join: Option<JoinHandle<()>>,
    rect: Option<RectI>,
}

impl SelectionWatcher {
    pub fn start() -> Result<SelectionWatcher> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, String>>(1);
        let join = std::thread::Builder::new()
            .name("selection".into())
            .spawn(move || run(tx, ready_tx))
            .context("spawning the selection thread")?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(SelectionWatcher { rx, thread_id, join: Some(join), rect: None }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(anyhow!(e))
            }
            Err(_) => {
                let _ = join.join();
                Err(anyhow!("selection thread died during start-up"))
            }
        }
    }

    /// Take in what the thread found (non-blocking). `true` if the rectangle changed.
    pub fn poll(&mut self) -> bool {
        let before = self.rect;
        while let Ok(r) = self.rx.try_recv() {
            self.rect = r;
        }
        self.rect != before
    }

    /// Where the highlighted text is (virtual-desktop pixels), if any.
    pub fn rect(&self) -> Option<RectI> {
        self.rect
    }
}

impl Drop for SelectionWatcher {
    fn drop(&mut self) {
        // SAFETY: plain message post to our own thread's queue (created before start returned).
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

thread_local! {
    /// First and last event of the current burst (set by the WinEvent callback, which runs
    /// on the watcher thread while it pumps messages).
    static BURST: Cell<Option<(Instant, Instant)>> = const { Cell::new(None) };
}

unsafe extern "system" fn on_event(_hook: HWINEVENTHOOK, _event: u32, _hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _time: u32) {
    let now = Instant::now();
    BURST.with(|b| b.set(Some(b.get().map_or((now, now), |(first, _)| (first, now)))));
}

/// Unhooks on drop.
struct Hooks(Vec<HWINEVENTHOOK>);

impl Drop for Hooks {
    fn drop(&mut self) {
        for &h in &self.0 {
            // SAFETY: handles from successful SetWinEventHook calls on this thread.
            let _ = unsafe { UnhookWinEvent(h) };
        }
    }
}

/// COM for this thread; uninitialised on drop (after every interface is released).
struct Com;

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: paired with the successful CoInitializeEx in `run`.
        unsafe { CoUninitialize() };
    }
}

fn setup() -> Result<(Com, IUIAutomation, Hooks)> {
    // SAFETY: COM init for this thread (MTA, as UIA recommends for clients).
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().context("CoInitializeEx")?;
    let com = Com;
    // SAFETY: standard in-proc creation of the UIA client object.
    let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.context("creating CUIAutomation")?;
    if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
        // SAFETY: plain property setters.
        unsafe {
            let _ = uia2.SetConnectionTimeout(UIA_TIMEOUT_MS);
            let _ = uia2.SetTransactionTimeout(UIA_TIMEOUT_MS);
        }
    }
    let mut hooks = Hooks(Vec::new());
    for event in [EVENT_OBJECT_TEXTSELECTIONCHANGED, EVENT_SYSTEM_FOREGROUND] {
        // SAFETY: out-of-context hook with a 'static callback; delivered to this thread.
        let h = unsafe { SetWinEventHook(event, event, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) };
        if h.is_invalid() {
            return Err(anyhow!("SetWinEventHook({event:#x}) failed"));
        }
        hooks.0.push(h);
    }
    Ok((com, uia, hooks))
}

fn run(tx: Sender<Option<RectI>>, ready: mpsc::SyncSender<Result<u32, String>>) {
    let mut msg = MSG::default();
    // SAFETY: creates this thread's message queue so PostThreadMessageW can reach it.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    // Declared in this order so the hooks and the UIA object go before COM is uninitialised.
    let (_com, uia, _hooks) = match setup() {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("{e:#}")));
            return;
        }
    };
    // SAFETY: trivial getters.
    let (thread_id, own_pid) = unsafe { (GetCurrentThreadId(), GetCurrentProcessId()) };
    let _ = ready.send(Ok(thread_id));
    log::info!("text selection watcher on (UI Automation; rectangles only)");

    let mut sent: Option<RectI> = None;
    let mut last_query = Instant::now() - MIN_GAP;
    loop {
        let timeout = match BURST.with(Cell::get) {
            None => INFINITE,
            Some((first, last)) => {
                let due = (last + QUIET).min(first + MAX_DELAY).max(last_query + MIN_GAP);
                due.saturating_duration_since(Instant::now()).as_millis() as u32
            }
        };
        // SAFETY: no handles; wakes on any message (WinEvents arrive as messages) or timeout.
        let r = unsafe { MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        if r != WAIT_TIMEOUT {
            // SAFETY: standard pump; WinEvent callbacks run inside PeekMessageW.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        log::info!("text selection watcher off");
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        let Some((first, last)) = BURST.with(Cell::get) else { continue };
        let now = Instant::now();
        if (now < last + QUIET && now < first + MAX_DELAY) || now < last_query + MIN_GAP {
            continue;
        }
        BURST.with(|b| b.set(None));
        last_query = now;
        let rect = query(&uia, own_pid);
        if rect != sent {
            log::debug!("text selection: {rect:?}");
            sent = rect;
            if tx.send(rect).is_err() {
                return; // the watcher is gone
            }
        }
    }
}

/// The focused element's selection, as one rectangle (union of its lines), clipped to the
/// element. `None` for no selection, no TextPattern, a password field, or any failure.
fn query(uia: &IUIAutomation, own_pid: u32) -> Option<RectI> {
    // SAFETY: COM calls on interfaces owned by this thread's MTA; every SAFEARRAY returned
    // is destroyed in `doubles`.
    unsafe {
        let el = uia.GetFocusedElement().ok()?;
        if el.CurrentProcessId().ok()? as u32 == own_pid || el.CurrentIsPassword().ok()?.as_bool() {
            return None;
        }
        let text: IUIAutomationTextPattern = el.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
        let ranges = text.GetSelection().ok()?;
        let mut quads = Vec::new();
        for i in 0..ranges.Length().ok()?.min(MAX_RANGES) {
            let Ok(range) = ranges.GetElement(i) else { continue };
            if let Ok(sa) = range.GetBoundingRectangles() {
                quads.extend(doubles(sa));
            }
            if quads.len() >= 4 * MAX_RECTS {
                break;
            }
        }
        let clip = el.CurrentBoundingRectangle().ok().map(|r| RectI::new(r.left, r.top, r.right, r.bottom));
        union_of_quads(&quads, clip)
    }
}

/// Copy a 1-D SAFEARRAY of doubles out and destroy it.
unsafe fn doubles(sa: *mut SAFEARRAY) -> Vec<f64> {
    if sa.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    // SAFETY: `sa` is a valid SAFEARRAY of VT_R8 from GetBoundingRectangles, owned by us.
    unsafe {
        if let (Ok(lo), Ok(hi)) = (SafeArrayGetLBound(sa, 1), SafeArrayGetUBound(sa, 1)) {
            let n = (hi - lo + 1).clamp(0, 4 * MAX_RECTS as i32) as usize;
            let mut data = std::ptr::null_mut();
            if n > 0 && SafeArrayAccessData(sa, &mut data).is_ok() {
                if !data.is_null() {
                    out.extend_from_slice(std::slice::from_raw_parts(data as *const f64, n));
                }
                let _ = SafeArrayUnaccessData(sa);
            }
        }
        let _ = SafeArrayDestroy(sa);
    }
    out
}

/// Union of UIA bounding rectangles given as (left, top, width, height) quadruples, clipped
/// to `clip` when that is a real rectangle. Degenerate (empty) rectangles are skipped: a
/// collapsed selection (just the caret) has none and gives `None`.
pub fn union_of_quads(quads: &[f64], clip: Option<RectI>) -> Option<RectI> {
    let mut out: Option<RectI> = None;
    for q in quads.chunks_exact(4) {
        let (x, y, w, h) = (q[0], q[1], q[2], q[3]);
        if !(w > 0.5 && h > 0.5) || !x.is_finite() || !y.is_finite() {
            continue;
        }
        let r = RectI::new(x.floor() as i32, y.floor() as i32, (x + w).ceil() as i32, (y + h).ceil() as i32);
        out = Some(match out {
            None => r,
            Some(u) => RectI::new(u.left.min(r.left), u.top.min(r.top), u.right.max(r.right), u.bottom.max(r.bottom)),
        });
    }
    let out = match clip.filter(|c| !c.is_empty()) {
        Some(c) => out?.intersect(&c),
        None => out?,
    };
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_unioned_and_clipped_to_the_element() {
        let quads = [100.0, 200.0, 300.0, 20.0, 80.0, 220.0, 150.5, 20.0];
        assert_eq!(union_of_quads(&quads, None), Some(RectI::new(80, 200, 400, 240)));
        assert_eq!(union_of_quads(&quads, Some(RectI::new(0, 0, 350, 230))), Some(RectI::new(80, 200, 350, 230)));
    }

    #[test]
    fn a_caret_or_a_selection_scrolled_away_is_none() {
        assert_eq!(union_of_quads(&[], None), None);
        assert_eq!(union_of_quads(&[100.0, 200.0, 0.0, 20.0], None), None);
        assert_eq!(union_of_quads(&[100.0, 200.0, 50.0, 20.0], Some(RectI::new(500, 500, 900, 900))), None);
    }
}
