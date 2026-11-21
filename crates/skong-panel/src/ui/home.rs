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
/// The journal's figures: type size, and the height of their row (room for the coin).
const STAT_SIZE: f32 = 24.0;
const STAT_ROW: f32 = 34.0;
/// The journal's list height when the frames are stacked (the page scrolls instead).
const STACKED_LIST: f32 = 340.0;
/// The settings' label column (Skin, Size, Mischief), so their controls line up.
const LABEL_W: f32 = 92.0;

pub(super) fn show(p: &mut Panel, ui: &mut egui::Ui, status: Option<&Status>, hello: Option<&Hello>, achievements: Option<&AchievementsInfo>) {
    let Some(h) = hello else {
        hint(ui, "waiting for the knight…");
        return;
    };
    let st_default = Status::default();
    let s = status.unwrap_or(&st_default);
    if ui.available_width() >= TWO_COLUMNS {
        // Side by side, both frames the full height (their bottoms line up); each keeps its
        // title in view and scrolls its own contents.
        // (Columns lay out justified, which would stretch wrapped text: plain top-down.)
        let plain = egui::Layout::top_down(egui::Align::Min);
        ui.columns(2, |cols| {
            cols[0].with_layout(plain, |ui| journal(p, ui, s, achievements, true));
            cols[1].with_layout(plain, |ui| settings(p, ui, s, h, true));
        });
    } else {
        egui::ScrollArea::vertical().id_salt("home_scroll").auto_shrink([false, false]).show(ui, |ui| {
            journal(p, ui, s, achievements, false);
            ui.add_space(10.0);
            settings(p, ui, s, h, false);
        });
    }
}

/// The contents of a frame below its title: scrolling and filling the frame (`fill`), or
/// laid out in full (the page itself scrolls).
fn body(ui: &mut egui::Ui, id: &str, fill: bool, add: impl FnOnce(&mut egui::Ui)) {
    if fill {
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).max_height(ui.available_height()).show(ui, add);
    } else {
        add(ui);
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

/// One figure of the journal's stats band: a large number (with the Geo coin before it
/// for `geo`), centred, and a small caption under it.
fn stat(ui: &mut egui::Ui, geo: Option<&Panel>, value: &str, caption: &str, tip: &str) {
    let font = style::heading(STAT_SIZE);
    let gap = 7.0;
    let text_w = ui.painter().layout_no_wrap(value.to_string(), font.clone(), pal::TEXT).size().x;
    let w = ui.available_width();
    let used = text_w + if geo.is_some() { GEO_SIZE + gap } else { 0.0 };
    ui.allocate_ui_with_layout(egui::vec2(w, STAT_ROW), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(egui::vec2(w, STAT_ROW));
        ui.spacing_mut().item_spacing.x = gap;
        ui.add_space(((w - used) * 0.5).max(0.0));
        if let Some(p) = geo {
            geo_icon(p, ui);
        }
        ui.label(RichText::new(value).font(font).color(pal::TEXT)).on_hover_text(tip);
    });
    ui.vertical_centered(|ui| {
        ui.add(egui::Label::new(RichText::new(caption).small().color(pal::SUB)).wrap());
    });
}

/// The journal's figures side by side in equal columns, with faint rules between them.
fn stats_band(p: &Panel, ui: &mut egui::Ui, s: &Status, info: Option<&AchievementsInfo>) {
    let geo = s.stakes.as_ref().map(|st| st.geo);
    let n = 1 + usize::from(geo.is_some()) + usize::from(info.is_some());
    let (left, width, top) = (ui.cursor().left(), ui.available_width(), ui.cursor().top());
    ui.columns(n, |cols| {
        let mut cols = cols.iter_mut();
        if let (Some(geo), Some(ui)) = (geo, cols.next()) {
            stat(ui, Some(p), &geo.to_string(), "Geo", "Geo the knight has collected (a score: nothing to spend it on)");
        }
        if let Some(ui) = cols.next() {
            stat(ui, None, &played(s.played_secs), "Time with the knight", "time Desk-Skong has run, over every session");
        }
        if let (Some(info), Some(ui)) = (info, cols.next()) {
            stat(ui, None, &format!("{} / {}", info.unlocked(), info.total()), "Achievements found", "achievements found");
        }
    });
    let bottom = ui.min_rect().bottom();
    for k in 1..n {
        let x = (left + width * k as f32 / n as f32).round() + 0.5;
        ui.painter().line_segment([egui::pos2(x, top + 6.0), egui::pos2(x, bottom - 4.0)], Stroke::new(1.0, pal::LINE_FAINT));
    }
}

