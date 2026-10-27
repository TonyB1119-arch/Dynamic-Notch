//! Quick notes: jot something down, copy it back later.
//!
//! Notes are short single-line texts kept in `%APPDATA%\DynamicNotch\notes.txt`
//! (one per line, newest first). Click a note to copy it.

use super::widgets::{scroll, TextField};
use super::{CardSize, Cx, Key, Module, ModuleId, Peek};
use crate::gfx::{Icon, TextStyle};
use crate::sys::clip;
use crate::ui::{ButtonStyle, Ui};
use crate::util::{app_dir, palette, Color, Rect, SendHwnd};

const ID: ModuleId = "notes";
const MAX_NOTES: usize = 40;
const ROW_H: f32 = 38.0;
const INPUT_H: f32 = 42.0;
const LIST_H: f32 = 4.0 * ROW_H;

pub struct Notes {
    items: Vec<String>,
    input: TextField,
    hwnd: SendHwnd,
    scroll: f32,
}

impl Notes {
    pub fn new() -> Self {
        Self { items: Vec::new(), input: TextField::new(300), hwnd: SendHwnd(0), scroll: 0.0 }
    }

    fn path() -> std::path::PathBuf {
        app_dir().join("notes.txt")
    }

    fn load(&mut self) {
        self.items = std::fs::read_to_string(Self::path())
            .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).take(MAX_NOTES).collect())
            .unwrap_or_default();
    }

    fn save(&self) {
        let _ = std::fs::write(Self::path(), self.items.join("\n") + "\n");
    }

    fn add(&mut self) {
        let t = self.input.text().trim().to_string();
        if t.is_empty() {
            return;
        }
        self.items.insert(0, t);
        self.items.truncate(MAX_NOTES);
        self.input.clear();
        self.scroll = 0.0;
        self.save();
    }
}

impl Module for Notes {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Notes"
    }
    fn icon(&self) -> Icon {
        Icon::Edit
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
        self.load();
    }

    fn glance(&self) -> Option<(Icon, String)> {
        Some((Icon::Edit, self.items.len().to_string()))
    }

    fn card(&self) -> Option<CardSize> {
        Some(CardSize::Small)
    }

    fn draw_card(&mut self, ui: &mut Ui, r: Rect) {
        if ui.card(r, true) {
            ui.fx.open_page = Some(ID);
        }
        let inner = r.inset_xy(12.0, 0.0);
        ui.p.icon(Icon::Edit, Rect::new(inner.x, r.cy() - 8.0, 16.0, 16.0), palette::YELLOW);
        let text = match self.items.first() {
            Some(n) => n.clone(),
            None => "Tap to add a note".to_string(),
        };
        let col = if self.items.is_empty() { palette::TEXT_DIM } else { palette::TEXT };
        ui.p.text(&text, Rect::new(inner.x + 24.0, r.y, inner.w - 24.0, r.h), &TextStyle::new(12.0, col));
    }

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        INPUT_H + 14.0 + LIST_H
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        if self.input.key(key, self.hwnd) {
            self.add();
        }
        cx.fx.layout_changed = true;
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let accent = palette::YELLOW;
        let field = Rect::new(r.x, r.y, r.w - 84.0, INPUT_H);
        let add = Rect::new(field.right() + 8.0, field.y + 5.0, 76.0, 32.0);
        if self.input.draw(ui, field, "Write a note and press Enter…", accent, true) {
            ui.fx.keyboard_focus = Some(true);
        }
        let can = !self.input.is_empty();
        let style =
            if can { ButtonStyle::inverted() } else { ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT) };
        if ui.pill_button(add, "Add", Some(Icon::Plus), style) && can {
            self.add();
            ui.fx.layout_changed = true;
        }

        let area = Rect::new(r.x, field.bottom() + 14.0, r.w, LIST_H);
        if self.items.is_empty() {
            ui.empty_state(area, Icon::Edit, "No notes yet");
            return;
        }
        scroll(ui, area, &mut self.scroll, self.items.len() as f32 * ROW_H);
        ui.p.push_clip(area);
        let mut delete: Option<usize> = None;
        let mut copied: Option<String> = None;
        for (i, n) in self.items.iter().enumerate() {
            let row = Rect::new(area.x, area.y + i as f32 * ROW_H - self.scroll, area.w, ROW_H - 6.0);
            if row.bottom() < area.y || row.y > area.bottom() {
                continue;
            }
            let trash = Rect::new(row.right() - 30.0, row.cy() - 12.0, 24.0, 24.0);
            let body = Rect::new(row.x, row.y, row.w - 36.0, row.h);
            if ui.card(body, true) {
                copied = Some(n.clone());
            }
            ui.p.text(n, Rect::new(row.x + 12.0, row.y, body.w - 24.0, row.h), &TextStyle::new(13.0, palette::TEXT));
            if ui.icon_button(trash, Icon::Trash, ButtonStyle::default().scale(0.55).fg(palette::TEXT_FAINT)) {
                delete = Some(i);
            }
        }
        ui.p.pop_clip();
        if let Some(i) = delete {
            self.items.remove(i);
            self.save();
        }
        if let Some(t) = copied {
            clip::set_text(self.hwnd.hwnd(), &t);
            ui.fx.peek(
                Peek::new(Icon::Check, palette::GREEN, "Note copied", crate::util::truncate_chars(&t, 40))
                    .key("notes")
                    .duration_ms(1500),
            );
        }
    }
}
