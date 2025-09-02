# tools/viewer/widget.py
from __future__ import annotations

import os
import time
from typing import Tuple

from phys.segments import gather_debug_segments  # helper that scans space
from PIL import ImageQt
from PySide6 import QtCore, QtGui, QtWidgets
from PySide6.QtGui import QAction
from tools.viewer.debugdraw import draw_segments_capsules  # if you created this file

from .scene import Scene


def make_checker_pixmap(w: int, h: int, cell: int = 16) -> QtGui.QPixmap:
    img = QtGui.QImage(w, h, QtGui.QImage.Format.Format_ARGB32_Premultiplied)
    p = QtGui.QPainter(img)
    c1 = QtGui.QColor(200, 200, 200)
    c2 = QtGui.QColor(160, 160, 160)
    for y in range(0, h, cell):
        for x in range(0, w, cell):
            p.fillRect(
                x, y, cell, cell, c1 if ((x // cell + y // cell) % 2 == 0) else c2
            )
    p.end()
    return QtGui.QPixmap.fromImage(img)


class OverlayWidget(QtWidgets.QWidget):
    def __init__(self, scene: Scene) -> None:
        super().__init__(parent=None)
        self.scene = scene

        # Fullscreen (monitor work area)
        screen = QtWidgets.QApplication.primaryScreen()
        if screen is not None:
            self.setGeometry(screen.availableGeometry())

        # Overlay-friendly window flags
        self.setWindowTitle("Desk-Skong Overlay")
        self.setWindowFlags(
            QtCore.Qt.WindowType.FramelessWindowHint
            | QtCore.Qt.WindowType.WindowStaysOnTopHint
            | QtCore.Qt.WindowType.Tool  # hide taskbar
        )

        # Per-pixel alpha
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_NoSystemBackground, True)

        # Modes
        self.transparent_mode = True
        self.click_through = False

        # Tray
        self._tray_actions = {}
        self._setup_tray()

        # View
        self.scale = float(os.environ.get("SKONG_SCALE", str(scene.cfg.scale)))
        self.scale = max(0.05, min(20.0, self.scale))
        self.bg = make_checker_pixmap(self.width(), self.height())

        # Input state forwarded to Scene
        self.in_left = False
        self.in_right = False
        self.in_down = False
        self.in_jump_press = False

        # Tick
        self._timer = QtCore.QTimer(self)
        self._timer.timeout.connect(self._on_tick)
        self._timer.start(16)  # ~60 Hz

    # ---------------- UI plumbing ----------------

    def _setup_tray(self):
        if not QtWidgets.QSystemTrayIcon.isSystemTrayAvailable():
            return
        icon = self.style().standardIcon(
            QtWidgets.QStyle.StandardPixmap.SP_ComputerIcon
        )
        tray = QtWidgets.QSystemTrayIcon(icon, self)
        menu = QtWidgets.QMenu()

        act_show = QAction("Show Window", self)
        act_hide = QAction("Hide Window", self)
        act_trans = QAction("Transparent Background", self, checkable=True)
        act_click = QAction("Click-through (mouse passthrough)", self, checkable=True)
        act_quit = QAction("Quit", self)

        act_show.triggered.connect(self.showNormal)
        act_hide.triggered.connect(self.hide)
        act_trans.toggled.connect(self.set_transparent_mode)
        act_click.toggled.connect(self.set_click_through)
        act_quit.triggered.connect(QtWidgets.QApplication.quit)

        menu.addAction(act_show)
        menu.addAction(act_hide)
        menu.addSeparator()
        menu.addAction(act_trans)
        menu.addAction(act_click)
        menu.addSeparator()
        menu.addAction(act_quit)

        tray.setContextMenu(menu)
        tray.setToolTip("Desk-Skong Overlay")
        tray.show()

        self.tray = tray
        self._tray_actions = {"transparent": act_trans, "clickthrough": act_click}

    def set_transparent_mode(self, on: bool):
        self.transparent_mode = bool(on)
        if self._tray_actions:
            self._tray_actions["transparent"].setChecked(self.transparent_mode)
        self.update()

    def set_click_through(self, on: bool):
        self.click_through = bool(on)
        if self._tray_actions:
            self._tray_actions["clickthrough"].setChecked(self.click_through)
        self.setWindowFlag(
            QtCore.Qt.WindowType.WindowTransparentForInput, self.click_through
        )
        self.show()  # re-apply flags

    def resizeEvent(self, e: QtGui.QResizeEvent) -> None:
        self.bg = make_checker_pixmap(self.width(), self.height())
        return super().resizeEvent(e)

    # ---------------- Input ----------------

    def keyPressEvent(self, e: QtGui.QKeyEvent) -> None:
        k = e.key()
        if k == QtCore.Qt.Key_Escape:
            QtWidgets.QApplication.quit()
        elif k in (QtCore.Qt.Key_A, QtCore.Qt.Key_Left):
            self.in_left = True
        elif k in (QtCore.Qt.Key_D, QtCore.Qt.Key_Right):
            self.in_right = True
        elif k in (QtCore.Qt.Key_S, QtCore.Qt.Key_Down):
            self.in_down = True
        elif k == QtCore.Qt.Key_Space:
            self.in_jump_press = True
        elif k == QtCore.Qt.Key_T:
            self.set_transparent_mode(not self.transparent_mode)
        elif k == QtCore.Qt.Key_C:
            self.set_click_through(not self.click_through)
        else:
            super().keyPressEvent(e)

    def keyReleaseEvent(self, e: QtGui.QKeyEvent) -> None:
        k = e.key()
        if k in (QtCore.Qt.Key_A, QtCore.Qt.Key_Left):
            self.in_left = False
        elif k in (QtCore.Qt.Key_D, QtCore.Qt.Key_Right):
            self.in_right = False
        elif k in (QtCore.Qt.Key_S, QtCore.Qt.Key_Down):
            self.in_down = False
        else:
            super().keyReleaseEvent(e)

    # ---------------- Tick & Paint ----------------

    def _on_tick(self):
        self.scene.set_input(
            left=self.in_left,
            right=self.in_right,
            down=self.in_down,
            jump_press=self.in_jump_press,
        )
        self.in_jump_press = False  # consume
        self.scene.step()
        self.update()

    def paintEvent(self, e: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHints(QtGui.QPainter.RenderHint.SmoothPixmapTransform, True)

        if self.transparent_mode:
            p.setCompositionMode(QtGui.QPainter.CompositionMode_Source)
            p.fillRect(self.rect(), QtCore.Qt.transparent)
            p.setCompositionMode(QtGui.QPainter.CompositionMode_SourceOver)
        else:
            p.drawPixmap(0, 0, self.bg)

        # --- physics debug: draw segments (always)
        plats, walls = gather_debug_segments(self.scene.space)
        draw_segments_capsules(
            p,
            self.width(),
            self.height(),
            platforms=plats,
            walls=walls,
            default_radius=2.0,
            width_scale=1.0,
        )

        # --- sprite draw
        pkt = self.scene.draw_packet(scale=self.scale)
        img_qt: QtGui.QImage = ImageQt.ImageQt(pkt.image)
        pix = QtGui.QPixmap.fromImage(img_qt)

        cw, ch = pkt.scaled_canvas_size
        ox, oy, fw, fh = pkt.scaled_frame_rect_tl

        # Physics → screen anchor (feet line)
        px, py = self.scene.player_pos
        feet_y = self.scene.feet_y_world()
        anchor_x_screen = int(round(px))
        anchor_y_screen = self.height() - int(round(feet_y))  # y-up to y-down

        # ✅ Your requested placement
        dst_x = int(round(anchor_x_screen - (ox + (fw * 0.5))))
        dst_y = int(round(anchor_y_screen - (oy + fh)))

        # Optional clamp so frame stays on-screen vertically
        frame_top = dst_y + oy
        frame_bottom = frame_top + fh
        if frame_top < 0:
            dst_y += -frame_top
        elif frame_bottom > self.height():
            dst_y -= frame_bottom - self.height()

        p.drawPixmap(dst_x + ox, dst_y + oy, pix)

        p.end()