/// Achievements: the stats band (Geo, time played, the count), the found ones, the hints.
fn journal(p: &mut Panel, ui: &mut egui::Ui, s: &Status, info: Option<&AchievementsInfo>, fill: bool) {
    frame(ui, |ui| {
        ui.set_width(ui.available_width());
        title(ui, "Journal");
        ui.add_space(4.0);
        stats_band(p, ui, s, info);
        ui.add_space(8.0);
        let Some(info) = info else {
            ui.vertical_centered(|ui| hint(ui, "this pet doesn't keep achievements"));
            return;
        };
        ui.vertical_centered(|ui| {
            if s.sandbox {
                ui.add(egui::Label::new(RichText::new("Sandbox on — achievements paused until restart. Geo collected now isn't kept.").color(pal::EMBER)).wrap());
            } else {
                hint(ui, "Each one stays hidden until the knight does it. Play with him, and see what he can do.");
            }
        });
        ui.add_space(2.0);
        style::ornament(ui, 120.0);
        ui.add_space(2.0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut found: Vec<_> = info.entries.iter().filter_map(|e| e.unlocked.as_ref()).collect();
        found.sort_by_key(|u| std::cmp::Reverse(u.at));
        let height = if fill { ui.available_height() } else { STACKED_LIST };
        egui::ScrollArea::vertical().id_salt("journal").auto_shrink([false, false]).max_height(height).show(ui, |ui| {
            for u in found {
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        style::marker(ui, true, pal::SOUL);
                        ui.label(RichText::new(&u.title).font(style::heading(17.0)).color(pal::TEXT));
                        if now.saturating_sub(u.at) < NEW_SECS {
                            ui.label(RichText::new("new").small().italics().color(pal::SOUL));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(ago(now, u.at)).small().color(pal::SUB)));
                    });
                    ui.indent(&u.id, |ui| ui.add(egui::Label::new(RichText::new(&u.description).color(pal::TEXT)).wrap()));
                });
                ui.add_space(10.0);
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

/// A labelled switch with a line of explanation under it, lined up with the label's text.
fn toggle(p: &Panel, ui: &mut egui::Ui, on: bool, label: &str, about: &str, c: Command) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut v = on;
        if ui.checkbox(&mut v, RichText::new(label).color(pal::TEXT)).changed() {
            p.cmd(c);
        }
        let pad = ui.spacing().icon_width + ui.spacing().icon_spacing;
        egui::Frame::new().inner_margin(egui::Margin { left: pad.round() as i8, ..egui::Margin::ZERO }).show(ui, |ui| hint(ui, about));
    });
    ui.add_space(6.0);
}

/// A settings row: the name in the label column, then its control(s) (wrapping under
/// themselves, never under the name).
fn row(ui: &mut egui::Ui, label: &str, tip: Option<&str>, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let h = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(egui::vec2(LABEL_W, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(egui::vec2(LABEL_W, h));
            let r = ui.label(RichText::new(label).color(pal::SUB));
            if let Some(tip) = tip {
                r.on_hover_text(tip);
            }
        });
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.spacing_mut().button_padding.x = 9.0;
            add(ui);
        });
    });
}

/// A group of settings: its heading, a little air around it.
fn group(ui: &mut egui::Ui, first: bool, name: &str) {
    if !first {
        ui.add_space(12.0);
    }
    style::section(ui, name);
    ui.add_space(2.0);
}

