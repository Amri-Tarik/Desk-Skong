//! Win32 platform layer: overlay window, desktop observation, input, timing.

pub mod capture;
pub mod desktop;
pub mod monitor;
pub mod runloop;
pub mod sprite_window;
pub mod tray;

pub use windows::Win32::UI::Input::KeyboardAndMouse as vk;
