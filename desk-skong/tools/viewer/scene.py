# tools/viewer/scene.py
from __future__ import annotations

import os
import time
from dataclasses import dataclass
from typing import Tuple

import pymunk
from anim.sprite_anim import AnimationConfig, Skin, SpritePlayer
from phys import CharacterBody, PhysicsWorld
from phys.contacts import setup_handlers
from phys.kinematics import apply_movement
from phys.layers import CT_PLAYER, CT_TERRAIN
from phys.segments import (
    OneWayPlatform,
    VerticalWall,
    install_handlers,
    request_phase_down,
    tick_phase,
)
from phys.types import MoveInput
from tools.edge_phys import EdgePhysConfig, EdgePhysicsBridge
from vision.edge_detect import EdgeConfig, EdgeDetector


def player_feet_y(player_phys: CharacterBody) -> float:
    """Bottom of the player's main hitbox (non-sensor, CT_PLAYER)."""
    bottoms = []
    for sh in player_phys.body.shapes:
        if sh.sensor:
            continue
        if getattr(sh, "collision_type", None) != CT_PLAYER:
            continue
        sh.cache_bb()
        bottoms.append(sh.bb.bottom)
    if bottoms:
        return min(bottoms)
    return player_phys.pos.y - (player_phys.h * 0.5)


@dataclass
class SceneConfig:
    width: int
    height: int
    scale: float = 1.5  # default visual scale


class Scene:
    def __init__(self, cfg: SceneConfig, skin: Skin, anim_cfg: AnimationConfig) -> None:
        self.cfg = cfg
        self.skin = skin
        self.anim_cfg = anim_cfg

        # ---- physics world
        self.world = PhysicsWorld()
        self.space: pymunk.Space = self.world.space
        self.space.time = 0.0
        self.space.iterations = 30

        W, H = cfg.width, cfg.height

        # world floor (bottom of screen)
        self.world.add_static_segment(
            (0.0, 0.0), (float(W), 0.0), collision_type=CT_TERRAIN, radius=4.0
        )

        # Side rails (solid walls).
        VerticalWall.add(self.space, (0.0, 0.0), (0.0, float(H)), radius=4.0)
        VerticalWall.add(
            self.space,
            (float(W), 0.0),
            (float(W), float(H)),
            radius=4.0,
        )

        # example objects to play with
        # OneWayPlatform.add(
        #     self.space,
        #     (float(W) * 0.3, float(H) * 0.35),
        #     (float(W) * 0.7, float(H) * 0.35),
        #     radius=2.0,
        # )

        # VerticalWall.add(
        #     self.space,
        #     (float(W) * 0.55, 0.0),
        #     (float(W) * 0.55, float(H) * 0.65),
        #     radius=2.0,
        # )

        # Player hitbox (requested size & spawn)
        self.player_phys = CharacterBody(self.space, x=W * 0.5, y=H * 0.8, w=40, h=80)
        setup_handlers(self.space, self.player_phys)

        # ---- animation
        self.player = SpritePlayer(self.skin, self.anim_cfg, start_group="idle")

        # input state
        self.in_left = False
        self.in_right = False
        self.in_down = False
        self.in_jump_press = False
        self.in_challenge = False

        # timing
        self._last_t = time.perf_counter()

        install_handlers(self.space)

    # ---------------- API the widget calls ----------------

    def set_input(
        self,
        *,
        left: bool | None = None,
        right: bool | None = None,
        down: bool | None = None,
        jump_press: bool | None = None,
        challenge: bool | None = None,
    ) -> None:
        if left is not None:
            self.in_left = left
        if right is not None:
            self.in_right = right
        if down is not None:
            self.in_down = down
        if jump_press:
            self.in_jump_press = True
        if challenge:
            self.in_challenge = True

    def step(self, dt_visual: float | None = None) -> None:
        """Advance physics (fixed substeps inside world.step), then animation."""
        now = time.perf_counter()
        dt = (now - self._last_t) if dt_visual is None else float(dt_visual)
        dt = max(0.0, min(1.0 / 15.0, dt))  # clamp long frames
        self._last_t = now
        self.space.time += dt

        # Build input for physics
        xaxis = (-1.0 if self.in_left else 0.0) + (1.0 if self.in_right else 0.0)
        self.player.set_facing_from_input(xaxis)
        move = MoveInput(x=xaxis, jump=self.in_jump_press)
        self.in_jump_press = False  # consume

        request_phase_down(
            self.space, now=self.space.time, hold=self.in_down, duration=0.15
        )

        tick_phase(self.space, now=self.space.time)
        apply_movement(self.player_phys, move, dt, now=self.space.time)
        self.world.step(dt)

        self._sync_anim_to_physics()
        self.player.update(dt)

        self.in_challenge = False  # consume

    def _sync_anim_to_physics(self) -> None:
        vx, vy = self.player_phys.vel
        c = self.player_phys.contacts
        grounded = c.grounded
        wall_l = c.wall_left
        wall_r = c.wall_right

        run_threshold = 30.0

        if not grounded:
            if wall_l or wall_r:
                target = (
                    "wall_slide" if self.player.config.get("wall_slide") else "idle"
                )
            elif vy > 40.0:
                target = "jump" if self.player.config.get("jump") else "idle"
            elif vy < -10.0:
                target = "fall" if self.player.config.get("fall") else "idle"
            else:
                target = "idle"
        else:
            if abs(vx) > run_threshold:
                target = "run"
            elif self.in_challenge:
                target = "knight_challenge"
            else:
                target = "idle"

        try:
            self.player.goto(target)
        except Exception:
            pass  # unknown group

    # ---- data for drawing ----

    def draw_packet(self, scale: float | None = None):
        return self.player.draw_packet(scale=scale or self.cfg.scale)

    def feet_y_world(self) -> float:
        return player_feet_y(self.player_phys)

    @property
    def player_pos(self) -> Tuple[float, float]:
        return (self.player_phys.pos.x, self.player_phys.pos.y)
