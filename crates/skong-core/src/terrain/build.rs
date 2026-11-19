//! Terrain construction: monitor floors and outer walls, plus one platform box per
//! listed window with occlusion clipping.
//!
//! All edge arithmetic is done on integer pixel rows/columns (half-open, like
//! [`RectI`]) and converted to `f32` at the end:
//!
//! * a window's **top ledge** lives on pixel row `top`, spanning columns `left..right`;
//! * its **left wall** lives on column `left`, its **right wall** on column `right - 1`
//!   (the wall line itself is at `x = right`), both spanning rows `top..bottom`.
//!
//! An edge pixel is visible when it lies inside some monitor work area and no window
//! earlier in the (front-to-back) list contains it. So a front window whose `bottom`
//! equals a ledge's `y` does not hide that ledge, but one whose `top` equals it does.
//!
//! Cost is O(n² · k) for n windows and k fragments per edge — microseconds for a
//! realistic desktop.
use super::*;
use crate::desktop::{Monitor, WindowInfo};
use crate::geom::RectI;

/// Half-open integer interval `start..end`.
pub(super) type Span = (i32, i32);

pub(super) fn build(desk: &DesktopSnapshot, ledges: &[ContentLedge], walls: &[ContentWallSeg], opts: &TerrainOptions) -> Terrain {
    let mut t = Terrain { generation: desk.generation, ..Default::default() };
    t.bounds = monitor_bounds(&desk.monitors);
    add_monitor_terrain(&mut t, &desk.monitors);

    let boxes: Vec<&WindowInfo> = desk.windows.iter().filter(|w| is_platform(w, opts)).collect();
    for (i, w) in boxes.iter().enumerate() {
        add_window_terrain(&mut t, w, &boxes[..i], &desk.monitors, opts);
    }
    t.windows = boxes.iter().map(|w| WindowBox { id: w.id, rect: w.rect.to_f() }).collect();
    add_content_ledges(&mut t, desk, ledges, opts);
    add_content_walls(&mut t, desk, walls, opts);
    t
}

