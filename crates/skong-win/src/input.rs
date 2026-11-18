//! What the user is doing, without spying on it:
//! * idle time via `GetLastInputInfo`;
//! * typing *rate* via Raw Input keyboard events delivered to a hidden window
//!   (`RIDEV_INPUTSINK`), counting key-downs only — which keys were pressed is never read.
//!   Unlike a low-level keyboard hook this adds no latency to anyone's typing;
//! * mouse buttons via `GetAsyncKeyState`.
use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEKEYBOARD,
    RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RI_KEY_BREAK, RegisterClassExW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT,
    WNDCLASSEXW,
};
use windows::core::w;

thread_local! {
    static KEYDOWNS: Cell<u32> = const { Cell::new(0) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_INPUT {
        let mut raw = RAWINPUT::default();
        let mut size = size_of::<RAWINPUT>() as u32;
        // SAFETY: lParam is the HRAWINPUT for this message; `raw` is large enough for a
        // keyboard packet and `size` says so.
        let n = unsafe { GetRawInputData(HRAWINPUT(lp.0 as _), RID_INPUT, Some(&mut raw as *mut _ as _), &mut size, size_of::<RAWINPUTHEADER>() as u32) };
        if n != u32::MAX && raw.header.dwType == RIM_TYPEKEYBOARD.0 {
            // SAFETY: dwType says the union holds keyboard data.
            let flags = unsafe { raw.data.keyboard.Flags };
            if flags as u32 & RI_KEY_BREAK == 0 {
                KEYDOWNS.with(|c| c.set(c.get() + 1));
            }
        }
    }
    // SAFETY: default processing (required for WM_INPUT cleanup too).
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub struct UserInput {
    hwnd: HWND,
    /// (time, key-downs counted in that tick) over the rate window.
    recent: VecDeque<(Instant, u32)>,
    window: Duration,
}

impl UserInput {
    pub fn new() -> Result<Self> {
        // SAFETY: hidden window that only receives WM_INPUT; destroyed in Drop.
        let hwnd = unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = w!("DeskSkongInput");
            let wc = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassExW(&wc);
            CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Desk-Skong input"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(hinst.into()), None).context("input window")?
        };
        let dev = RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x06, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
        // SAFETY: registering keyboard raw input for our own window.
        if let Err(e) = unsafe { RegisterRawInputDevices(&[dev], size_of::<RAWINPUTDEVICE>() as u32) } {
            log::warn!("raw keyboard input unavailable ({e}); typing rate will read 0");
        }
        Ok(Self { hwnd, recent: VecDeque::new(), window: Duration::from_secs(3) })
    }

    /// Key-downs per second over the last few seconds. Call once per tick.
    pub fn keys_per_sec(&mut self) -> f32 {
        let now = Instant::now();
        let n = KEYDOWNS.with(|c| c.replace(0));
        if n > 0 {
            self.recent.push_back((now, n));
        }
        while self.recent.front().is_some_and(|(t, _)| now.duration_since(*t) > self.window) {
            self.recent.pop_front();
        }
        self.recent.iter().map(|(_, n)| *n).sum::<u32>() as f32 / self.window.as_secs_f32()
    }
}

impl Drop for UserInput {
    fn drop(&mut self) {
        // SAFETY: our own window.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Seconds since the last keyboard/mouse input anywhere in the session.
pub fn idle_secs() -> f32 {
    let mut lii = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: plain queries.
    unsafe {
        if !GetLastInputInfo(&mut lii).as_bool() {
            return 0.0;
        }
        GetTickCount().wrapping_sub(lii.dwTime) as f32 / 1000.0
    }
}

/// (left, right) mouse buttons physically down.
pub fn mouse_buttons() -> (bool, bool) {
    // SAFETY: pure queries.
    let down = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0;
    (down(VK_LBUTTON.0 as i32), down(VK_RBUTTON.0 as i32))
}
