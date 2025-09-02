# tools/sprite_viewer.py
from __future__ import annotations
import os, sys, time, pathlib

from PIL import ImageQt
from PySide6 import QtCore, QtGui, QtWidgets

from anim.sprite_anim import Skin, AnimationConfig, SpritePlayer


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
        self.resize(1280, 720)
        self.setMinimumSize(640, 360)

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

        # checker background
        p.drawPixmap(0, 0, self.bg)

        pkt = self.player.draw_packet(scale=self.scale)
        img: QtGui.QImage = ImageQt.ImageQt(pkt["image"])
        pix = QtGui.QPixmap.fromImage(img)

        cw, ch = pkt["canvas_size"]
        ox, oy = pkt["scaled_offset_tl"]

        # bottom-align canvas in window with padding
        dst_x = self.pad_x
        dst_y = self.height() - self.pad_y - int(round(ch))

        # draw canvas outline (debug)
        pen = QtGui.QPen(QtGui.QColor(0, 0, 0, 120))
        pen.setWidth(1)
        p.setPen(pen)
        p.setBrush(QtCore.Qt.BrushStyle.NoBrush)
        p.drawRect(dst_x, dst_y, int(round(cw)), int(round(ch)))

        # draw sprite at its top-left offset inside canvas
        p.drawPixmap(dst_x + ox, dst_y + oy, pix)

        # labels
        p.setPen(QtGui.QColor(20, 20, 20))
        info = f"group={self.player.current_group}  frame={pkt['name']}  scale={self.scale:.2f}"
        p.drawText(12, 20, info)

        # frame-change log
        if pkt["name"] != self.last_frame_name:
            self.last_frame_name = pkt["name"]

        p.end()


def main():
    app = QtWidgets.QApplication(sys.argv)
    w = Viewer()
    w.show()
    sys.exit(app.exec())
