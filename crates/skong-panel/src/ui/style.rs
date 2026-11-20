//! The panel's look: Hallownest at dusk. Near-black blue-grey, bone-white text, muted
//! silver lines; Cinzel (an engraved-capitals serif, standing in for the game's Trajan) for
//! headings and Crimson Pro for everything else, both bundled (`skong_core::fonts`, OFL).
//! Square frames with thin pale borders and small corner ticks, ornamental dividers drawn
//! in code, buttons as framed text. No saturated colours: the only warm tone is a muted
//! ember for the sandbox warning.
//!
//! Type sizes (egui points): body and buttons 17, hints / subtext 15 (never below 14),
//! section titles 21, the window's name 28, logs 14 (monospace).
use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, TextStyle, Vec2};

/// Hallownest at dusk.
pub mod palette {
    use eframe::egui::Color32;
    /// Window background.
    pub const BG: Color32 = Color32::from_rgb(13, 16, 21);
    /// Inside frames (a shade lighter).
    pub const FRAME: Color32 = Color32::from_rgb(18, 22, 29);
    /// Text fields, tracks.
    pub const DEEP: Color32 = Color32::from_rgb(9, 11, 15);
    /// Frame borders, dividers.
    pub const LINE: Color32 = Color32::from_rgb(92, 102, 118);
    pub const LINE_FAINT: Color32 = Color32::from_rgb(48, 55, 67);
    /// Bone white: body text.
    pub const TEXT: Color32 = Color32::from_rgb(232, 228, 218);
    /// Pale grey-white: hints, subtext (kept bright: readable, just quieter).
    pub const SUB: Color32 = Color32::from_rgb(192, 197, 206);
    /// Pale soul-blue white: headings, the selected tab.
    pub const SOUL: Color32 = Color32::from_rgb(214, 224, 238);
    /// Muted soul-orange: the sandbox warning, "stopped", refusals.
    pub const EMBER: Color32 = Color32::from_rgb(214, 156, 108);
    pub const EMBER_BG: Color32 = Color32::from_rgb(40, 29, 22);
    /// Muted rose: the destructive "reset".
    pub const ROSE: Color32 = Color32::from_rgb(206, 140, 132);
}
use palette as pal;

/// The engraved heading family (Cinzel).
pub fn heading_family() -> FontFamily {
    FontFamily::Name("heading".into())
}

/// Cinzel at `size`.
pub fn heading(size: f32) -> FontId {
    FontId::new(size, heading_family())
}

/// The bundled fonts: Crimson Pro first for proportional text (egui's own fonts stay as
/// fallbacks for symbols and emoji), Cinzel as the `heading` family.
pub fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    f.font_data.insert("cinzel".into(), Arc::new(FontData::from_static(skong_core::fonts::HEADING)));
    f.font_data.insert("crimson".into(), Arc::new(FontData::from_static(skong_core::fonts::BODY)));
    let fallbacks = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    f.families.insert(FontFamily::Proportional, std::iter::once("crimson".to_string()).chain(fallbacks.iter().cloned()).collect());
    f.families.insert(heading_family(), ["cinzel".to_string(), "crimson".to_string()].into_iter().chain(fallbacks).collect());
    f
}

/// Every text style, overridden (egui's defaults are 10–14 px).
pub fn text_styles() -> [(TextStyle, FontId); 5] {
    [
        (TextStyle::Small, FontId::proportional(15.0)),
        (TextStyle::Body, FontId::proportional(17.0)),
        (TextStyle::Button, FontId::proportional(17.0)),
        (TextStyle::Heading, heading(24.0)),
        (TextStyle::Monospace, FontId::monospace(14.0)),
    ]
}

