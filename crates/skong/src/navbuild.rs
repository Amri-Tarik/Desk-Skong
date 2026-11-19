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
/// Content ledges shorter than this (px) aren't worth planning routes over; physics still
/// lands on them. Keeps busy screens (hundreds of detected lines) from exploding the graph.
/// Walls are never trimmed: physics enforces every wall, so a planner that didn't know about
/// one would route the knight straight into it (stuck against a prompt box border).
const NAV_CONTENT_MIN_LEN: f32 = 120.0;
const NAV_MAX_CONTENT_LEDGES: usize = 40;

/// The terrain the planner sees: everything, minus the least useful content *ledges*, except
/// the one the knight stands on: without it no route can start, and every plan from that
/// ledge fails (he got stuck on a little prompt-box ledge, unable to go anywhere).
fn nav_terrain(t: &Terrain, standing: Option<skong_core::controller::Support>) -> Terrain {
    use skong_core::terrain::{Ledge, LedgeKind};
    let mut t = t.clone();
    let under = |l: &Ledge| standing.is_some_and(|s| s.kind == l.kind && s.y == l.y && s.x0 == l.x0 && s.x1 == l.x1);
    let mut content: Vec<_> = t.ledges.iter().filter(|l| l.kind == LedgeKind::Content && l.x1 - l.x0 >= NAV_CONTENT_MIN_LEN && !under(l)).copied().collect();
    content.sort_by(|a, b| (b.x1 - b.x0).total_cmp(&(a.x1 - a.x0)));
    content.truncate(NAV_MAX_CONTENT_LEDGES);
    content.extend(t.ledges.iter().filter(|l| l.kind == LedgeKind::Content && under(l)).copied());
    t.ledges.retain(|l| l.kind != LedgeKind::Content);
    t.ledges.extend(content);
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
                    let t = nav_terrain(&t, ch.support);
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

#[cfg(test)]
mod tests {
    use super::*;
    use skong_core::controller::{Abilities, CharacterParams};
    use skong_core::desktop::{DesktopSnapshot, Monitor, WindowInfo};
    use skong_core::geom::{RectI, Vec2};
    use skong_core::nav::{NavGoal, TravelStyle};
    use skong_core::terrain::{ContentLedge, TerrainOptions};

    #[test]
    fn a_route_can_start_from_a_short_trimmed_content_ledge() {
        // The user's case: standing on a small ledge in a prompt box (shorter than the
        // planner's minimum), every plan failed.
        let mon = Monitor { rect: RectI::new(0, 0, 2560, 1440), work: RectI::new(0, 0, 2560, 1400), primary: true, scale_milli: 1000 };
        let win = WindowInfo {
            id: 1,
            rect: RectI::new(200, 100, 1800, 1400),
            title: "w".into(),
            class: String::new(),
            process: String::new(),
            pid: 0,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        };
        let desk = DesktopSnapshot { generation: 1, monitors: vec![mon], windows: vec![win] };
        let ledges = [ContentLedge { x0: 600, x1: 680, y: 1300, owner: Some(1) }];
        let t = Terrain::from_desktop_and_content(&desk, &ledges, &[], &TerrainOptions::default());
        let mut ch = Character::new(CharacterParams::hollow_knight(38.4), Abilities::default(), Vec2::new(640.0, 1290.0));
        let mut ev = vec![];
        for _ in 0..240 {
            ch.step(&Default::default(), &t, 1.0 / 120.0, &mut ev);
        }
        assert_eq!(ch.pos.y, 1300.0, "standing on the short ledge");
        let goal = NavGoal::Point(Vec2::new(1500.0, 1400.0));
        let trimmed = NavGraph::build(&nav_terrain(&t, None), &ch);
        // The bug: no surface under him (plans started from the floor below, where he isn't, and
        // following failed at once).
        assert!(trimmed.surface_at(&ch).is_none());
        let kept = NavGraph::build(&nav_terrain(&t, ch.support), &ch);
        assert!(kept.surface_at(&ch).is_some());
        assert!(kept.plan(&ch, goal, TravelStyle::Normal).is_some());
    }
}
