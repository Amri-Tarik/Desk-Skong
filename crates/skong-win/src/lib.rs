//! Win32 platform layer: overlay window, desktop observation, input, timing.

pub mod capture;
pub mod debug_overlay;
pub mod desktop;
pub mod input;
pub mod monitor;
pub mod process;
pub mod runloop;
pub mod sprite_window;
pub mod tray;

pub use windows::Win32::UI::Input::KeyboardAndMouse as vk;
