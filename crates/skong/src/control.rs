//! The pet's side of the control-panel protocol (see `skong_ipc`).
//!
//! [`ControlServer::poll`] runs every tick and never blocks: it drains panel messages
//! (commands, manual keys, log level), greets new panels with the [`Hello`] and the log
//! backlog, and streams new log lines. Status is published ~4 Hz, and only while a panel
//! is connected — with no panel, a tick costs two empty channel polls.
use std::sync::Arc;
use std::time::{Duration, Instant};

use skong_ipc::{ClientId, Command, Hello, ManualKeys, PanelMsg, PetMsg, Server, ServerEvent, Status, decode, encode};
use skong_win::process::UsageSampler;

use crate::logring::{self, LogRing};

const STATUS_EVERY: Duration = Duration::from_millis(250);
/// Log lines per `Logs` message (keeps backlog messages moderate).
const LOG_BATCH: usize = 500;

pub struct ControlServer {
    server: Option<Server>,
    ring: Option<&'static LogRing>,
    hello: Hello,
    /// Next log sequence number to stream.
    sent_seq: u64,
    /// Keys of the panel currently driving the knight.
    keys: Option<(ClientId, ManualKeys)>,
    /// Manual mode was switched on because a panel started driving.
    auto_manual: bool,
    last_status: Instant,
    usage: UsageSampler,
}

