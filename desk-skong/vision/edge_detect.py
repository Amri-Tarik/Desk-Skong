# vision/edge_detect.py
from __future__ import annotations

from dataclasses import dataclass
from typing import List, Tuple

import cv2
import numpy as np


# A simple line segment in screen coords (pixels)
@dataclass
class Segment:
    x1: int
    y1: int
    x2: int
    y2: int


@dataclass
class DetectedEdges:
    horizontals: List[Segment]
    verticals: List[Segment]


@dataclass
class EdgeConfig:
    # --- Canny (luma) ---
    canny1: int = 12  # LOWER -> more sensitive
    canny2: int = 36
    canny_blur: int = 0  # 0 = no blur (keeps thin lines)

    # --- Optional color-aware edges (Lab chroma channels) ---
    color_aware: bool = True
    canny1_color: int = 8  # chroma thresholds even looser
    canny2_color: int = 28

    # --- Hough ---
    hough_rho: float = 1.0
    hough_theta: float = np.pi / 180.0
    hough_threshold: int = 15  # LOWER -> accept weaker lines
    hough_min_line_len_frac: float = 0.03  # 8% of min(screen_w, screen_h)
    hough_max_line_gap: int = 2

    # Orientation gate
    axis_eps_deg: float = 8.0  # allow a bit more slant

    # Merge tolerances (pixels)
    y_merge_tol: int = 4
    x_merge_tol: int = 4
    gap_merge_tol: int = 12


def _angle_deg(x1: int, y1: int, x2: int, y2: int) -> float:
    dx = float(x2 - x1)
    dy = float(y2 - y1)
    ang = np.degrees(np.arctan2(dy, dx))
    # normalize to [0, 180)
    if ang < 0:
        ang += 180.0
    return ang


def _is_axis_aligned(ang_deg: float, eps: float) -> Tuple[bool, str]:
    """Return (ok, kind) where kind in {'h','v'} if near 0° or 90°."""
    if abs(ang_deg - 0.0) <= eps or abs(ang_deg - 180.0) <= eps:
        return True, "h"
    if abs(ang_deg - 90.0) <= eps:
        return True, "v"
    return False, ""


def _merge_horizontals(lines: List[Segment], y_tol: int, gap_tol: int) -> List[Segment]:
    # group by similar y, then merge x-intervals
    if not lines:
        return []
    # use average y of each segment
    annot = [((seg.y1 + seg.y2) * 0.5, seg) for seg in lines]
    annot.sort(key=lambda t: t[0])
    merged: List[Segment] = []
    i = 0
    n = len(annot)
    while i < n:
        group_y = annot[i][0]
        group: List[Segment] = [annot[i][1]]
        i += 1
        while i < n and abs(annot[i][0] - group_y) <= y_tol:
            group.append(annot[i][1])
            i += 1
        # merge x-intervals for this y band
        intervals = []
        gy_vals = []
        for seg in group:
            xlo = min(seg.x1, seg.x2)
            xhi = max(seg.x1, seg.x2)
            intervals.append((xlo, xhi))
            gy_vals.append((seg.y1 + seg.y2) * 0.5)
        intervals.sort()
        merged_ints = []
        cur_lo, cur_hi = intervals[0]
        for lo, hi in intervals[1:]:
            if lo <= cur_hi + gap_tol:
                cur_hi = max(cur_hi, hi)
            else:
                merged_ints.append((cur_lo, cur_hi))
                cur_lo, cur_hi = lo, hi
        merged_ints.append((cur_lo, cur_hi))
        # representative y
        ry = int(round(sum(gy_vals) / max(1, len(gy_vals))))
        for lo, hi in merged_ints:
            merged.append(Segment(int(lo), ry, int(hi), ry))
    return merged


def _merge_verticals(lines: List[Segment], x_tol: int, gap_tol: int) -> List[Segment]:
    if not lines:
        return []
    annot = [((seg.x1 + seg.x2) * 0.5, seg) for seg in lines]
    annot.sort(key=lambda t: t[0])
    merged: List[Segment] = []
    i = 0
    n = len(annot)
    while i < n:
        group_x = annot[i][0]
        group: List[Segment] = [annot[i][1]]
        i += 1
        while i < n and abs(annot[i][0] - group_x) <= x_tol:
            group.append(annot[i][1])
            i += 1
        intervals = []
        gx_vals = []
        for seg in group:
            ylo = min(seg.y1, seg.y2)
            yhi = max(seg.y1, seg.y2)
            intervals.append((ylo, yhi))
            gx_vals.append((seg.x1 + seg.x2) * 0.5)
        intervals.sort()
        merged_ints = []
        cur_lo, cur_hi = intervals[0]
        for lo, hi in intervals[1:]:
            if lo <= cur_hi + gap_tol:
                cur_hi = max(cur_hi, hi)
            else:
                merged_ints.append((cur_lo, cur_hi))
                cur_lo, cur_hi = lo, hi
        merged_ints.append((cur_lo, cur_hi))
        rx = int(round(sum(gx_vals) / max(1, len(gx_vals))))
        for lo, hi in merged_ints:
            merged.append(Segment(rx, int(lo), rx, int(hi)))
    return merged


