//! Notification-area icon with a data-driven popup menu.
//!
//! A hidden message-only window receives the icon callbacks; events are queued and read
//! by the app with [`Tray::take_events`]. The menu is shown with `TrackPopupMenu`
//! (`TPM_RETURNCMD`), which returns the chosen command id synchronously.
use std::cell::RefCell;

use anyhow::{Context, Result};
use skong_core::canvas::Image;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
    GetCursorPos, HICON, HMENU, HWND_MESSAGE, ICONINFO, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW,
    RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW, WINDOW_EX_STYLE, WINDOW_STYLE,
};
use windows::core::{HSTRING, PCWSTR, w};

const WM_TRAY: u32 = WM_APP + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Right click / keyboard context menu on the icon.
    MenuRequested,
    LeftClick,
}

/// Menu description; ids must be non-zero.
#[derive(Clone, Debug)]
pub enum MenuItem {
    Action { id: u32, label: String, checked: bool, enabled: bool },
    Submenu { label: String, items: Vec<MenuItem> },
    Separator,
}

impl MenuItem {
    pub fn action(id: u32, label: impl Into<String>) -> Self {
        MenuItem::Action { id, label: label.into(), checked: false, enabled: true }
    }
    pub fn check(id: u32, label: impl Into<String>, checked: bool) -> Self {
        MenuItem::Action { id, label: label.into(), checked, enabled: true }
    }
    pub fn label(label: impl Into<String>) -> Self {
        MenuItem::Action { id: u32::MAX, label: label.into(), checked: false, enabled: false }
    }
    pub fn sub(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        MenuItem::Submenu { label: label.into(), items }
    }
}

thread_local! {
    static EVENTS: RefCell<Vec<TrayEvent>> = const { RefCell::new(Vec::new()) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static RECREATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        // NOTIFYICON_VERSION_4: LOWORD(lParam) is the mouse/notification event.
        let ev = (lp.0 & 0xffff) as u32;
        let e = match ev {
            WM_RBUTTONUP | WM_CONTEXTMENU => Some(TrayEvent::MenuRequested),
            WM_LBUTTONUP => Some(TrayEvent::LeftClick),
            _ => None,
        };
        if let Some(e) = e {
            EVENTS.with(|q| q.borrow_mut().push(e));
        }
        return LRESULT(0);
    }
    if msg != 0 && msg == TASKBAR_CREATED.with(|c| c.get()) {
        // Explorer restarted: the app re-adds the icon on next poll.
        RECREATE.with(|c| c.set(true));
        return LRESULT(0);
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    tip: String,
}

impl Tray {
    pub fn new(tip: &str, icon: &Image) -> Result<Self> {
        // SAFETY: standard window-class registration and message-only window creation.
        let hwnd = unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = w!("DeskSkongTray");
            let wc = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassExW(&wc);
            TASKBAR_CREATED.with(|c| c.set(RegisterWindowMessageW(w!("TaskbarCreated"))));
            CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Desk-Skong tray"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst.into()), None)
                .context("tray window")?
        };
        let mut t = Tray { hwnd, icon: make_icon(icon)?, tip: tip.to_string() };
        t.add()?;
        Ok(t)
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        for (d, s) in nid.szTip.iter_mut().zip(self.tip.encode_utf16().take(127)) {
            *d = s;
        }
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        nid
    }

    fn add(&mut self) -> Result<()> {
        let nid = self.data();
        // SAFETY: nid is fully initialised and owned by this frame.
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &nid).ok().context("Shell_NotifyIcon add")?;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
        Ok(())
    }

    /// Replace the icon image (e.g. after a skin change).
    pub fn set_icon(&mut self, img: &Image) -> Result<()> {
        let new = make_icon(img)?;
        // SAFETY: swap our owned icon, then release the old one.
        unsafe {
            let old = std::mem::replace(&mut self.icon, new);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
            let _ = DestroyIcon(old);
        }
        Ok(())
    }

    /// Events since the last call. Also re-adds the icon after an Explorer restart.
    pub fn take_events(&mut self) -> Vec<TrayEvent> {
        if RECREATE.with(|c| c.replace(false)) {
            let _ = self.add();
        }
        EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()))
    }

    /// Show `items` at the cursor; returns the chosen id. Blocks while the menu is open.
    pub fn show_menu(&self, items: &[MenuItem]) -> Option<u32> {
        // SAFETY: menus are created and destroyed here; the owner window is ours.
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            build_menu(menu, items);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // Required so the menu closes when clicking elsewhere.
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, self.hwnd, None);
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            let id = cmd.0 as u32;
            (id != 0 && id != u32::MAX).then_some(id)
        }
    }
}

/// SAFETY: `menu` must be a valid popup menu; submenus are owned by their parent.
unsafe fn build_menu(menu: HMENU, items: &[MenuItem]) {
    for it in items {
        // SAFETY: AppendMenuW with strings that outlive the call.
        unsafe {
            match it {
                MenuItem::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                MenuItem::Action { id, label, checked, enabled } => {
                    let mut f = MF_STRING;
                    if *checked {
                        f |= MF_CHECKED;
                    }
                    if !*enabled {
                        f |= MF_GRAYED;
                    }
                    let _ = AppendMenuW(menu, f, *id as usize, &HSTRING::from(label.as_str()));
                }
                MenuItem::Submenu { label, items } => {
                    if let Ok(sub) = CreatePopupMenu() {
                        build_menu(sub, items);
                        let _ = AppendMenuW(menu, MF_STRING | MF_POPUP, sub.0 as usize, &HSTRING::from(label.as_str()));
                    }
                }
            }
        }
    }
}

/// 32x32 icon from a premultiplied image (fitted, centered, straight alpha for Windows).
fn make_icon(img: &Image) -> Result<HICON> {
    const S: u32 = 32;
    let fitted = img.fit_square(S);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: S as i32,
            biHeight: -(S as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: creates a DIB we fill immediately and a mask bitmap; both are released after
    // CreateIconIndirect copies them.
    unsafe {
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).context("icon DIB")?;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u32, (S * S) as usize);
        for (d, &p) in dst.iter_mut().zip(&fitted.px) {
            *d = unpremultiply(p);
        }
        let mask = CreateBitmap(S as i32, S as i32, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.context("CreateIconIndirect")
    }
}

fn unpremultiply(p: u32) -> u32 {
    let a = p >> 24;
    if a == 0 || a == 255 {
        return p;
    }
    let f = |c: u32| ((c * 255 + a / 2) / a).min(255);
    (a << 24) | (f((p >> 16) & 0xff) << 16) | (f((p >> 8) & 0xff) << 8) | f(p & 0xff)
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: removing our icon and destroying resources we own.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            let _ = DestroyIcon(self.icon);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
