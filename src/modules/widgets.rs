//! Small reusable UI pieces for the settings, Wi-Fi, Bluetooth, Gemini and
//! notes pages: a single-line text field, a switch, a segmented control, a
//! slider and a selectable chip. Everything is immediate mode, like `Ui`.

use super::Key;
use crate::gfx::TextStyle;
use crate::sys::clip;
use crate::ui::{rect_key, Ui};
use crate::util::{palette, Color, Rect, SendHwnd};

// ---------------------------------------------------------------------------
// Text field
// ---------------------------------------------------------------------------

pub struct TextField {
    chars: Vec<char>,
    caret: usize,
    select_all: bool,
    max: usize,
    masked: bool,
    outline: bool,
}

impl TextField {
    pub fn new(max: usize) -> Self {
        Self { chars: Vec::new(), caret: 0, select_all: false, max, masked: false, outline: true }
    }

    /// Draw without the hairline border (a flat filled field).
    pub fn flat(mut self) -> Self {
        self.outline = false;
        self
    }

    /// Show bullets instead of the text (API keys, passwords).
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn set(&mut self, s: &str) {
        self.chars = s.chars().filter(|c| !c.is_control()).take(self.max).collect();
        self.caret = self.chars.len();
        self.select_all = false;
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.caret = 0;
        self.select_all = false;
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    fn insert(&mut self, s: &str) {
        if self.select_all {
            self.clear();
        }
        for c in s.chars().map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c }).filter(|c| !c.is_control()) {
            if self.chars.len() < self.max {
                self.chars.insert(self.caret, c);
                self.caret += 1;
            }
        }
    }

    /// Feed a key press. Returns true when Enter was pressed.
    pub fn key(&mut self, key: Key, hwnd: SendHwnd) -> bool {
        match key {
            Key::Char(c) => {
                let mut b = [0u8; 4];
                self.insert(c.encode_utf8(&mut b));
            }
            Key::Backspace => {
                if self.select_all {
                    self.clear();
                } else if self.caret > 0 {
                    self.caret -= 1;
                    self.chars.remove(self.caret);
                }
            }
            Key::Delete => {
                if self.select_all {
                    self.clear();
                } else if self.caret < self.chars.len() {
                    self.chars.remove(self.caret);
                }
            }
            Key::Left => {
                self.select_all = false;
                self.caret = self.caret.saturating_sub(1);
            }
            Key::Right => {
                self.select_all = false;
                self.caret = (self.caret + 1).min(self.chars.len());
            }
            Key::Home => {
                self.select_all = false;
                self.caret = 0;
            }
            Key::End => {
                self.select_all = false;
                self.caret = self.chars.len();
            }
            Key::SelectAll => self.select_all = !self.chars.is_empty(),
            Key::Paste => {
                if let Some(t) = clip::get_text(hwnd.hwnd()) {
                    self.insert(&t);
                }
            }
            Key::Copy | Key::Cut => {
                if !self.masked && self.select_all && !self.chars.is_empty() {
                    clip::set_text(hwnd.hwnd(), &self.text());
                    if key == Key::Cut {
                        self.clear();
                    }
                }
            }
            Key::Enter { .. } => return true,
            Key::Up | Key::Down | Key::Tab | Key::Escape => {}
        }
        false
    }

    /// Draw the field. Returns true when it was clicked (the caller then
    /// gives it focus and asks the shell for keyboard input).
    pub fn draw(&mut self, ui: &mut Ui, r: Rect, hint: &str, accent: Color, focused: bool) -> bool {
        self.draw_round(ui, r, hint, accent, focused, 10.0)
    }

    /// A field with no box: just the text (at `size`) and its caret. Returns true when clicked.
    pub fn draw_bare(&mut self, ui: &mut Ui, r: Rect, hint: &str, focused: bool, size: f32) -> bool {
        if ui.hovered(r) {
            ui.hot = true;
        }
        let clicked = ui.clicked(r);
        let st = TextStyle::new(size, palette::TEXT);
        let shown: String = if self.masked { "•".repeat(self.chars.len()) } else { self.text() };
        if shown.is_empty() {
            ui.p.text(hint, r, &st.color(palette::TEXT_FAINT));
        }
        let before: String =
            if self.masked { "•".repeat(self.caret) } else { self.chars[..self.caret].iter().collect() };
        let (cx, _) = ui.p.measure(&before, &st);
        let shift = (cx - r.w + 4.0).max(0.0);
        ui.p.push_clip(r);
        if !shown.is_empty() {
            let (tw, _) = ui.p.measure(&shown, &st);
            if self.select_all && focused {
                ui.p.fill_rounded(
                    Rect::new(r.x - shift, r.cy() - size * 0.65, tw, size * 1.3),
                    4.0,
                    Color::white(0.25),
                );
            }
            ui.p.text(&shown, Rect::new(r.x - shift, r.y, tw + 20.0, r.h), &st);
        }
        if focused {
            ui.p.fill_rect(Rect::new(r.x + cx - shift, r.cy() - size * 0.6, 1.6, size * 1.2), Color::white(0.95));
        }
        ui.p.pop_clip();
        clicked
    }

    /// Same, with a custom corner radius (pass half the height for a pill).
    pub fn draw_round(&mut self, ui: &mut Ui, r: Rect, hint: &str, _accent: Color, focused: bool, radius: f32) -> bool {
        ui.p.fill_rounded(r, radius, Color::white(0.08));
        if self.outline {
            ui.p.stroke_rounded(r.inset(0.5), radius, 1.0, Color::white(if focused { 0.35 } else { 0.10 }));
        }
        if ui.hovered(r) {
            ui.hot = true;
        }
        let clicked = ui.clicked(r);
        let tr = r.inset_xy((radius * 0.75).clamp(10.0, 18.0), 0.0);
        let st = TextStyle::new(13.0, palette::TEXT);
        let shown: String = if self.masked { "•".repeat(self.chars.len()) } else { self.text() };
        if shown.is_empty() {
            ui.p.text(hint, tr, &st.color(palette::TEXT_FAINT));
        }
        let before: String =
            if self.masked { "•".repeat(self.caret) } else { self.chars[..self.caret].iter().collect() };
        let (cx, _) = ui.p.measure(&before, &st);
        let shift = (cx - tr.w + 4.0).max(0.0);
        ui.p.push_clip(tr);
        if !shown.is_empty() {
            let (tw, _) = ui.p.measure(&shown, &st);
            if self.select_all && focused {
                ui.p.fill_rounded(Rect::new(tr.x - shift, r.cy() - 9.0, tw, 18.0), 4.0, Color::white(0.25));
            }
            ui.p.text(&shown, Rect::new(tr.x - shift, tr.y, tw + 20.0, tr.h), &st);
        }
        if focused {
            ui.p.fill_rect(Rect::new(tr.x + cx - shift, r.cy() - 8.0, 1.6, 16.0), Color::white(0.95));
        }
        ui.p.pop_clip();
        clicked
    }
}

