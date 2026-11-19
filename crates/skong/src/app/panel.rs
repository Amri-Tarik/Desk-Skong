//! Control panel + debug overlay glue for [`Pet`]: kept out of `app.rs` so the main loop
//! only gains a call to [`Pet::service_panel`] and a match arm for [`Pet::apply_panel`].
use anyhow::Result;
use skong_core::ai::Navigator;
use skong_core::nav::MoveKind;
use skong_core::terrain::LedgeKind;
use skong_ipc::{CaptureInfo, Command, Hello, KnightInfo, MotionInfo, NavInfo, Status, TerrainInfo};
use skong_win::runloop::Flow;

use super::Pet;
use crate::settings::{BOTHER, SCALES};

impl Pet {
    /// Once per tick, paused or not: panel traffic (returns its commands), overlay redraw
    /// when something changed, ~4 Hz status while a panel is connected.
    pub(super) fn service_panel(&mut self) -> Vec<Command> {
        let cmds = self.control.poll();
        self.overlay.sync(&self.terrain, self.nav.graph(), self.nav.follower().map(|f| f.route()));
        if self.control.status_due() {
            let s = self.status();
            self.control.publish(s);
        }
        cmds
    }

    pub(super) fn hello(&self) -> Hello {
        Hello {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            skins: self.data.skins.iter().map(|s| if s.name == skong_core::assets::VANILLA { "Vanilla (game)".to_string() } else { s.name.clone() }).collect(),
            bench_styles: self.benches.styles.iter().map(|s| s.name.clone()).collect(),
            scales: SCALES.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
            bother_levels: BOTHER.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
            log_level: None,
        }
    }

    /// Commands that only exist for the panel / overlay.
    pub(super) fn apply_panel(&mut self, c: Command) -> Result<Flow> {
        match c {
            Command::SetManual(on) => {
                if on != self.manual {
                    return self.apply(Command::ToggleManual);
                }
            }
            Command::ToggleVision => {
                let on = !self.content.is_enabled();
                self.content.set_enabled(on, self.watcher.snapshot());
                log::info!("vision (content terrain) {}", if on { "on" } else { "off" });
            }
            Command::ToggleDebugOverlay => self.overlay.toggle(&self.terrain),
            Command::SetOverlayLayers(l) => self.overlay.set_layers(l),
            Command::OpenPanel => open_panel(),
            other => return self.apply(other),
        }
        Ok(Flow::Continue)
    }

    fn status(&self) -> Status {
        let t = &self.terrain;
        let count = |k: LedgeKind| t.ledges.iter().filter(|l| l.kind == k).count();
        let totals = self.content.totals();
        let ms = |d: std::time::Duration| d.as_secs_f32() * 1e3;
        Status {
            paused: self.paused,
            manual: self.manual,
            click_through: self.settings.click_through,
            fight_mode: self.brain.fight_mode(),
            share_visible: self.settings.share_visible,
            vision: self.content.is_enabled(),
            overlay: self.overlay.is_on(),
            overlay_layers: self.overlay.layers,
            skin: self.skin,
            scale: self.settings.scale,
            bother: self.settings.bother,
            benches_placed: self.benches.placed.len(),
            benches_available: (0..self.benches.styles.len()).map(|i| self.benches.available(i)).collect(),
            knight: KnightInfo {
                x: self.ch.pos.x,
                y: self.ch.pos.y,
                state: format!("{:?}", self.ch.state),
                clip: self.data.lib.get(self.anim.clip()).name.clone(),
                act: self.act.as_ref().map(|p| self.data.acts.get(p.act()).name.clone()),
                facing_right: self.ch.facing == skong_core::controller::Side::Right,
            },
            terrain: TerrainInfo {
                ground: count(LedgeKind::Ground),
                one_way: count(LedgeKind::OneWay),
                content: count(LedgeKind::Content),
                walls: t.walls.len(),
                windows: t.windows.len(),
            },
            capture: self.content.is_enabled().then(|| CaptureInfo {
                running: self.content.is_running(),
                updates: totals.updates,
                frames: totals.frames,
                last_analysis_ms: totals.last.map_or(0.0, |s| ms(s.analysis)),
                last_latency_ms: totals.last.map_or(0.0, |s| ms(s.latency)),
            }),
            brain: (!self.brain_debug.is_empty()).then(|| self.brain_debug.clone()),
            nav: self.nav_info(),
            stakes: Some(self.stakes.info(self.brain.soul())),
            ..Status::default() // process usage, panel count: filled by ControlServer::publish
        }
    }

    fn nav_info(&self) -> Option<NavInfo> {
        let g = self.nav.graph()?;
        let st = g.stats();
        let motion = self.nav.follower().map(|f| {
            let r = f.route();
            MotionInfo {
                goal: f.goal().map_or_else(|| "-".to_string(), |g| format!("{g:?}")),
                style: format!("{:?}", f.style()),
                moves: r.moves.iter().map(|m| format!("{m:?}")).collect(),
                eta: r.eta,
                status: format!("{:?}", self.nav.status()),
            }
        });
        Some(NavInfo {
            generation: g.generation,
            surfaces: g.surfaces().len(),
            edges: MoveKind::ALL.iter().zip(g.edge_counts()).map(|(k, n)| (format!("{k:?}"), n)).collect(),
            build_ms: st.millis,
            pairs: st.pairs,
            cache_hits: st.cache_hits,
            sims: st.sims,
            motion,
        })
    }
}

/// Launch `skong-panel.exe` from the pet's own directory.
fn open_panel() {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("skong-panel.exe")));
    let Some(exe) = exe.filter(|p| p.is_file()) else {
        log::warn!("skong-panel.exe not found next to the pet executable (build it with `cargo build --release -p skong-panel`)");
        return;
    };
    match std::process::Command::new(&exe).spawn() {
        Ok(child) => log::info!("control panel started (pid {})", child.id()),
        Err(e) => log::warn!("could not start {}: {e}", exe.display()),
    }
}
