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
    /// Build for this terrain, incrementally from the previous graph.
    Build(Box<Terrain>, Box<Character>),
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

impl NavBuilder {
    pub fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (graph_tx, graph_rx) = channel();
        let thread = std::thread::Builder::new()
            .name("nav-builder".into())
            .spawn(move || {
                let mut prev: Option<Arc<NavGraph>> = None;
                while let Ok(mut job) = job_rx.recv() {
                    // Let a burst of changes settle, then keep only the newest request
                    // (a Reset anywhere in the burst still forces a fresh build).
                    std::thread::sleep(COALESCE);
                    let mut reset = matches!(job, Job::Reset(..));
                    loop {
                        match job_rx.try_recv() {
                            Ok(j) => {
                                reset |= matches!(j, Job::Reset(..));
                                job = j;
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let (Job::Build(t, ch) | Job::Reset(t, ch)) = job;
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

    pub fn request(&self, terrain: &Terrain, template: &Character) {
        if let Some(tx) = &self.jobs {
            let _ = tx.send(Job::Build(Box::new(terrain.clone()), Box::new(template.clone())));
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
