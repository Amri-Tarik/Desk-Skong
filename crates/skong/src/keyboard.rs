//! Manual control (debug): reads the global keyboard state, or the keys held in a focused
//! control panel (which then don't leak to other apps).
//!
//! ←/→ move · ↑ jump (hold for height) · ↓ down (↓+↑ drops through) · LShift dash ·
//! LCtrl hold = superdash charge · Z walk · X sprint
use skong_core::controller::{Gait, MoveInput};
use skong_ipc::ManualKeys;
use skong_win::runloop::key_down;
use skong_win::vk;

#[derive(Default)]
pub struct Keyboard {
    jump_was: bool,
    dash_was: bool,
}

impl Keyboard {
    /// This tick's input from `panel` keys when a panel is driving, else the global keyboard.
    pub fn read(&mut self, panel: Option<ManualKeys>) -> MoveInput {
        self.input(panel.unwrap_or_else(global_keys))
    }

    fn input(&mut self, k: ManualKeys) -> MoveInput {
        let gait = if k.walk {
            Gait::Walk
        } else if k.sprint {
            Gait::Sprint
        } else {
            Gait::Run
        };
        let input = MoveInput {
            x: (k.right as i32 - k.left as i32) as f32,
            gait,
            up: false,
            down: k.down,
            jump_held: k.jump,
            jump_pressed: k.jump && !self.jump_was,
            dash_pressed: k.dash && !self.dash_was,
            superdash_held: k.superdash,
        };
        self.jump_was = k.jump;
        self.dash_was = k.dash;
        input
    }
}

fn global_keys() -> ManualKeys {
    ManualKeys {
        left: key_down(vk::VK_LEFT),
        right: key_down(vk::VK_RIGHT),
        up: key_down(vk::VK_UP),
        down: key_down(vk::VK_DOWN),
        jump: key_down(vk::VK_UP),
        dash: key_down(vk::VK_LSHIFT),
        // Ctrl+Alt are held while pressing hotkeys; don't treat that as a charge.
        superdash: key_down(vk::VK_LCONTROL) && !key_down(vk::VK_MENU),
        walk: key_down(vk::VK_Z),
        sprint: key_down(vk::VK_X),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_are_edges_of_held_keys() {
        let mut kb = Keyboard::default();
        let held = ManualKeys { right: true, jump: true, dash: true, walk: true, ..Default::default() };
        let a = kb.input(held);
        assert!(a.jump_pressed && a.dash_pressed && a.jump_held && a.x == 1.0 && a.gait == Gait::Walk);
        let b = kb.input(held);
        assert!(!b.jump_pressed && !b.dash_pressed && b.jump_held);
        let c = kb.input(ManualKeys::default());
        assert!(c.x == 0.0 && !c.jump_held && c.gait == Gait::Run);
    }
}
