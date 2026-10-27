//! Launcher: one-click shortcuts to programs, folders and web addresses.
//! The list comes from `[tools] launcher` in config.toml ("Name|target").

use crate::gfx::{Icon, TextStyle};
use crate::modules::{Cx, Module, ModuleId, SystemEvent};
use crate::sys::shell_open;
use crate::ui::Ui;
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "launcher";
const COLS: usize = 4;
const CELL_H: f32 = 62.0;
const GAP: f32 = 8.0;
const MAX_ITEMS: usize = 12;

pub struct Launcher {
    items: Vec<(String, String)>,
    src: Vec<String>,
}

impl Launcher {
    pub fn new() -> Self {
        Self { items: Vec::new(), src: Vec::new() }
    }

    fn sync(&mut self, src: &[String]) {
        if self.src == src {
            return;
        }
        self.items = src
            .iter()
            .filter_map(|s| {
                let (name, target) = s.split_once('|')?;
                let (name, target) = (name.trim(), target.trim());
                (!name.is_empty() && !target.is_empty()).then(|| (name.to_string(), target.to_string()))
            })
            .take(MAX_ITEMS)
            .collect();
        self.src = src.to_vec();
    }
}

impl Module for Launcher {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Launcher"
    }
    fn icon(&self) -> Icon {
        Icon::Link
    }

    fn start(&mut self, cx: &mut Cx) {
        self.sync(&cx.cfg.tools.launcher);
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        if matches!(ev, SystemEvent::ConfigReloaded) {
            self.sync(&cx.cfg.tools.launcher);
        }
    }

    fn glance(&self) -> Option<(Icon, String)> {
        Some((Icon::Link, format!("{} apps", self.items.len())))
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        let rows = self.items.len().div_ceil(COLS).max(1);
        rows as f32 * (CELL_H + GAP) - GAP + 4.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        self.sync(&ui.cfg.tools.launcher);
        if self.items.is_empty() {
            ui.empty_state(r, Icon::Link, "Add shortcuts under [tools] launcher in config.toml");
            return;
        }
        let cw = (r.w - GAP * (COLS as f32 - 1.0)) / COLS as f32;
        let tints = [palette::BLUE, palette::PURPLE, palette::PINK, palette::ORANGE, palette::GREEN, palette::TEAL];
        let mut launch: Option<String> = None;
        for (i, (name, target)) in self.items.iter().enumerate() {
            let (c, row) = (i % COLS, i / COLS);
            let cell = Rect::new(r.x + c as f32 * (cw + GAP), r.y + row as f32 * (CELL_H + GAP), cw, CELL_H);
            if ui.card(cell, true) {
                launch = Some(target.clone());
            }
            let tint = tints[name.bytes().map(|b| b as usize).sum::<usize>() % tints.len()];
            let badge = Rect::new(cell.cx() - 15.0, cell.y + 8.0, 30.0, 30.0);
            ui.p.fill_rounded(badge, 10.0, tint.with_a(0.20));
            let letter = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('•').to_uppercase().to_string();
            ui.p.text(&letter, badge, &TextStyle::new(15.0, tint).semibold().center());
            ui.p.text(
                name,
                Rect::new(cell.x + 4.0, cell.y + 40.0, cell.w - 8.0, 16.0),
                &TextStyle::new(11.5, Color::white(0.85)).center(),
            );
        }
        if let Some(t) = launch {
            shell_open(&t);
            ui.fx.collapse = true;
        }
    }
}
