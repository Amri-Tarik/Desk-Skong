from __future__ import annotations

from typing import Iterable, Optional

from PySide6 import QtCore, QtGui


def _y_up_to_qt(win_h: int, y_up: float) -> int:
    # pymunk is y-up; Qt is y-down
    return int(round(win_h - y_up))


def _draw_seg_list(
    p: QtGui.QPainter,
    win_h: int,
    shapes: Iterable,
    color: QtGui.QColor,
    *,
    default_radius: float = 2.0,
    width_scale: float = 1.0,
    override_radius: Optional[float] = None,
) -> None:
    pen = QtGui.QPen(color)
    pen.setCapStyle(QtCore.Qt.PenCapStyle.RoundCap)
    pen.setJoinStyle(QtCore.Qt.PenJoinStyle.RoundJoin)

    for sh in shapes:
        a = sh.a  # Vec2d
        b = sh.b
        r = (
            float(override_radius)
            if override_radius is not None
            else float(getattr(sh, "radius", default_radius) or default_radius)
        )
        w = max(1, int(round(2.0 * r * width_scale)))
        pen.setWidth(w)
        p.setPen(pen)
        p.drawLine(
            int(round(a.x)),
            _y_up_to_qt(win_h, a.y),
            int(round(b.x)),
            _y_up_to_qt(win_h, b.y),
        )


def draw_segments_capsules(
    p: QtGui.QPainter,
    win_w: int,
    win_h: int,
    platforms,
    walls,
    *,
    default_radius: float = 2.0,
    width_scale: float = 1.0,
    override_radius: Optional[float] = None,
    color_plat: QtGui.QColor = QtGui.QColor(70, 200, 255, 220),
    color_wall: QtGui.QColor = QtGui.QColor(255, 140, 0, 220),
) -> None:
    """Render platform & wall segments as thick (capsule-like) lines.
    Each shape's radius controls thickness unless override_radius is supplied."""
    _draw_seg_list(
        p,
        win_h,
        platforms,
        color_plat,
        default_radius=default_radius,
        width_scale=width_scale,
        override_radius=override_radius,
    )
    _draw_seg_list(
        p,
        win_h,
        walls,
        color_wall,
        default_radius=default_radius,
        width_scale=width_scale,
        override_radius=override_radius,
    )
