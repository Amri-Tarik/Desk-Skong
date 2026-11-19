//! The pet's debug overlay toggle: terrain, nav graph and current route drawn over the
//! desktop (see `skong_win::debug_overlay`). Off by default and free when off; when on it
//! redraws only when the terrain, the nav graph or the route changes.
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use skong_core::nav::{self, NavGraph, Route};
use skong_core::terrain::Terrain;
use skong_ipc::OverlayLayers;
use skong_win::debug_overlay::{DebugLine, DebugOverlay, Scene};

#[derive(Default)]
pub struct PetOverlay {
    win: Option<DebugOverlay>,
    pub layers: OverlayLayers,
    /// Graph last drawn (held so pointer identity is a sound change test).
    graph: Option<Arc<NavGraph>>,
    /// Hash of the terrain + route + layers last drawn.
    key: Option<u64>,
    /// Caller-supplied extra segments (screen px), e.g. experimental debug data.
    pub extra: Vec<DebugLine>,
}

impl PetOverlay {
    pub fn is_on(&self) -> bool {
        self.win.is_some()
    }

    pub fn toggle(&mut self, terrain: &Terrain) {
        if self.win.take().is_some() {
            self.graph = None;
            log::info!("debug overlay off");
            return;
        }
        match DebugOverlay::new(terrain.bounds) {
            Ok(w) => {
                self.win = Some(w);
                self.key = None;
                log::info!("debug overlay on (terrain {}, nav graph {}, route {})", self.layers.terrain, self.layers.nav, self.layers.route);
            }
            Err(e) => log::warn!("debug overlay: {e:#}"),
        }
    }

    pub fn set_layers(&mut self, layers: OverlayLayers) {
        self.layers = layers;
        self.key = None;
    }

    /// Call every tick; does nothing unless on and something changed.
    pub fn sync(&mut self, terrain: &Terrain, graph: Option<&Arc<NavGraph>>, route: Option<&Route>) {
        let Some(win) = self.win.as_mut() else { return };
        let key = scene_key(terrain, route, self.layers, &self.extra);
        let same_graph = match (&self.graph, graph) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if self.key == Some(key) && same_graph {
            return;
        }
        self.key = Some(key);
        self.graph = graph.cloned();
        if win.bounds() != terrain.bounds {
            match DebugOverlay::new(terrain.bounds) {
                Ok(w) => *win = w,
                Err(e) => {
                    log::warn!("debug overlay: {e:#}");
                    self.win = None;
                    return;
                }
            }
        }
        let t0 = std::time::Instant::now();
        let l = self.layers;
        let g = graph.filter(|_| l.nav);
        let spans = g.map(|g| nav::debug_spans(g)).unwrap_or_default();
        let mut paths = g.map(|g| nav::debug_paths(g, None)).unwrap_or_default();
        if l.route
            && let Some(r) = route
        {
            paths.extend(nav::route_paths(r));
        }
        win.draw(&Scene { terrain: l.terrain.then_some(terrain), spans: &spans, paths: &paths, lines: &self.extra, legend: true });
        if let Err(e) = win.present() {
            log::warn!("debug overlay: {e:#}");
        }
        log::debug!("debug overlay redrawn in {:.1} ms ({} spans, {} paths)", t0.elapsed().as_secs_f64() * 1e3, spans.len(), paths.len());
    }
}

/// Cheap change detector (~µs for hundreds of ledges); only computed while the overlay is on.
fn scene_key(t: &Terrain, route: Option<&Route>, layers: OverlayLayers, extra: &[DebugLine]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut f = |v: f32| v.to_bits().hash(&mut h);
    for l in &t.ledges {
        f(l.x0);
        f(l.x1);
        f(l.y);
    }
    for w in &t.walls {
        f(w.x);
        f(w.y0);
        f(w.y1);
    }
    f(t.bounds.left);
    f(t.bounds.top);
    f(t.bounds.right);
    f(t.bounds.bottom);
    for l in extra {
        f(l.a.x);
        f(l.a.y);
        f(l.b.x);
        f(l.b.y);
    }
    if let Some(r) = route {
        f(r.eta);
        f(r.end.x);
        f(r.end.y);
        r.moves.hash(&mut h);
    }
    (t.ledges.len(), t.walls.len(), extra.len(), route.is_some(), layers.terrain, layers.nav, layers.route).hash(&mut h);
    h.finish()
}
