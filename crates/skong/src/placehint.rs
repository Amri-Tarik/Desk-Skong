//! The cue while the test panel places something: a small click-through label riding just
//! below-right of the cursor ("training dummy: click a spot · Esc / right-click cancels ·
//! 23 s"). It only follows the cursor; it never moves it.
use skong_core::canvas::{Image, premul};
use skong_win::sprite_window::SpriteWindow;
use skong_win::text::TextPainter;

const FACES: &[&str] = &["Segoe UI", "Tahoma", "Arial"];
const PX: i32 = 15;
const PAD: (i32, i32) = (8, 5);
/// Offset from the cursor's hot spot (clear of the arrow).
const OFFSET: (i32, i32) = (18, 22);

pub struct PlaceHint {
    win: SpriteWindow,
    painter: Option<TextPainter>,
    /// Text last drawn.
    shown: String,
    size: (i32, i32),
}

impl PlaceHint {
    pub fn new(capturable: bool) -> anyhow::Result<PlaceHint> {
        let win = SpriteWindow::new()?;
        win.set_click_through(true);
        win.set_capturable(capturable);
        let painter = TextPainter::new(FACES, PX, false).map_err(|e| log::warn!("place hint font: {e:#}")).ok();
        Ok(PlaceHint { win, painter, shown: String::new(), size: (0, 0) })
    }

    /// Show `text` by the cursor at (`x`, `y`).
    pub fn update(&mut self, text: &str, x: i32, y: i32) {
        let (px, py) = (x + OFFSET.0, y + OFFSET.1);
        if text != self.shown {
            let img = self.render(text);
            self.size = (img.width as i32, img.height as i32);
            if let Err(e) = self.win.present(&img, px, py) {
                log::warn!("place hint: {e:#}");
            }
            self.shown = text.to_string();
        } else if let Err(e) = self.win.move_to(px, py) {
            log::warn!("place hint: {e:#}");
        }
    }

    fn render(&mut self, text: &str) -> Image {
        let Some(p) = self.painter.as_mut() else {
            // No font: a small pin-coloured square still says "placing".
            let mut img = Image::new(14, 14);
            img.px.fill(premul(230, 90, 60, 230));
            return img;
        };
        let w = p.measure(text).ceil() as i32 + 2 * PAD.0;
        let h = p.line_height().ceil() as i32 + 2 * PAD.1;
        let mut img = Image::new(w.max(1) as u32, h.max(1) as u32);
        // Near-black, slightly translucent, a warm stripe on the left.
        let back = premul(20, 20, 24, 225);
        let stripe = premul(230, 110, 70, 255);
        for (i, px) in img.px.iter_mut().enumerate() {
            *px = if (i as i32 % w) < 3 { stripe } else { back };
        }
        if let Err(e) = p.draw(&mut img, PAD.0 + 2, PAD.1, text, [240, 238, 232], 1.0) {
            log::warn!("place hint: {e:#}");
        }
        img
    }
}
