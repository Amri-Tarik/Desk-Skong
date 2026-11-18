//! Manual control (debug): reads the global keyboard state.
//!
//! ←/→ move · ↑ jump (hold for height) · ↓ down (↓+↑ drops through) · LShift dash ·
//! LCtrl hold = superdash charge · Z walk · X sprint
use skong_core::controller::{Gait, MoveInput};
use skong_win::runloop::key_down;
use skong_win::vk;

#[derive(Default)]
pub struct Keyboard {
    jump_was: bool,
    dash_was: bool,
}

impl Keyboard {
    pub fn read(&mut self) -> MoveInput {
        let jump = key_down(vk::VK_UP);
        let dash = key_down(vk::VK_LSHIFT);
        let x = (key_down(vk::VK_RIGHT) as i32 - key_down(vk::VK_LEFT) as i32) as f32;
        let gait = if key_down(vk::VK_Z) {
            Gait::Walk
        } else if key_down(vk::VK_X) {
            Gait::Sprint
        } else {
            Gait::Run
        };
        let input = MoveInput {
            x,
            gait,
            up: false,
            down: key_down(vk::VK_DOWN),
            jump_held: jump,
            jump_pressed: jump && !self.jump_was,
            dash_pressed: dash && !self.dash_was,
            // Ctrl+Alt are held while pressing hotkeys; don't treat that as a charge.
            superdash_held: key_down(vk::VK_LCONTROL) && !key_down(vk::VK_MENU),
        };
        self.jump_was = jump;
        self.dash_was = dash;
        input
    }
}
