from __future__ import annotations

import os
import time
from dataclasses import dataclass
from typing import Callable, List, Optional, Tuple

import pymunk
from pymunk import Arbiter, Space

from .layers import CT_PLAYER, CT_TERRAIN, CT_WALL

# ---------------------------------------------------------------------
# Debug logger (toggle with SKONG_SEGDBG=1)
# ---------------------------------------------------------------------
_SEGDBG = os.environ.get("SKONG_SEGDBG", "0") == "1"


def _log(msg: str) -> None:
    if _SEGDBG:
        print(f"[segments] {msg}")


# ---------------------------------------------------------------------
# Phase-through state (press & hold “down” to phase)
# ---------------------------------------------------------------------
@dataclass
class _PhaseState:
    until: float = 0.0
    active: bool = False

    def start(self, now: float, duration: float) -> None:
        self.until = now + duration
        self.active = True
        _log(f"phase START: until={self.until:.3f}")

    def tick(self, now: float) -> None:
        if self.active and now >= self.until:
            self.active = False
            _log("phase END")

    def extend_if_holding(self, now: float, duration: float) -> None:
        # Keep extending while key is held (no spammy logs)
        if self.active:
            self.until = max(self.until, now + duration)
        else:
            self.start(now, duration)


# Global-ish state attached to the space so multiple spaces work.
_PHASE_KEY = "_skong_phase_state"


def _get_phase(space: Space) -> _PhaseState:
    st = getattr(space, _PHASE_KEY, None)
    if st is None:
        st = _PhaseState()
        setattr(space, _PHASE_KEY, st)
    return st


def request_phase_down(
    space: Space,
    *,
    now: Optional[float] = None,
    hold: bool = False,
    duration: float = 0.15,
) -> None:
    """
    - Call with hold=True each tick the user holds “Down”.
    - Call with hold=False on release (optional).
    """
    now = time.perf_counter() if now is None else now
    st = _get_phase(space)
    if hold:
        st.extend_if_holding(now, duration)
    else:
        # On release, don't immediately kill; let the timer lapse naturally.
        pass


def tick_phase(space: Space, now: Optional[float] = None) -> None:
    now = time.perf_counter() if now is None else now
    _get_phase(space).tick(now)


# ---------------------------------------------------------------------
# Segment builders
# ---------------------------------------------------------------------
@dataclass
class OneWayPlatform:
    shape: pymunk.Segment

    @classmethod
    def add(
        cls,
        space: Space,
        a: Tuple[float, float],
        b: Tuple[float, float],
        *,
        radius: float = 4.0,
        friction: float = 0.9,
        elasticity: float = 0.0,
    ) -> "OneWayPlatform":
        """
        Horizontal platform (y-constant). Autopass from below, solid from above.
        Can be phased through from above while the phase timer is active.
        """
        body = space.static_body
        seg = pymunk.Segment(body, a, b, radius)
        seg.collision_type = CT_TERRAIN
        seg.friction = friction
        seg.elasticity = elasticity

        # mark as one-way
        setattr(seg, "oneway", True)
        # cache nominal Y level (midpoint) for quick comparisons
        plat_y = (a[1] + b[1]) * 0.5
        setattr(seg, "plat_y", float(plat_y))

        space.add(seg)
        return cls(seg)


@dataclass
class VerticalWall:
    shape: pymunk.Segment

    @classmethod
    def add(
        cls,
        space: Space,
        a: Tuple[float, float],
        b: Tuple[float, float],
        *,
        radius: float = 6.0,
        friction: float = 0.9,
        elasticity: float = 0.0,
    ) -> "VerticalWall":
        """
        Solid vertical wall. Can be phased through ONLY while phase timer active.
        """
        body = space.static_body
        seg = pymunk.Segment(body, a, b, radius)
        seg.collision_type = CT_WALL
        seg.friction = friction
        seg.elasticity = elasticity
        setattr(seg, "is_wall", True)
        space.add(seg)
        return cls(seg)


# ---------------------------------------------------------------------
# Handlers
# ---------------------------------------------------------------------


def install_handlers(
    space: Space, get_down_held: Optional[Callable[[], bool]] = None
) -> None:
    """
    Register pre_solve handlers for:
      - (dynamic) vs CT_TERRAIN (with oneway flag)
      - (dynamic) vs CT_WALL
    """

    # Horizontal one-way platforms + regular terrain (we only modify if 'oneway' is set)
    def pre_solve_terrain(arb: Arbiter, sp: Space, data):
        st = _get_phase(sp)
        sA, sB = (
            arb.shapes
        )  # Ordered as in handler: dynamic first if we ensure it; see below.
        # Identify static vs dynamic
        if sA.body is sp.static_body:
            static_s, dyn_s = sA, sB
        elif sB.body is sp.static_body:
            static_s, dyn_s = sB, sA
        else:
            # Two dynamics; not our case.
            return

        is_oneway = bool(getattr(static_s, "oneway", False))
        if not is_oneway:
            # Normal floor, keep solid
            return

        # Player vertical velocity
        vy = float(dyn_s.body.velocity.y)

        # Player feet from shape's AABB bottom (robust, uses actual hitbox)
        dyn_s.cache_bb()
        feet_y = float(dyn_s.bb.bottom)

        # Platform nominal Y (cached) + small grace to account for radius
        plat_y = float(getattr(static_s, "plat_y", (static_s.a.y + static_s.b.y) * 0.5))
        grace = float(static_s.radius)

        on_top = feet_y >= (plat_y + grace * 0.25)
        rising = vy > 1e-3

        if st.active and on_top:
            arb.process_collision = False
            return False

        # Autopass from below: if rising and feet are below (or close to) platform height
        if rising and feet_y <= (plat_y + grace * 0.6):
            arb.process_collision = False
            return False

        # Otherwise, let it collide (solid top)
        return True

    space.on_collision(
        CT_PLAYER,
        CT_TERRAIN,  # 'None' means any type on the other side
        pre_solve=pre_solve_terrain,
    )

    # Vertical walls: only phase when timer active
    def pre_solve_wall(arb: Arbiter, sp: Space, data):
        st = _get_phase(sp)

        sA, sB = arb.shapes
        # Identify static vs dynamic to avoid phasing two dynamics accidentally
        if sA.body is sp.static_body:
            static_s, dyn_s = sA, sB
        elif sB.body is sp.static_body:
            static_s, dyn_s = sB, sA
        else:
            return

        if not getattr(static_s, "is_wall", False):
            return

        if st.active:
            _log("pre WALL: PHASE -> ignore")
            arb.process_collision = False
        else:
            # Solid wall
            pass

    space.on_collision(CT_PLAYER, CT_WALL, pre_solve=pre_solve_wall)


# ---------------------------------------------------------------------
# Debug helpers for rendering
# ---------------------------------------------------------------------

from typing import Tuple as _T


def gather_debug_segments(
    space: Space,
) -> _T[List[pymunk.Segment], List[pymunk.Segment]]:
    """
    Return (platform_like, wall_like) segments currently in the space.
    """
    plats: List[pymunk.Segment] = []
    walls: List[pymunk.Segment] = []
    for sh in space.shapes:
        if isinstance(sh, pymunk.Segment):
            if getattr(sh, "is_wall", False) or sh.collision_type == CT_WALL:
                walls.append(sh)
            elif getattr(sh, "oneway", False):
                plats.append(sh)
    return plats, walls
