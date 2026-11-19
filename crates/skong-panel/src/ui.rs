//! The panel window (egui). Left: controls mirroring the tray + live status. Right: logs.
//!
//! While the window is focused (and no text field has focus): arrows / Up jump / Shift dash /
//! Ctrl superdash / Z walk / X sprint drive the knight while the pet is in manual mode (a toggle,
//! off by default — the panel never turns it on by itself), and the plain letters K A W R G H S B C P D trigger the same actions as Ctrl+Alt+letter.
use std::time::Duration;

use eframe::egui::{self, Color32, Key, RichText, TextWrapMode};
use skong_ipc::{SPELLS, StakesCommand, Command, LogLevel, LogLine, ManualKeys, OverlayLayers, PanelMsg, Status};

use crate::link::Link;

/// Shortcuts available as plain letters while the panel is focused.
const LETTERS: &[(Key, Command, &str)] = &[
    (Key::K, Command::ToggleManual, "manual control"),
    (Key::A, Command::RandomAct, "do something"),
    (Key::W, Command::Wake, "wake"),
    (Key::R, Command::RespawnAtCursor, "come here (cursor)"),
    (Key::G, Command::GoToCursor { hurry: false }, "walk to cursor"),
    (Key::H, Command::GoToCursor { hurry: true }, "hurry to cursor"),
    (Key::S, Command::NextSkin, "next skin"),
    (Key::B, Command::PlaceBenchNearKnight(0), "bench near knight"),
    (Key::C, Command::ToggleClickThrough, "click-through"),
    (Key::P, Command::TogglePause, "pause"),
    (Key::D, Command::ToggleDebugOverlay, "debug overlay"),
];

