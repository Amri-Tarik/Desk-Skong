# vision/edge_track.py
from __future__ import annotations

from collections import defaultdict, deque
from dataclasses import dataclass
from typing import Deque, Dict, Iterable, List, Optional, Tuple

import numpy as np

from .edge_detect import DetectedEdges, Segment


@dataclass
class TrackConfig:
    # History
    window: int = 5  # keep last N scans
    min_persist: int = 2  # require presence in >= this many scans

    # Geometry tolerances (pixels/degrees)
    y_tol: int = 4  # vertical tolerance to group horizontals
    x_tol: int = 4  # horizontal tolerance to group verticals
    gap_merge_tol: int = 12  # gap tolerated when merging collinear intervals
    min_overlap_px: int = 8  # min interval intersection to count as “same line”

    # Jitter smoothing for the final y/x of merged lines (0..1, larger = steadier)
    smooth_alpha: float = 0.6


class EdgeTracker:
    """
    Temporal aggregator for detected edges. Feed each scan’s edges to `update()`.
    It returns a DetectedEdges containing only the stable (persistent) lines.

    Heuristic:
      - Keep last `window` frames of merged segments.
      - Cluster horizontals by y within `y_tol` and merge x-intervals per frame.
      - Across frames, intervals that overlap by ≥ min_overlap_px are counted.
      - Keep intervals whose “presence count” ≥ min_persist.
      - Smooth final y (or x for verticals) with EMA to reduce jitter.
    """

    def __init__(self, cfg: TrackConfig | None = None) -> None:
        self.cfg = cfg or TrackConfig()
        self.hist_h: Deque[List[Segment]] = deque(maxlen=self.cfg.window)
        self.hist_v: Deque[List[Segment]] = deque(maxlen=self.cfg.window)
        self._prev_stable_h: Optional[List[Segment]] = None
        self._prev_stable_v: Optional[List[Segment]] = None

    # -------- public API --------
    def update(self, edges: DetectedEdges) -> DetectedEdges:
        # Push current frame (already merged by your detector; good)
        self.hist_h.append(list(edges.horizontals))
        self.hist_v.append(list(edges.verticals))

        stable_h = self._stable_horiz(list(self.hist_h))
        stable_v = self._stable_vert(list(self.hist_v))

        # Smooth vs previous
        stable_h = self._smooth_lines(stable_h, self._prev_stable_h, axis="h")
        stable_v = self._smooth_lines(stable_v, self._prev_stable_v, axis="v")

        self._prev_stable_h = stable_h
        self._prev_stable_v = stable_v

        return DetectedEdges(stable_h, stable_v)

    # -------- internals --------
    def _stable_horiz(self, frames: List[List[Segment]]) -> List[Segment]:
        if not frames:
            return []
        cfg = self.cfg

        # Group by y within tolerance, across all frames.
        # Then within a group, merge x-intervals per-frame, and count persistence.
        clusters: List[List[Tuple[int, Segment]]] = []  # (frame_idx, segment)
        for i, segs in enumerate(frames):
            for s in segs:
                self._assign_horiz(clusters, i, s, cfg.y_tol)

        # For each cluster, merge by intervals with presence counts.
        out: List[Segment] = []
        for cluster in clusters:
            # cluster is list of (frame_idx, seg) with similar y
            ys = [seg.y1 for _, seg in cluster]
            y_ref = int(round(np.median(ys)))  # robust y anchor for the cluster

            # Collect per-frame intervals
            per_frame: Dict[int, List[Tuple[int, int]]] = defaultdict(list)
            for fi, seg in cluster:
                x1 = min(seg.x1, seg.x2)
                x2 = max(seg.x1, seg.x2)
                per_frame[fi].append((x1, x2))

            # Merge intervals per frame
            for fi in per_frame:
                per_frame[fi] = _merge_intervals(per_frame[fi], gap=cfg.gap_merge_tol)

            # Accumulate intervals across frames by overlapping bins
            # Approach: build an accumulator of “presence” over x by sweeping
            # To keep it simple, we’ll match intervals greedily across frames.
            presence_bins: List[Tuple[int, int, int]] = []  # (x1, x2, count)
            for fi in sorted(per_frame.keys()):
                for iv in per_frame[fi]:
                    matched = False
                    for k, (bx1, bx2, cnt) in enumerate(presence_bins):
                        if _overlap(iv, (bx1, bx2)) >= cfg.min_overlap_px:
                            # unify/expand bin
                            nx1 = min(bx1, iv[0])
                            nx2 = max(bx2, iv[1])
                            presence_bins[k] = (nx1, nx2, cnt + 1)
                            matched = True
                            break
                    if not matched:
                        presence_bins.append((iv[0], iv[1], 1))

            # Keep bins that appear in enough frames; keep final as merged union
            keep: List[Tuple[int, int]] = [
                (x1, x2) for (x1, x2, cnt) in presence_bins if cnt >= cfg.min_persist
            ]
            keep = _merge_intervals(keep, gap=cfg.gap_merge_tol)
            for x1, x2 in keep:
                out.append(Segment(x1=x1, y1=y_ref, x2=x2, y2=y_ref))

        # Optional: merge again across clusters (in case adjacent clusters merged)
        out = _merge_h_segments(out, y_tol=self.cfg.y_tol, gap=self.cfg.gap_merge_tol)
        return out

    def _stable_vert(self, frames: List[List[Segment]]) -> List[Segment]:
        if not frames:
            return []
        cfg = self.cfg

        clusters: List[List[Tuple[int, Segment]]] = []
        for i, segs in enumerate(frames):
            for s in segs:
                self._assign_vert(clusters, i, s, cfg.x_tol)

        out: List[Segment] = []
        for cluster in clusters:
            xs = [seg.x1 for _, seg in cluster]
            x_ref = int(round(np.median(xs)))

            per_frame: Dict[int, List[Tuple[int, int]]] = defaultdict(list)
            for fi, seg in cluster:
                y1 = min(seg.y1, seg.y2)
                y2 = max(seg.y1, seg.y2)
                per_frame[fi].append((y1, y2))

            for fi in per_frame:
                per_frame[fi] = _merge_intervals(per_frame[fi], gap=cfg.gap_merge_tol)

            presence_bins: List[Tuple[int, int, int]] = []
            for fi in sorted(per_frame.keys()):
                for iv in per_frame[fi]:
                    matched = False
                    for k, (by1, by2, cnt) in enumerate(presence_bins):
                        if _overlap(iv, (by1, by2)) >= cfg.min_overlap_px:
                            ny1 = min(by1, iv[0])
                            ny2 = max(by2, iv[1])
                            presence_bins[k] = (ny1, ny2, cnt + 1)
                            matched = True
                            break
                    if not matched:
                        presence_bins.append((iv[0], iv[1], 1))

            keep: List[Tuple[int, int]] = [
                (y1, y2) for (y1, y2, cnt) in presence_bins if cnt >= cfg.min_persist
            ]
            keep = _merge_intervals(keep, gap=cfg.gap_merge_tol)
            for y1, y2 in keep:
                out.append(Segment(x1=x_ref, y1=y1, x2=x_ref, y2=y2))

        out = _merge_v_segments(out, x_tol=self.cfg.x_tol, gap=self.cfg.gap_merge_tol)
        return out

    def _assign_horiz(
        self,
        clusters: List[List[Tuple[int, Segment]]],
        frame_idx: int,
        seg: Segment,
        y_tol: int,
    ) -> None:
        y = (seg.y1 + seg.y2) // 2
        for cl in clusters:
            cy = (cl[0][1].y1 + cl[0][1].y2) // 2
            if abs(cy - y) <= y_tol:
                cl.append((frame_idx, seg))
                return
        clusters.append([(frame_idx, seg)])

    def _assign_vert(
        self,
        clusters: List[List[Tuple[int, Segment]]],
        frame_idx: int,
        seg: Segment,
        x_tol: int,
    ) -> None:
        x = (seg.x1 + seg.x2) // 2
        for cl in clusters:
            cx = (cl[0][1].x1 + cl[0][1].x2) // 2
            if abs(cx - x) <= x_tol:
                cl.append((frame_idx, seg))
                return
        clusters.append([(frame_idx, seg)])

    def _smooth_lines(
        self,
        cur: List[Segment],
        prev: Optional[List[Segment]],
        *,
        axis: str,
    ) -> List[Segment]:
        if not prev or not cur:
            return cur
        a = self.cfg.smooth_alpha
        out: List[Segment] = []
        # Greedy nearest neighbor smoothing on primary axis (y for h, x for v)
        used = [False] * len(prev)
        for s in cur:
            best_i = -1
            best_d = 1e9
            if axis == "h":
                y = s.y1
                for i, p in enumerate(prev):
                    d = abs(p.y1 - y)
                    if d < best_d and d <= self.cfg.y_tol * 2:
                        best_d, best_i = d, i
                if best_i >= 0:
                    py = prev[best_i].y1
                    sy = int(round(a * py + (1 - a) * y))
                    out.append(Segment(s.x1, sy, s.x2, sy))
                    used[best_i] = True
                else:
                    out.append(s)
            else:
                x = s.x1
                for i, p in enumerate(prev):
                    d = abs(p.x1 - x)
                    if d < best_d and d <= self.cfg.x_tol * 2:
                        best_d, best_i = d, i
                if best_i >= 0:
                    px = prev[best_i].x1
                    sx = int(round(a * px + (1 - a) * x))
                    out.append(Segment(sx, s.y1, sx, s.y2))
                    used[best_i] = True
                else:
                    out.append(s)
        return out


