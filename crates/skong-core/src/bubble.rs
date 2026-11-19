//! Speech / thought bubble layout (pure): word wrapping with an injected text measure,
//! box sizing around the text, and placement above a speaker, kept on screen.
//!
//! The platform side measures and draws the text (GDI) and composes the art; everything
//! here is plain geometry so it can be tested without a window.
use crate::geom::{RectI, Vec2};

/// How a bubble looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BubbleStyle {
    /// Someone talking: dark box with the dialogue fleurs above and below.
    #[default]
    Speech,
    /// A dream-nailed thought: the dream dialogue cloud, italic text.
    Thought,
}

/// Text box metrics (px at the current scale).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleMetrics {
    /// Widest text line before wrapping.
    pub max_text_width: f32,
    /// Padding between the text and the box edge (x, y).
    pub pad: Vec2,
    /// Height of one text line.
    pub line_height: f32,
    /// Extra room above / below the box for ornaments (fleurs).
    pub ornament_top: f32,
    pub ornament_bottom: f32,
    /// Gap between the speaker's head and the bubble's bottom.
    pub lift: f32,
    /// Smallest box width (ornaments need room).
    pub min_width: f32,
}

impl BubbleMetrics {
    /// Defaults for a font of `line_height` px.
    pub fn for_line_height(line_height: f32, style: BubbleStyle) -> Self {
        let lh = line_height.max(1.0);
        match style {
            BubbleStyle::Speech => Self {
                max_text_width: lh * 15.0,
                pad: Vec2::new(lh * 1.1, lh * 0.7),
                line_height: lh,
                ornament_top: lh * 1.2,
                ornament_bottom: lh * 0.8,
                lift: lh * 0.4,
                min_width: lh * 9.0,
            },
            BubbleStyle::Thought => Self {
                max_text_width: lh * 13.0,
                pad: Vec2::new(lh * 1.6, lh * 1.1),
                line_height: lh,
                ornament_top: 0.0,
                ornament_bottom: 0.0,
                lift: lh * 0.6,
                min_width: lh * 7.0,
            },
        }
    }
}

/// Greedy word wrap: lines no wider than `max_width` (a single over-long word is split by
/// characters). Explicit `\n` breaks are kept; runs of spaces collapse.
pub fn wrap(text: &str, max_width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&candidate) <= max_width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            if measure(word) <= max_width {
                line = word.to_string();
            } else {
                // Split a word that is wider than a line on its own.
                for ch in word.chars() {
                    let mut next = line.clone();
                    next.push(ch);
                    if !line.is_empty() && measure(&next) > max_width {
                        out.push(std::mem::take(&mut line));
                        line.push(ch);
                    } else {
                        line = next;
                    }
                }
            }
        }
        out.push(line);
    }
    while out.len() > 1 && out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// A laid-out bubble: wrapped lines and sizes, origin-free.
#[derive(Clone, Debug, PartialEq)]
pub struct BubbleLayout {
    pub lines: Vec<String>,
    /// Width of each line (for centering).
    pub widths: Vec<f32>,
    /// The text box (excluding ornaments).
    pub box_size: Vec2,
    /// The whole image: box plus ornaments above / below.
    pub size: Vec2,
    /// Top-left of the box inside the image.
    pub box_at: Vec2,
    pub metrics: BubbleMetrics,
}

impl BubbleLayout {
    pub fn new(text: &str, m: BubbleMetrics, measure: &dyn Fn(&str) -> f32) -> Self {
        let lines = wrap(text, m.max_text_width, measure);
        let widths: Vec<f32> = lines.iter().map(|l| measure(l)).collect();
        let text_w = widths.iter().copied().fold(0.0, f32::max);
        let text_h = m.line_height * lines.len() as f32;
        let box_size = Vec2::new((text_w + 2.0 * m.pad.x).max(m.min_width).ceil(), (text_h + 2.0 * m.pad.y).ceil());
        let size = Vec2::new(box_size.x, box_size.y + m.ornament_top + m.ornament_bottom);
        Self { lines, widths, box_size, size, box_at: Vec2::new(0.0, m.ornament_top), metrics: m }
    }

