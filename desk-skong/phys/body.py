import pymunk

from .config import CFG
from .layers import *
from .types import ContactState, MoveState


class CharacterBody:
    """
    Dynamic body + main box collider + 3 sensors:
      - foot sensor (ground check)
      - left/right side sensors (wall check)
    All sizes in pixels; physics runs y-up.
    """

    def __init__(self, space: pymunk.Space, x: float, y: float, w: float, h: float):
        self.space = space
        self.w = w
        self.h = h

        mass = w * h * CFG.PLAYER_DENSITY
        moment = float("inf")
        self.body = pymunk.Body(mass, moment)
        self.body.position = (x, y)
        self.body.angle = 0.0  # ensure upright
        self.body.angular_velocity = 0  # no rotation

        # Main collider (box)
        hw, hh = w / 2, h / 2
        verts = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
        self.shape = pymunk.Poly(self.body, verts, radius=CFG.BODY_CORNER_RADIUS)
        self.shape.friction = CFG.PLAYER_FRICTION
        self.shape.elasticity = CFG.RESTITUTION
        self.shape.collision_type = CT_PLAYER

        # Sensors
        foot_half = (w * CFG.FOOT_SENSOR_WIDTH_RATIO) * 0.5
        self.foot = pymunk.Segment(
            self.body, (-foot_half, -hh - 1), (foot_half, -hh - 1), CFG.SENSOR_THICKNESS
        )
        self.foot.sensor = True
        self.foot.collision_type = CT_FOOT_SENSOR

        sh = h * CFG.SIDE_SENSOR_HEIGHT_RATIO * 0.5
        self.side_l = pymunk.Segment(
            self.body, (-hw - 2, -sh), (-hw - 2, +sh), CFG.SENSOR_THICKNESS
        )
        self.side_l.sensor = True
        self.side_l.collision_type = CT_LEFT_SENSOR

        self.side_r = pymunk.Segment(
            self.body, (+hw + 2, -sh), (+hw + 2, +sh), CFG.SENSOR_THICKNESS
        )
        self.side_r.sensor = True
        self.side_r.collision_type = CT_RIGHT_SENSOR

        space.add(self.body, self.shape, self.foot, self.side_l, self.side_r)

        self.contacts = ContactState()
        self.move = MoveState()

    @property
    def pos(self):
        return self.body.position

    @property
    def vel(self):
        return self.body.velocity

    def set_vel(self, vx: float, vy: float):
        self.body.velocity = (vx, vy)