/// Install fonts, sizes, colours and spacing on `ctx` (the panel and the setup window).
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = pal::BG;
    v.window_fill = pal::FRAME;
    v.window_stroke = Stroke::new(1.0, pal::LINE);
    v.window_corner_radius = 1.into();
    v.menu_corner_radius = 1.into();
    v.extreme_bg_color = pal::DEEP;
    v.faint_bg_color = Color32::from_rgb(20, 24, 31);
    v.code_bg_color = pal::DEEP;
    v.hyperlink_color = pal::SOUL;
    v.warn_fg_color = pal::EMBER;
    v.error_fg_color = pal::ROSE;
    v.weak_text_color = Some(pal::SUB);
    v.selection.bg_fill = Color32::from_rgb(44, 52, 66);
    v.selection.stroke = Stroke::new(1.0, pal::SOUL);
    v.striped = true;
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = pal::FRAME;
    w.noninteractive.weak_bg_fill = pal::FRAME;
    w.noninteractive.bg_stroke = Stroke::new(1.0, pal::LINE_FAINT);
    w.noninteractive.fg_stroke = Stroke::new(1.0, pal::TEXT);
    // Buttons: framed text — a dark fill, a thin pale border that brightens on hover.
    for (wv, fill, edge, fg) in [
        (&mut w.inactive, Color32::from_rgb(19, 23, 30), pal::LINE, pal::TEXT),
        (&mut w.hovered, Color32::from_rgb(28, 33, 42), pal::SOUL, Color32::WHITE),
        (&mut w.active, Color32::from_rgb(36, 42, 54), pal::SOUL, Color32::WHITE),
        (&mut w.open, Color32::from_rgb(24, 29, 37), pal::LINE, pal::TEXT),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::new(1.0, edge);
        wv.fg_stroke = Stroke::new(1.2, fg);
        wv.corner_radius = 1.into();
        wv.expansion = 0.0;
    }
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.text_styles = text_styles().into_iter().collect();
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 5.0);
        s.spacing.interact_size = egui::vec2(40.0, 28.0);
        s.spacing.icon_width = 18.0;
        s.spacing.icon_width_inner = 10.0;
        s.spacing.indent = 26.0;
        s.spacing.combo_width = 220.0;
    });
}

/// A square frame with a thin pale border and small ticks at its corners.
pub fn frame<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let r = egui::Frame::new().fill(pal::FRAME).stroke(Stroke::new(1.0, pal::LINE_FAINT)).inner_margin(egui::Margin::symmetric(20, 16)).show(ui, add);
    corner_ticks(ui.painter(), r.response.rect, 7.0, pal::LINE);
    r.inner
}

/// Short L-shaped ticks on the four corners of `rect`.
pub fn corner_ticks(p: &egui::Painter, rect: Rect, len: f32, color: Color32) {
    let s = Stroke::new(1.0, color);
    let r = rect.shrink(0.5);
    for (c, dx, dy) in [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.left_bottom(), 1.0, -1.0), (r.right_bottom(), -1.0, -1.0)] {
        p.line_segment([c, c + egui::vec2(dx * len, 0.0)], s);
        p.line_segment([c, c + egui::vec2(0.0, dy * len)], s);
    }
}

/// A rhombus centred at `c`, half-diagonals (`rx`, `ry`): filled and / or outlined.
pub fn rhombus(p: &egui::Painter, c: Pos2, rx: f32, ry: f32, fill: Color32, stroke: Stroke) {
    let pts = vec![c + egui::vec2(0.0, -ry), c + egui::vec2(rx, 0.0), c + egui::vec2(0.0, ry), c + egui::vec2(-rx, 0.0)];
    p.add(Shape::convex_polygon(pts, fill, stroke));
}

/// The ornamental divider (the same original motif as the achievement banner): a hairline
/// tapering away from an outlined diamond with a small solid one inside, a small diamond on
/// each side, a dot past each end. Symmetric about `centre`.
pub fn paint_ornament(p: &egui::Painter, centre: Pos2, half: f32, color: Color32) {
    let gap = 11.0;
    let steps = 24;
    for side in [-1.0f32, 1.0] {
        for i in 0..steps {
            let (a, b) = (i as f32 / steps as f32, (i + 1) as f32 / steps as f32);
            let x0 = gap + (half - gap) * a;
            let x1 = gap + (half - gap) * b;
            let fade = 1.0 - a * a;
            p.line_segment([centre + egui::vec2(side * x0, 0.0), centre + egui::vec2(side * x1, 0.0)], Stroke::new(1.0, color.gamma_multiply(fade)));
        }
        rhombus(p, centre + egui::vec2(side * 22.0, 0.0), 2.8, 4.0, color, Stroke::NONE);
        p.circle_filled(centre + egui::vec2(side * (half + 5.0), 0.0), 1.5, color.gamma_multiply(0.8));
    }
    rhombus(p, centre, 6.5, 9.5, Color32::TRANSPARENT, Stroke::new(1.2, color));
    rhombus(p, centre, 2.2, 3.6, color, Stroke::NONE);
}

