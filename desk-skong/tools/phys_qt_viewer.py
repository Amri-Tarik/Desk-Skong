# tools/phys_qt_viewer.py
from __future__ import annotations

import pathlib
import sys
import time
from typing import Tuple

import pymunk
from PySide6 import QtCore, QtGui, QtWidgets

ROOT = pathlib.Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

# phys modules
from phys import CFG, CharacterBody, PhysicsWorld
from phys.contacts import setup_handlers
from phys.kinematics import apply_movement
from phys.layers import CT_TERRAIN, CT_WALL
from phys.types import MoveInput

# ---------- helpers ----------


def color(r, g, b, a=255):
    return QtGui.QColor(r, g, b, a)


def pen(color: QtGui.QColor, width: int = 1):
    p = QtGui.QPen(color)
    p.setWidth(width)
    return p


def brush(color: QtGui.QColor):
    return QtGui.QBrush(color)


# ---------- main widget ----------


class PhysViewer(QtWidgets.QWidget):
    def __init__(self):
        super().__init__(None)
        self.setWindowTitle("Desk-Skong • Physics Viewer")
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_OpaquePaintEvent, False)
        self.setAutoFillBackground(False)

        # Size window to primary monitor's usable area
        screen = QtWidgets.QApplication.primaryScreen()
        if screen is None:
            self.resize(1280, 720)
        else:
            self.setGeometry(screen.availableGeometry())

        self.W = self.width()
        self.H = self.height()

        # World setup
        self.world = PhysicsWorld()
        self.space = self.world.space
        self.space.time = 0.0  # used by our contact timestamps

        # Terrain: flat floor along bottom, optional side walls as guard rails
        margin = 0.0
        floor_y = margin
        self.world.add_static_segment(
            (0.0, floor_y), (self.W, floor_y), collision_type=CT_TERRAIN, radius=6
        )
        # guard walls near edges (optional, comment out if not wanted)
        self.world.add_static_segment(
            (0.0, floor_y), (0.0, self.H), collision_type=CT_WALL, radius=6
        )
        self.world.add_static_segment(
            (self.W, floor_y),
            (self.W, self.H),
            collision_type=CT_WALL,
            radius=6,
        )

        # Player in center
        px, py = self.W * 0.5, self.H * 0.5
        self.player = CharacterBody(self.space, x=px, y=py, w=60, h=110)
        setup_handlers(self.space, self.player)

        # Input state
        self.move_left = False
        self.move_right = False
        self.want_jump = False

        # Timer / stepping
        self._last = time.perf_counter()
        self.timer = QtCore.QTimer(self)
        self.timer.timeout.connect(self._tick)
        self.timer.start(
            16
        )  # visual refresh ~60Hz; physics itself is fixed 30Hz inside world.step

        # Fonts
        self.hud_font = QtGui.QFont("Consolas", 10)

    # ----- coordinate mapping (world y-up -> Qt y-down) -----

    def to_screen(self, x: float, y: float) -> Tuple[int, int]:
        return int(round(x)), int(round(self.H - y))

    def _tick(self):
        # Build input
        xaxis = (-1.0 if self.move_left else 0.0) + (1.0 if self.move_right else 0.0)
        inp = MoveInput(x=xaxis, jump=self.want_jump)
        # consume jump press (buffer handled inside kinematics)
        self.want_jump = False

        # dt & sim time
        now = time.perf_counter()
        dt = max(
            0.0, min(0.1, now - self._last)
        )  # clamp to avoid huge steps on focus changes
        self._last = now
        self.space.time += dt

        # Apply movement and step physics
        apply_movement(self.player, inp, dt, now=self.space.time)
        self.world.step(dt)

        # Redraw
        self.update()

    # ----- keys -----

    def keyPressEvent(self, e: QtGui.QKeyEvent) -> None:
        k = e.key()
        if k == QtCore.Qt.Key_Escape:
            QtWidgets.QApplication.quit()
        elif k in (QtCore.Qt.Key_A, QtCore.Qt.Key_Left):
            self.move_left = True
        elif k in (QtCore.Qt.Key_D, QtCore.Qt.Key_Right):
            self.move_right = True
        elif k in (QtCore.Qt.Key_Space,):
            self.want_jump = True
        elif k == QtCore.Qt.Key_R:
            # reset player to center
            self.player.body.position = (self.W * 0.5, self.H * 0.5)
            self.player.body.velocity = (0.0, 0.0)
        else:
            super().keyPressEvent(e)

    def keyReleaseEvent(self, e: QtGui.QKeyEvent) -> None:
        k = e.key()
        if k in (QtCore.Qt.Key_A, QtCore.Qt.Key_Left):
            self.move_left = False
        elif k in (QtCore.Qt.Key_D, QtCore.Qt.Key_Right):
            self.move_right = False
        else:
            super().keyReleaseEvent(e)

    # ----- drawing -----

    def paintEvent(self, e: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHints(QtGui.QPainter.RenderHint.Antialiasing, True)

        # background (dark)
        p.fillRect(self.rect(), color(24, 26, 30))

        # draw terrain (segments)
        p.setPen(pen(color(90, 140, 220), 2))
        for s in self.space.shapes:
            if isinstance(s, pymunk.Segment) and s.body is self.space.static_body:
                a = self.to_screen(*s.a)
                b = self.to_screen(*s.b)
                p.drawLine(a[0], a[1], b[0], b[1])

        # draw player body poly
        grounded = self.player.contacts.grounded
        wall_l = self.player.contacts.wall_left
        wall_r = self.player.contacts.wall_right

        body_col = color(90, 230, 120, 220) if grounded else color(255, 220, 90, 220)
        p.setPen(pen(color(0, 0, 0, 200), 1))
        p.setBrush(brush(body_col))
        poly = self.player.shape
        # transform local verts to world, then to screen
        pts = []
        for v in poly.get_vertices():
            wx, wy = self.player.body.local_to_world(v)
            sx, sy = self.to_screen(wx, wy)
            pts.append(QtCore.QPoint(sx, sy))
        if pts:
            p.drawPolygon(QtGui.QPolygon(pts))

        # draw sensors (lines)
        p.setPen(pen(color(255, 120, 120) if wall_l else color(200, 80, 80), 2))
        a = self.player.side_l.a
        b = self.player.side_l.b
        A = self.player.body.local_to_world(a)
        B = self.player.body.local_to_world(b)
        As = self.to_screen(*A)
        Bs = self.to_screen(*B)
        p.drawLine(As[0], As[1], Bs[0], Bs[1])

        p.setPen(pen(color(120, 180, 255) if wall_r else color(80, 120, 200), 2))
        a = self.player.side_r.a
        b = self.player.side_r.b
        A = self.player.body.local_to_world(a)
        B = self.player.body.local_to_world(b)
        As = self.to_screen(*A)
        Bs = self.to_screen(*B)
        p.drawLine(As[0], As[1], Bs[0], Bs[1])

        p.setPen(pen(color(120, 255, 120) if grounded else color(80, 160, 80), 3))
        a = self.player.foot.a
        b = self.player.foot.b
        A = self.player.body.local_to_world(a)
        B = self.player.body.local_to_world(b)
        As = self.to_screen(*A)
        Bs = self.to_screen(*B)
        p.drawLine(As[0], As[1], Bs[0], Bs[1])

        # HUD
        p.setFont(self.hud_font)
        p.setPen(color(230, 230, 235))
        text = (
            f"pos=({self.player.pos.x:.1f},{self.player.pos.y:.1f})  "
            f"vel=({self.player.vel.x:.1f},{self.player.vel.y:.1f})  "
            f"grounded={grounded}  wallL={wall_l} wallR={wall_r}"
        )
        p.drawText(12, 24, text)

        p.end()


# ---------- main ----------


def main():
    app = QtWidgets.QApplication(sys.argv)
    w = PhysViewer()
    w.show()
    sys.exit(app.exec())
