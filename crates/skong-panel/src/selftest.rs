//! `--self-test`: headless end-to-end check against a running pet. Connects, prints the
//! Hello, three Status messages and some logs, toggles pause twice (verifying each via
//! status), and exits non-zero on failure.
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use skong_ipc::{Command, PanelMsg};

use crate::link::Link;

fn wait_until<T>(link: &Link, what: &str, secs: u64, mut f: impl FnMut(&crate::link::Shared) -> Option<T>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f(&link.shared()) {
            return Ok(v);
        }
        if Instant::now() > deadline {
            bail!("timed out waiting for {what}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn run(pipe: String, overlay: bool) -> Result<()> {
    let link = Link::start(pipe.clone(), || {});
    println!("self-test: connecting to {pipe}");
    let hello = wait_until(&link, "connection + Hello", 10, |s| s.hello.clone())?;
    println!(
        "HELLO pet v{} pid {}: {} skins, bench styles {:?}, scales {:?}, mischief {:?}, log level {:?}",
        hello.version,
        hello.pid,
        hello.skins.len(),
        hello.bench_styles,
        hello.scales.iter().map(|s| &s.0).collect::<Vec<_>>(),
        hello.bother_levels.iter().map(|s| &s.0).collect::<Vec<_>>(),
        hello.log_level
    );

    let mut seen = 0;
    while seen < 3 {
        let (n, st) = wait_until(&link, "status", 5, |s| (s.statuses > seen).then(|| (s.statuses, s.status.clone())))?;
        seen = n;
        if let Some(st) = st {
            let k = &st.knight;
            println!(
                "STATUS #{n}: paused={} manual={} vision={} overlay={} knight ({:.0},{:.0}) {} clip '{}' act {:?} | terrain ground {} one-way {} content {} walls {} windows {} | cpu {:.1}% ram {:.1} MB | panels {} | brain {:?}",
                st.paused, st.manual, st.vision, st.overlay, k.x, k.y, k.state, k.clip, k.act,
                st.terrain.ground, st.terrain.one_way, st.terrain.content, st.terrain.walls, st.terrain.windows,
                st.process.cpu_percent, st.process.working_set_mb, st.panels, st.brain
            );
            if let Some(c) = st.capture {
                println!("  capture: running={} updates={} frames={} last analysis {:.2} ms latency {:.1} ms", c.running, c.updates, c.frames, c.last_analysis_ms, c.last_latency_ms);
            }
            if let Some(n) = &st.nav {
                let edges: Vec<String> = n.edges.iter().filter(|e| e.1 > 0).map(|(k, c)| format!("{k} {c}")).collect();
                println!("  nav: gen {} surfaces {} edges [{}] build {:.1} ms cache hits {}/{} motion {:?}", n.generation, n.surfaces, edges.join(", "), n.build_ms, n.cache_hits, n.pairs, n.motion);
            }
        }
    }

    let logs: Vec<String> = link.shared().logs.iter().rev().take(8).map(|l| format!("  [{:>8.3}s {:5} {}] {}", l.t, l.level.name(), l.target, l.msg)).collect();
    println!("LOGS: {} lines received; latest:", link.shared().logs.len());
    for l in logs.iter().rev() {
        println!("{l}");
    }

    let paused0 = link.shared().status.as_ref().is_some_and(|s| s.paused);
    for expect in [!paused0, paused0] {
        link.send(PanelMsg::Command(Command::TogglePause));
        wait_until(&link, "pause toggle in status", 5, |s| s.status.as_ref().is_some_and(|st| st.paused == expect).then_some(()))?;
        println!("TogglePause -> paused={expect} (confirmed by status)");
    }
    if overlay {
        for expect in [true, false] {
            link.send(PanelMsg::Command(Command::ToggleDebugOverlay));
            let cpu = wait_until(&link, "overlay toggle in status", 5, |s| s.status.as_ref().filter(|st| st.overlay == expect).map(|st| st.process.cpu_percent))?;
            println!("ToggleDebugOverlay -> overlay={expect} (confirmed by status, pet cpu {cpu:.1}%)");
            if expect {
                std::thread::sleep(Duration::from_secs(3));
                let st = link.shared().status.clone().unwrap_or_default();
                println!("  with overlay on for 3 s: pet cpu {:.1}% ram {:.1} MB", st.process.cpu_percent, st.process.working_set_mb);
            }
        }
    }
    println!("self-test OK");
    Ok(())
}
