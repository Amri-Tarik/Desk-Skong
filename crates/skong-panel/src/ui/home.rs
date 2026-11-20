//! The front page: the achievements journal (with the Geo wallet) and the everyday
//! settings. Nothing here makes the knight do anything on request — that's the sandbox.
use eframe::egui::{self, Color32, RichText};
use skong_ipc::{AchievementsInfo, Command, Hello, Status};

use super::palette as pal;
use super::{Panel, ago, card, title};

/// Unlocks this recent are marked new (s).
const NEW_SECS: u64 = 15 * 60;
/// Below this width the two cards stack.
const TWO_COLUMNS: f32 = 760.0;

pub(super) fn show(p: &mut Panel, ui: &mut egui::Ui, status: Option<&Status>, hello: Option<&Hello>, achievements: Option<&AchievementsInfo>) {
    let Some(h) = hello else {
        ui.weak("waiting for the knight…");
        return;
    };
    let st_default = Status::default();
    let s = status.unwrap_or(&st_default);
    if ui.available_width() >= TWO_COLUMNS {
        ui.columns(2, |cols| {
            journal(p, &mut cols[0], s, achievements);
            egui::ScrollArea::vertical().id_salt("settings_scroll").auto_shrink([false, false]).show(&mut cols[1], |ui| settings(p, ui, s, h));
        });
    } else {
        egui::ScrollArea::vertical().id_salt("home_scroll").auto_shrink([false, false]).show(ui, |ui| {
            journal(p, ui, s, achievements);
            ui.add_space(8.0);
            settings(p, ui, s, h);
        });
    }
}

/// A Geo coin drawn in code (no game art): a pale gold disc, dark rim, inner ring, glint.
fn geo_coin(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    let r = size * 0.5;
    p.circle_filled(c, r, Color32::from_rgb(46, 40, 28));
    p.circle_filled(c, r - 1.6, Color32::from_rgb(222, 200, 138));
    p.circle_stroke(c, r * 0.55, egui::Stroke::new(1.2, Color32::from_rgb(156, 130, 74)));
    p.circle_filled(c + egui::vec2(-r * 0.35, -r * 0.35), r * 0.18, Color32::from_rgb(252, 242, 212));
}

/// Achievements: count, progress, the Geo wallet, the found ones and the hints.
fn journal(p: &mut Panel, ui: &mut egui::Ui, s: &Status, info: Option<&AchievementsInfo>) {
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            title(ui, "Achievements");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(st) = &s.stakes {
                    ui.label(RichText::new(st.geo.to_string()).size(18.0).strong().color(pal::GOLD)).on_hover_text("Geo the knight has collected (a score: nothing to spend it on)");
                    geo_coin(ui, 18.0);
                }
            });
        });
        let Some(info) = info else {
            ui.weak("this pet doesn't keep achievements");
            return;
        };
        let (got, total) = (info.unlocked(), info.total());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{got} / {total}")).size(22.0).strong().color(pal::TEXT));
            let frac = if total == 0 { 0.0 } else { got as f32 / total as f32 };
            ui.add(egui::ProgressBar::new(frac).desired_width(ui.available_width().min(260.0)).fill(Color32::from_rgb(150, 126, 70)));
        });
        if s.sandbox {
            ui.label(RichText::new("Sandbox on — achievements paused until restart. Geo collected now isn't kept.").color(pal::AMBER));
        } else {
            ui.label(RichText::new("Each one stays hidden until the knight does it. Play with him, and see what he can do.").small().color(pal::WEAK));
        }
        ui.add_space(6.0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut found: Vec<_> = info.entries.iter().filter_map(|e| e.unlocked.as_ref()).collect();
        found.sort_by_key(|u| std::cmp::Reverse(u.at));
        egui::ScrollArea::vertical().id_salt("journal").auto_shrink([false, false]).max_height(ui.available_height() - 4.0).show(ui, |ui| {
            for u in found {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◆").color(pal::GOLD));
                    ui.label(RichText::new(&u.title).strong().color(pal::GOLD));
                    if now.saturating_sub(u.at) < NEW_SECS {
                        ui.label(RichText::new("new").small().color(pal::GREEN));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(ago(now, u.at)).small().color(pal::WEAK)));
                });
                ui.indent(&u.id, |ui| ui.label(RichText::new(&u.description).color(pal::TEXT)));
                ui.add_space(4.0);
            }
            for e in info.entries.iter().filter(|e| e.unlocked.is_none()) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◇").color(pal::WEAK));
                    ui.label(RichText::new("???").color(pal::WEAK));
                    if let Some(h) = &e.hint {
                        ui.label(RichText::new(h).small().italics().color(pal::WEAK));
                    }
                });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = if p.reset_armed { "Really forget them all?" } else { "Reset achievements…" };
                if ui.add(egui::Button::new(RichText::new(label).small().color(pal::RED)).frame(p.reset_armed)).clicked() {
                    if p.reset_armed {
                        p.cmd(Command::ResetAchievements);
                    }
                    p.reset_armed = !p.reset_armed;
                }
                if p.reset_armed && ui.small_button("Keep them").clicked() {
                    p.reset_armed = false;
                }
            });
        });
    });
}

