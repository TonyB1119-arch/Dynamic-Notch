//! To-do list: add tasks, tick them off, clear what is done. Saved in
//! `%APPDATA%\DynamicNotch\todo.txt` ("- task" open, "x task" done).

use crate::gfx::{Icon, TextStyle};
use crate::modules::widgets::{scroll, TextField};
use crate::modules::{Cx, Key, Module, ModuleId};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{app_dir, palette, Color, Rect, SendHwnd};

const ID: ModuleId = "todo";
const MAX_ITEMS: usize = 60;
const ROW_H: f32 = 34.0;
const INPUT_H: f32 = 42.0;
const LIST_H: f32 = 4.0 * ROW_H;
const HEAD_H: f32 = 26.0;

#[derive(Clone, Debug, PartialEq)]
struct Item {
    done: bool,
    text: String,
}

pub struct Todo {
    items: Vec<Item>,
    input: TextField,
    hwnd: SendHwnd,
    scroll: f32,
}

impl Todo {
    pub fn new() -> Self {
        Self { items: Vec::new(), input: TextField::new(200), hwnd: SendHwnd(0), scroll: 0.0 }
    }

    fn path() -> std::path::PathBuf {
        app_dir().join("todo.txt")
    }

    fn load(&mut self) {
        let text = std::fs::read_to_string(Self::path()).unwrap_or_default();
        self.items = parse(&text);
    }

    fn save(&self) {
        let _ = std::fs::write(Self::path(), render(&self.items));
    }

    fn add(&mut self) {
        let t = self.input.text().trim().to_string();
        if t.is_empty() || self.items.len() >= MAX_ITEMS {
            return;
        }
        // new tasks go on top of the open ones
        self.items.insert(0, Item { done: false, text: t });
        self.input.clear();
        self.scroll = 0.0;
        self.save();
    }

    fn open_count(&self) -> usize {
        self.items.iter().filter(|i| !i.done).count()
    }
}

fn parse(text: &str) -> Vec<Item> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_end();
            let (mark, rest) = l.split_at(l.char_indices().nth(2).map_or(l.len(), |(i, _)| i));
            let done = match mark {
                "x " => true,
                "- " => false,
                _ => return None,
            };
            let t = rest.trim();
            (!t.is_empty()).then(|| Item { done, text: t.to_string() })
        })
        .take(MAX_ITEMS)
        .collect()
}

fn render(items: &[Item]) -> String {
    items.iter().map(|i| format!("{} {}\n", if i.done { "x" } else { "-" }, i.text)).collect()
}

impl Module for Todo {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "To-Do"
    }
    fn icon(&self) -> Icon {
        Icon::Check
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
        self.load();
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let n = self.open_count();
        Some((Icon::Check, if n == 0 { "All done".to_string() } else { format!("{n} left") }))
    }

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        INPUT_H + 10.0 + HEAD_H + LIST_H
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        if self.input.key(key, self.hwnd) {
            self.add();
        }
        cx.fx.layout_changed = true;
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let field = Rect::new(r.x, r.y, r.w - 84.0, INPUT_H);
        let add = Rect::new(field.right() + 8.0, field.y + 5.0, 76.0, 32.0);
        if self.input.draw(ui, field, "Add a task and press Enter…", palette::BLUE, true) {
            ui.fx.keyboard_focus = Some(true);
        }
        let can = !self.input.is_empty();
        let style =
            if can { ButtonStyle::inverted() } else { ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT) };
        if ui.pill_button(add, "Add", Some(Icon::Plus), style) && can {
            self.add();
            ui.fx.layout_changed = true;
        }

        // progress: how much of the list is done
        let head = Rect::new(r.x, field.bottom() + 10.0, r.w, HEAD_H);
        let total = self.items.len();
        let done = total - self.open_count();
        ui.p.text(
            &format!("{} of {} done", done, total),
            Rect::new(head.x + 4.0, head.y, 130.0, head.h),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        ui.ticks(
            Rect::new(head.x + 140.0, head.cy() - 5.0, (head.w - 140.0 - 130.0).max(40.0), 10.0),
            if total == 0 { 0.0 } else { done as f32 / total as f32 },
            Color::white(0.9),
        );
        if done > 0 {
            let clear = Rect::new(head.right() - 118.0, head.y, 118.0, head.h);
            if ui.pill_button(clear, "Clear done", None, ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM))
            {
                self.items.retain(|i| !i.done);
                self.save();
            }
        }

        let area = Rect::new(r.x, head.bottom(), r.w, LIST_H);
        if self.items.is_empty() {
            ui.empty_state(area, Icon::Check, "Nothing to do. Enjoy it.");
            return;
        }
        scroll(ui, area, &mut self.scroll, self.items.len() as f32 * ROW_H);
        ui.p.push_clip(area);
        let mut toggle: Option<usize> = None;
        let mut delete: Option<usize> = None;
        for (i, it) in self.items.iter().enumerate() {
            let row = Rect::new(area.x, area.y + i as f32 * ROW_H - self.scroll, area.w, ROW_H - 4.0);
            if row.bottom() < area.y || row.y > area.bottom() {
                continue;
            }
            let hover = ui.hovered(row);
            ui.row_bg(row, hover, None);
            let box_r = Rect::new(row.x + 12.0, row.cy() - 9.0, 18.0, 18.0);
            if it.done {
                ui.p.fill_circle(box_r.cx(), box_r.cy(), 9.0, palette::BLUE);
                ui.p.icon(Icon::Check, box_r.inset(3.0), Color::white(1.0));
            } else {
                ui.p.stroke_circle(box_r.cx(), box_r.cy(), 8.5, 1.5, Color::white(0.4));
            }
            let col = if it.done { palette::TEXT_FAINT } else { palette::TEXT };
            ui.p.text(&it.text, Rect::new(row.x + 42.0, row.y, row.w - 90.0, row.h), &TextStyle::new(13.0, col));
            let trash = Rect::new(row.right() - 30.0, row.cy() - 12.0, 24.0, 24.0);
            if ui.icon_button(trash, Icon::Trash, ButtonStyle::default().scale(0.55).fg(palette::TEXT_FAINT)) {
                delete = Some(i);
            } else if ui.clicked(Rect::new(row.x, row.y, row.w - 36.0, row.h)) {
                toggle = Some(i);
            }
        }
        ui.p.pop_clip();
        if let Some(i) = delete {
            self.items.remove(i);
            self.save();
        } else if let Some(i) = toggle {
            self.items[i].done = !self.items[i].done;
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let items = vec![Item { done: false, text: "milk".into() }, Item { done: true, text: "call mom".into() }];
        let text = render(&items);
        assert_eq!(text, "- milk\nx call mom\n");
        assert_eq!(parse(&text), items);
        assert!(parse("garbage\n- \n").is_empty());
    }
}
