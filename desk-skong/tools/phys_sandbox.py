import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from phys import CFG, CharacterBody, PhysicsWorld
from phys.contacts import CT_TERRAIN, setup_handlers
from phys.kinematics import apply_movement
from phys.types import MoveInput


def main():
    world = PhysicsWorld()
    space = world.space
    space.time = 0.0  # we use this to stamp “now”

    # Terrain: floor & two walls
    world.add_static_segment(
        (0, 100), (2000, 100), collision_type=CT_TERRAIN
    )  # floor y=100
    world.add_static_segment(
        (50, 100), (50, 800), collision_type=CT_TERRAIN
    )  # left wall
    world.add_static_segment((1550, 100), (1550, 800), collision_type=CT_TERRAIN)

    # Player
    player = CharacterBody(space, x=200, y=300, w=60, h=110)
    setup_handlers(space, player)

    last = time.perf_counter()
    t = 0.0

    print(
        "Use A/D to move, SPACE to jump (in this demo, we simulate inputs below). Ctrl+C to stop."
    )
    # crude input demo: walk right, jump every few seconds
    phase = 0.0

    try:
        while True:
            now = time.perf_counter()
            dt = now - last
            last = now
            t += dt
            space.time = t

            phase += dt
            inp = MoveInput(
                x=1.0 if (phase % 6.0) < 5.0 else 0.0,
                jump=(abs((phase % 3.0) - 0.05) < 0.02),
            )

            apply_movement(player, inp, dt, now=t)
            world.step(dt)

            if int(t * 10) % 10 == 0:  # print ~10 Hz
                print(
                    f"pos={player.pos} vel={player.vel} grounded={player.contacts.grounded} "
                    f"wallL={player.contacts.wall_left} wallR={player.contacts.wall_right}",
                    end="\r",
                )

    except KeyboardInterrupt:
        print("\nbye")