/// A labelled switch with a line of explanation under it.
fn toggle(p: &Panel, ui: &mut egui::Ui, on: bool, label: &str, about: &str, c: Command) {
    let mut v = on;
    if ui.checkbox(&mut v, RichText::new(label).color(pal::TEXT)).changed() {
        p.cmd(c);
    }
    ui.indent(label, |ui| ui.label(RichText::new(about).small().color(pal::WEAK)));
    ui.add_space(2.0);
}

/// Skin, size, mischief, the switches, benches.
fn settings(p: &mut Panel, ui: &mut egui::Ui, s: &Status, h: &Hello) {
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        title(ui, "The knight");
        ui.add_space(4.0);
        egui::Grid::new("settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Skin");
            let current = h.skins.get(s.skin).cloned().unwrap_or_default();
            egui::ComboBox::from_id_salt("skin").selected_text(current).width(200.0).height(400.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.skin_filter).hint_text("search…").desired_width(180.0));
                let f = p.skin_filter.to_lowercase();
                for (i, name) in h.skins.iter().enumerate().filter(|(_, n)| f.is_empty() || n.to_lowercase().contains(&f)) {
                    if ui.selectable_label(i == s.skin, name.as_str()).clicked() {
                        p.cmd(Command::SetSkin(i));
                    }
                }
            });
            ui.end_row();

            ui.label("Size");
            ui.horizontal_wrapped(|ui| {
                for (name, v) in &h.scales {
                    if ui.selectable_label((s.scale - v).abs() < 1e-3, name.as_str()).clicked() {
                        p.cmd(Command::SetScale(*v));
                    }
                }
            });
            ui.end_row();

            ui.label("Mischief").on_hover_text("how often he comes to bother you (cursor battles…)");
            ui.horizontal_wrapped(|ui| {
                for (name, v) in &h.bother_levels {
                    if ui.selectable_label((s.bother - v).abs() < 1e-3, name.as_str()).clicked() {
                        p.cmd(Command::SetBother(*v));
                    }
                }
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        toggle(p, ui, s.share_visible, "Visible in screen shares", "show him in screenshots, Discord, OBS… (hidden from them by default)", Command::ToggleShareVisible);
        toggle(p, ui, s.lantern, "Lantern on dark backgrounds", "he holds up his lantern over dark-mode windows or a dark wallpaper", Command::ToggleLantern);
        toggle(p, ui, s.text_curiosity, "Curious about highlighted text", "he comes to look where you select text (only where it is: the text is never read)", Command::ToggleTextCuriosity);
        toggle(p, ui, s.fight_mode, "Fight the cursor", "round after round, until you switch it off", Command::ToggleFightMode);
        toggle(p, ui, s.vision, "Stands on lines inside windows", "watches window contents for ledges (screen capture, never saved); off: title bars and the floor only", Command::ToggleVision);
        toggle(p, ui, s.paused, "Pause", "hide him and stop everything until you switch it back", Command::TogglePause);

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Place a bench");
            for (i, name) in h.bench_styles.iter().enumerate() {
                let ok = s.benches_available.get(i).copied().unwrap_or(true);
                if ui.add_enabled(ok, egui::Button::new(name.as_str())).on_hover_text("next to the knight").clicked() {
                    p.cmd(Command::PlaceBenchNearKnight(i));
                }
            }
            if ui.add_enabled(s.benches_placed > 0, egui::Button::new("Remove all")).clicked() {
                p.cmd(Command::ClearBenches);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut log_file = s.log_file;
            let tip = format!("Keeps the log in {} (last 5 files) to attach to a bug report. Crash reports are written there either way. Nothing is sent anywhere.", if s.log_dir.is_empty() { "the logs folder" } else { &s.log_dir });
            if ui.checkbox(&mut log_file, RichText::new("Write a log file (for bug reports)").color(pal::TEXT)).on_hover_text(tip).changed() {
                p.cmd(Command::ToggleLogFile);
            }
            if ui.small_button("Open folder").on_hover_text("log and crash files").clicked() {
                p.cmd(Command::OpenLogFolder);
            }
        });
        if ui
            .button("Re-extract game art…")
            .on_hover_text("Read the sprites from your Hollow Knight install again (after a game update, or if some look wrong). They stay on this PC only.")
            .clicked()
        {
            p.cmd(Command::ReextractArt);
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Anywhere: Ctrl+Alt+G / H he walks / hurries to your cursor · Ctrl+Alt+W wakes him · Ctrl+Alt+M the tray menu · Ctrl+Alt+Q quits").small().color(pal::WEAK));
    });
}
