//! The panel window is Desk-Skong's main window (egui).
//!
//! * **Home** (`ui/home.rs`): achievements with the Geo wallet, and the everyday settings.
//! * **Sandbox** (`ui/sandbox.rs`): locked behind a consent ("achievements are disabled for
//!   the rest of this session"); then quick actions, keyboard driving, the debug overlay and
//!   the Test tab (`ui/test_tab.rs`: make him do one specific thing, place things where you
//!   click). The pet enforces the gate too: it refuses those commands until it's on.
//! * **Diagnostics** (shown with "Advanced"): live status and the pet's logs.
//!
//! Lifecycle: closing this window quits Desk-Skong (it sends `Quit` and waits for it to
//! be written). **Hide to tray** closes only the window; the pet keeps running and a click
//! on its tray icon opens the window again. When the pet quits (tray, Ctrl+Alt+Q) it says
//! goodbye and this window closes too.
//!
//! While the window is focused (and no text field has focus): the letters W G H S B P act
//! like Ctrl+Alt+letter; with the sandbox on also K A R D, and arrows / Up jump / Shift
//! dash / Ctrl superdash / Z walk / X sprint drive the knight in manual mode (a toggle,
//! off by default — the panel never turns it on by itself).
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, RichText, TextWrapMode};
use skong_ipc::{Command, LogLevel, LogLine, ManualKeys, PanelMsg, Status};

use crate::link::Link;

mod home;
#[cfg(test)]
mod render;
mod sandbox;
mod test_tab;

/// Shortcuts available as plain letters while the panel is focused (the sandbox-only ones
/// are ignored until it's on, as the pet would refuse them).
const LETTERS: &[(Key, Command, &str)] = &[
    (Key::W, Command::Wake, "wake"),
    (Key::G, Command::GoToCursor { hurry: false }, "walk to cursor"),
    (Key::H, Command::GoToCursor { hurry: true }, "hurry to cursor"),
    (Key::S, Command::NextSkin, "next skin"),
    (Key::B, Command::PlaceBenchNearKnight(0), "bench near knight"),
    (Key::P, Command::TogglePause, "pause"),
    (Key::K, Command::ToggleManual, "manual control (sandbox)"),
    (Key::A, Command::RandomAct, "do something (sandbox)"),
    (Key::R, Command::RespawnAtCursor, "come here (sandbox)"),
    (Key::D, Command::ToggleDebugOverlay, "debug overlay (sandbox)"),
];

/// The one line that explains the window's lifecycle.
pub const TRAY_HINT: &str = "Desk-Skong keeps running in the tray — click the tray icon to bring this window back; close this window to quit.";

/// How long a "refused" line stays in the footer.
const REFUSED_SECS: u64 = 8;

pub mod style;
pub use style::palette;
use style::palette as pal;
pub use style::{frame, hint, title};

/// Default and smallest window sizes (points): room for the larger type; below
/// `home::TWO_COLUMNS` the Home page stacks its two frames.
const DEFAULT_SIZE: [f32; 2] = [1180.0, 820.0];
const MIN_SIZE: [f32; 2] = [640.0, 520.0];

pub fn run(pipe: String) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Desk-Skong").with_inner_size(DEFAULT_SIZE).with_min_inner_size(MIN_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        "Desk-Skong",
        options,
        Box::new(move |cc| {
            style::apply(&cc.egui_ctx);
            let geo = crate::geo::load().map(|img| {
                let pixels = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &crate::geo::rgba_premultiplied(&img));
                cc.egui_ctx.load_texture("geo", pixels, egui::TextureOptions::LINEAR)
            });
            let ctx = cc.egui_ctx.clone();
            let link = Link::start(pipe, move || ctx.request_repaint());
            Ok(Box::new(Panel::new(link, geo)))
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
    tab: Tab,
    /// Diagnostics tab (status, logs) shown.
    advanced: bool,
    /// "Reset achievements" armed (a second click resets).
    reset_armed: bool,
    /// The sandbox consent box is ticked.
    consent: bool,
    /// Test tab: the visitor picked, the bench style to place.
    visit_who: usize,
    bench_style: usize,
    /// Closing on purpose (hide to tray, the pet said goodbye): don't send Quit.
    closing: bool,
    /// Raise requests already handled.
    raises: u64,
    /// The last refusal shown, and since when.
    refusals: u64,
    refused_at: Option<Instant>,
    /// Ever connected to a pet (else we're still waiting for the first one).
    was_connected: bool,
    started: Instant,
    /// We launched skong.exe ourselves (don't offer again for a while).
    launched_pet: Option<Instant>,
    /// Bring the window back to its normal level next frame (after a raise).
    unpin: bool,
    /// The Geo coin from the player's extracted art (`None`: a drawn stand-in).
    geo: Option<egui::TextureHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Home,
    Sandbox,
    Diagnostics,
}