// ---------------------------------------------------------------------------
// Switch, segmented control, slider, chip
// ---------------------------------------------------------------------------

/// iOS-style switch centered in `r`: green when on, white knob. Returns true when `r` was clicked.
pub fn switch(ui: &mut Ui, r: Rect, on: bool, _accent: Color) -> bool {
    let track = Rect::centered(r.cx(), r.cy(), 42.0, 24.0);
    if ui.hovered(r) {
        ui.hot = true;
    }
    // the knob slides and the colour fades instead of snapping
    let t = ui.glide(rect_key(r, "switch"), if on { 1.0 } else { 0.0 });
    ui.p.fill_rounded(track, 12.0, Color::white(0.15).lerp(palette::GREEN, t));
    if t < 0.98 {
        ui.p.stroke_rounded(track.inset(0.5), 12.0, 1.0, Color::white(0.08 * (1.0 - t)));
    }
    let kx = (track.x + 12.0) + (track.w - 24.0) * t;
    ui.p.fill_circle(kx, track.cy() + 0.6, 9.0, Color::black(0.18));
    ui.p.fill_circle(kx, track.cy(), 9.0, Color::white(1.0));
    ui.clicked(r)
}

/// Row of mutually exclusive options. Returns the index that was clicked.
pub fn segmented(ui: &mut Ui, r: Rect, labels: &[&str], selected: Option<usize>, _accent: Color) -> Option<usize> {
    ui.p.fill_rounded(r, r.h * 0.5, Color::white(0.06));
    let n = labels.len().max(1) as f32;
    let w = r.w / n;
    // the white highlight glides to the chosen option
    let pos = selected.map(|i| i as f32);
    let slide = ui.glide(rect_key(r, "segmented"), pos.unwrap_or(0.0));
    if pos.is_some() {
        ui.p.fill_rounded(Rect::new(r.x + slide * w, r.y, w, r.h).inset(2.0), (r.h - 4.0) * 0.5, Color::white(0.95));
    }
    let mut hit = None;
    for (i, label) in labels.iter().enumerate() {
        let seg = Rect::new(r.x + i as f32 * w, r.y, w, r.h);
        let hover = ui.hovered(seg);
        if hover {
            ui.hot = true;
        }
        let near = if pos.is_some() { (1.0 - (slide - i as f32).abs()).clamp(0.0, 1.0) } else { 0.0 };
        if hover && near < 0.5 {
            ui.p.fill_rounded(seg.inset(2.0), (r.h - 4.0) * 0.5, Color::white(0.08));
        }
        let fg = palette::TEXT_DIM.lerp(palette::INK, near);
        ui.p.text(label, seg, &TextStyle::new(12.0, fg).medium().center());
        if ui.clicked(seg) {
            hit = Some(i);
        }
    }
    hit
}

