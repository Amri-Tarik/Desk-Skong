# phys/contacts.py
from __future__ import annotations

import pymunk

from .layers import *
from .types import ContactState

# Collision type constants (example)


def setup_handlers(space: pymunk.Space, player) -> None:
    """
    Configure sensor ↔ terrain contacts for Pymunk 7.1+.

    Expectation:
      - Player has attributes (or a nested state obj) we can set:
          grounded: bool
          touching_left: bool
          touching_right: bool
          head_hit: bool (momentary; you can clear it each tick)
    """

    cs: ContactState = player.contacts

    def set_grounded(val: bool):
        cs.grounded = val
        if val:
            cs.last_ground_time = getattr(space, "time", 0.0)

    def set_wall_left(val: bool):
        cs.wall_left = val

    def set_wall_right(val: bool):
        cs.wall_right = val

    # Foot ↔ terrain: grounded flag
    space.on_collision(
        CT_FOOT_SENSOR,
        CT_TERRAIN,
        begin=lambda arb, sp, data: set_grounded(True),
        separate=lambda arb, sp, data: set_grounded(False),
    )
    space.on_collision(
        CT_LEFT_SENSOR,
        CT_WALL,
        begin=lambda arb, sp, data: set_wall_left(True),
        separate=lambda arb, sp, data: set_wall_left(False),
    )
    space.on_collision(
        CT_RIGHT_SENSOR,
        CT_WALL,
        begin=lambda arb, sp, data: set_wall_right(True),
        separate=lambda arb, sp, data: set_wall_right(False),
    )
