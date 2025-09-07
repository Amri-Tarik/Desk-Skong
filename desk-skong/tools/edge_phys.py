# tools/edge_phys.py
from __future__ import annotations

import os
from dataclasses import dataclass
from typing import List, Optional, Tuple

import numpy as np
import pymunk
from phys.layers import CT_TERRAIN, CT_WALL
from phys.segments import OneWayPlatform, VerticalWall
from PySide6 import QtCore, QtGui, QtWidgets
from vision.edge_detect import DetectedEdges, EdgeConfig, EdgeDetector, Segment
from vision.edge_track import EdgeTracker, TrackConfig


def grab_screen_rgba_and_qimage(
    screen: QtGui.QScreen,
) -> tuple[np.ndarray, QtGui.QImage]:
    pix = screen.grabWindow(0)
    img = pix.toImage().convertToFormat(QtGui.QImage.Format.Format_RGBA8888)
    w, h = img.width(), img.height()
    bpl = img.bytesPerLine()
    buf = img.constBits()
    data = np.frombuffer(buf, dtype=np.uint8, count=bpl * h)
    rgba = data.reshape((h, bpl // 4, 4))[:, :w, :].copy()
    return rgba, img


@dataclass
class EdgePhysConfig:
    interval_sec: float = 0.5  # how often to rescan
    horiz_radius: float = 2.0  # pymunk Segment radius for platforms
    vert_radius: float = 6.0  # pymunk Segment radius for walls
    # If your window height differs from capture height, set a transform
    # otherwise we’ll use QScreen geometry.
    world_height_px: Optional[int] = None

    # Visualization flag (lines drawn by the viewer; no physics dependency)
    visualize: bool = bool(int(os.environ.get("SKONG_EDGE_VIS", "0")))


class EdgePhysicsBridge(QtCore.QObject):
    """
    Periodically detects screen edges and mirrors them into the physics world
    as OneWayPlatform / VerticalWall shapes. Keeps a handle to shapes it created,
    and replaces them each scan.
    """

    def __init__(
        self,
        space: pymunk.Space,
        detector: EdgeDetector,
        cfg: EdgePhysConfig,
        parent: Optional[QtCore.QObject] = None,
    ) -> None:
        super().__init__(parent)
        self.space = space
        self.detector = detector
        self.cfg = cfg

        self._timer = QtCore.QTimer(self)
        self._timer.timeout.connect(self._scan_once)
        self._timer.start(int(self.cfg.interval_sec * 1000))

        # book-keeping: shapes we injected on last scan
        self._last_shapes: List[pymunk.Shape] = []

        # for optional visualization by the UI
        self.last_edges: Optional[DetectedEdges] = None
        self.last_capture_size: Tuple[int, int] = (0, 0)

        self.tracker = EdgeTracker(
            TrackConfig(
                window=3,  # tune: 4–6 is a good range
                min_persist=3,  # require 3 hits to “stabilize”
                y_tol=4,
                x_tol=4,
                gap_merge_tol=4,
                min_overlap_px=8,
                smooth_alpha=0.6,
            )
        )

    # -- public control -------------------------------------------------

    def stop(self) -> None:
        self._timer.stop()

    def clear(self) -> None:
        # remove our previous shapes from the space
        if self._last_shapes:
            try:
                self.space.remove(*self._last_shapes)
            except Exception:
                pass
            self._last_shapes.clear()

    # -- scanning + projection -----------------------------------------

    def _scan_once(self) -> None:
        screen = QtWidgets.QApplication.primaryScreen()
        if not screen:
            return
        rgba, _img = grab_screen_rgba_and_qimage(screen)
        Hc, Wc = rgba.shape[:2]
        self.last_capture_size = (Wc, Hc)

        # 1) raw detections (your tuned detector)
        raw_edges = self.detector.detect(rgba)

        # 2) stabilize over time
        stable_edges = self.tracker.update(raw_edges)
        self.last_edges = stable_edges if self.cfg.visualize else None

        # 3) use stable edges for physics
        self._rebuild_physics(stable_edges, screen_geometry=screen.geometry())

    def _rebuild_physics(
        self, edges: DetectedEdges, screen_geometry: QtCore.QRect
    ) -> None:
        # remove old
        self.clear()

        W = screen_geometry.width()
        H_screen = screen_geometry.height()
        H_world = self.cfg.world_height_px or H_screen

        created: List[pymunk.Shape] = []

        # Horizontal → OneWayPlatform
        for seg in edges.horizontals:
            # screen y-down to y-up; use y1 (≈ y2) as platform height
            y_screen = seg.y1
            y_up = float(H_world - y_screen)
            x1 = float(seg.x1)
            x2 = float(seg.x2)
            shp = OneWayPlatform.add(
                self.space,
                (x1, y_up),
                (x2, y_up),
                radius=self.cfg.horiz_radius,
            ).shape
            created.append(shp)

        # Vertical → VerticalWall
        for seg in edges.verticals:
            y1_up = float(H_world - seg.y1)
            y2_up = float(H_world - seg.y2)
            x = float(seg.x1)  # ≈ seg.x2
            shp = VerticalWall.add(
                self.space,
                (x, y1_up),
                (x, y2_up),
                radius=self.cfg.vert_radius,
            ).shape
            created.append(shp)

        self._last_shapes = created