/// Horizontal slider: a row of ticks that fill up (no thumb). While the
/// button is held the fill follows the mouse; the new value (0..1) is returned
/// once, when the button is released, so callers can write it to the config
/// without spamming the file.
pub fn slider(ui: &mut Ui, r: Rect, frac: f32, _accent: Color) -> Option<f32> {
    let mut shown = frac.clamp(0.0, 1.0);
    if ui.pressed(r) {
        if let Some((mx, _)) = ui.input.mouse {
            shown = ((mx - r.x) / r.w).clamp(0.0, 1.0);
        }
        ui.animate();
    }
    let hover = ui.hovered(r);
    if hover {
        ui.hot = true;
    }
    let h = if hover { 18.0 } else { 16.0 };
    ui.ticks(Rect::new(r.x, r.cy() - h * 0.5, r.w, h), shown, Color::white(if hover { 1.0 } else { 0.92 }));
    ui.clicked_at(r)
}

/// Selectable chip: filled with the accent when `on`, with an optional
/// badge number (selection order). Returns true when clicked.
pub fn chip(ui: &mut Ui, r: Rect, label: &str, on: bool, badge: Option<usize>, _accent: Color) -> bool {
    let hover = ui.hovered(r);
    if hover {
        ui.hot = true;
    }
    let bg = if on {
        Color::white(if hover { 1.0 } else { 0.94 })
    } else if hover {
        Color::white(0.10)
    } else {
        Color::white(0.05)
    };
    ui.p.fill_rounded(r, 10.0, bg);
    let fg = if on { palette::INK } else { palette::TEXT_DIM };
    let tr = if badge.is_some() { Rect::new(r.x + 26.0, r.y, r.w - 32.0, r.h) } else { r.inset_xy(8.0, 0.0) };
    let st = TextStyle::new(12.0, fg).medium();
    ui.p.text(label, tr, &if badge.is_some() { st } else { st.center() });
    if let Some(n) = badge {
        let b = Rect::new(r.x + 7.0, r.cy() - 8.0, 16.0, 16.0);
        ui.p.fill_circle(b.cx(), b.cy(), 8.0, Color::black(0.14));
        ui.p.text(&n.to_string(), b, &TextStyle::new(10.5, palette::INK).semibold().center());
    }
    ui.clicked(r)
}

/// Apply the mouse wheel to a scroll offset for a list `area` whose content
/// is `content_h` tall.
pub fn scroll(ui: &Ui, area: Rect, off: &mut f32, content_h: f32) {
    let w = ui.wheel(area);
    if w != 0.0 {
        *off -= w * 36.0;
    }
    *off = off.clamp(0.0, (content_h - area.h).max(0.0));
}

/// Quote a string for the config file.
pub fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::toml_str;

    #[test]
    fn quotes() {
        assert_eq!(toml_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
