# tools/edge_scan.py
from __future__ import annotations

import sys
from typing import Optional

import numpy as np
from PySide6 import QtCore, QtGui, QtWidgets
from vision.edge_detect import DetectedEdges, EdgeConfig, EdgeDetector


# tools/edge_scan.py
def grab_screen_rgba_and_qimage(
    screen: QtGui.QScreen,
) -> tuple[np.ndarray, QtGui.QImage]:
    """
    Return (rgba_array, qimage). rgba is contiguous uint8 HxWx4.
    """
    pix = screen.grabWindow(0)  # primary desktop
    img = pix.toImage().convertToFormat(QtGui.QImage.Format.Format_RGBA8888)

    w, h = img.width(), img.height()
    bpl = img.bytesPerLine()  # row stride in bytes (may be padded)

    buf = img.constBits()  # memoryview
    data = np.frombuffer(buf, dtype=np.uint8, count=bpl * h)

    # Reshape by stride, crop to actual width, then COPY to make it C-contiguous.
    # After this, arr.flags.c_contiguous == True and arr.dtype == uint8.
    arr = data.reshape((h, bpl // 4, 4))[:, :w, :].copy()

    return arr, img


class EdgeOverlay(QtWidgets.QWidget):
    def __init__(
        self, interval_ms: int = 1000, parent: Optional[QtWidgets.QWidget] = None
    ):
        super().__init__(parent)
        self.setWindowTitle("Edge Overlay")
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self.setAttribute(QtCore.Qt.WidgetAttribute.WA_NoSystemBackground, True)
        self.setWindowFlags(
            QtCore.Qt.WindowType.FramelessWindowHint
            | QtCore.Qt.WindowType.WindowStaysOnTopHint
            | QtCore.Qt.WindowType.Tool
        )

        # full-screen window on primary monitor
        screen = QtWidgets.QApplication.primaryScreen()
        if screen:
            self.setGeometry(screen.availableGeometry())

        # detector
        self.detector = EdgeDetector(EdgeConfig())

        # results
        self.edges: DetectedEdges = DetectedEdges([], [])
        self.show_edges = True

        # timers
        self.refresh = QtCore.QTimer(self)
        self.refresh.setInterval(interval_ms)
        self.refresh.timeout.connect(self.scan_once)
        self.refresh.start()

        # enable keyboard
        self.setFocusPolicy(QtCore.Qt.FocusPolicy.StrongFocus)

    def keyPressEvent(self, e: QtGui.QKeyEvent) -> None:
        k = e.key()
        if k == QtCore.Qt.Key_Escape:
            QtWidgets.QApplication.quit()
        elif k == QtCore.Qt.Key_Space:
            self.scan_once()
        elif k == QtCore.Qt.Key_E:  # toggle overlay visibility
            self.show_edges = not self.show_edges
            self.update()
        else:
            super().keyPressEvent(e)

    def scan_once(self) -> None:
        screen = QtWidgets.QApplication.primaryScreen()
        if not screen:
            return
        rgba, img_qt = grab_screen_rgba_and_qimage(screen)
        try:
            self.edges = self.detector.detect(rgba)
        except Exception as e:
            # Debug dump
            print("[edge_scan] detect() failed:", repr(e))
            print(
                "[edge_scan] rgba:",
                "shape=",
                rgba.shape,
                "dtype=",
                rgba.dtype,
                "strides=",
                rgba.strides,
                "c_contig=",
                rgba.flags.c_contiguous,
                "f_contig=",
                rgba.flags.f_contiguous,
            )
            # Save QImage and NumPy view for manual inspection
            img_qt.save("edge_scan_qimage.png")  # saved by Qt directly
            try:
                from PIL import Image

                Image.fromarray(rgba, "RGBA").save("edge_scan_rgba.png")
                print("[edge_scan] wrote edge_scan_qimage.png and edge_scan_rgba.png")
            except Exception as ee:
                print("[edge_scan] PIL save failed:", repr(ee))
            # Keep previous edges; don’t crash the overlay
            return
        self.update()

    def paintEvent(self, e: QtGui.QPaintEvent) -> None:
        p = QtGui.QPainter(self)
        p.setRenderHints(QtGui.QPainter.RenderHint.Antialiasing, True)

        # full transparency background
        p.setCompositionMode(QtGui.QPainter.CompositionMode_Source)
        p.fillRect(self.rect(), QtCore.Qt.transparent)
        p.setCompositionMode(QtGui.QPainter.CompositionMode_SourceOver)

        if not self.show_edges:
            p.end()
            return

        # Draw verticals (walls) in red, horizontals (platforms) in green
        wall_pen = QtGui.QPen(QtGui.QColor(255, 60, 60, 220))
        wall_pen.setWidth(1)  # thinner
        plat_pen = QtGui.QPen(QtGui.QColor(80, 220, 120, 220))
        plat_pen.setWidth(1)  # thinner

        p.setPen(plat_pen)
        for s in self.edges.horizontals:
            p.drawLine(s.x1, s.y1, s.x2, s.y2)

        p.setPen(wall_pen)
        for s in self.edges.verticals:
            p.drawLine(s.x1, s.y1, s.x2, s.y2)

        # Tiny legend
        p.setPen(QtGui.QPen(QtGui.QColor(230, 230, 230, 220)))
        p.setBrush(QtCore.Qt.BrushStyle.NoBrush)
        p.drawText(12, 24, "E: toggle overlay | Space: rescan | Esc: quit")

        p.end()


def main():
    app = QtWidgets.QApplication(sys.argv)
    w = EdgeOverlay(interval_ms=1000)
    w.show()
    sys.exit(app.exec())
