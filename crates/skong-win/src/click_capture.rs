//! One click, captured (the test panel's "📍 Place…"): while it lives, a low-level mouse
//! hook takes the next left click anywhere on screen — the click (down and up) is swallowed,
//! so it never reaches the app underneath — and reports where it was. A right click or Esc
//! cancels (also swallowed). Clicks on the control panel itself pass through (its Cancel
//! button works).
//!
//! The hooks live on their own thread with its own message loop, so a slow pet tick never
//! lags the mouse; they exist only while a [`ClickCapture`] does (dropping it unhooks and
//! joins). Nothing here moves the cursor or any window.
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use anyhow::{Result, anyhow, bail};
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GA_ROOT, GetAncestor, GetMessageW, GetWindowTextW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WindowFromPoint,
};

/// Clicks on windows titled like this pass through (the control panel).
const PASS_THROUGH_TITLE: &str = "Desk-Skong control panel";

/// What the user did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    /// Left-clicked here (virtual-desktop px).
    At(i32, i32),
    /// Right-clicked or pressed Esc.
    Cancelled,
}

// The hook procedures get no user data: one capture at a time, its state in statics.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// 0 waiting, 1 left down taken (swallow its up), 2 right down taken (swallow its up), 3 done.
static STATE: AtomicU8 = AtomicU8::new(0);
/// 0 nothing yet, 1 picked, 2 cancelled.
static RESULT: AtomicU8 = AtomicU8::new(0);
static X: AtomicI32 = AtomicI32::new(0);
static Y: AtomicI32 = AtomicI32::new(0);

const WAITING: u8 = 0;
const LEFT_DOWN: u8 = 1;
const RIGHT_DOWN: u8 = 2;
const DONE: u8 = 3;

/// The live capture: hooks installed on their own thread until dropped.
pub struct ClickCapture {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

/// Is the click at `pt` on the control panel (it passes through)?
fn on_panel(pt: POINT) -> bool {
    // SAFETY: plain window queries; a stale or null HWND just yields no title.
    unsafe {
        let root = GetAncestor(WindowFromPoint(pt), GA_ROOT);
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(root, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n]).starts_with(PASS_THROUGH_TITLE)
    }
}

unsafe extern "system" fn mouse_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && ACTIVE.load(Ordering::Acquire) {
        // SAFETY: for WH_MOUSE_LL with HC_ACTION, lParam points at an MSLLHOOKSTRUCT.
        let info = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
        let state = STATE.load(Ordering::Acquire);
        let swallow = match (state, w.0 as u32) {
            (WAITING, WM_LBUTTONDOWN) if !on_panel(info.pt) => {
                X.store(info.pt.x, Ordering::Release);
                Y.store(info.pt.y, Ordering::Release);
                STATE.store(LEFT_DOWN, Ordering::Release);
                true
            }
            (LEFT_DOWN, WM_LBUTTONUP) => {
                RESULT.store(1, Ordering::Release);
                STATE.store(DONE, Ordering::Release);
                true
            }
            (WAITING, WM_RBUTTONDOWN) => {
                STATE.store(RIGHT_DOWN, Ordering::Release);
                true
            }
            (RIGHT_DOWN, WM_RBUTTONUP) => {
                RESULT.store(2, Ordering::Release);
                STATE.store(DONE, Ordering::Release);
                true
            }
            _ => false,
        };
        if swallow {
            return LRESULT(1);
        }
    }
    // SAFETY: passing the event on unchanged.
    unsafe { CallNextHookEx(None, code, w, l) }
}

unsafe extern "system" fn key_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && ACTIVE.load(Ordering::Acquire) {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lParam points at a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if info.vkCode == VK_ESCAPE.0 as u32 {
            match w.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if STATE.load(Ordering::Acquire) != DONE {
                        RESULT.store(2, Ordering::Release);
                        STATE.store(DONE, Ordering::Release);
                    }
                    return LRESULT(1);
                }
                WM_KEYUP | WM_SYSKEYUP => return LRESULT(1),
                _ => {}
            }
        }
    }
    // SAFETY: passing the event on unchanged.
    unsafe { CallNextHookEx(None, code, w, l) }
}

impl ClickCapture {
    /// Install the hooks (fails if another capture is live).
    pub fn start() -> Result<ClickCapture> {
        if ACTIVE.swap(true, Ordering::AcqRel) {
            bail!("a click capture is already running");
        }
        STATE.store(WAITING, Ordering::Release);
        RESULT.store(0, Ordering::Release);
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();
        let thread = std::thread::Builder::new()
            .name("click-capture".into())
            .spawn(move || {
                // SAFETY: hooks installed and removed on this thread, which pumps messages
                // (required for low-level hooks) until WM_QUIT.
                unsafe {
                    let mut msg = MSG::default();
                    // Create this thread's message queue before anyone posts to it.
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    let module = match GetModuleHandleW(None) {
                        Ok(m) => m,
                        Err(e) => {
                            let _ = tx.send(Err(format!("GetModuleHandleW: {e}")));
                            return;
                        }
                    };
                    let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(module.into()), 0);
                    let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), Some(module.into()), 0);
                    let (mouse, keys) = match (mouse, keys) {
                        (Ok(m), Ok(k)) => (m, k),
                        (m, k) => {
                            for h in [m.ok(), k.ok()].into_iter().flatten() {
                                let _ = UnhookWindowsHookEx(h);
                            }
                            let _ = tx.send(Err("SetWindowsHookExW failed".into()));
                            return;
                        }
                    };
                    let _ = tx.send(Ok(GetCurrentThreadId()));
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                    let _ = UnhookWindowsHookEx(mouse);
                    let _ = UnhookWindowsHookEx(keys);
                }
            });
        let thread = match thread {
            Ok(t) => t,
            Err(e) => {
                ACTIVE.store(false, Ordering::Release);
                bail!("click capture thread: {e}");
            }
        };
        match rx.recv() {
            Ok(Ok(thread_id)) => Ok(ClickCapture { thread_id, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                ACTIVE.store(false, Ordering::Release);
                Err(anyhow!("click capture: {e}"))
            }
            Err(_) => {
                let _ = thread.join();
                ACTIVE.store(false, Ordering::Release);
                bail!("click capture thread ended early")
            }
        }
    }

    /// The click (or the cancel), once it happened.
    pub fn poll(&self) -> Option<Captured> {
        match RESULT.load(Ordering::Acquire) {
            1 => Some(Captured::At(X.load(Ordering::Acquire), Y.load(Ordering::Acquire))),
            2 => Some(Captured::Cancelled),
            _ => None,
        }
    }
}

impl Drop for ClickCapture {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
        // SAFETY: posting WM_QUIT to our own hook thread (its queue exists).
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
