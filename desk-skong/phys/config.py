from dataclasses import dataclass


@dataclass(frozen=True)
class Config:
    # Simulation
    HZ: float = 120.0
    SUBSTEPS: int = 2
    GRAVITY_Y: float = -2400.0  # y-up in physics (negative = “down”)

    # Solver tuning
    SOLVER_ITERATIONS: int = 50  # ← default is ~10; 20–40 is typical for platformers
    COLLISION_SLOP: float = 0.0  # ← penetration allowance (in your world units/pixels)

    # Shapes
    FLOOR_RADIUS: float = 3.0  # ← thickness for static segments (ground/walls)
    BODY_CORNER_RADIUS: float = 1.5  # ← soften player collider corners

    # Character tuning (pixels/second or pixels/second^2)
    RUN_SPEED: float = 300.0
    AIR_CONTROL: float = 0.5
    JUMP_IMPULSE: float = 1000.0
    COYOTE_TIME: float = 0.08  # seconds after leaving ground still allowed to jump
    JUMP_BUFFER: float = 0.10  # seconds buffering the jump press
    MAX_FALL_SPEED: float = -800.0
    WALL_SLIDE_SPEED: float = -150.0
    WALL_JUMP_X: float = 400.0
    WALL_JUMP_Y: float = 1000.0

    # Shapes
    PLAYER_DENSITY: float = 0.015
    PLAYER_FRICTION: float = 0.8
    WORLD_FRICTION: float = 0.9
    RESTITUTION: float = 0.0
    SENSOR_THICKNESS: float = 10.0
    FOOT_SENSOR_WIDTH_RATIO: float = 0.6
    SIDE_SENSOR_HEIGHT_RATIO: float = 0.8

    # wall slide config
    WALL_JUMP_HOLD_TIME: float = 0.14  # seconds to preserve horizontal push
    WALL_JUMP_INPUT_LOCK: float = 0.12  # seconds to ignore x input after wall jump
    WALL_JUMP_OVERRIDE_INPUT: bool = True  # if True, force vx during hold (no blending)
    WALL_JUMP_BLEND_AIR_CONTROL: float = (
        0.15  # if not overriding, blend strength during hold
    )


CFG = Config()