/// Skin and size, his behaviour, privacy, benches, upkeep.
fn settings(p: &mut Panel, ui: &mut egui::Ui, s: &Status, h: &Hello, fill: bool) {
    frame(ui, |ui| {
        ui.set_width(ui.available_width());
        title(ui, "The Knight");
        body(ui, "settings_scroll", fill, |ui| {
            ui.add_space(4.0);
            group(ui, true, "Appearance");
            row(ui, "Skin", None, |ui| {
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
            });
            row(ui, "Size", None, |ui| {
                for (name, v) in &h.scales {
                    if ui.selectable_label((s.scale - v).abs() < 1e-3, name.as_str()).clicked() {
                        p.cmd(Command::SetScale(*v));
                    }
                }
            });
            ui.add_space(4.0);
            toggle(p, ui, s.lantern, "Lantern on dark backgrounds", "he holds up his lantern over dark-mode windows or a dark wallpaper", Command::ToggleLantern);

            group(ui, false, "Behaviour");
            row(ui, "Mischief", Some("how often he comes to bother you (cursor battles…)"), |ui| {
                for (name, v) in &h.bother_levels {
                    if ui.selectable_label((s.bother - v).abs() < 1e-3, name.as_str()).clicked() {
                        p.cmd(Command::SetBother(*v));
                    }
                }
            });
            ui.add_space(4.0);
            toggle(p, ui, s.fight_mode, "Fight the cursor", "round after round, until you switch it off", Command::ToggleFightMode);
            toggle(p, ui, s.text_curiosity, "Curious about highlighted text", "he comes to look where you select text (only where it is: the text is never read)", Command::ToggleTextCuriosity);
            toggle(p, ui, s.vision, "Stands on lines inside windows", "watches window contents for ledges (screen capture, never saved); off: title bars and the floor only", Command::ToggleVision);
            toggle(p, ui, s.paused, "Pause", "hide him and stop everything until you switch it back", Command::TogglePause);

            group(ui, false, "Privacy");
            toggle(p, ui, s.share_visible, "Visible in screen shares", "show him in screenshots, Discord, OBS… (hidden from them by default)", Command::ToggleShareVisible);

            group(ui, false, "Benches");
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

            group(ui, false, "Upkeep");
            let mut log_file = s.log_file;
            let tip = format!("Keeps the log in {} (last 5 files) to attach to a bug report. Crash reports are written there either way. Nothing is sent anywhere.", if s.log_dir.is_empty() { "the logs folder" } else { &s.log_dir });
            if ui.checkbox(&mut log_file, RichText::new("Write a log file (for bug reports)").color(pal::TEXT)).on_hover_text(tip).changed() {
                p.cmd(Command::ToggleLogFile);
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open folder").on_hover_text("log and crash files").clicked() {
                    p.cmd(Command::OpenLogFolder);
                }
                if ui
                    .button("Save trace")
                    .on_hover_text("Saves the knight's last 20 seconds of movement (what he was doing, why, and every step) as a trace file in the logs folder. Click it right after something odd happens. A move/stop flicker is saved on its own.")
                    .clicked()
                {
                    p.cmd(Command::SaveTrace);
                }
                if ui
                    .button("Re-extract game art…")
                    .on_hover_text("Read the sprites from your Hollow Knight install again (after a game update, or if some look wrong). They stay on this PC only.")
                    .clicked()
                {
                    p.cmd(Command::ReextractArt);
                }
            });
            ui.add_space(14.0);
            style::ornament(ui, 120.0);
            ui.vertical_centered(|ui| {
                hint(ui, "Anywhere: Ctrl+Alt+G / H he walks / hurries to your cursor · Ctrl+Alt+W wakes him · Ctrl+Alt+M the tray menu · Ctrl+Alt+Q quits");
            });
            ui.add_space(4.0);
        });
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
