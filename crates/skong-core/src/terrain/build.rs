//! Terrain construction. Placeholder: monitor floors + outer walls only.
//! TODO(phys_window-terrain): window platform boxes with occlusion clipping.
use super::*;

pub(super) fn build(desk: &DesktopSnapshot, _opts: &TerrainOptions) -> Terrain {
    let mut t = Terrain { generation: desk.generation, ..Default::default() };
    let mut b: Option<RectF> = None;
    for m in &desk.monitors {
        let r = m.rect.to_f();
        let w = m.work.to_f();
        b = Some(match b {
            None => r,
            Some(a) => RectF::new(a.left.min(r.left), a.top.min(r.top), a.right.max(r.right), a.bottom.max(r.bottom)),
        });
        t.ledges.push(Ledge { x0: w.left, x1: w.right, y: w.bottom, kind: LedgeKind::Ground, owner: None });
        t.walls.push(Wall { x: w.left, y0: r.top, y1: w.bottom, face: Facing::Right, owner: None });
        t.walls.push(Wall { x: w.right, y0: r.top, y1: w.bottom, face: Facing::Left, owner: None });
    }
    t.bounds = b.unwrap_or_default();
    t
}
