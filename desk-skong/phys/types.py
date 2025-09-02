from dataclasses import dataclass


@dataclass
class MoveInput:
    x: float = 0.0  # -1..1
    jump: bool = False


@dataclass
class ContactState:
    grounded: bool = False
    wall_left: bool = False
    wall_right: bool = False
    ceiling: bool = False
    last_ground_time: float = -999.0  # for coyote
    last_jump_press: float = -999.0  # for jump buffering


@dataclass
class MoveState:
    wall_hold_t: float = 0.0  # remaining time of horizontal “hold”
    input_lock_t: float = 0.0  # remaining time to ignore x input
    hold_vx: float = 0.0  # preserved horizontal velocity during the hold
