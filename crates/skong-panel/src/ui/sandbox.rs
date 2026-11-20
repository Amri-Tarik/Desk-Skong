//! The Sandbox tab. Locked: what it does and what it costs, a consent box and "Enable
//! sandbox". Unlocked: quick actions, keyboard driving and the debug overlay on the left,
//! the Test tab (`test_tab.rs`: one specific thing now, 📍 where you click) on the right.
use eframe::egui::{self, Color32, RichText};
use skong_ipc::{Command, Hello, OverlayLayers, Status};

use super::palette as pal;
use super::{LETTERS, Panel, card, test_tab, title};

pub(super) fn show(p: &mut Panel, ui: &mut egui::Ui, status: Option<&Status>, hello: Option<&Hello>, on: bool) {
    if !on {
        gate(p, ui);
        return;
    }
    let st_default = Status::default();
    let s = status.unwrap_or(&st_default);
    egui::Panel::left("sandbox_tools").resizable(true).default_size(290.0).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| tools(p, ui, s));
    });
    egui::CentralPanel::default().show(ui, |ui| test_tab::show(p, ui, status, hello));
}

/// The consent: nothing debug is reachable before it.
fn gate(p: &mut Panel, ui: &mut egui::Ui) {
    ui.add_space(8.0);
    card(ui, |ui| {
        ui.set_max_width(620.0);
        title(ui, "Sandbox");
        ui.add_space(4.0);
        ui.label(
            "Make the knight do things on request: call a visitor and pick the scene, send enemies or a gauntlet, light the scarlet flame, spill Geo, cast spells, drive him with the keyboard — and choose where things appear with 📍 (your next click on screen). Debug tools too: the terrain overlay, diagnostics.",
        );
        ui.add_space(8.0);
        egui::Frame::new().fill(Color32::from_rgb(52, 36, 22)).corner_radius(4.0).inner_margin(10.0).show(ui, |ui| {
            ui.label(RichText::new("Achievements are disabled for the rest of this session.").strong().color(pal::AMBER));
            ui.label("Once the sandbox is on, nothing unlocks and no progress counts (Geo collected isn't kept either) until you restart Desk-Skong. The achievements you already have are safe. There's no switching it off: restart to play for real again.");
        });
        ui.add_space(8.0);
        ui.checkbox(&mut p.consent, "I understand: no achievements until I restart Desk-Skong");
        ui.add_space(4.0);
        let button = egui::Button::new(RichText::new("Enable sandbox").strong()).min_size(egui::vec2(160.0, 30.0));
        if ui.add_enabled(p.consent, button).clicked() {
            p.cmd(Command::EnableSandbox);
            p.consent = false;
        }
    });
}

/// Quick actions, manual control, the debug overlay, the shortcuts.
fn tools(p: &mut Panel, ui: &mut egui::Ui, s: &Status) {
    title(ui, "Quick actions");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Do something!  [A]").on_hover_text("a random act now").clicked() {
            p.cmd(Command::RandomAct);
        }
        if ui.button("Come here  [R]").on_hover_text("respawn at the mouse cursor").clicked() {
            p.cmd(Command::RespawnAtCursor);
        }
        if ui.button("Wake  [W]").clicked() {
            p.cmd(Command::Wake);
        }
    });
    ui.add_space(8.0);
    title(ui, "Drive him");
    let mut manual = s.manual;
    if ui.checkbox(&mut manual, "Manual control  [K]").changed() {
        p.cmd(Command::ToggleManual);
    }
    ui.checkbox(&mut p.drive, "Keys drive the knight while this window has focus")
        .on_hover_text("arrows move, Up jumps, Shift dashes, Ctrl charges a superdash, Z walk, X sprint (manual control on)");
    ui.add_space(8.0);
    title(ui, "Debug overlay");
    let mut overlay = s.overlay;
    if ui.checkbox(&mut overlay, "Terrain / nav overlay  [D]").changed() {
        p.cmd(Command::ToggleDebugOverlay);
    }
    ui.indent("layers", |ui| {
        let mut l: OverlayLayers = s.overlay_layers;
        let mut changed = ui.checkbox(&mut l.terrain, "terrain (ledges, walls)").changed();
        changed |= ui.checkbox(&mut l.nav, "nav graph (surfaces, edges)").changed();
        changed |= ui.checkbox(&mut l.route, "current route").changed();
        if changed {
            p.cmd(Command::SetOverlayLayers(l));
        }
    });
    ui.add_space(8.0);
    egui::CollapsingHeader::new("Shortcuts (this window focused)").show(ui, |ui| {
        for (k, _, what) in LETTERS {
            ui.label(RichText::new(format!("{k:?}  {what}")).small());
        }
        ui.label(RichText::new("Arrows / Up / Shift / Ctrl / Z / X  drive the knight").small());
        ui.label(RichText::new("Ctrl+Alt+letter works everywhere (global hotkeys)").small());
    });
}
