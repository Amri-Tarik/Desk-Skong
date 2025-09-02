# phys/segments.py
from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Tuple

import pymunk

# Use the same collision type constants you already use elsewhere
from .layers import CT_TERRAIN, CT_WALL


@dataclass
class OneWayPlatform:
    """
    CURRENTLY SOLID (terrain-like) for maximum stability.
    This mirrors PhysicsWorld.add_static_segment(...) and will collide with the
    player's main hitbox exactly like your floor does.
    """

    shape: pymunk.Segment

    @classmethod
    def add(
        cls,
        space: pymunk.Space,
        a: Tuple[float, float],
        b: Tuple[float, float],
        *,
        radius: float = 4.0,
        friction: float = 0.9,
        elasticity: float = 0.0,
    ) -> "OneWayPlatform":
        body = space.static_body
        seg = pymunk.Segment(body, a, b, radius)
        seg.collision_type = CT_TERRAIN  # solid, identical behavior to your floor
        seg.friction = friction
        seg.elasticity = elasticity
        space.add(seg)
        return cls(seg)


@dataclass
class VerticalWall:
    """
    Solid vertical wall. Same construction as terrain, but uses CT_WALL so you can
    toggle special behavior later if you want (right now it's just solid).
    """

    shape: pymunk.Segment

    @classmethod
    def add(
        cls,
        space: pymunk.Space,
        a: Tuple[float, float],
        b: Tuple[float, float],
        *,
        radius: float = 6.0,
        friction: float = 0.9,
        elasticity: float = 0.0,
    ) -> "VerticalWall":
        body = space.static_body
        seg = pymunk.Segment(body, a, b, radius)
        seg.collision_type = CT_WALL
        seg.friction = friction
        seg.elasticity = elasticity
        space.add(seg)
        return cls(seg)


# --- Compatibility stubs (no-op for now) ------------------------------------


def install_handlers(space: pymunk.Space, get_drop_input=None) -> None:
    """
    No handlers needed for solid segments. Kept for API compatibility with the viewer.
    """
    return None


def request_phase_down(space: pymunk.Space, duration: float = 0.20) -> None:
    """
    No-op placeholder. When/if we reintroduce one-way behavior, we’ll implement this.
    """
    return None


def tick_phase(player_phys, dt: float) -> None:
    """
    No-op placeholder to keep the existing call sites unchanged.
    """
    return None


from typing import List, Tuple


def gather_debug_segments(
    space: pymunk.Space,
) -> Tuple[List[pymunk.Segment], List[pymunk.Segment]]:
    """Return (platform_like, wall_like) segment shapes currently in the space."""
    plats: List[pymunk.Segment] = []
    walls: List[pymunk.Segment] = []
    for sh in space.shapes:
        # only draw Chipmunk segments (ignore polys/circles)
        if isinstance(sh, pymunk.Segment):
            if sh.collision_type == CT_WALL:
                walls.append(sh)
            elif sh.collision_type == CT_TERRAIN:
                # our simplified one-way currently uses CT_TERRAIN too (same as floor)
                plats.append(sh)
    return plats, walls
