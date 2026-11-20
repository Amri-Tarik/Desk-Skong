//! The front page: the journal (achievements, with the Geo wallet and the time spent with
//! the knight) and the everyday settings. Nothing here makes the knight do anything on
//! request — that's the sandbox.
use eframe::egui::{self, Color32, RichText, Stroke};
use skong_ipc::{AchievementsInfo, Command, Hello, Status};

use super::palette as pal;
use super::style::{self, hint};
use super::{Panel, ago, frame, title};

/// Unlocks this recent are marked new (s).
const NEW_SECS: u64 = 15 * 60;
/// Below this width the two frames stack.
pub const TWO_COLUMNS: f32 = 900.0;
/// The Geo icon's size beside the count (points).
const GEO_SIZE: f32 = 26.0;

pub(super) fn show(p: &mut Panel, ui: &mut egui::Ui, status: Option<&Status>, hello: Option<&Hello>, achievements: Option<&AchievementsInfo>) {
    let Some(h) = hello else {
        hint(ui, "waiting for the knight…");
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
            ui.add_space(10.0);
            settings(p, ui, s, h);
        });
    }
}

/// Time played as the game's save slots show it: "12h 34m" ("0h 05m" early on).
pub fn played(secs: u64) -> String {
    format!("{}h {:02}m", secs / 3600, secs / 60 % 60)
}

/// The Geo icon: the real coin sprite from the player's art, else a drawn stand-in — a
/// pale, rounded stone with a dark rim and a carved ring (like the game's Geo, not gold).
fn geo_icon(p: &Panel, ui: &mut egui::Ui) {
    if let Some(tex) = &p.geo {
        let [w, h] = tex.size();
        let k = GEO_SIZE / w.max(h).max(1) as f32;
        ui.add(egui::Image::new((tex.id(), egui::vec2(w as f32 * k, h as f32 * k))));
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(GEO_SIZE, GEO_SIZE), egui::Sense::hover());
    let pt = ui.painter();
    let (c, r) = (rect.center(), GEO_SIZE * 0.42);
    pt.circle_filled(c, r + 1.5, Color32::from_rgb(24, 26, 30));
    pt.circle_filled(c, r, Color32::from_rgb(214, 210, 196));
    pt.circle_filled(c + egui::vec2(r * 0.12, r * 0.14), r * 0.82, Color32::from_rgb(188, 184, 170));
    pt.circle_stroke(c, r * 0.45, Stroke::new(1.6, Color32::from_rgb(92, 92, 90)));
    pt.circle_filled(c + egui::vec2(-r * 0.42, -r * 0.42), r * 0.16, Color32::from_rgb(246, 244, 236));
}