impl ControlServer {
    /// Serve `\\.\pipe\desk-skong`. Never fails: without the pipe, panels just can't connect.
    pub fn start() -> Self {
        let server = match Server::start(skong_ipc::PIPE_NAME) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("control panel pipe unavailable: {e}");
                None
            }
        };
        let ring = logring::ring();
        Self {
            server,
            ring,
            hello: Hello::default(),
            sent_seq: ring.map_or(0, LogRing::next_seq),
            keys: None,
            auto_manual: false,
            last_status: Instant::now(),
            usage: UsageSampler::new(),
        }
    }

    /// Static info greeting each new panel.
    pub fn set_hello(&mut self, hello: Hello) {
        self.hello = hello;
    }

    pub fn panels(&self) -> usize {
        self.server.as_ref().map_or(0, Server::client_count)
    }

    /// Keys of a driving panel; `None` = read the global keyboard.
    pub fn manual_keys(&self) -> Option<ManualKeys> {
        self.keys.map(|(_, k)| k)
    }

    /// Handle panel traffic. `manual` is the pet's current manual-mode flag; returned
    /// commands are applied like hotkeys.
    pub fn poll(&mut self, manual: bool) -> Vec<Command> {
        let Some(server) = self.server.as_mut() else { return Vec::new() };
        let mut cmds = Vec::new();
        for ev in server.poll() {
            match ev {
                ServerEvent::Connected(id) => {
                    log::info!("control panel #{id} connected");
                    self.hello.log_level = Some(logring::panel_level());
                    server.send(id, encode(&PetMsg::Hello(self.hello.clone())).into());
                    if let Some(ring) = self.ring {
                        if server.client_count() == 1 {
                            self.sent_seq = ring.next_seq(); // nothing was streamed while alone
                        }
                        // Backlog = everything already streamed; newer lines go to all panels below.
                        let mut backlog = ring.since(0);
                        backlog.retain(|l| l.seq < self.sent_seq);
                        for chunk in backlog.chunks(LOG_BATCH) {
                            server.send(id, encode(&PetMsg::Logs(chunk.to_vec())).into());
                        }
                    }
                    self.last_status = Instant::now() - STATUS_EVERY; // status right away
                }
                ServerEvent::Line(id, line) => match decode::<PanelMsg>(&line) {
                    Ok(msg) => Self::handle(&mut self.keys, &mut self.auto_manual, id, msg, manual, &mut cmds),
                    Err(e) => log::warn!("control panel #{id}: bad message ({e}): {}", line.chars().take(120).collect::<String>()),
                },
                ServerEvent::Disconnected(id) => {
                    log::info!("control panel #{id} disconnected");
                    let release = PanelMsg::ManualInput { active: false, keys: ManualKeys::default() };
                    Self::handle(&mut self.keys, &mut self.auto_manual, id, release, manual, &mut cmds);
                }
            }
        }
        if let Some(ring) = self.ring
            && server.client_count() > 0
        {
            let new = ring.since(self.sent_seq);
            if let Some(last) = new.last() {
                self.sent_seq = last.seq + 1;
                for chunk in new.chunks(LOG_BATCH) {
                    server.broadcast(&encode(&PetMsg::Logs(chunk.to_vec())));
                }
            }
        }
        cmds
    }

    fn handle(keys: &mut Option<(ClientId, ManualKeys)>, auto_manual: &mut bool, id: ClientId, msg: PanelMsg, manual: bool, cmds: &mut Vec<Command>) {
        match msg {
            PanelMsg::Command(c) => cmds.push(c),
            PanelMsg::ManualInput { active: true, keys: k } => {
                *keys = Some((id, k));
                if !manual && !*auto_manual {
                    *auto_manual = true;
                    cmds.push(Command::SetManual(true));
                }
            }
            PanelMsg::ManualInput { active: false, .. } => {
                if keys.is_some_and(|(owner, _)| owner == id) {
                    *keys = None;
                    if std::mem::take(auto_manual) {
                        cmds.push(Command::SetManual(false));
                    }
                }
            }
            PanelMsg::SetLogLevel(l) => logring::set_panel_level(l),
        }
    }

    /// Time to publish a status? (Only with panels connected.)
    pub fn status_due(&self) -> bool {
        self.panels() > 0 && self.last_status.elapsed() >= STATUS_EVERY
    }

    /// Send `status` to every panel, adding process usage and connection info.
    pub fn publish(&mut self, mut status: Status) {
        self.last_status = Instant::now();
        let u = self.usage.sample();
        status.process.cpu_percent = u.cpu_percent;
        status.process.working_set_mb = u.working_set_bytes as f32 / (1024.0 * 1024.0);
        status.panels = self.panels();
        status.driven = self.keys.is_some();
        if let Some(s) = self.server.as_mut() {
            let line: Arc<str> = encode(&PetMsg::Status(Box::new(status))).into();
            s.broadcast(&line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driving_panel_toggles_manual_only_if_it_enabled_it() {
        let (mut keys, mut auto, mut cmds) = (None, false, Vec::new());
        let k = ManualKeys { left: true, ..Default::default() };
        ControlServer::handle(&mut keys, &mut auto, 1, PanelMsg::ManualInput { active: true, keys: k }, false, &mut cmds);
        ControlServer::handle(&mut keys, &mut auto, 1, PanelMsg::ManualInput { active: true, keys: k }, true, &mut cmds);
        assert_eq!(cmds, [Command::SetManual(true)]);
        assert_eq!(keys, Some((1, k)));
        // Another panel releasing doesn't steal the wheel.
        ControlServer::handle(&mut keys, &mut auto, 2, PanelMsg::ManualInput { active: false, keys: k }, true, &mut cmds);
        assert!(keys.is_some());
        ControlServer::handle(&mut keys, &mut auto, 1, PanelMsg::ManualInput { active: false, keys: k }, true, &mut cmds);
        assert_eq!(cmds, [Command::SetManual(true), Command::SetManual(false)]);
        assert!(keys.is_none());
        // Already in manual mode: driving leaves the user's choice alone.
        cmds.clear();
        ControlServer::handle(&mut keys, &mut auto, 3, PanelMsg::ManualInput { active: true, keys: k }, true, &mut cmds);
        ControlServer::handle(&mut keys, &mut auto, 3, PanelMsg::ManualInput { active: false, keys: k }, true, &mut cmds);
        assert!(cmds.is_empty());
        ControlServer::handle(&mut keys, &mut auto, 3, PanelMsg::Command(Command::Wake), true, &mut cmds);
        assert_eq!(cmds, [Command::Wake]);
    }
}