    /// Top-left of line `i` inside the image (lines are centered).
    pub fn line_origin(&self, i: usize) -> Vec2 {
        let w = self.widths.get(i).copied().unwrap_or(0.0);
        let text_h = self.metrics.line_height * self.lines.len() as f32;
        let top = self.box_at.y + (self.box_size.y - text_h) * 0.5;
        Vec2::new(((self.box_size.x - w) * 0.5).round(), (top + self.metrics.line_height * i as f32).round())
    }

    /// Screen top-left for a bubble over a speaker whose head top is at `anchor`, kept
    /// inside `area` (the work area of the speaker's monitor). Flips below the anchor when
    /// there is no room above.
    pub fn place(&self, anchor: Vec2, area: RectI) -> (i32, i32) {
        let (w, h) = (self.size.x, self.size.y);
        let a = area.to_f();
        let x = (anchor.x - w * 0.5).clamp(a.left, (a.right - w).max(a.left));
        let above = anchor.y - self.metrics.lift - h;
        let y = if above >= a.top { above } else { (anchor.y + self.metrics.lift).min((a.bottom - h).max(a.top)) };
        (x.round() as i32, y.round() as i32)
    }
}

/// Seconds a line stays up: reading time with sane bounds.
pub fn read_secs(text: &str) -> f32 {
    (1.4 + 0.065 * text.chars().count() as f32).clamp(2.4, 7.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 px per character.
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn wraps_on_words() {
        let lines = wrap("the quick brown fox jumps", 100.0, &mono);
        assert_eq!(lines, ["the quick", "brown fox", "jumps"]);
        assert!(lines.iter().all(|l| mono(l) <= 100.0));
    }

    #[test]
    fn splits_long_words_and_keeps_breaks() {
        assert_eq!(wrap("abcdefghijklmn", 50.0, &mono), ["abcde", "fghij", "klmn"]);
        assert_eq!(wrap("one\ntwo  three", 200.0, &mono), ["one", "two three"]);
        assert_eq!(wrap("", 50.0, &mono), [""]);
    }

    #[test]
    fn layout_sizes_and_centers() {
        let m = BubbleMetrics { max_text_width: 100.0, pad: Vec2::new(10.0, 5.0), line_height: 20.0, ornament_top: 8.0, ornament_bottom: 4.0, lift: 6.0, min_width: 50.0 };
        let l = BubbleLayout::new("the quick brown fox", m, &mono);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.box_size, Vec2::new(110.0, 50.0));
        assert_eq!(l.size, Vec2::new(110.0, 62.0));
        // "the quick" is 90 px wide: 10 px margin each side, first line at the box top + pad.
        assert_eq!(l.line_origin(0), Vec2::new(10.0, 13.0));
        assert_eq!(l.line_origin(1), Vec2::new(10.0, 33.0));
        // Short text still gets the minimum width.
        assert_eq!(BubbleLayout::new("hi", m, &mono).box_size.x, 50.0);
    }

    #[test]
    fn placement_stays_on_screen() {
        let m = BubbleMetrics { max_text_width: 100.0, pad: Vec2::new(10.0, 5.0), line_height: 20.0, ornament_top: 0.0, ornament_bottom: 0.0, lift: 6.0, min_width: 50.0 };
        let l = BubbleLayout::new("hi there", m, &mono); // 100 x 30
        let area = RectI::new(0, 0, 1000, 800);
        assert_eq!(l.place(Vec2::new(500.0, 400.0), area), (450, 364));
        // Clamped at the left edge, flipped below near the top.
        assert_eq!(l.place(Vec2::new(10.0, 400.0), area).0, 0);
        assert_eq!(l.place(Vec2::new(500.0, 20.0), area).1, 26);
        assert_eq!(l.place(Vec2::new(995.0, 400.0), area).0, 900);
    }

    #[test]
    fn read_time_grows_with_length() {
        assert!(read_secs("Hi.") >= 2.4);
        assert!(read_secs(&"word ".repeat(20)) > read_secs("a short line"));
        assert!(read_secs(&"x".repeat(500)) <= 7.0);
    }
}
