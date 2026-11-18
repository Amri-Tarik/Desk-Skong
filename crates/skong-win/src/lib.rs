//! Win32 platform layer: overlay window, desktop observation, input, timing.

pub mod desktop;
pub mod monitor;
pub mod runloop;
pub mod sprite_window;

pub use windows::Win32::UI::Input::KeyboardAndMouse as vk;
