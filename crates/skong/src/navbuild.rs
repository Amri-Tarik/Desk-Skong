//! Builds navigation graphs on a background thread so the 120 Hz tick never stalls
//! (a cold build of a busy desktop can take ~15 ms). Requests are coalesced: while a
//! window is being dragged only the latest terrain is built, and each build reuses the
//! previous graph's simulated edges where nothing changed.
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use skong_core::controller::Character;
use skong_core::nav::NavGraph;
use skong_core::terrain::Terrain;

enum Job {
    /// Build for this terrain, incrementally from the previous graph. `lazy` = only screen
    /// content changed: such builds are throttled to one per [`LAZY_INTERVAL`].
    Build(Box<Terrain>, Box<Character>, bool),
    /// Character parameters changed (scale): discard cached edges, build from scratch.
    Reset(Box<Terrain>, Box<Character>),
}

pub struct NavBuilder {
    jobs: Option<Sender<Job>>,
    graphs: Receiver<Arc<NavGraph>>,
    thread: Option<JoinHandle<()>>,
}

/// Minimum spacing between builds, so a window drag doesn't rebuild every tick.
const COALESCE: Duration = Duration::from_millis(60);
/// Content-only changes (text streaming, scrolling…) rebuild at most this often.
const LAZY_INTERVAL: Duration = Duration::from_millis(1000);
/// Content surfaces shorter than this (px) aren't worth planning routes over; physics still
/// lands on them. Keeps busy screens (hundreds of detected lines) from exploding the graph.
const NAV_CONTENT_MIN_LEN: f32 = 120.0;
const NAV_MAX_CONTENT_LEDGES: usize = 40;
const NAV_MAX_CONTENT_WALLS: usize = 30;

/// The terrain the planner sees: everything, minus the least useful content geometry.
fn nav_terrain(t: &Terrain) -> Terrain {
    use skong_core::terrain::{LedgeKind, WallKind};
    let mut t = t.clone();
    let mut content: Vec<_> = t.ledges.iter().filter(|l| l.kind == LedgeKind::Content && l.x1 - l.x0 >= NAV_CONTENT_MIN_LEN).copied().collect();
    content.sort_by(|a, b| (b.x1 - b.x0).total_cmp(&(a.x1 - a.x0)));
    content.truncate(NAV_MAX_CONTENT_LEDGES);
    t.ledges.retain(|l| l.kind != LedgeKind::Content);
    t.ledges.extend(content);
    let mut walls: Vec<_> = t.walls.iter().filter(|w| w.kind == WallKind::Content && w.y1 - w.y0 >= NAV_CONTENT_MIN_LEN).copied().collect();
    walls.sort_by(|a, b| (b.y1 - b.y0).total_cmp(&(a.y1 - a.y0)));
    walls.truncate(NAV_MAX_CONTENT_WALLS);
    t.walls.retain(|w| w.kind != WallKind::Content);
    t.walls.extend(walls);
    t
}

impl NavBuilder {
    pub fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (graph_tx, graph_rx) = channel();
        let thread = std::thread::Builder::new()
            .name("nav-builder".into())
            .spawn(move || {
                let mut prev: Option<Arc<NavGraph>> = None;
                let mut last_build: Option<Instant> = None;
                while let Ok(mut job) = job_rx.recv() {
                    // Let a burst of changes settle, then keep only the newest request
                    // (a Reset anywhere in the burst still forces a fresh build; any non-lazy
                    // request makes the burst urgent).
                    let mut reset = matches!(job, Job::Reset(..));
                    let mut lazy = matches!(job, Job::Build(_, _, true));
                    let mut wait = COALESCE;
                    loop {
                        if lazy && let Some(t) = last_build {
                            wait = wait.max(LAZY_INTERVAL.saturating_sub(t.elapsed()));
                        }
                        std::thread::sleep(wait);
                        wait = Duration::ZERO;
                        let mut more = false;
                        loop {
                            match job_rx.try_recv() {
                                Ok(j) => {
                                    reset |= matches!(j, Job::Reset(..));
                                    lazy &= matches!(j, Job::Build(_, _, true));
                                    job = j;
                                    more = true;
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => return,
                            }
                        }
                        // An urgent request arrived while throttling: build now.
                        if !more || !lazy {
                            break;
                        }
                    }
                    let (Job::Build(t, ch, _) | Job::Reset(t, ch)) = job;
                    let t = nav_terrain(&t);
                    last_build = Some(Instant::now());
                    let start = Instant::now();
                    let g = match prev.as_ref().filter(|_| !reset) {
                        Some(p) => p.rebuild(&t, &ch),
                        None => NavGraph::build(&t, &ch),
                    };
                    let s = g.stats();
                    log::debug!("nav graph: {} surfaces, {} edges, {} sims in {:?}", s.surfaces, s.edges, s.sims, start.elapsed());
                    let g = Arc::new(g);
                    prev = Some(g.clone());
                    if graph_tx.send(g).is_err() {
                        return;
                    }
                }
            })
            .ok();
        if thread.is_none() {
            log::error!("could not start the navigation thread; the knight will stay on his platform");
        }
        Self { jobs: Some(job_tx), graphs: graph_rx, thread }
    }

    /// `lazy`: only screen content changed (throttled); window/desktop changes are urgent.
    pub fn request(&self, terrain: &Terrain, template: &Character, lazy: bool) {
        if let Some(tx) = &self.jobs {
            let _ = tx.send(Job::Build(Box::new(terrain.clone()), Box::new(template.clone()), lazy));
        }
    }

    pub fn reset(&self, terrain: &Terrain, template: &Character) {
        if let Some(tx) = &self.jobs {
            let _ = tx.send(Job::Reset(Box::new(terrain.clone()), Box::new(template.clone())));
        }
    }

    /// Newest finished graph, if any arrived since the last call.
    pub fn poll(&self) -> Option<Arc<NavGraph>> {
        self.graphs.try_iter().last()
    }
}

impl Drop for NavBuilder {
    fn drop(&mut self) {
        self.jobs = None; // closes the channel; the thread exits after its current build
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
