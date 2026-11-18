//! Blocking main loop: a high-resolution waitable timer for fixed-rate ticks, plus the
//! Win32 message pump (needed for WinEvent hooks, hotkeys and window messages).
//! The thread sleeps in `MsgWaitForMultipleObjectsEx` between ticks — no spinning.
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer, TIMER_ALL_ACCESS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, PostQuitMessage,
    QS_ALLINPUT, TranslateMessage, WM_HOTKEY, WM_QUIT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// Per-tick context handed to the app callback.
pub struct Tick {
    /// Fixed simulation step in seconds.
    pub dt: f32,
    /// Hotkey ids pressed since the previous tick.
    pub hotkeys: Vec<i32>,
    pub now: Instant,
}

pub struct RunLoop {
    timer: HANDLE,
    period: Duration,
    hotkeys: Vec<i32>,
}

impl RunLoop {
    pub fn new(hz: f64) -> Result<Self> {
        // SAFETY: creates a timer handle owned by Self and closed in Drop.
        let timer = unsafe { CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }
            .context("CreateWaitableTimerExW (high resolution)")?;
        Ok(Self { timer, period: Duration::from_secs_f64(1.0 / hz), hotkeys: Vec::new() })
    }

    /// Register a global Ctrl+Alt+`vk` hotkey delivered as `Tick::hotkeys` entry `id`.
    pub fn register_hotkey(&mut self, id: i32, vk: VIRTUAL_KEY) -> Result<()> {
        self.register_hotkey_mods(id, MOD_CONTROL | MOD_ALT, vk)
    }

    pub fn register_hotkey_mods(&mut self, id: i32, mods: HOT_KEY_MODIFIERS, vk: VIRTUAL_KEY) -> Result<()> {
        // SAFETY: thread-level hotkey (no hwnd) — WM_HOTKEY is posted to this thread's queue.
        unsafe { RegisterHotKey(None, id, mods | MOD_NOREPEAT, vk.0 as u32) }.with_context(|| format!("RegisterHotKey {id} (already in use?)"))?;
        self.hotkeys.push(id);
        Ok(())
    }

    /// Run until the callback returns [`Flow::Quit`] or WM_QUIT arrives.
    pub fn run(&mut self, mut on_tick: impl FnMut(&mut Tick) -> Result<Flow>) -> Result<()> {
        let mut next = Instant::now();
        let mut tick = Tick { dt: self.period.as_secs_f32(), hotkeys: Vec::new(), now: next };
        loop {
            self.arm(next)?;
            // SAFETY: waiting on our own timer handle + the thread message queue.
            let r = unsafe { MsgWaitForMultipleObjectsEx(Some(&[self.timer]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
            if r == WAIT_OBJECT_0 {
                // Catch up on missed ticks (bounded) so sim time tracks wall time.
                let now = Instant::now();
                let mut steps = 0;
                while next <= now && steps < 8 {
                    tick.now = now;
                    let flow = on_tick(&mut tick)?;
                    tick.hotkeys.clear();
                    if let Flow::Quit = flow {
                        return Ok(());
                    }
                    next += self.period;
                    steps += 1;
                }
                if next <= now {
                    next = now + self.period; // too far behind (sleep/debugger): resync
                }
            } else if !self.pump(&mut tick.hotkeys) {
                return Ok(());
            }
        }
    }

    fn arm(&self, at: Instant) -> Result<()> {
        let wait = at.saturating_duration_since(Instant::now());
        // Negative = relative, in 100 ns units. Zero would be "absolute 0" → fire immediately anyway.
        let due = -(wait.as_nanos() as i64 / 100).max(1);
        // SAFETY: re-arming our one-shot timer.
        unsafe { SetWaitableTimer(self.timer, &due, 0, None, None, false) }.context("SetWaitableTimer")
    }

    /// Drain the message queue. Returns false on WM_QUIT.
    fn pump(&self, hotkeys: &mut Vec<i32>) -> bool {
        let mut msg = MSG::default();
        // SAFETY: standard message pump on the current thread.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                if msg.message == WM_HOTKEY {
                    hotkeys.push(msg.wParam.0 as i32);
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }
}

impl Drop for RunLoop {
    fn drop(&mut self) {
        // SAFETY: releasing resources we registered/created.
        unsafe {
            for id in &self.hotkeys {
                let _ = UnregisterHotKey(None, *id);
            }
            let _ = CloseHandle(self.timer);
        }
    }
}

/// Ask the loop to exit at the next pump.
pub fn request_quit() {
    // SAFETY: posts WM_QUIT to the current thread.
    unsafe { PostQuitMessage(0) };
}

/// Is the key physically down right now (global, focus-independent)?
pub fn key_down(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: pure query.
    (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0
}