impl Panel {
    fn new(link: Link, geo: Option<egui::TextureHandle>) -> Self {
        Self {
            geo,
            tab: Tab::Home,
            advanced: false,
            reset_armed: false,
            consent: false,
            visit_who: 0,
            bench_style: 0,
            link,
            skin_filter: String::new(),
            log_text: String::new(),
            log_min: LogLevel::Info,
            capture_level: LogLevel::Info,
            autoscroll: true,
            drive: true,
            sent_keys: (false, ManualKeys::default()),
            session: 0,
            closing: false,
            raises: 0,
            refusals: 0,
            refused_at: None,
            was_connected: false,
            started: Instant::now(),
            launched_pet: None,
            unpin: false,
        }
    }

    fn cmd(&self, c: Command) {
        self.link.send(PanelMsg::Command(c));
    }

    /// Keyboard driving + letter shortcuts.
    fn keyboard(&mut self, ctx: &egui::Context, connected: bool, sandbox: bool) {
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
        let active = connected && sandbox && self.drive && focused && !typing;
        let now = (active, if active { keys } else { ManualKeys::default() });
        if now != self.sent_keys && connected {
            self.link.send(PanelMsg::ManualInput { active: now.0, keys: now.1 });
        }
        self.sent_keys = now;
        if connected && focused && !typing && !mods.ctrl && !mods.alt && !mods.command {
            for c in letters.into_iter().filter(|c| sandbox || !c.needs_sandbox()) {
                self.cmd(c);
            }
        }
    }