pub fn run(pipe: String) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Desk-Skong control panel").with_inner_size([1100.0, 700.0]).with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Desk-Skong control panel",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let link = Link::start(pipe, move || ctx.request_repaint());
            Ok(Box::new(Panel::new(link)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

struct Panel {
    link: Link,
    skin_filter: String,
    log_text: String,
    log_min: LogLevel,
    /// Level the pet captures for us (sent on change and after each reconnect).
    capture_level: LogLevel,
    autoscroll: bool,
    drive: bool,
    /// Last ManualInput sent: (active, keys).
    sent_keys: (bool, ManualKeys),
    session: u64,
}

impl Panel {
    fn new(link: Link) -> Self {
        Self {
            link,
            skin_filter: String::new(),
            log_text: String::new(),
            log_min: LogLevel::Info,
            capture_level: LogLevel::Info,
            autoscroll: true,
            drive: true,
            sent_keys: (false, ManualKeys::default()),
            session: 0,
        }
    }

    fn cmd(&self, c: Command) {
        self.link.send(PanelMsg::Command(c));
    }

    /// Keyboard driving + letter shortcuts.
    fn keyboard(&mut self, ctx: &egui::Context, connected: bool) {
        let typing = ctx.egui_wants_keyboard_input();
        let (focused, keys, mods, letters) = ctx.input(|i| {
            let keys = ManualKeys {
                left: i.key_down(Key::ArrowLeft),
                right: i.key_down(Key::ArrowRight),
                up: i.key_down(Key::ArrowUp),
                down: i.key_down(Key::ArrowDown),
                jump: i.key_down(Key::ArrowUp),
                dash: i.modifiers.shift,
                // Ctrl+Alt is a global pet hotkey chord, not a charge.
                superdash: i.modifiers.ctrl && !i.modifiers.alt,
                walk: i.key_down(Key::Z),
                sprint: i.key_down(Key::X),
            };
            let letters: Vec<Command> = LETTERS.iter().filter(|(k, _, _)| i.key_pressed(*k)).map(|(_, c, _)| *c).collect();
            (i.focused, keys, i.modifiers, letters)
        });
        let active = connected && self.drive && focused && !typing;
        let now = (active, if active { keys } else { ManualKeys::default() });
        if now != self.sent_keys && connected {
            self.link.send(PanelMsg::ManualInput { active: now.0, keys: now.1 });
        }
        self.sent_keys = now;
        if connected && focused && !typing && !mods.ctrl && !mods.alt && !mods.command {
            for c in letters {
                self.cmd(c);
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, st: Option<&Status>, hello: Option<&skong_ipc::Hello>, connected: bool) {
        ui.add_enabled_ui(connected, |ui| {
            let Some(h) = hello else {
                ui.label("waiting for the pet…");
                return;
            };
            let st_default = Status::default();
            let s = st.unwrap_or(&st_default);
            egui::Grid::new("settings").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Skin");
                let current = h.skins.get(s.skin).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("skin").selected_text(current).width(200.0).height(400.0).show_ui(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.skin_filter).hint_text("search…").desired_width(180.0));
                    let f = self.skin_filter.to_lowercase();
                    for (i, name) in h.skins.iter().enumerate().filter(|(_, n)| f.is_empty() || n.to_lowercase().contains(&f)) {
                        if ui.selectable_label(i == s.skin, name.as_str()).clicked() {
                            self.cmd(Command::SetSkin(i));
                        }
                    }
                });
                ui.end_row();

                ui.label("Size");
                ui.horizontal_wrapped(|ui| {
                    for (name, v) in &h.scales {
                        if ui.selectable_label((s.scale - v).abs() < 1e-3, name.as_str()).clicked() {
                            self.cmd(Command::SetScale(*v));
                        }
                    }
                });
                ui.end_row();

                ui.label("Mischief");
                ui.horizontal_wrapped(|ui| {
                    for (name, v) in &h.bother_levels {
                        if ui.selectable_label((s.bother - v).abs() < 1e-3, name.as_str()).clicked() {
                            self.cmd(Command::SetBother(*v));
                        }
                    }
                });
                ui.end_row();

                ui.label("Bench");
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in h.bench_styles.iter().enumerate() {
                        let ok = s.benches_available.get(i).copied().unwrap_or(true);
                        if ui.add_enabled(ok, egui::Button::new(name.as_str())).on_hover_text("place near the knight").clicked() {
                            self.cmd(Command::PlaceBenchNearKnight(i));
                        }
                    }
                    if ui.add_enabled(s.benches_placed > 0, egui::Button::new("Remove all")).clicked() {
                        self.cmd(Command::ClearBenches);
                    }
                });
                ui.end_row();

                ui.label("Discover");
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in ["Bench", "Geo deposit"].into_iter().enumerate() {
                        if ui.button(name).on_hover_text("reveal one in the window the knight stands on").clicked() {
                            self.cmd(Command::Discover(i as u8));
                        }
                    }
                    if ui.button("Forget all").clicked() {
                        self.cmd(Command::ForgetDiscoveries);
                    }
                });
                ui.end_row();
            });

            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Do something!  [A]").clicked() {
                    self.cmd(Command::RandomAct);
                }
                if ui.button("Wake  [W]").clicked() {
                    self.cmd(Command::Wake);
                }
                if ui.button("Come here  [R]").on_hover_text("respawn at the mouse cursor").clicked() {
                    self.cmd(Command::RespawnAtCursor);
                }
                if ui.button("Next skin  [S]").clicked() {
                    self.cmd(Command::NextSkin);
                }
                if ui.button("Invite a visitor").on_hover_text("an NPC drops by; the knight runs over to talk").clicked() {
                    self.cmd(Command::InviteVisitor);
                }
                if ui.button("Dream nail").on_hover_text("dream-nail the window he stands on").clicked() {
                    self.cmd(Command::DreamNail);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Stakes");
                for (label, tip, c) in [
                    ("Die", "kill the knight (he leaves a shade)", StakesCommand::Die),
                    ("Drop geo", "burst geo out of the cursor", StakesCommand::DropGeo),
                    ("Geo shower", "geo tumbles out of a portal over a settled ledge he can reach", StakesCommand::SpillGeo),
                    ("Fight shade", "send him after his shade", StakesCommand::HuntShade),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked() {
                        self.cmd(Command::Stakes(c));
                    }
                }
                for (i, (_, name)) in SPELLS.iter().enumerate() {
                    if ui.small_button(*name).on_hover_text("play this spell / nail art now").clicked() {
                        self.cmd(Command::Stakes(StakesCommand::Cast(i as u8)));
                    }
                }
            });
            let toggle = |ui: &mut egui::Ui, on: bool, label: &str, c: Command| {
                let mut v = on;
                if ui.checkbox(&mut v, label).changed() {
                    self.cmd(c);
                }
            };
            toggle(ui, s.manual, "Manual control  [K]", Command::ToggleManual);
            toggle(ui, s.click_through, "Click-through  [C]", Command::ToggleClickThrough);
            toggle(ui, s.fight_mode, "Fight the cursor", Command::ToggleFightMode);
            toggle(ui, s.share_visible, "Visible in screen shares", Command::ToggleShareVisible);
            toggle(ui, s.text_curiosity, "Curious about highlighted text", Command::ToggleTextCuriosity);
            toggle(ui, s.paused, "Pause  [P]", Command::TogglePause);
            toggle(ui, s.vision, "Vision (content terrain)", Command::ToggleVision);
            toggle(ui, s.overlay, "Debug overlay  [D]", Command::ToggleDebugOverlay);
            ui.indent("layers", |ui| {
                let mut l: OverlayLayers = s.overlay_layers;
                let mut changed = ui.checkbox(&mut l.terrain, "terrain (ledges, walls)").changed();
                changed |= ui.checkbox(&mut l.nav, "nav graph (surfaces, edges)").changed();
                changed |= ui.checkbox(&mut l.route, "current route").changed();
                if changed {
                    self.cmd(Command::SetOverlayLayers(l));
                }
            });
            ui.checkbox(&mut self.drive, "Keys drive the knight in manual mode").on_hover_text(
                "arrows move, Up jumps, Shift dashes, Ctrl charges a superdash, Z walk, X sprint.\nManual mode is switched on while this window has focus.",
            );
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Quit pet").color(Color32::from_rgb(230, 90, 90))).clicked() {
                    self.cmd(Command::Quit);
                }
            });
        });
    }

    fn status(ui: &mut egui::Ui, st: Option<&Status>) {
        let Some(s) = st else {
            ui.weak("no status yet");
            return;
        };
        let k = &s.knight;
        egui::Grid::new("status").num_columns(2).spacing([8.0, 2.0]).striped(true).show(ui, |ui| {
            let mut row = |a: &str, b: String| {
                ui.label(a);
                ui.monospace(b);
                ui.end_row();
            };
            row("Knight", format!("({:.0}, {:.0}) {} {}", k.x, k.y, k.state, if k.facing_right { "→" } else { "←" }));
            row("Clip / act", format!("{} / {}", k.clip, k.act.as_deref().unwrap_or("-")));
            row("Mode", format!("{}{}{}", if s.manual { "manual" } else { "brain" }, if s.driven { " (panel driving)" } else { "" }, if s.paused { ", paused" } else { "" }));
            let t = &s.terrain;
            row("Terrain", format!("ground {}  one-way {}  content {}\nwalls {}  windows {}", t.ground, t.one_way, t.content, t.walls, t.windows));
            if let Some(c) = &s.capture {
                row("Vision", format!("{} · {} updates · {} frames\nanalysis {:.2} ms · latency {:.1} ms", if c.running { "running" } else { "unavailable" }, c.updates, c.frames, c.last_analysis_ms, c.last_latency_ms));
            }
            row("Process", format!("CPU {:.1}% · RAM {:.1} MB · {} panel(s)", s.process.cpu_percent, s.process.working_set_mb, s.panels));
            if let Some(st) = &s.stakes {
                let shade = st.shade.map_or_else(|| "none".to_string(), |(x, y, g)| format!("at ({x:.0}, {y:.0}) holding {g} geo"));
                row("Stakes", format!("{} geo (+{} loose) · soul {:.0}%\nshade {shade}\n{} deaths · {} shades beaten", st.geo, st.loose_geo, st.soul * 100.0, st.deaths, st.shades_defeated));
            }
            if let Some(n) = &s.nav {
                let edges: Vec<String> = n.edges.iter().filter(|e| e.1 > 0).map(|(k, c)| format!("{k} {c}")).collect();
                row("Nav graph", format!("gen {} · {} surfaces · build {:.1} ms\ncache {}/{} pairs · {} sims\n{}", n.generation, n.surfaces, n.build_ms, n.cache_hits, n.pairs, n.sims, wrap_list(&edges, 3)));
                match &n.motion {
                    Some(m) => row("Motion", format!("{} → {}\n{} · eta {:.1} s\n{}", m.style, m.goal, m.status, m.eta, wrap_list(&m.moves, 4))),
                    None => row("Motion", "-".into()),
                }
            }
        });
        if let Some(b) = &s.brain {
            ui.add_space(4.0);
            ui.label("Brain");
            ui.add(egui::Label::new(RichText::new(b).monospace().small()).wrap());
        }
    }

    fn logs(&mut self, ui: &mut egui::Ui, connected: bool) {
        ui.horizontal(|ui| {
            ui.label("Show");
            egui::ComboBox::from_id_salt("min").selected_text(self.log_min.name()).width(70.0).show_ui(ui, |ui| {
                for l in LogLevel::ALL {
                    ui.selectable_value(&mut self.log_min, l, l.name());
                }
            });
            ui.label("capture");
            let before = self.capture_level;
            egui::ComboBox::from_id_salt("capture").selected_text(self.capture_level.name()).width(70.0).show_ui(ui, |ui| {
                for l in LogLevel::ALL {
                    ui.selectable_value(&mut self.capture_level, l, l.name());
                }
            })
            .response
            .on_hover_text("most verbose level the pet records for panels (stderr keeps RUST_LOG)");
            if before != self.capture_level && connected {
                self.link.send(PanelMsg::SetLogLevel(self.capture_level));
                self.log_min = self.log_min.max(self.capture_level);
            }
            ui.add(egui::TextEdit::singleline(&mut self.log_text).hint_text("filter…").desired_width(160.0));
            ui.checkbox(&mut self.autoscroll, "autoscroll");
            if ui.button("Copy").on_hover_text("copy the visible (filtered) lines").clicked() {
                let text: String = self.filtered().into_iter().map(|l| format_line(&l) + "\n").collect();
                ui.ctx().copy_text(text);
            }
            if ui.button("Clear").clicked() {
                self.link.shared().logs.clear();
            }
        });
        ui.separator();

        let lines = self.filtered();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(self.autoscroll).show_rows(ui, row_h, lines.len(), |ui, range| {
            for l in &lines[range] {
                ui.add(egui::Label::new(RichText::new(format_line(l)).monospace().color(level_color(l.level))).wrap_mode(TextWrapMode::Extend));
            }
        });
    }

    fn filtered(&self) -> Vec<LogLine> {
        let text = self.log_text.to_lowercase();
        self.link
            .shared()
            .logs
            .iter()
            .filter(|l| l.level <= self.log_min && (text.is_empty() || l.msg.to_lowercase().contains(&text) || l.target.to_lowercase().contains(&text)))
            .cloned()
            .collect()
    }
}

