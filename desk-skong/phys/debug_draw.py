def draw_space_qt(p, space, to_screen=lambda x, y: (x, y), color=(0, 255, 0, 160)):
    """Very light visualizer: outlines segments and polys."""
    from PySide6 import QtGui

    pen = QtGui.QPen(QtGui.QColor(*color))
    pen.setWidth(1)
    p.setPen(pen)

    for s in space.shapes:
        if s.__class__.__name__ == "Segment":
            a = to_screen(*s.a)
            b = to_screen(*s.b)
            p.drawLine(a[0], a[1], b[0], b[1])
        elif s.__class__.__name__ == "Poly":
            vs = [to_screen(*v) for v in s.get_vertices()]
            if vs:
                for i in range(len(vs)):
                    x1, y1 = vs[i]
                    x2, y2 = vs[(i + 1) % len(vs)]
                    p.drawLine(x1, y1, x2, y2)
