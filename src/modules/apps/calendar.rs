//! Calendar: a month at a time with today highlighted, ISO week numbers and
//! a tick bar showing how far through the month you are.

use std::time::{Duration, Instant};

use super::{civil_from_days, days_from_civil, days_in_month, iso_week, weekday, Local, MONTHS};
use crate::gfx::{Icon, TextStyle};
use crate::modules::{Cx, Module, ModuleId};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "calendar";
const HEAD_H: f32 = 34.0;
const CELL_H: f32 = 29.0;
const WEEK_COL: f32 = 34.0;

pub struct Calendar {
    /// The month on screen.
    year: i64,
    month: i64,
    monday_first: bool,
    refresh_at: Option<Instant>,
}

impl Calendar {
    pub fn new() -> Self {
        let now = Local::now();
        Self { year: now.year, month: now.month, monday_first: false, refresh_at: None }
    }

    fn step(&mut self, delta: i64) {
        let idx = self.year * 12 + (self.month - 1) + delta;
        self.year = idx.div_euclid(12);
        self.month = idx.rem_euclid(12) + 1;
    }
}

impl Module for Calendar {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Calendar"
    }
    fn icon(&self) -> Icon {
        Icon::Home
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let n = Local::now();
        Some((Icon::Home, format!("{} {}", &MONTHS[(n.month - 1) as usize][..3], n.day)))
    }

    fn next_tick(&self) -> Option<Instant> {
        self.refresh_at
    }

    fn on_tick(&mut self, cx: &mut Cx) {
        self.refresh_at = None;
        cx.fx.redraw = true;
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        HEAD_H + 10.0 + 8.0 + 12.0 + 18.0 + 6.0 * CELL_H + 4.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        self.refresh_at = Some(ui.now + Duration::from_secs(60));
        let today = Local::now();
        let viewing_now = self.year == today.year && self.month == today.month;

        // --- header ---------------------------------------------------------
        let title = format!("{} {}", MONTHS[(self.month - 1) as usize], self.year);
        ui.p.text(&title, Rect::new(r.x + 4.0, r.y, 220.0, HEAD_H), &TextStyle::new(18.0, palette::TEXT).semibold());
        let next = Rect::new(r.right() - 32.0, r.y + 2.0, 30.0, 30.0);
        let prev = Rect::new(next.x - 34.0, r.y + 2.0, 30.0, 30.0);
        let style = ButtonStyle::default().scale(0.55).fg(palette::TEXT_DIM);
        if ui.icon_button(prev, Icon::Prev, style) {
            self.step(-1);
        }
        if ui.icon_button(next, Icon::Next, style) {
            self.step(1);
        }
        let mut x = prev.x - 8.0;
        let first = Rect::new(x - 84.0, r.y + 4.0, 84.0, 26.0);
        x = first.x - 8.0;
        let label = if self.monday_first { "Mon first" } else { "Sun first" };
        if ui.pill_button(first, label, None, ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM)) {
            self.monday_first = !self.monday_first;
        }
        if !viewing_now {
            let t = Rect::new(x - 64.0, r.y + 4.0, 64.0, 26.0);
            if ui.pill_button(t, "Today", None, ButtonStyle::inverted()) {
                self.year = today.year;
                self.month = today.month;
            }
        }

        // --- how far through the month ------------------------------------
        let dim = days_in_month(self.year, self.month);
        let bar = Rect::new(r.x + 4.0, r.y + HEAD_H + 10.0, r.w - 8.0, 8.0);
        let frac = if viewing_now {
            (today.day as f32 - 1.0 + today.hour as f32 / 24.0) / dim as f32
        } else if (self.year, self.month) < (today.year, today.month) {
            1.0
        } else {
            0.0
        };
        ui.ticks(bar, frac, Color::white(0.9));

        // --- weekday names and the grid ------------------------------------
        let top = bar.bottom() + 12.0;
        let col_w = (r.w - WEEK_COL) / 7.0;
        let names = ["S", "M", "T", "W", "T", "F", "S"];
        ui.p.text("Wk", Rect::new(r.x, top, WEEK_COL, 18.0), &TextStyle::new(10.5, palette::TEXT_FAINT).center());
        for c in 0..7 {
            let idx = if self.monday_first { (c + 1) % 7 } else { c };
            ui.p.text(
                names[idx],
                Rect::new(r.x + WEEK_COL + c as f32 * col_w, top, col_w, 18.0),
                &TextStyle::new(11.0, palette::TEXT_FAINT).semibold().center(),
            );
        }
        let grid = Rect::new(r.x, top + 18.0 + 6.0, r.w, 6.0 * CELL_H);
        let wheel = ui.wheel(grid);
        if wheel != 0.0 {
            self.step(if wheel < 0.0 { 1 } else { -1 });
        }

        let first_day = days_from_civil(self.year, self.month, 1);
        let lead = if self.monday_first { (weekday(first_day) + 6) % 7 } else { weekday(first_day) };
        let start = first_day - lead;
        for row in 0..6i64 {
            let y = grid.y + row as f32 * CELL_H;
            let d0 = start + row * 7;
            let (wy, wm, wd) = civil_from_days(d0 + if self.monday_first { 0 } else { 1 });
            ui.p.text(
                &iso_week(wy, wm, wd).to_string(),
                Rect::new(r.x, y, WEEK_COL, CELL_H),
                &TextStyle::new(10.5, palette::TEXT_FAINT).center(),
            );
            for c in 0..7i64 {
                let day = d0 + c;
                let (cy_, cm, cd) = civil_from_days(day);
                let cell = Rect::new(r.x + WEEK_COL + c as f32 * col_w, y, col_w, CELL_H);
                let in_month = cm == self.month;
                let is_today = cy_ == today.year && cm == today.month && cd == today.day;
                let weekend = matches!(weekday(day), 0 | 6);
                let (cx, cy) = (cell.cx(), cell.cy());
                if is_today {
                    ui.p.fill_circle(cx, cy, 13.0, Color::white(0.95));
                } else if ui.hovered(cell) {
                    ui.p.fill_circle(cx, cy, 13.0, Color::white(0.08));
                }
                let col = if is_today {
                    palette::INK
                } else if !in_month {
                    Color::white(0.22)
                } else if weekend {
                    palette::TEXT_DIM
                } else {
                    palette::TEXT
                };
                let weight = TextStyle::new(13.0, col);
                ui.p.text(&cd.to_string(), cell, &if is_today { weight.semibold().center() } else { weight.center() });
            }
        }
    }
}