    /// Close requests, the pet's goodbye, raise requests.
    fn lifecycle(&mut self, ctx: &egui::Context, connected: bool, bye: bool, raises: u64) {
        if bye && !self.closing {
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            // The window's X: the panel is the app, so the pet quits with it.
            self.closing = true;
            if connected {
                self.cmd(Command::Quit);
                self.link.flush(Duration::from_millis(800));
            }
        }
        if self.unpin {
            self.unpin = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if raises != self.raises {
            // Tray click / second launch: un-minimize and come to the front (briefly on
            // top, as Windows won't always hand focus to a background process).
            self.raises = raises;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.unpin = true;
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Hide to tray: only this window goes; the pet stays (a tray click relaunches it).
    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Top bar: name, connection, the sandbox banner.
    fn top_bar(&mut self, ui: &mut egui::Ui, connected: bool, hello: Option<&skong_ipc::Hello>, sandbox: bool) {
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Desk-Skong").font(style::heading(28.0)).color(pal::SOUL));
            ui.add_space(14.0);
            let (text, color, filled) = match (hello, connected) {
                (Some(h), true) => (format!("the knight is here  ·  v{}", h.version), pal::SUB, true),
                _ if self.was_connected => ("Desk-Skong stopped".to_string(), pal::EMBER, false),
                _ => ("waiting for the knight…".to_string(), pal::SUB, false),
            };
            style::marker(ui, filled, color);
            ui.label(RichText::new(text).small().color(color));
            if self.sent_keys.0 {
                ui.label(RichText::new("·  driving with the keyboard").small().color(pal::TEXT));
            }
        });
        if sandbox && connected {
            ui.add_space(6.0);
            egui::Frame::new().fill(pal::EMBER_BG).stroke(egui::Stroke::new(1.0, pal::EMBER.gamma_multiply(0.6))).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Sandbox on — achievements paused until restart").color(pal::EMBER));
            });
        }
        ui.add_space(8.0);
    }

    /// Not connected: why, and a way to start the pet again.
    fn offline(&mut self, ui: &mut egui::Ui) {
        let waited = self.started.elapsed() > Duration::from_secs(3);
        if !self.was_connected && !waited {
            hint(ui, "waiting for the knight…");
            return;
        }
        frame(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.was_connected {
                title(ui, "Desk-Skong stopped");
                ui.label("The knight isn't running any more (it may have crashed: the log file has the details).");
            } else {
                title(ui, "Desk-Skong isn't running");
                ui.label("This window is its control panel; start the knight to use it.");
            }
            ui.add_space(8.0);
            let exe = pet_exe();
            let recently = self.launched_pet.is_some_and(|t| t.elapsed() < Duration::from_secs(10));
            ui.horizontal(|ui| {
                if ui.add_enabled(exe.is_some() && !recently, style::wide_button("Start Desk-Skong", 190.0)).clicked()
                    && let Some(exe) = &exe
                {
                    match std::process::Command::new(exe).spawn() {
                        Ok(_) => self.launched_pet = Some(Instant::now()),
                        Err(e) => eprintln!("could not start {}: {e}", exe.display()),
                    }
                }
                hint(ui, if recently { "starting…" } else { "or close this window." });
            });
        });
    }

    /// Bottom bar: Hide to tray, the hint, Advanced, the latest refusal.
    fn footer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, connected: bool, refused: Option<&str>) {
        ui.add_space(4.0);
        style::ornament(ui, 260.0);
        if let (Some(why), Some(at)) = (refused, self.refused_at)
            && at.elapsed() < Duration::from_secs(REFUSED_SECS)
        {
            ui.add(egui::Label::new(RichText::new(format!("Refused: {why}")).small().color(pal::EMBER)).wrap());
        }
        ui.horizontal(|ui| {
            let hide = style::wide_button("Hide to tray", 160.0);
            if ui.add_enabled(connected, hide).on_hover_text("close this window; the knight stays. Click the tray icon to bring it back.").on_disabled_hover_text("the knight isn't running: there's no tray to hide in").clicked() {
                self.hide_to_tray(ctx);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.checkbox(&mut self.advanced, RichText::new("Advanced").small().color(pal::SUB)).on_hover_text("show the Diagnostics tab (live status, logs)").changed() && !self.advanced && self.tab == Tab::Diagnostics {
                    self.tab = Tab::Home;
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| hint(ui, TRAY_HINT));
            });
        });
        ui.add_space(8.0);
    }

    fn status(ui: &mut egui::Ui, st: Option<&Status>) {
        let Some(s) = st else {
            hint(ui, "no status yet");
            return;
        };
        let k = &s.knight;
        egui::Grid::new("status").num_columns(2).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
            let mut row = |a: &str, b: String| {
                ui.label(RichText::new(a).color(pal::SUB));
                ui.monospace(b);
                ui.end_row();
            };
            row("Knight", format!("({:.0}, {:.0}) {} {}", k.x, k.y, k.state, if k.facing_right { "→" } else { "←" }));
            row("Clip / act", format!("{} / {}", k.clip, k.act.as_deref().unwrap_or("-")));
            row("Mode", format!("{}{}{}{}", if s.manual { "manual" } else { "brain" }, if s.driven { " (panel driving)" } else { "" }, if s.paused { ", paused" } else { "" }, if s.sandbox { ", sandbox" } else { "" }));
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
            ui.add_space(6.0);
            style::subtitle(ui, "Brain");
            ui.add(egui::Label::new(RichText::new(b).monospace()).wrap());
        }
    }

    /// Diagnostics: live status on the left, the pet's logs on the right.
    fn diagnostics(&mut self, ui: &mut egui::Ui, status: Option<&Status>, connected: bool) {
        egui::Panel::left("diag_status").resizable(true).default_size(400.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                title(ui, "Status");
                Self::status(ui, status.filter(|_| connected));
            });
        });
        egui::CentralPanel::default().show(ui, |ui| self.logs(ui, connected));
    }

    fn logs(&mut self, ui: &mut egui::Ui, connected: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Show");
            egui::ComboBox::from_id_salt("min").selected_text(self.log_min.name()).width(90.0).show_ui(ui, |ui| {
                for l in LogLevel::ALL {
                    ui.selectable_value(&mut self.log_min, l, l.name());
                }
            });
            ui.label("capture");
            let before = self.capture_level;
            egui::ComboBox::from_id_salt("capture").selected_text(self.capture_level.name()).width(90.0).show_ui(ui, |ui| {
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
        let (connected, hello, status, session, achievements, bye, raises, refused, refusals) = {
            let s = self.link.shared();
            (s.connected, s.hello.clone(), s.status.clone(), s.session, s.achievements.clone(), s.bye, s.raises, s.refused.clone(), s.refusals)
        };
        self.was_connected |= connected;
        if refusals != self.refusals {
            self.refusals = refusals;
            self.refused_at = Some(Instant::now());
        }
        let sandbox = connected && status.as_ref().is_some_and(|s| s.sandbox);
        if connected && session != self.session {
            // New connection: re-apply our preferences.
            self.session = session;
            self.sent_keys = (false, ManualKeys::default());
            if self.capture_level > LogLevel::Info {
                self.link.send(PanelMsg::SetLogLevel(self.capture_level));
            }
        }
        self.lifecycle(&ctx, connected, bye, raises);
        self.keyboard(&ctx, connected, sandbox);

        egui::Panel::top("bar").show(ui, |ui| self.top_bar(ui, connected, hello.as_ref(), sandbox));
        egui::Panel::bottom("footer").show(ui, |ui| self.footer(ui, &ctx, connected, refused.as_deref()));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let count = achievements.as_ref().map_or_else(String::new, |a| format!("  {} / {}", a.unlocked(), a.total()));
                if style::tab(ui, self.tab == Tab::Home, &format!("Home{count}")).clicked() {
                    self.tab = Tab::Home;
                }
                let placing = status.as_ref().and_then(|s| s.test.as_ref()).is_some_and(|t| t.placing.is_some());
                let label = match (sandbox, placing) {
                    (false, _) => "Sandbox (locked)",
                    (true, false) => "Sandbox",
                    (true, true) => "Sandbox  · placing",
                };
                if style::tab(ui, self.tab == Tab::Sandbox, label).on_hover_text("make him do things on request, choose where they appear, debug tools (pauses achievements until restart)").clicked() {
                    self.tab = Tab::Sandbox;
                }
                if self.advanced && style::tab(ui, self.tab == Tab::Diagnostics, "Diagnostics").clicked() {
                    self.tab = Tab::Diagnostics;
                }
            });
            ui.add_space(6.0);
            if !connected {
                self.offline(ui);
                return;
            }
            match self.tab {
                Tab::Home => home::show(self, ui, status.as_ref(), hello.as_ref(), achievements.as_ref()),
                Tab::Sandbox => sandbox::show(self, ui, status.as_ref(), hello.as_ref(), sandbox),
                Tab::Diagnostics => self.diagnostics(ui, status.as_ref(), connected),
            }
        });

        // Idle: a slow tick keeps the connection state fresh; data arrival wakes us anyway.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

