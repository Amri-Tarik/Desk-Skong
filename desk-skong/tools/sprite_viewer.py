# tools/sprite_viewer.py
from __future__ import annotations

import os
import pathlib
import sys
import time

from anim.sprite_anim import AnimationConfig, Skin, SpritePlayer
from PIL import ImageQt
from PySide6 import QtCore, QtGui, QtWidgets
from PySide6.QtGui import QAction

# --- add repo root to sys.path
ROOT = pathlib.Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


# --- data paths (relative to repo root)
SKIN_JSON = ROOT / "data" / "Knight_tk2d.json"
ATLAS_PNG = ROOT / "data" / "Knight.png"
ANIMS_JSON = ROOT / "data" / "animations.json"


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


class Viewer(QtWidgets.QWidget):
    def __init__(self) -> None:
        super().__init__(parent=None)
        self.setWindowTitle("Sprite Viewer")
        # Make the window resize to the size of the current monitor
        screen = QtWidgets.QApplication.primaryScreen()
        if screen is not None:
            geometry = screen.availableGeometry()
            self.setGeometry(geometry)

        # Overlay-friendly window flags
        self.setWindowFlags(
            QtCore.Qt.WindowType.FramelessWindowHint
            | QtCore.Qt.WindowType.WindowStaysOnTopHint
            | QtCore.Qt.WindowType.Tool  # no taskbar button
        )

        self._tray_actions = {}
        self._setup_tray()

        # Per-pixel alpha
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_NoSystemBackground, True)

        # mode toggles
        self.transparent_mode = True  # start transparent; press 'T' to see checkerboard
        self.click_through = False  # press 'C' to toggle

        # load data
        self.skin = Skin.load(str(SKIN_JSON), str(ATLAS_PNG))
        with open(ANIMS_JSON, "r", encoding="utf-8") as f:
            anim_text = f.read()
        self.config = (
            AnimationConfig.load_str(anim_text)
            if hasattr(AnimationConfig, "load_str")
            else AnimationConfig.load_file(str(ANIMS_JSON))
        )

        # start in idle (must exist in animations.json)
        self.player = SpritePlayer(self.skin, self.config, start_group="idle")

        # timing
        self.last = time.perf_counter()
        self.timer = QtCore.QTimer(self)
        self.timer.timeout.connect(self.on_tick)
        self.timer.start(16)  # ~60 fps

        # view params
        self.scale = float(os.environ.get("SKONG_SCALE", "0.5"))
        self.scale = max(0.05, min(20.0, self.scale))
        self.bg = make_checker_pixmap(self.width(), self.height())

        # step mode (left/right arrows)
        self.pause = False

        # bottom-align placement offset from window edges
        self.pad_x = 40
        self.pad_y = 40

        # logging
        self.last_frame_name = None

    def on_tick(self):
        now = time.perf_counter()
        dt = now - self.last
        self.last = now
        if not self.pause:
            self.player.update(dt)
        self.update()

    def keyPressEvent(self, e: QtGui.QKeyEvent) -> None:
        k = e.key()
        if k == QtCore.Qt.Key_Escape:
            QtWidgets.QApplication.quit()
        elif k == QtCore.Qt.Key_1:
            self.player.goto("idle")
        elif k == QtCore.Qt.Key_2:
            self.player.goto("run")
        elif k in (QtCore.Qt.Key_Plus, QtCore.Qt.Key_Equal):
            self.scale = min(20.0, self.scale * 1.1)
            self.update()
        elif k in (QtCore.Qt.Key_Minus, QtCore.Qt.Key_Underscore):
            self.scale = max(0.05, self.scale / 1.1)
            self.update()
        elif k == QtCore.Qt.Key_Space:
            self.pause = not self.pause
        elif k == QtCore.Qt.Key_Right:
            self.pause = True
            # manual step
            self.player.update(999.0)  # force advance one or more frames; clamp inside
        elif k == QtCore.Qt.Key_Left:
            # simple “step back”: restart group and advance to prev index
            self.pause = True
            # crude back step: reset then advance to len-1 from current
            name = self.player.current_group
            self.player.goto(name)  # resets to 0
            # spin length-1 frames
            for _ in range(len(self._seq()) - 1):
                self.player.update(999.0)
        elif k == QtCore.Qt.Key_T:
            self.set_transparent_mode(not self.transparent_mode)
        elif k == QtCore.Qt.Key_C:
            self.set_click_through(not self.click_through)

        else:
            super().keyPressEvent(e)

    def _seq(self):
        # convenience to peek the resolved sequence (list of indices)
        # not part of public API; works for our viewer
        return getattr(self.player, "_seq")

    def resizeEvent(self, e: QtGui.QResizeEvent) -> None:
        self.bg = make_checker_pixmap(self.width(), self.height())
        super().resizeEvent(e)

    def paintEvent(self, e: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHints(QtGui.QPainter.RenderHint.SmoothPixmapTransform, False)

        if self.transparent_mode:
            # Clear to fully transparent (for overlay)
            p.setCompositionMode(QtGui.QPainter.CompositionMode_Source)
            p.fillRect(self.rect(), QtCore.Qt.transparent)
            p.setCompositionMode(QtGui.QPainter.CompositionMode_SourceOver)
        else:
            # Debug checkerboard
            p.drawPixmap(0, 0, self.bg)

        pkt = self.player.draw_packet(scale=self.scale)
        img_qt: QtGui.QImage = ImageQt.ImageQt(pkt.image)
        pix = QtGui.QPixmap.fromImage(img_qt)

        cw, ch = pkt.scaled_canvas_size
        ox, oy, fw, fh = pkt.scaled_frame_rect_tl

        # bottom-align canvas in window with padding (initial)
        dst_x = self.pad_x
        dst_y = self.height() - self.pad_y - int(round(ch))

        # --- Clamp so the *frame* is fully visible vertically ---
        # frame top-left in window:
        frame_top = dst_y + oy
        frame_bottom = frame_top + fh

        if frame_top < 0:
            # push canvas down so frame top is at y=0
            dst_y += -frame_top
        elif frame_bottom > self.height():
            # pull canvas up so frame bottom fits
            dst_y -= frame_bottom - self.height()

        # Optional outlines in debug mode
        if not self.transparent_mode:
            pen = QtGui.QPen(QtGui.QColor(0, 0, 0, 120))
            pen.setWidth(1)
            p.setPen(pen)
            p.setBrush(QtCore.Qt.BrushStyle.NoBrush)
            p.drawRect(dst_x, dst_y, int(round(cw)), int(round(ch)))

            pen2 = QtGui.QPen(QtGui.QColor(255, 80, 80, 160))
            pen2.setWidth(1)
            p.setPen(pen2)
            p.drawRect(dst_x + ox, dst_y + oy, fw, fh)

        # Draw sprite
        p.drawPixmap(dst_x + ox, dst_y + oy, pix)

        # Labels (only in debug)
        if not self.transparent_mode:
            p.setPen(QtGui.QColor(20, 20, 20))
            info = f"group={self.player.current_group}  scale={self.scale:.2f}"
            p.drawText(12, 20, info)

        p.end()

    def _setup_tray(self):
        """Create a system tray icon with a context menu to recover from click-through."""
        if not QtWidgets.QSystemTrayIcon.isSystemTrayAvailable():
            print(
                "System tray not available; click-through might be hard to disable without a hotkey."
            )
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

        # Keep references so GC doesn't kill the tray/menu/actions
        self.tray = tray
        self._tray_actions = {
            "transparent": act_trans,
            "clickthrough": act_click,
        }

    def set_transparent_mode(self, on: bool):
        self.transparent_mode = bool(on)
        # keep tray and keyboard toggles in sync
        if self._tray_actions:
            self._tray_actions["transparent"].setChecked(self.transparent_mode)
        self.update()

    def set_click_through(self, on: bool):
        self.click_through = bool(on)
        if self._tray_actions:
            self._tray_actions["clickthrough"].setChecked(self.click_through)
        # Toggle mouse passthrough
        self.setWindowFlag(
            QtCore.Qt.WindowType.WindowTransparentForInput, self.click_through
        )
        self.show()  # re-apply flags


def main():
    app = QtWidgets.QApplication(sys.argv)
    w = Viewer()
    w.show()
    sys.exit(app.exec())
