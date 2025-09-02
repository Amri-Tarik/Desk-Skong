from .config import CFG
from .types import MoveInput


def apply_movement(char, inp: MoveInput, dt: float, now: float):
    """
    Platformer kinematics with wall-jump horizontal hold / input lock.
    """
    vx, vy = char.vel
    c = char.contacts
    m = char.move

    grounded = c.grounded
    wall_l = c.wall_left
    wall_r = c.wall_right

    # Buffer jump press timestamp
    if inp.jump:
        c.last_jump_press = now

    # Horizontal input (optionally locked after wall jump)
    x_inp = 0.0 if m.input_lock_t > 0.0 else inp.x

    # Target horizontal speed from input
    run_target = x_inp * CFG.RUN_SPEED
    accel_factor = 1.0 if grounded else CFG.AIR_CONTROL

    # Base horizontal blend toward target (will be overridden/augmented by wall-jump hold below)
    vx += (run_target - vx) * accel_factor

    # Jump logic (ground or coyote)
    can_jump_time = (now - c.last_ground_time) <= CFG.COYOTE_TIME
    buffered = (now - c.last_jump_press) <= CFG.JUMP_BUFFER
    if buffered and (grounded or can_jump_time):
        vy = CFG.JUMP_IMPULSE
        grounded = False  # will refresh next tick

    # Wall slide + wall jump
    touching_wall = (wall_l or wall_r) and not grounded and x_inp != 0.0
    if touching_wall and vy < CFG.WALL_SLIDE_SPEED:
        vy = CFG.WALL_SLIDE_SPEED

    # If buffered jump while on wall (not grounded) → wall jump
    if buffered and (wall_l or wall_r) and not grounded:
        push = CFG.WALL_JUMP_X if wall_l else -CFG.WALL_JUMP_X
        vx = push
        vy = CFG.WALL_JUMP_Y
        # Start hold and input lock windows
        m.hold_vx = push
        m.wall_hold_t = CFG.WALL_JUMP_HOLD_TIME
        m.input_lock_t = CFG.WALL_JUMP_INPUT_LOCK

    # Apply wall-jump hold behavior
    if m.wall_hold_t > 0.0:
        m.wall_hold_t -= dt
        if CFG.WALL_JUMP_OVERRIDE_INPUT:
            # Hard override: force vx to held value for the duration
            vx = m.hold_vx
        else:
            # Soft blend: bias vx toward held value, still allow some control
            blend = CFG.WALL_JUMP_BLEND_AIR_CONTROL
            vx += (m.hold_vx - vx) * blend

    # Count down input lock
    if m.input_lock_t > 0.0:
        m.input_lock_t -= dt
        if m.input_lock_t < 0.0:
            m.input_lock_t = 0.0

    # Clamp fall speed
    if vy < CFG.MAX_FALL_SPEED:
        vy = CFG.MAX_FALL_SPEED

    # Optional: on landing, you can cancel the remaining hold/lock if desired
    # if grounded and (m.wall_hold_t > 0.0 or m.input_lock_t > 0.0):
    #     m.wall_hold_t = 0.0
    #     m.input_lock_t = 0.0

    char.set_vel(vx, vy)