/// `skong.exe` next to this panel, if there is one.
fn pet_exe() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?.parent()?.join("skong.exe");
    exe.is_file().then_some(exe)
}

fn format_line(l: &LogLine) -> String {
    format!("{:>9.3} {:<5} {}: {}", l.t, l.level.name(), l.target, l.msg)
}

fn level_color(l: LogLevel) -> Color32 {
    match l {
        LogLevel::Error => pal::ROSE,
        LogLevel::Warn => pal::EMBER,
        LogLevel::Info => pal::TEXT,
        LogLevel::Debug => Color32::from_rgb(170, 190, 216),
        LogLevel::Trace => Color32::from_rgb(160, 166, 178),
    }
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
fn ago(now: u64, at: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// `items` joined with ", ", `per_line` per line.
fn wrap_list(items: &[String], per_line: usize) -> String {
    items.chunks(per_line.max(1)).map(|c| c.join(", ")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_shortcuts_respect_the_sandbox() {
        // Every letter that would be refused is marked as such for the user.
        for (k, c, what) in LETTERS {
            assert_eq!(c.needs_sandbox(), what.ends_with("(sandbox)"), "{k:?} {what}");
        }
    }

    #[test]
    fn ago_reads_naturally() {
        assert_eq!(ago(100, 90), "just now");
        assert_eq!(ago(4000, 100), "1 h ago");
        assert_eq!(ago(100_000, 0), "yesterday");
    }
}