impl eframe::App for Panel {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (connected, hello, status, session) = {
            let s = self.link.shared();
            (s.connected, s.hello.clone(), s.status.clone(), s.session)
        };
        if connected && session != self.session {
            // New connection: re-apply our preferences.
            self.session = session;
            self.sent_keys = (false, ManualKeys::default());
            if self.capture_level > LogLevel::Info {
                self.link.send(PanelMsg::SetLogLevel(self.capture_level));
            }
        }
        self.keyboard(&ctx, connected);

        egui::Panel::top("bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                match (&hello, connected) {
                    (Some(h), true) => ui.label(RichText::new(format!("Connected to Desk-Skong v{} (pid {})", h.version, h.pid)).color(Color32::from_rgb(90, 200, 110))),
                    _ => ui.label(RichText::new("Not connected — waiting for the pet (skong.exe)…").color(Color32::from_rgb(230, 160, 60))),
                };
                if self.sent_keys.0 {
                    ui.label(RichText::new("· driving with the keyboard").strong());
                }
            });
        });
        egui::Panel::left("controls").resizable(true).default_size(360.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Controls");
                self.controls(ui, status.as_ref(), hello.as_ref(), connected);
                ui.separator();
                ui.heading("Status");
                Self::status(ui, status.as_ref().filter(|_| connected));
                ui.separator();
                ui.collapsing("Shortcuts (when this window is focused)", |ui| {
                    for (k, _, what) in LETTERS {
                        ui.label(format!("{k:?}  {what}"));
                    }
                    ui.label("Arrows / Up / Shift / Ctrl / Z / X  drive the knight");
                    ui.label("Ctrl+Alt+letter works everywhere (global hotkeys)");
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| self.logs(ui, connected));

        // Idle: a slow tick keeps the connection state fresh; data arrival wakes us anyway.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn format_line(l: &LogLine) -> String {
    format!("{:>9.3} {:<5} {}: {}", l.t, l.level.name(), l.target, l.msg)
}

fn level_color(l: LogLevel) -> Color32 {
    match l {
        LogLevel::Error => Color32::from_rgb(240, 90, 90),
        LogLevel::Warn => Color32::from_rgb(235, 180, 70),
        LogLevel::Info => Color32::from_rgb(210, 210, 210),
        LogLevel::Debug => Color32::from_rgb(130, 170, 220),
        LogLevel::Trace => Color32::from_rgb(140, 140, 140),
    }
}

/// `items` joined with ", ", `per_line` per line.
fn wrap_list(items: &[String], per_line: usize) -> String {
    items.chunks(per_line.max(1)).map(|c| c.join(", ")).collect::<Vec<_>>().join("\n")
}