/// Achievements: count, progress, the Geo wallet and time played, the found ones, the hints.
fn journal(p: &mut Panel, ui: &mut egui::Ui, s: &Status, info: Option<&AchievementsInfo>) {
    frame(ui, |ui| {
        ui.set_width(ui.available_width());
        title(ui, "Journal");
        // Statistics: Geo and time played (plain numbers, not achievements).
        ui.horizontal_wrapped(|ui| {
            if let Some(st) = &s.stakes {
                geo_icon(p, ui);
                ui.label(RichText::new(st.geo.to_string()).font(style::heading(22.0)).color(pal::TEXT)).on_hover_text("Geo the knight has collected (a score: nothing to spend it on)");
                ui.add_space(18.0);
            }
            ui.label(RichText::new("Time with the knight").small().color(pal::SUB));
            ui.label(RichText::new(played(s.played_secs)).font(style::heading(18.0)).color(pal::TEXT)).on_hover_text("time Desk-Skong has run, over every session");
        });
        ui.add_space(4.0);
        let Some(info) = info else {
            hint(ui, "this pet doesn't keep achievements");
            return;
        };
        let (got, total) = (info.unlocked(), info.total());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{got} / {total}")).font(style::heading(22.0)).color(pal::SOUL));
            ui.add_space(6.0);
            let frac = if total == 0 { 0.0 } else { got as f32 / total as f32 };
            style::progress(ui, frac, ui.available_width().min(300.0));
        });
        if s.sandbox {
            ui.label(RichText::new("Sandbox on — achievements paused until restart. Geo collected now isn't kept.").color(pal::EMBER));
        } else {
            hint(ui, "Each one stays hidden until the knight does it. Play with him, and see what he can do.");
        }
        ui.add_space(4.0);
        style::ornament(ui, 120.0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut found: Vec<_> = info.entries.iter().filter_map(|e| e.unlocked.as_ref()).collect();
        found.sort_by_key(|u| std::cmp::Reverse(u.at));
        egui::ScrollArea::vertical().id_salt("journal").auto_shrink([false, false]).max_height(ui.available_height() - 4.0).show(ui, |ui| {
            for u in found {
                ui.horizontal(|ui| {
                    style::marker(ui, true, pal::SOUL);
                    ui.label(RichText::new(&u.title).font(style::heading(17.0)).color(pal::TEXT));
                    if now.saturating_sub(u.at) < NEW_SECS {
                        ui.label(RichText::new("new").small().italics().color(pal::SOUL));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(ago(now, u.at)).small().color(pal::SUB)));
                });
                ui.indent(&u.id, |ui| ui.add(egui::Label::new(RichText::new(&u.description).color(pal::TEXT)).wrap()));
                ui.add_space(6.0);
            }
            for e in info.entries.iter().filter(|e| e.unlocked.is_none()) {
                ui.horizontal_wrapped(|ui| {
                    style::marker(ui, false, pal::LINE);
                    ui.label(RichText::new("???").color(pal::SUB));
                    if let Some(h) = &e.hint {
                        ui.label(RichText::new(h).small().italics().color(pal::SUB));
                    }
                });
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let label = if p.reset_armed { "Really forget them all?" } else { "Reset achievements…" };
                if ui.add(egui::Button::new(RichText::new(label).small().color(pal::ROSE)).frame(p.reset_armed)).clicked() {
                    if p.reset_armed {
                        p.cmd(Command::ResetAchievements);
                    }
                    p.reset_armed = !p.reset_armed;
                }
                if p.reset_armed && ui.button(RichText::new("Keep them").small()).clicked() {
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
    ui.indent(label, |ui| hint(ui, about));
    ui.add_space(4.0);
}

/// Skin, size, mischief, the switches, benches.
fn settings(p: &mut Panel, ui: &mut egui::Ui, s: &Status, h: &Hello) {
    frame(ui, |ui| {
        ui.set_width(ui.available_width());
        title(ui, "The Knight");
        egui::Grid::new("settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label(RichText::new("Skin").color(pal::SUB));
            let current = h.skins.get(s.skin).cloned().unwrap_or_default();
            egui::ComboBox::from_id_salt("skin").selected_text(current).width(240.0).height(440.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.skin_filter).hint_text("search…").desired_width(220.0));
                let f = p.skin_filter.to_lowercase();
                for (i, name) in h.skins.iter().enumerate().filter(|(_, n)| f.is_empty() || n.to_lowercase().contains(&f)) {
                    if ui.selectable_label(i == s.skin, name.as_str()).clicked() {
                        p.cmd(Command::SetSkin(i));
                    }
                }
            });
            ui.end_row();

            ui.label(RichText::new("Size").color(pal::SUB));
            ui.horizontal_wrapped(|ui| {
                for (name, v) in &h.scales {
                    if ui.selectable_label((s.scale - v).abs() < 1e-3, name.as_str()).clicked() {
                        p.cmd(Command::SetScale(*v));
                    }
                }
            });
            ui.end_row();

            ui.label(RichText::new("Mischief").color(pal::SUB)).on_hover_text("how often he comes to bother you (cursor battles…)");
            ui.horizontal_wrapped(|ui| {
                for (name, v) in &h.bother_levels {
                    if ui.selectable_label((s.bother - v).abs() < 1e-3, name.as_str()).clicked() {
                        p.cmd(Command::SetBother(*v));
                    }
                }
            });
            ui.end_row();
        });
        ui.add_space(10.0);
        toggle(p, ui, s.share_visible, "Visible in screen shares", "show him in screenshots, Discord, OBS… (hidden from them by default)", Command::ToggleShareVisible);
        toggle(p, ui, s.lantern, "Lantern on dark backgrounds", "he holds up his lantern over dark-mode windows or a dark wallpaper", Command::ToggleLantern);
        toggle(p, ui, s.text_curiosity, "Curious about highlighted text", "he comes to look where you select text (only where it is: the text is never read)", Command::ToggleTextCuriosity);
        toggle(p, ui, s.fight_mode, "Fight the cursor", "round after round, until you switch it off", Command::ToggleFightMode);
        toggle(p, ui, s.vision, "Stands on lines inside windows", "watches window contents for ledges (screen capture, never saved); off: title bars and the floor only", Command::ToggleVision);
        toggle(p, ui, s.paused, "Pause", "hide him and stop everything until you switch it back", Command::TogglePause);

        style::ornament(ui, 120.0);
        style::subtitle(ui, "Benches");
        ui.horizontal_wrapped(|ui| {
            for (i, name) in h.bench_styles.iter().enumerate() {
                let ok = s.benches_available.get(i).copied().unwrap_or(true);
                if ui.add_enabled(ok, egui::Button::new(name.as_str())).on_hover_text("place one next to the knight").clicked() {
                    p.cmd(Command::PlaceBenchNearKnight(i));
                }
            }
            if ui.add_enabled(s.benches_placed > 0, egui::Button::new("Remove all")).clicked() {
                p.cmd(Command::ClearBenches);
            }
        });
        ui.add_space(8.0);
        style::subtitle(ui, "Upkeep");
        ui.horizontal_wrapped(|ui| {
            let mut log_file = s.log_file;
            let tip = format!("Keeps the log in {} (last 5 files) to attach to a bug report. Crash reports are written there either way. Nothing is sent anywhere.", if s.log_dir.is_empty() { "the logs folder" } else { &s.log_dir });
            if ui.checkbox(&mut log_file, RichText::new("Write a log file (for bug reports)").color(pal::TEXT)).on_hover_text(tip).changed() {
                p.cmd(Command::ToggleLogFile);
            }
            if ui.button("Open folder").on_hover_text("log and crash files").clicked() {
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
        ui.add_space(10.0);
        hint(ui, "Anywhere: Ctrl+Alt+G / H he walks / hurries to your cursor · Ctrl+Alt+W wakes him · Ctrl+Alt+M the tray menu · Ctrl+Alt+Q quits");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_played_reads_like_a_save_slot() {
        assert_eq!(played(0), "0h 00m");
        assert_eq!(played(59), "0h 00m");
        assert_eq!(played(5 * 60), "0h 05m");
        assert_eq!(played(12 * 3600 + 34 * 60 + 59), "12h 34m");
        assert_eq!(played(250 * 3600), "250h 00m");
    }
}