# ---- helpers ----------------------------------------------------------


def _merge_intervals(ivals: List[Tuple[int, int]], gap: int) -> List[Tuple[int, int]]:
    if not ivals:
        return []
    ivals = [(min(a, b), max(a, b)) for a, b in ivals]
    ivals.sort()
    out = [ivals[0]]
    for a, b in ivals[1:]:
        la, lb = out[-1]
        if a <= lb + gap:
            out[-1] = (la, max(lb, b))
        else:
            out.append((a, b))
    return out


def _overlap(a: Tuple[int, int], b: Tuple[int, int]) -> int:
    return max(0, min(a[1], b[1]) - max(a[0], b[0]))


def _merge_h_segments(segs: List[Segment], y_tol: int, gap: int) -> List[Segment]:
    if not segs:
        return []
    segs = sorted(
        segs, key=lambda s: (int(round((s.y1 + s.y2) * 0.5)), min(s.x1, s.x2))
    )
    buckets: List[List[Segment]] = []
    for s in segs:
        y = (s.y1 + s.y2) // 2
        placed = False
        for b in buckets:
            by = (b[0].y1 + b[0].y2) // 2
            if abs(by - y) <= y_tol:
                b.append(s)
                placed = True
                break
        if not placed:
            buckets.append([s])
    out: List[Segment] = []
    for b in buckets:
        y = int(round(np.median([(s.y1 + s.y2) * 0.5 for s in b])))
        ivals = [(min(s.x1, s.x2), max(s.x1, s.x2)) for s in b]
        ivals = _merge_intervals(ivals, gap)
        for x1, x2 in ivals:
            out.append(Segment(x1, y, x2, y))
    return out


def _merge_v_segments(segs: List[Segment], x_tol: int, gap: int) -> List[Segment]:
    if not segs:
        return []
    segs = sorted(
        segs, key=lambda s: (int(round((s.x1 + s.x2) * 0.5)), min(s.y1, s.y2))
    )
    buckets: List[List[Segment]] = []
    for s in segs:
        x = (s.x1 + s.x2) // 2
        placed = False
        for b in buckets:
            bx = (b[0].x1 + b[0].x2) // 2
            if abs(bx - x) <= x_tol:
                b.append(s)
                placed = True
                break
        if not placed:
            buckets.append([s])
    out: List[Segment] = []
    for b in buckets:
        x = int(round(np.median([(s.x1 + s.x2) * 0.5 for s in b])))
        ivals = [(min(s.y1, s.y2), max(s.y1, s.y2)) for s in b]
        ivals = _merge_intervals(ivals, gap)
        for y1, y2 in ivals:
            out.append(Segment(x, y1, x, y2))
    return out
