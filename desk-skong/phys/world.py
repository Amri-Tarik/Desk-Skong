import pymunk

from .config import CFG


class PhysicsWorld:
    """Owns the Pymunk space and runs a fixed-step integrator."""

    def __init__(self):
        self.space = pymunk.Space()
        self.space.gravity = (0.0, CFG.GRAVITY_Y)

        # stability: more iterations, tighter slop
        self.space.iterations = CFG.SOLVER_ITERATIONS
        self.space.collision_slop = CFG.COLLISION_SLOP

        self._accum = 0.0
        self.dt = 1.0 / CFG.HZ

    def step(self, dt: float):
        """Accumulate real time and advance the simulation in fixed steps."""
        self._accum += dt
        # Run N substeps if you ever want; default 1 for 30Hz
        while self._accum >= self.dt:
            sub_dt = self.dt / max(1, CFG.SUBSTEPS)
            for _ in range(CFG.SUBSTEPS):
                self.space.step(sub_dt)
            self._accum -= self.dt

    # --- quick helpers for adding terrain ---
    def add_static_segment(
        self,
        a,
        b,
        friction=None,
        restitution=None,
        collision_type=None,
        radius: float = None,
    ):
        r = CFG.FLOOR_RADIUS if radius is None else radius
        s = pymunk.Segment(self.space.static_body, a, b, r)
        s.friction = CFG.WORLD_FRICTION if friction is None else friction
        s.elasticity = CFG.RESTITUTION if restitution is None else restitution
        if collision_type is not None:
            s.collision_type = collision_type
        self.space.add(s)
        return s

    def add_static_box(self, x, y, w, h, **kw):
        """Axis-aligned rectangle centered at (x,y), for quick blocking boxes."""
        l, b = x - w / 2, y - h / 2
        r, t = x + w / 2, y + h / 2
        vs = [(l, b), (r, b), (r, t), (l, t)]
        shape = pymunk.Poly(self.space.static_body, vs)
        shape.friction = CFG.WORLD_FRICTION
        shape.elasticity = CFG.RESTITUTION
        if "collision_type" in kw and kw["collision_type"] is not None:
            shape.collision_type = kw["collision_type"]
        self.space.add(shape)
        return shape