/// Windows in front of a content owner (fullscreen and tiny ones included since they do
/// cover pixels); `None` when the content must be dropped (owner gone or fullscreen,
/// wallpaper content disabled).
fn content_front<'a>(all: &'a [&'a WindowInfo], owner: Option<WindowId>, opts: &TerrainOptions) -> Option<&'a [&'a WindowInfo]> {
    match owner {
        Some(id) => match all.iter().position(|w| w.id == id) {
            Some(i) if !all[i].fullscreen => Some(&all[..i]),
            _ => None, // owner gone, or fullscreen (games, video)
        },
        None if opts.include_wallpaper_content => Some(all),
        None => None,
    }
}

/// Content ledges: visible parts (work areas minus every window in front of the owner),
/// minus stretches that duplicate a frame/ground ledge within `content_dedupe_px`.
fn add_content_ledges(t: &mut Terrain, desk: &DesktopSnapshot, content: &[ContentLedge], opts: &TerrainOptions) {
    let all: Vec<&WindowInfo> = desk.windows.iter().collect();
    let frame_ledges = t.ledges.len();
    for c in content {
        let Some(front) = content_front(&all, c.owner, opts) else { continue };
        let mut spans = visible_on_row(c.y, (c.x0, c.x1), front, &desk.monitors);
        let near = t.ledges[..frame_ledges]
            .iter()
            .filter(|l| (l.y - c.y as f32).abs() <= opts.content_dedupe_px)
            .map(|l| (l.x0.floor() as i32, l.x1.ceil() as i32));
        spans = subtract_all(spans, near);
        for (x0, x1) in spans {
            if ((x1 - x0) as f32) >= opts.min_content_len {
                t.ledges.push(Ledge { x0: x0 as f32, x1: x1 as f32, y: c.y as f32, kind: LedgeKind::Content, owner: c.owner });
            }
        }
    }
}

/// Content walls: visible parts of the solid column next to the wall line, minus
/// stretches duplicating a window/monitor wall within `content_dedupe_px`, minus
/// openings narrower than `min_content_gap`.
fn add_content_walls(t: &mut Terrain, desk: &DesktopSnapshot, content: &[ContentWallSeg], opts: &TerrainOptions) {
    let all: Vec<&WindowInfo> = desk.windows.iter().collect();
    let mut kept: Vec<ContentWallSeg> = Vec::new();
    for c in content {
        let Some(front) = content_front(&all, c.owner, opts) else { continue };
        let solid = if c.face == Facing::Left { c.x } else { c.x - 1 };
        let mut spans = visible_on_col(solid, (c.y0, c.y1), front, &desk.monitors);
        let near = t
            .walls
            .iter()
            .filter(|w| (w.x - c.x as f32).abs() <= opts.content_dedupe_px)
            .map(|w| (w.y0.floor() as i32, w.y1.ceil() as i32));
        spans = subtract_all(spans, near);
        kept.extend(spans.into_iter().map(|(y0, y1)| ContentWallSeg { y0, y1, ..*c }));
    }
    // Narrow openings: right-facing wall at xa (solid on its left), left-facing at xb > xa
    // (solid on its right), overlapping vertically, closer than min_content_gap.
    let narrow = |a: &ContentWallSeg, b: &ContentWallSeg| {
        a.face == Facing::Right && b.face == Facing::Left && b.x > a.x && ((b.x - a.x) as f32) < opts.min_content_gap
    };
    let mut cuts: Vec<Vec<Span>> = vec![Vec::new(); kept.len()];
    for (i, a) in kept.iter().enumerate() {
        for (j, b) in kept.iter().enumerate() {
            if narrow(a, b) {
                let o = (a.y0.max(b.y0), a.y1.min(b.y1));
                if o.0 < o.1 {
                    cuts[i].push(o);
                    cuts[j].push(o);
                }
            }
        }
    }
    for (c, cut) in kept.iter().zip(cuts) {
        for (y0, y1) in subtract_all(vec![(c.y0, c.y1)], cut.into_iter()) {
            if ((y1 - y0) as f32) >= opts.min_content_wall_len {
                t.walls.push(Wall { x: c.x as f32, y0: y0 as f32, y1: y1 as f32, face: c.face, owner: c.owner, kind: WallKind::Content });
            }
        }
    }
}

/// Windows that become platform boxes (and occluders). Fullscreen and tiny windows
/// are ignored entirely: they neither produce edges nor hide other windows' edges.
fn is_platform(w: &WindowInfo, opts: &TerrainOptions) -> bool {
    !w.fullscreen && w.rect.width() >= opts.min_window_size && w.rect.height() >= opts.min_window_size
}

/// Union of all monitor rects (empty when there are no monitors).
fn monitor_bounds(monitors: &[Monitor]) -> RectF {
    monitors
        .iter()
        .map(|m| m.rect)
        .reduce(|a, r| RectI::new(a.left.min(r.left), a.top.min(r.top), a.right.max(r.right), a.bottom.max(r.bottom)))
        .map(|r| r.to_f())
        .unwrap_or_default()
}

/// Ground along every work-area bottom and inward-facing walls on work-area sides,
/// except where an adjacent monitor's work area continues the playable space.
fn add_monitor_terrain(t: &mut Terrain, monitors: &[Monitor]) {
    for (i, m) in monitors.iter().enumerate() {
        let w = m.work;
        if w.is_empty() {
            continue;
        }
        let others = || monitors.iter().enumerate().filter(move |(j, _)| *j != i).map(|(_, o)| o.work);

        // Ground: a neighbour whose work area contains row `w.bottom` lets you fall through.
        let ground = subtract_all(vec![(w.left, w.right)], others().filter_map(|o| row_span(&o, w.bottom)));
        for (x0, x1) in ground {
            t.ledges.push(Ledge { x0: x0 as f32, x1: x1 as f32, y: w.bottom as f32, kind: LedgeKind::Ground, owner: None });
        }

        // Left wall faces right; open where a neighbour covers column `left - 1`.
        let left = subtract_all(vec![(w.top, w.bottom)], others().filter_map(|o| col_span(&o, w.left - 1)));
        push_walls(t, w.left, &left, Facing::Right, None, 0.0);
        // Right wall faces left; open where a neighbour covers column `right`.
        let right = subtract_all(vec![(w.top, w.bottom)], others().filter_map(|o| col_span(&o, w.right)));
        push_walls(t, w.right, &right, Facing::Left, None, 0.0);
    }
}

/// Top ledge and side walls of one window, clipped to work areas and occluders.
fn add_window_terrain(t: &mut Terrain, w: &WindowInfo, front: &[&WindowInfo], monitors: &[Monitor], opts: &TerrainOptions) {
    let r = w.rect;
    let owner = Some(w.id);

    for (x0, x1) in visible_on_row(r.top, (r.left, r.right), front, monitors) {
        if ((x1 - x0) as f32) >= opts.min_ledge_len {
            t.ledges.push(Ledge { x0: x0 as f32, x1: x1 as f32, y: r.top as f32, kind: LedgeKind::OneWay, owner });
        }
    }

    let left = visible_on_col(r.left, (r.top, r.bottom), front, monitors);
    push_walls(t, r.left, &left, Facing::Left, owner, opts.min_wall_len);
    let right = visible_on_col(r.right - 1, (r.top, r.bottom), front, monitors);
    push_walls(t, r.right, &right, Facing::Right, owner, opts.min_wall_len);
}

fn push_walls(t: &mut Terrain, x: i32, spans: &[Span], face: Facing, owner: Option<WindowId>, min_len: f32) {
    let kind = if owner.is_some() { WallKind::Window } else { WallKind::Monitor };
    for &(y0, y1) in spans {
        if ((y1 - y0) as f32) >= min_len {
            t.walls.push(Wall { x: x as f32, y0: y0 as f32, y1: y1 as f32, face, owner, kind });
        }
    }
}

/// Visible parts of `span` (columns) on pixel row `y`.
pub(super) fn visible_on_row(y: i32, span: Span, front: &[&WindowInfo], monitors: &[Monitor]) -> Vec<Span> {
    let allowed = clip_to(span, monitors.iter().filter_map(|m| row_span(&m.work, y)));
    subtract_all(allowed, front.iter().filter_map(|o| row_span(&o.rect, y)))
}

/// Visible parts of `span` (rows) on pixel column `x`.
pub(super) fn visible_on_col(x: i32, span: Span, front: &[&WindowInfo], monitors: &[Monitor]) -> Vec<Span> {
    let allowed = clip_to(span, monitors.iter().filter_map(|m| col_span(&m.work, x)));
    subtract_all(allowed, front.iter().filter_map(|o| col_span(&o.rect, x)))
}

/// Columns of `r` on row `y`, if `r` contains that row.
fn row_span(r: &RectI, y: i32) -> Option<Span> {
    (r.top <= y && y < r.bottom && r.left < r.right).then_some((r.left, r.right))
}

/// Rows of `r` on column `x`, if `r` contains that column.
fn col_span(r: &RectI, x: i32) -> Option<Span> {
    (r.left <= x && x < r.right && r.top < r.bottom).then_some((r.top, r.bottom))
}

/// Intersect `span` with the union of `areas`. Result is sorted and disjoint.
fn clip_to(span: Span, areas: impl Iterator<Item = Span>) -> Vec<Span> {
    let mut parts: Vec<Span> = areas.map(|(a, b)| (a.max(span.0), b.min(span.1))).filter(|(a, b)| a < b).collect();
    parts.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(parts.len());
    for (a, b) in parts {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Remove every cut from a sorted, disjoint span list.
pub(super) fn subtract_all(mut spans: Vec<Span>, cuts: impl Iterator<Item = Span>) -> Vec<Span> {
    for cut in cuts {
        if spans.is_empty() {
            break;
        }
        spans = spans.into_iter().flat_map(|s| subtract(s, cut)).collect();
    }
    spans
}

/// `s` minus `cut`: zero, one or two pieces.
fn subtract(s: Span, cut: Span) -> impl Iterator<Item = Span> {
    let before = (s.0, s.1.min(cut.0));
    let after = (s.0.max(cut.1), s.1);
    [before, after].into_iter().filter(|(a, b)| a < b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: TerrainOptions = TerrainOptions {
        min_ledge_len: 1.0,
        min_wall_len: 1.0,
        min_window_size: 1,
        min_content_len: 1.0,
        content_dedupe_px: 3.0,
        include_wallpaper_content: false,
        min_content_wall_len: 1.0,
        min_content_gap: 40.0,
    };

    fn mon(rect: RectI, work: RectI) -> Monitor {
        Monitor { rect, work, primary: false, scale_milli: 1000 }
    }

    /// 1000x800 monitor with a 40 px taskbar at the bottom.
    fn one_monitor() -> Vec<Monitor> {
        vec![mon(RectI::new(0, 0, 1000, 800), RectI::new(0, 0, 1000, 760))]
    }

    fn two_monitors_side_by_side() -> Vec<Monitor> {
        vec![
            mon(RectI::new(0, 0, 1000, 800), RectI::new(0, 0, 1000, 760)),
            // Taller monitor on the right, offset upward, no taskbar.
            mon(RectI::new(1000, -200, 2000, 900), RectI::new(1000, -200, 2000, 900)),
        ]
    }

    fn win(id: u64, l: i32, t: i32, r: i32, b: i32) -> WindowInfo {
        WindowInfo {
            id,
            rect: RectI::new(l, t, r, b),
            title: format!("w{id}"),
            class: String::new(),
            process: String::new(),
            pid: 0,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        }
    }

    fn build_with(monitors: Vec<Monitor>, windows: Vec<WindowInfo>, opts: &TerrainOptions) -> Terrain {
        build(&DesktopSnapshot { generation: 7, monitors, windows }, &[], &[], opts)
    }

    fn ledges_of(t: &Terrain, id: u64) -> Vec<(f32, f32, f32)> {
        t.ledges.iter().filter(|l| l.owner == Some(id)).map(|l| (l.x0, l.x1, l.y)).collect()
    }

    fn walls_of(t: &Terrain, id: u64, face: Facing) -> Vec<(f32, f32, f32)> {
        t.walls.iter().filter(|w| w.owner == Some(id) && w.face == face).map(|w| (w.x, w.y0, w.y1)).collect()
    }

    fn monitor_walls(t: &Terrain) -> Vec<(f32, f32, f32, Facing)> {
        let mut v: Vec<_> = t.walls.iter().filter(|w| w.owner.is_none()).map(|w| (w.x, w.y0, w.y1, w.face)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        v
    }

    fn grounds(t: &Terrain) -> Vec<(f32, f32, f32)> {
        t.ledges.iter().filter(|l| l.kind == LedgeKind::Ground).map(|l| (l.x0, l.x1, l.y)).collect()
    }

    #[test]
    fn empty_desktop() {
        let t = build_with(vec![], vec![], &W);
        assert!(t.ledges.is_empty() && t.walls.is_empty() && t.windows.is_empty());
        assert_eq!(t.bounds, RectF::default());
    }

    #[test]
    fn single_monitor_floor_and_walls() {
        let t = build_with(one_monitor(), vec![], &W);
        assert_eq!(t.generation, 7);
        assert_eq!(t.bounds, RectF::new(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(grounds(&t), vec![(0.0, 1000.0, 760.0)]);
        assert!(t.ledges.iter().all(|l| l.owner.is_none()));
        assert_eq!(monitor_walls(&t), vec![(0.0, 0.0, 760.0, Facing::Right), (1000.0, 0.0, 760.0, Facing::Left)]);
    }

    #[test]
    fn single_window_box() {
        let t = build_with(one_monitor(), vec![win(1, 100, 200, 400, 500)], &W);
        assert_eq!(ledges_of(&t, 1), vec![(100.0, 400.0, 200.0)]);
        assert!(t.ledges.iter().filter(|l| l.owner == Some(1)).all(|l| l.kind == LedgeKind::OneWay));
        assert_eq!(walls_of(&t, 1, Facing::Left), vec![(100.0, 200.0, 500.0)]);
        assert_eq!(walls_of(&t, 1, Facing::Right), vec![(400.0, 200.0, 500.0)]);
        assert_eq!(t.windows, vec![WindowBox { id: 1, rect: RectF::new(100.0, 200.0, 400.0, 500.0) }]);
        assert_eq!(t.window_rect(1), Some(RectF::new(100.0, 200.0, 400.0, 500.0)));
    }

    #[test]
    fn front_window_splits_ledge_of_back_window() {
        // Window 1 (front) straddles the top edge of window 2 (back) in the middle.
        let t = build_with(one_monitor(), vec![win(1, 300, 100, 500, 400), win(2, 100, 200, 800, 600)], &W);
        assert_eq!(ledges_of(&t, 2), vec![(100.0, 300.0, 200.0), (500.0, 800.0, 200.0)]);
        assert_eq!(ledges_of(&t, 1), vec![(300.0, 500.0, 100.0)]);
        // The front window spans columns 300..500 only, so the back walls stay whole.
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(100.0, 200.0, 600.0)]);
        assert_eq!(walls_of(&t, 2, Facing::Right), vec![(800.0, 200.0, 600.0)]);
    }

    #[test]
    fn back_window_does_not_clip_front_window() {
        let t = build_with(one_monitor(), vec![win(1, 100, 200, 800, 600), win(2, 300, 100, 500, 400)], &W);
        assert_eq!(ledges_of(&t, 1), vec![(100.0, 800.0, 200.0)]);
        // Window 2's walls disappear from row 200 down (behind window 1).
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(300.0, 100.0, 200.0)]);
        assert_eq!(walls_of(&t, 2, Facing::Right), vec![(500.0, 100.0, 200.0)]);
        assert_eq!(ledges_of(&t, 2), vec![(300.0, 500.0, 100.0)]);
    }

    #[test]
    fn fully_covered_window_has_no_edges_but_is_listed() {
        let t = build_with(one_monitor(), vec![win(1, 0, 0, 900, 700), win(2, 100, 100, 300, 300)], &W);
        assert!(ledges_of(&t, 2).is_empty());
        assert!(walls_of(&t, 2, Facing::Left).is_empty() && walls_of(&t, 2, Facing::Right).is_empty());
        assert_eq!(t.windows.iter().map(|w| w.id).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn front_bottom_touching_ledge_does_not_occlude() {
        // Front window's bottom == back window's top: the ledge row is not covered.
        let t = build_with(one_monitor(), vec![win(1, 0, 100, 500, 200), win(2, 100, 200, 400, 400)], &W);
        assert_eq!(ledges_of(&t, 2), vec![(100.0, 400.0, 200.0)]);
    }

    #[test]
    fn front_top_equal_to_ledge_occludes() {
        // Same top row: the front window owns that stretch of ledge.
        let t = build_with(one_monitor(), vec![win(1, 200, 200, 300, 400), win(2, 100, 200, 400, 400)], &W);
        assert_eq!(ledges_of(&t, 2), vec![(100.0, 200.0, 200.0), (300.0, 400.0, 200.0)]);
    }

    #[test]
    fn side_by_side_touching_windows() {
        // Window 1 ends at x=300, window 2 starts at x=300. Neither covers the other.
        let t = build_with(one_monitor(), vec![win(1, 100, 100, 300, 400), win(2, 300, 200, 600, 500)], &W);
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(300.0, 200.0, 500.0)]);
        assert_eq!(walls_of(&t, 1, Facing::Right), vec![(300.0, 100.0, 400.0)]);
        assert_eq!(ledges_of(&t, 2), vec![(300.0, 600.0, 200.0)]);
    }

    #[test]
    fn right_wall_boundaries() {
        // Front window starting at back's last column (right - 1) hides that wall...
        let t = build_with(one_monitor(), vec![win(1, 299, 0, 500, 300), win(2, 100, 100, 300, 400)], &W);
        assert_eq!(walls_of(&t, 2, Facing::Right), vec![(300.0, 300.0, 400.0)]);
        // ...one starting exactly at back's right does not.
        let t = build_with(one_monitor(), vec![win(1, 300, 0, 500, 300), win(2, 100, 100, 300, 400)], &W);
        assert_eq!(walls_of(&t, 2, Facing::Right), vec![(300.0, 100.0, 400.0)]);
    }

    #[test]
    fn left_wall_boundaries() {
        // Front window ending at back's left does not cover it; one ending one past does.
        let t = build_with(one_monitor(), vec![win(1, 0, 0, 100, 300), win(2, 100, 100, 300, 400)], &W);
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(100.0, 100.0, 400.0)]);
        let t = build_with(one_monitor(), vec![win(1, 0, 0, 101, 300), win(2, 100, 100, 300, 400)], &W);
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(100.0, 300.0, 400.0)]);
    }

    #[test]
    fn stacked_z_order_three_windows() {
        let windows = vec![
            win(1, 150, 50, 250, 150),  // front, covers x 150..250 on row 100
            win(2, 350, 80, 450, 120),  // middle, covers x 350..450 on row 100
            win(3, 100, 100, 500, 300), // back
        ];
        let t = build_with(one_monitor(), windows, &W);
        assert_eq!(ledges_of(&t, 3), vec![(100.0, 150.0, 100.0), (250.0, 350.0, 100.0), (450.0, 500.0, 100.0)]);
        // Window 2 is unaffected by window 3 (behind it) and window 1 (disjoint columns).
        assert_eq!(ledges_of(&t, 2), vec![(350.0, 450.0, 80.0)]);
    }

    #[test]
    fn partially_off_screen_window_is_clipped() {
        // Sticks out left, above and below the work area.
        let t = build_with(one_monitor(), vec![win(1, -200, -50, 300, 900)], &W);
        assert!(ledges_of(&t, 1).is_empty(), "top row is off-screen");
        assert!(walls_of(&t, 1, Facing::Left).is_empty(), "left column is off-screen");
        assert_eq!(walls_of(&t, 1, Facing::Right), vec![(300.0, 0.0, 760.0)], "stops at the taskbar");

        let t = build_with(one_monitor(), vec![win(2, 800, 300, 1300, 500)], &W);
        assert_eq!(ledges_of(&t, 2), vec![(800.0, 1000.0, 300.0)]);
        assert!(walls_of(&t, 2, Facing::Right).is_empty());
    }

    #[test]
    fn nothing_over_the_taskbar() {
        // Top exactly at the work-area bottom: entirely on the taskbar.
        let t = build_with(one_monitor(), vec![win(1, 100, 760, 400, 800)], &W);
        assert!(ledges_of(&t, 1).is_empty());
        assert!(walls_of(&t, 1, Facing::Left).is_empty());
        // One row higher: ledge present, walls one px tall.
        let t = build_with(one_monitor(), vec![win(1, 100, 759, 400, 800)], &W);
        assert_eq!(ledges_of(&t, 1), vec![(100.0, 400.0, 759.0)]);
        assert_eq!(walls_of(&t, 1, Facing::Left), vec![(100.0, 759.0, 760.0)]);
    }

    #[test]
    fn fullscreen_and_tiny_windows_are_skipped() {
        let mut fs = win(1, 0, 0, 1000, 800);
        fs.fullscreen = true;
        let opts = TerrainOptions::default();
        let t = build_with(one_monitor(), vec![fs, win(2, 10, 10, 40, 400), win(3, 100, 100, 300, 300)], &opts);
        assert_eq!(t.windows.iter().map(|w| w.id).collect::<Vec<_>>(), vec![3]);
        assert!(t.ledges.iter().all(|l| l.owner.is_none() || l.owner == Some(3)));
        assert!(t.walls.iter().all(|w| w.owner.is_none() || w.owner == Some(3)));
        // Skipped windows do not occlude.
        assert_eq!(ledges_of(&t, 3), vec![(100.0, 300.0, 100.0)]);
    }

    #[test]
    fn short_fragments_are_dropped() {
        let opts = TerrainOptions { min_ledge_len: 24.0, min_wall_len: 30.0, ..TerrainOptions::default() };
        // Front window leaves 20 px of back ledge on the left and 100 px on the right.
        let t = build_with(one_monitor(), vec![win(1, 120, 50, 400, 225), win(2, 100, 200, 500, 400)], &opts);
        assert_eq!(ledges_of(&t, 2), vec![(400.0, 500.0, 200.0)]);
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(100.0, 200.0, 400.0)]);
        // Front window hides all but 25 px of the back window's right wall.
        let t = build_with(one_monitor(), vec![win(1, 350, 0, 600, 200), win(2, 100, 175, 400, 225)], &opts);
        assert!(walls_of(&t, 2, Facing::Right).is_empty());
        assert_eq!(walls_of(&t, 2, Facing::Left), vec![(100.0, 175.0, 225.0)]);
        assert_eq!(ledges_of(&t, 2), vec![(100.0, 350.0, 175.0)]);
    }

    #[test]
    fn multi_monitor_no_wall_where_monitors_touch() {
        let t = build_with(two_monitors_side_by_side(), vec![], &W);
        assert_eq!(t.bounds, RectF::new(0.0, -200.0, 2000.0, 900.0));
        assert_eq!(grounds(&t), vec![(0.0, 1000.0, 760.0), (1000.0, 2000.0, 900.0)]);
        assert_eq!(
            monitor_walls(&t),
            vec![
                (0.0, 0.0, 760.0, Facing::Right),
                // Right monitor's left wall only where the left work area doesn't continue.
                (1000.0, -200.0, 0.0, Facing::Right),
                (1000.0, 760.0, 900.0, Facing::Right),
                (2000.0, -200.0, 900.0, Facing::Left),
            ]
        );
    }

    #[test]
    fn multi_monitor_stacked_ground_opens_into_lower_monitor() {
        let monitors = vec![
            mon(RectI::new(0, -600, 800, 0), RectI::new(0, -600, 800, 0)),
            mon(RectI::new(-200, 0, 1200, 900), RectI::new(-200, 0, 1200, 860)),
        ];
        let t = build_with(monitors, vec![], &W);
        // The upper monitor's floor is fully open: the lower work area continues below it.
        assert_eq!(grounds(&t), vec![(-200.0, 1200.0, 860.0)]);
    }

    #[test]
    fn window_spanning_two_monitors() {
        let t = build_with(two_monitors_side_by_side(), vec![win(1, 800, 700, 1200, 850)], &W);
        // Ledge is continuous across the seam.
        assert_eq!(ledges_of(&t, 1), vec![(800.0, 1200.0, 700.0)]);
        // Left wall on the left monitor stops at its taskbar; right wall on the taller one.
        assert_eq!(walls_of(&t, 1, Facing::Left), vec![(800.0, 700.0, 760.0)]);
        assert_eq!(walls_of(&t, 1, Facing::Right), vec![(1200.0, 700.0, 850.0)]);
    }

    #[test]
    fn window_partly_outside_offset_monitors() {
        // Right monitor starts lower: the region x >= 1000, y < 400 is off-desktop.
        let monitors = vec![
            mon(RectI::new(0, 0, 1000, 800), RectI::new(0, 0, 1000, 800)),
            mon(RectI::new(1000, 400, 2000, 1200), RectI::new(1000, 400, 2000, 1200)),
        ];
        let t = build_with(monitors, vec![win(1, 900, 300, 1100, 600)], &W);
        assert_eq!(ledges_of(&t, 1), vec![(900.0, 1000.0, 300.0)]);
        assert_eq!(walls_of(&t, 1, Facing::Right), vec![(1100.0, 400.0, 600.0)]);
    }

    #[test]
    fn span_helpers() {
        assert_eq!(clip_to((0, 100), [(50, 80), (-10, 20), (70, 90)].into_iter()), vec![(0, 20), (50, 90)]);
        assert_eq!(clip_to((0, 100), [(20, 30), (30, 40)].into_iter()), vec![(20, 40)]);
        assert_eq!(subtract_all(vec![(0, 100)], [(10, 20), (90, 200), (-5, 1)].into_iter()), vec![(1, 10), (20, 90)]);
        assert_eq!(subtract_all(vec![(0, 10)], [(0, 10)].into_iter()), vec![]);
        assert_eq!(subtract_all(vec![(0, 10)], [(10, 20)].into_iter()), vec![(0, 10)]);
    }

    #[test]
    fn many_windows_build_fast() {
        let windows: Vec<_> = (0..200)
            .map(|i: i32| {
                let (x, y) = ((i * 37) % 1700, (i * 53) % 700 - 150);
                win(i as u64 + 1, x, y, x + 300, y + 250)
            })
            .collect();
        let desk = DesktopSnapshot { generation: 1, monitors: two_monitors_side_by_side(), windows };
        let start = std::time::Instant::now();
        let t = build(&desk, &[], &[], &TerrainOptions::default());
        let elapsed = start.elapsed();
        assert_eq!(t.windows.len(), 200);
        // Generous bound for unoptimised CI builds; typical is far below 1 ms.
        assert!(elapsed.as_millis() < 50, "build took {elapsed:?}");
    }

    fn content_of(t: &Terrain) -> Vec<(f32, f32, f32, Option<u64>)> {
        t.ledges.iter().filter(|l| l.kind == LedgeKind::Content).map(|l| (l.x0, l.x1, l.y, l.owner)).collect()
    }

    fn cl(x0: i32, x1: i32, y: i32, owner: Option<u64>) -> ContentLedge {
        ContentLedge { x0, x1, y, owner }
    }

    fn build_content(windows: Vec<WindowInfo>, content: &[ContentLedge], opts: &TerrainOptions) -> Terrain {
        build(&DesktopSnapshot { generation: 1, monitors: one_monitor(), windows }, content, &[], opts)
    }

    #[test]
    fn content_is_clipped_occluded_and_deduped() {
        let opts = TerrainOptions { include_wallpaper_content: true, ..W };
        let windows = vec![win(1, 300, 100, 500, 400), win(2, 100, 200, 800, 600)];
        let content = [
            cl(150, 700, 300, Some(2)), // partly behind window 1
            cl(320, 480, 300, Some(1)),
            cl(100, 800, 201, Some(2)), // window 2's own frame: duplicate of its top ledge
            cl(0, 1000, 770, None),     // on the taskbar
            cl(0, 1000, 758, None),     // duplicate of the taskbar-top ground
            cl(600, 900, 650, None),    // wallpaper below window 2
            cl(50, 150, 250, None),     // wallpaper, partly behind window 2
        ];
        let t = build_content(windows, &content, &opts);
        assert_eq!(
            content_of(&t),
            vec![
                (150.0, 300.0, 300.0, Some(2)),
                (500.0, 700.0, 300.0, Some(2)),
                (320.0, 480.0, 300.0, Some(1)),
                (600.0, 900.0, 650.0, None),
                (50.0, 100.0, 250.0, None),
            ]
        );
        // Window terrain is unchanged by content.
        assert_eq!(ledges_of(&t, 2).len(), 2 + 2);
    }

    #[test]
    fn wallpaper_content_is_off_by_default() {
        let t = build_content(vec![], &[cl(100, 400, 300, None)], &TerrainOptions::default());
        assert!(content_of(&t).is_empty());
        let t = build_content(vec![], &[cl(100, 400, 300, None)], &TerrainOptions { include_wallpaper_content: true, ..TerrainOptions::default() });
        assert_eq!(content_of(&t), vec![(100.0, 400.0, 300.0, None)]);
    }

    #[test]
    fn content_inside_or_behind_fullscreen_windows_is_dropped() {
        let mut fs = win(1, 0, 0, 1000, 800);
        fs.fullscreen = true;
        let opts = TerrainOptions { include_wallpaper_content: true, ..W };
        let content = [cl(100, 400, 300, Some(1)), cl(100, 400, 350, Some(2)), cl(100, 400, 500, None)];
        let t = build_content(vec![fs, win(2, 50, 200, 600, 600)], &content, &opts);
        assert!(content_of(&t).is_empty(), "{:?}", content_of(&t));
    }

    #[test]
    fn content_needs_a_live_owner_and_minimum_length() {
        let opts = TerrainOptions::default();
        let content = [cl(100, 400, 300, Some(9)), cl(100, 120, 300, Some(1)), cl(100, 400, 320, Some(1))];
        let t = build_content(vec![win(1, 50, 200, 600, 600)], &content, &opts);
        assert_eq!(content_of(&t), vec![(100.0, 400.0, 320.0, Some(1))]);
        assert!(Terrain::from_desktop(&DesktopSnapshot { generation: 1, monitors: one_monitor(), windows: vec![win(1, 50, 200, 600, 600)] }, &opts)
            .ledges
            .iter()
            .all(|l| l.kind != LedgeKind::Content));
    }

    fn cw(x: i32, y0: i32, y1: i32, face: Facing, owner: Option<u64>) -> ContentWallSeg {
        ContentWallSeg { x, y0, y1, face, owner }
    }

    fn content_walls(t: &Terrain) -> Vec<(f32, f32, f32, Facing)> {
        t.walls.iter().filter(|w| w.kind == WallKind::Content).map(|w| (w.x, w.y0, w.y1, w.face)).collect()
    }

    #[test]
    fn content_walls_are_clipped_deduped_and_kept_out_of_narrow_gaps() {
        let opts = TerrainOptions { min_content_wall_len: 20.0, ..TerrainOptions::default() };
        let windows = vec![win(1, 300, 100, 500, 300), win(2, 100, 200, 800, 700)];
        let walls = [
            cw(600, 150, 790, Facing::Left, Some(2)), // runs onto the taskbar
            cw(400, 250, 600, Facing::Left, Some(2)), // partly behind window 1
            cw(101, 300, 500, Facing::Left, Some(2)), // duplicate of window 2 frame
            // A 20 px wide opening between two strokes: cut where they overlap.
            cw(650, 400, 600, Facing::Right, Some(2)),
            cw(670, 450, 650, Facing::Left, Some(2)),
        ];
        let t = build(&DesktopSnapshot { generation: 1, monitors: one_monitor(), windows }, &[], &walls, &opts);
        let mut got = content_walls(&t);
        got.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        assert_eq!(
            got,
            vec![
                (400.0, 300.0, 600.0, Facing::Left),
                (600.0, 150.0, 760.0, Facing::Left),
                (650.0, 400.0, 450.0, Facing::Right),
                (670.0, 600.0, 650.0, Facing::Left),
            ]
        );
        assert!(t.walls.iter().filter(|w| w.kind == WallKind::Window).all(|w| w.owner.is_some()));
        assert!(t.walls.iter().filter(|w| w.kind == WallKind::Monitor).all(|w| w.owner.is_none()));
    }
}
