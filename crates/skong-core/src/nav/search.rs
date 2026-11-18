//! Route search: Dijkstra over *landing states* (start, or "just landed via edge e"), so the
//! walking cost between a landing x and the next take-off x is exact.
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::graph::SIM_DT;
use super::script::{Start, run_up};
use super::{Edge, MoveKind, NavGoal, NavGraph, Route, RouteStep, SurfaceId, TravelStyle};
use crate::controller::{Character, MoveInput};
use crate::geom::Vec2;

/// Fixed overhead per edge (s): prefer fewer moves at equal time.
const EDGE_COST: f32 = 0.05;
/// Extra cost (s) for flashy moves in a normal-mood route.
const FLASHY_COST: f32 = 0.25;
/// Longest fall predicted when planning from mid-air (s).
const MAX_FALL: f32 = 4.0;
/// Point goals: surfaces whose closest point is within this many px of the best are
/// considered equally close (then the cheapest wins).
const POINT_SLACK: f32 = 1.0;

pub(super) fn plan(g: &NavGraph, from: &Character, goal: NavGoal, style: TravelStyle) -> Option<Route> {
    let (start, start_x) = start_state(g, from)?;
    let v = from.params.gait_speed_px(style.gait());
    let n = 1 + g.edges.len();
    let mut cost = vec![f32::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    // State 0 = start, state 1 + e = landed via edge e.
    let state = |i: usize| -> (SurfaceId, f32) {
        if i == 0 { (start, start_x) } else { (g.edges[i - 1].to, g.edges[i - 1].land.x) }
    };

    let mut heap = BinaryHeap::new();
    cost[0] = 0.0;
    heap.push(Reverse((0u32, 0usize)));
    while let Some(Reverse((bits, i))) = heap.pop() {
        let c = f32::from_bits(bits);
        if c > cost[i] {
            continue;
        }
        let (s, x) = state(i);
        for (k, e) in g.edges_from(s).iter().enumerate() {
            if !style.allows(e) {
                continue;
            }
            let j = 1 + g.out_start[s] + k;
            let nc = c + approach_cost(from, e, x, v) + e.duration + penalty(e, style);
            if nc < cost[j] {
                cost[j] = nc;
                prev[j] = i;
                heap.push(Reverse((nc.to_bits(), j)));
            }
        }
    }

    // Pick the final state and the point on its surface.
    let mut best: Option<(f32, f32, usize, f32)> = None; // (distance, total cost, state, x)
    for (i, &c) in cost.iter().enumerate() {
        if !c.is_finite() {
            continue;
        }
        let (s, x) = state(i);
        let surf = &g.surfaces[s];
        let (dist, fx) = match goal {
            NavGoal::Point(p) => {
                let fx = surf.clamp(p.x);
                (Vec2::new(fx - p.x, surf.y - p.y).length(), fx)
            }
            NavGoal::OnWindow(id) => {
                if surf.owner != Some(id) {
                    continue;
                }
                // Stay near where we land, but not on the very edge.
                let m = (from.size.x + 8.0).min((surf.hi - surf.lo) * 0.5);
                (0.0, x.clamp(surf.lo + m, surf.hi - m))
            }
        };
        let total = c + (fx - x).abs() / v;
        let better = match best {
            None => true,
            Some((bd, bc, _, _)) => dist < bd - POINT_SLACK || (dist <= bd + POINT_SLACK && total < bc),
        };
        if better {
            best = Some((dist, total, i, fx));
        }
    }
    let (_, eta, last, fx) = best?;

    let mut steps = Vec::new();
    let mut i = last;
    while i != 0 {
        let e = g.edges[i - 1];
        steps.push(RouteStep { edge: e, from: g.surfaces[e.from], to: g.surfaces[e.to] });
        i = prev[i];
    }
    steps.reverse();
    let finish = g.surfaces[state(last).0];
    Some(Route {
        moves: steps.iter().map(|s| s.edge.kind).collect(),
        eta,
        end: Vec2::new(fx, finish.y),
        steps,
        finish: Some(finish),
        generation: g.generation,
    })
}

/// Surface and feet x to plan from: where the character stands, or where it will land.
fn start_state(g: &NavGraph, ch: &Character) -> Option<(SurfaceId, f32)> {
    if let Some(s) = g.surface_at(ch) {
        return Some((s, ch.pos.x));
    }
    let mut c = ch.clone();
    let mut events = Vec::new();
    for _ in 0..(MAX_FALL / SIM_DT) as usize {
        c.step(&MoveInput::default(), &g.terrain, SIM_DT, &mut events);
        events.clear();
        if c.is_grounded() {
            return g.surface_at(&c).map(|s| (s, c.pos.x));
        }
    }
    None
}

/// Time to walk from `x` to the edge's take-off (with a run-up when needed).
fn approach_cost(ch: &Character, e: &Edge, x: f32, v: f32) -> f32 {
    let s = e.script;
    let walk = (s.takeoff - x).abs() / v;
    match s.start {
        Start::Run => {
            let runup = run_up(ch, s.gait);
            let ahead = (s.takeoff - x) * s.dir.sign();
            if ahead < runup { walk + 2.0 * (runup - ahead.max(0.0)) / v } else { walk }
        }
        Start::Stand => walk + 0.15,
        Start::Pass => walk,
    }
}

fn penalty(e: &Edge, style: TravelStyle) -> f32 {
    let flashy = matches!(e.kind, MoveKind::DoubleJump | MoveKind::WallJump | MoveKind::Dash | MoveKind::ShadowDash);
    EDGE_COST + if style == TravelStyle::Normal && flashy { FLASHY_COST } else { 0.0 }
}
