//! Platformer character controller (AABB vs terrain segments).
//! TODO(phys_controller): implemented on the controller branch. Contract below is fixed.

/// Movement speed chosen by the player / AI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gait {
    /// Slow stroll ("Walk" clip).
    Walk,
    /// Normal ("Run" clip).
    #[default]
    Run,
    /// Fast ("Sprint" clip).
    Sprint,
}

/// One tick of intent. Identical for keyboard control and the AI.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveInput {
    /// -1..1 horizontal intent.
    pub x: f32,
    pub gait: Gait,
    pub up: bool,
    pub down: bool,
    /// Jump button currently held (variable jump height).
    pub jump_held: bool,
    /// Jump pressed this tick (edge).
    pub jump_pressed: bool,
    pub dash_pressed: bool,
}