class EdgeDetector:
    def __init__(self, config: EdgeConfig | None = None) -> None:
        self.cfg = config or EdgeConfig()

    def detect(self, rgba_image: np.ndarray) -> DetectedEdges:
        """
        rgba_image: np.ndarray (H, W, 4) uint8, contiguous.
        """
        if rgba_image is None:
            raise ValueError("detect(): rgba_image is None")
        if rgba_image.ndim != 3 or rgba_image.shape[2] != 4:
            raise ValueError(
                f"detect(): expected HxWx4 array, got shape={rgba_image.shape}"
            )
        if rgba_image.dtype != np.uint8:
            rgba_image = rgba_image.astype(np.uint8, copy=False)

        rgba = np.ascontiguousarray(rgba_image)
        h, w = rgba.shape[:2]
        if h == 0 or w == 0:
            return DetectedEdges([], [])

        # --- Luma edge map
        bgr = cv2.cvtColor(rgba, cv2.COLOR_RGBA2BGR)
        gray = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)
        if self.cfg.canny_blur and self.cfg.canny_blur >= 3:
            k = int(self.cfg.canny_blur) | 1
            gray = cv2.GaussianBlur(gray, (k, k), 0)
        edges_gray = cv2.Canny(gray, self.cfg.canny1, self.cfg.canny2)

        # --- Color-aware edge map (Lab a*, b*)
        bgr = cv2.cvtColor(rgba, cv2.COLOR_RGBA2BGR)

        # Gray edges (unchanged)
        gray = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)
        if self.cfg.canny_blur and self.cfg.canny_blur >= 3:
            k = int(self.cfg.canny_blur) | 1
            gray = cv2.GaussianBlur(gray, (k, k), 0)
        edges_gray = cv2.Canny(gray, self.cfg.canny1, self.cfg.canny2)

        # Color-aware edges on Lab chroma (A,B)
        if self.cfg.color_aware:
            try:
                lab = cv2.cvtColor(bgr, cv2.COLOR_BGR2LAB)  # <- 3-channel input
                _, A, B = cv2.split(lab)
                edges_a = cv2.Canny(A, self.cfg.canny1_color, self.cfg.canny2_color)
                edges_b = cv2.Canny(B, self.cfg.canny1_color, self.cfg.canny2_color)
                edges = cv2.bitwise_or(edges_gray, cv2.bitwise_or(edges_a, edges_b))
            except cv2.error as e:
                # Fallback gracefully if this build lacks LAB (unlikely)
                print("[edge_detect] BGR2LAB failed; using luma only:", repr(e))
                edges = edges_gray
        else:
            edges = edges_gray

        min_line_len = int(self.cfg.hough_min_line_len_frac * min(w, h))
        raw = cv2.HoughLinesP(
            edges,
            rho=self.cfg.hough_rho,
            theta=self.cfg.hough_theta,
            threshold=self.cfg.hough_threshold,
            minLineLength=max(5, min_line_len),
            maxLineGap=self.cfg.hough_max_line_gap,
        )
        if raw is None:
            return DetectedEdges([], [])

        horiz_raw: List[Segment] = []
        vert_raw: List[Segment] = []

        for x1, y1, x2, y2 in raw[:, 0, :]:
            ang = _angle_deg(x1, y1, x2, y2)
            ok, kind = _is_axis_aligned(ang, self.cfg.axis_eps_deg)
            if not ok:
                continue
            length = ((x2 - x1) ** 2 + (y2 - y1) ** 2) ** 0.5
            if kind == "h":
                if length >= self.cfg.hough_min_line_len_frac * w:
                    horiz_raw.append(Segment(int(x1), int(y1), int(x2), int(y2)))
            else:
                if length >= self.cfg.hough_min_line_len_frac * h:
                    vert_raw.append(Segment(int(x1), int(y1), int(x2), int(y2)))

        horiz = _merge_horizontals(
            horiz_raw, self.cfg.y_merge_tol, self.cfg.gap_merge_tol
        )
        vert = _merge_verticals(vert_raw, self.cfg.x_merge_tol, self.cfg.gap_merge_tol)
        return DetectedEdges(horiz, vert)