/// A divider across the available width (at most `max_half` each side).
pub fn ornament(ui: &mut egui::Ui, max_half: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 22.0), Sense::hover());
    let half = (w * 0.5 - 12.0).clamp(30.0, max_half);
    paint_ornament(ui.painter(), rect.center(), half, pal::LINE);
}

/// A centred section title in Cinzel with the divider under it.
pub fn title(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).font(heading(21.0)).color(pal::SOUL));
    });
    ornament(ui, 150.0);
    ui.add_space(2.0);
}

/// A smaller, left-aligned sub-heading in Cinzel.
pub fn subtitle(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(heading(17.0)).color(pal::SOUL));
}

/// Subtext: 15 px, pale grey-white, wrapping.
pub fn hint(ui: &mut egui::Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Label::new(RichText::new(text.into()).small().color(pal::SUB)).wrap())
}

/// A tab: Cinzel text; the selected one bright with a small diamond and a line under it.
pub fn tab(ui: &mut egui::Ui, selected: bool, text: &str) -> Response {
    let font = heading(17.0);
    let color = if selected { pal::SOUL } else { pal::SUB };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + egui::vec2(24.0, 14.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let color = if resp.hovered() && !selected { pal::TEXT } else { color };
    p.galley_with_override_text_color(rect.center() - galley.size() * 0.5 - egui::vec2(0.0, 3.0), galley, color);
    if selected {
        let y = rect.bottom() - 3.0;
        p.line_segment([Pos2::new(rect.left() + 6.0, y), Pos2::new(rect.right() - 6.0, y)], Stroke::new(1.0, pal::LINE));
        rhombus(p, Pos2::new(rect.center().x, y), 3.5, 4.5, pal::SOUL, Stroke::NONE);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A thin progress line with a diamond riding its end.
pub fn progress(ui: &mut egui::Ui, frac: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), Sense::hover());
    let p = ui.painter();
    let y = rect.center().y;
    p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(3.0, pal::LINE_FAINT));
    let x = rect.left() + rect.width() * frac.clamp(0.0, 1.0);
    p.line_segment([Pos2::new(rect.left(), y), Pos2::new(x, y)], Stroke::new(3.0, pal::SUB));
    rhombus(p, Pos2::new(x, y), 4.5, 6.5, pal::BG, Stroke::new(1.2, pal::TEXT));
}

/// A small diamond marker (found / not found yet), sized to a text line.
pub fn marker(ui: &mut egui::Ui, filled: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 18.0), Sense::hover());
    let fill = if filled { color } else { Color32::TRANSPARENT };
    rhombus(ui.painter(), rect.center(), 4.5, 6.5, fill, Stroke::new(1.0, color));
}

/// A framed text button with a minimum width (the hide / enable buttons).
pub fn wide_button(text: &str, width: f32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).font(heading(16.0))).min_size(egui::vec2(width, 34.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_style_is_large_enough() {
        let styles = text_styles();
        for (style, font) in &styles {
            assert!(font.size >= 14.0, "{style:?} is {} px", font.size);
        }
        let size = |s: TextStyle| styles.iter().find(|(t, _)| *t == s).unwrap().1.size;
        assert!((16.0..=17.0).contains(&size(TextStyle::Body)));
        assert!((14.0..=15.0).contains(&size(TextStyle::Small)));
        assert!(size(TextStyle::Heading) > size(TextStyle::Body));
        // Every style egui knows about is overridden.
        assert_eq!(styles.len(), egui::Style::default().text_styles.len());
    }

    #[test]
    fn the_bundled_fonts_are_installed_first() {
        let f = fonts();
        assert_eq!(f.families[&FontFamily::Proportional][0], "crimson");
        assert_eq!(f.families[&heading_family()][0], "cinzel");
        assert!(f.families[&FontFamily::Proportional].len() > 1, "egui's fonts stay as fallbacks");
        assert!(!f.font_data["cinzel"].bytes().is_empty());
    }

    #[test]
    fn text_is_pale_on_a_dark_ground() {
        // Relative luminance contrast (WCAG): subtext stays well above 7:1.
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let contrast = |a: Color32, b: Color32| (lum(a) + 0.05) / (lum(b) + 0.05);
        for c in [pal::TEXT, pal::SUB, pal::SOUL] {
            assert!(contrast(c, pal::FRAME) > 9.0, "{c:?}");
        }
        assert!(contrast(pal::EMBER, pal::EMBER_BG) > 6.0);
    }
}
