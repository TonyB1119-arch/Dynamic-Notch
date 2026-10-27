//! Tools: one tab that holds a set of small apps — calculator, calendar,
//! world clock, launcher, quick actions, system info, focus timer, wellness
//! reminders, to-do list, randomizer and breathing guide.
//!
//! The apps are `Module`s owned by this one (like the Wi-Fi and Bluetooth
//! lists inside the control center). A grid of tiles picks one; a back
//! button returns to the grid. Nothing runs for an app until you use it.

use std::any::Any;
use std::time::Instant;

use super::apps::{
    actions::Actions, breathe::Breathe, calc::Calc, calendar::Calendar, dice::Dice, focus::Focus, launcher::Launcher,
    sysinfo::SysInfo, todo::Todo, wellness::Wellness, worldclock::WorldClock,
};
use super::{Activity, CardSize, Cx, Indicator, Key, Module, ModuleId, SystemEvent};
use crate::gfx::{Icon, TextStyle};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "tools";
const COLS: usize = 4;
const CELL_H: f32 = 74.0;
const GAP: f32 = 8.0;
const BAR_H: f32 = 34.0;
const BAR_GAP: f32 = 8.0;

pub struct Tools {
    apps: Vec<Box<dyn Module>>,
    open: Option<usize>,
}

impl Tools {
    pub fn new() -> Self {
        let apps: Vec<Box<dyn Module>> = vec![
            Box::new(Calc::new()),
            Box::new(Calendar::new()),
            Box::new(WorldClock::new()),
            Box::new(Launcher::new()),
            Box::new(Actions::new()),
            Box::new(SysInfo::new()),
            Box::new(Focus::new()),
            Box::new(Wellness::new()),
            Box::new(Todo::new()),
            Box::new(Dice::new()),
            Box::new(Breathe::new()),
        ];
        Self { apps, open: None }
    }

    fn grid_height(&self) -> f32 {
        let rows = self.apps.len().div_ceil(COLS).max(1);
        rows as f32 * (CELL_H + GAP) - GAP + 4.0
    }
}

impl Module for Tools {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Tools"
    }
    fn icon(&self) -> Icon {
        Icon::Sparkle
    }

    fn start(&mut self, cx: &mut Cx) {
        for a in &mut self.apps {
            a.start(cx);
        }
    }

    fn stop(&mut self) {
        for a in &mut self.apps {
            a.stop();
        }
    }

    /// Helper threads of the apps only need a redraw when they finish.
    fn on_message(&mut self, _msg: Box<dyn Any + Send>, cx: &mut Cx) {
        cx.fx.redraw = true;
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        for a in &mut self.apps {
            a.on_system(ev, cx);
        }
    }

    fn next_tick(&self) -> Option<Instant> {
        self.apps.iter().filter_map(|a| a.next_tick()).min()
    }

    fn on_tick(&mut self, cx: &mut Cx) {
        for a in &mut self.apps {
            if a.next_tick().is_some_and(|t| t <= cx.now) {
                a.on_tick(cx);
            }
        }
    }

    fn activity(&self) -> Option<Activity> {
        self.apps.iter().filter_map(|a| a.activity()).max_by_key(|a| a.priority)
    }

    fn indicators(&self, out: &mut Vec<Indicator>) {
        for a in &self.apps {
            a.indicators(out);
        }
    }

    // -- home card ---------------------------------------------------------------

    fn card(&self) -> Option<CardSize> {
        Some(CardSize::Small)
    }

    fn draw_card(&mut self, ui: &mut Ui, r: Rect) {
        if ui.card(r, true) {
            self.open = None;
            ui.fx.open_page = Some(ID);
        }
        let inner = r.inset_xy(12.0, 0.0);
        ui.p.icon(Icon::Sparkle, Rect::new(inner.x, r.cy() - 8.0, 16.0, 16.0), palette::BLUE);
        // a running focus timer or the open to-do count is more useful than a label
        let status = self
            .apps
            .iter()
            .find(|a| a.id() == "focus")
            .and_then(|a| a.activity().and(a.glance()))
            .or_else(|| self.apps.iter().find(|a| a.id() == "todo").and_then(|a| a.glance()))
            .map(|(_, t)| t);
        let text = match status {
            Some(s) => format!("Tools · {s}"),
            None => "Tools".to_string(),
        };
        ui.p.text(
            &text,
            Rect::new(inner.x + 24.0, r.y, inner.w - 24.0, r.h),
            &TextStyle::new(12.0, palette::TEXT).medium(),
        );
    }

    // -- page ------------------------------------------------------------------

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        self.open.is_some_and(|i| self.apps[i].wants_keyboard())
    }

    fn page_height(&self, w: f32) -> f32 {
        match self.open {
            Some(i) => BAR_H + BAR_GAP + self.apps[i].page_height(w),
            None => self.grid_height(),
        }
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        if let Some(i) = self.open {
            self.apps[i].on_key(key, cx);
        }
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let Some(i) = self.open else {
            self.draw_grid(ui, r);
            return;
        };
        // header: back to the grid, then the app's name
        let back = Rect::new(r.x, r.y, 108.0, 30.0);
        if ui.pill_button(
            back,
            "All tools",
            Some(Icon::Prev),
            ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM),
        ) {
            self.open = None;
            ui.fx.keyboard_focus = Some(false);
            ui.fx.layout_changed = true;
            return;
        }
        ui.p.text(
            self.apps[i].title(),
            Rect::new(back.right() + 14.0, r.y, r.w - 130.0, 30.0),
            &TextStyle::new(15.0, palette::TEXT).semibold(),
        );
        let body = Rect::new(r.x, r.y + BAR_H + BAR_GAP, r.w, (r.h - BAR_H - BAR_GAP).max(0.0));
        self.apps[i].draw_page(ui, body);
    }
}

impl Tools {
    fn draw_grid(&mut self, ui: &mut Ui, r: Rect) {
        let cw = (r.w - GAP * (COLS as f32 - 1.0)) / COLS as f32;
        let tints = [
            palette::BLUE,
            palette::PURPLE,
            palette::TEAL,
            palette::PINK,
            palette::ORANGE,
            palette::GREEN,
            palette::BLUE,
            palette::TEAL,
            palette::GREEN,
            palette::PURPLE,
            palette::TEAL,
        ];
        let mut picked = None;
        for (i, a) in self.apps.iter().enumerate() {
            let (c, row) = (i % COLS, i / COLS);
            let cell = Rect::new(r.x + c as f32 * (cw + GAP), r.y + row as f32 * (CELL_H + GAP), cw, CELL_H);
            if ui.card(cell, true) {
                picked = Some(i);
            }
            let tint = tints[i % tints.len()];
            let badge = Rect::new(cell.x + 12.0, cell.y + 12.0, 32.0, 32.0);
            ui.p.fill_rounded(badge, 11.0, tint.with_a(0.20));
            ui.p.icon(a.icon(), badge.inset(7.0), tint);
            ui.p.text(
                a.title(),
                Rect::new(cell.x + 12.0, cell.y + 46.0, cell.w - 20.0, 16.0),
                &TextStyle::new(12.5, palette::TEXT).medium(),
            );
            if let Some((_, status)) = a.glance() {
                ui.p.text(
                    &status,
                    Rect::new(cell.x + 52.0, cell.y + 12.0, cell.w - 60.0, 32.0),
                    &TextStyle::new(10.5, palette::TEXT_FAINT).right(),
                );
            }
        }
        if let Some(i) = picked {
            self.open = Some(i);
            ui.fx.layout_changed = true;
        }
    }
}
