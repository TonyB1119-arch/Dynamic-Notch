//! A month calendar. Today is marked in red, a click selects a day and tells how far away it is.
//! It shows dates only: it does not read appointments from any calendar account.

use windows::Win32::System::SystemInformation::GetLocalTime;

use super::{Cx, Module, ModuleId};
use crate::gfx::{Icon, TextStyle};
use crate::ui::{gkey, ButtonStyle, Ui};
use crate::util::{palette, Color, Rect, WEEKDAY_NAMES};

const ID: ModuleId = "calendar";
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

type Ymd = (i32, u32, u32);

fn today() -> Ymd {
    let st = unsafe { GetLocalTime() };
    (st.wYear as i32, st.wMonth as u32, st.wDay as u32)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 (proleptic Gregorian).
fn day_number((y, m, d): Ymd) -> i64 {
    let y = y as i64 - (m <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 0 = Monday ... 6 = Sunday.
fn weekday(date: Ymd) -> usize {
    // 1970-01-01 was a Thursday
    (day_number(date) + 3).rem_euclid(7) as usize
}

pub struct Calendar {
    view: Option<(i32, u32)>,
    sel: Option<Ymd>,
}

impl Calendar {
    pub fn new() -> Self {
        Self { view: None, sel: None }
    }

    fn step(&mut self, dir: i32) {
        let (y, m) = self.view.unwrap_or_else(|| {
            let t = today();
            (t.0, t.1)
        });
        let idx = y * 12 + (m as i32 - 1) + dir;
        self.view = Some((idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1));
    }
}

fn relative(sel: Ymd, now: Ymd) -> String {
    let d = day_number(sel) - day_number(now);
    match d {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        d if d > 0 => format!("In {d} days"),
        d => format!("{} days ago", -d),
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
        Icon::Grid
    }
    fn start(&mut self, _cx: &mut Cx) {}

    fn has_page(&self) -> bool {
        true
    }
    fn page_height(&self, _width: f32) -> f32 {
        210.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let now = today();
        let (vy, vm) = *self.view.get_or_insert((now.0, now.1));
        let sel = *self.sel.get_or_insert(now);
        let red = palette::RED;

        // left: the selected day
        let left_w = (r.w * 0.36).min(230.0);
        let wd = weekday(sel);
        let name = WEEKDAY_NAMES[(wd + 1) % 7].to_uppercase();
        ui.p.text(&name, Rect::new(r.x, r.y + 4.0, left_w, 22.0), &TextStyle::new(15.0, red).semibold());
        ui.p.text(
            &format!("{}", sel.2),
            Rect::new(r.x - 2.0, r.y + 24.0, left_w, 84.0),
            &TextStyle::new(76.0, Color::white(1.0)).weight(200),
        );
        ui.p.text(
            &format!("{} {}", MONTHS[(sel.1 - 1) as usize], sel.0),
            Rect::new(r.x, r.y + 112.0, left_w, 22.0),
            &TextStyle::new(14.5, Color::white(0.9)).semibold(),
        );
        ui.p.text(
            &relative(sel, now),
            Rect::new(r.x, r.y + 134.0, left_w, 20.0),
            &TextStyle::new(13.0, palette::TEXT_DIM),
        );
        if sel != now {
            let b = Rect::new(r.x, r.y + 164.0, 88.0, 28.0);
            if ui.pill_button(b, "Today", None, ButtonStyle::default().scale(0.9)) {
                self.sel = Some(now);
                self.view = Some((now.0, now.1));
            }
        }

        // right: the month
        let g = Rect::new(r.x + left_w + 24.0, r.y, r.w - left_w - 24.0, r.h);
        ui.p.text(
            &MONTHS[(vm - 1) as usize].to_uppercase(),
            Rect::new(g.x, g.y, 160.0, 24.0),
            &TextStyle::new(14.5, red).semibold(),
        );
        for (dir, dx) in [(-1, 56.0), (1, 28.0)] {
            let b = Rect::new(g.right() - dx - 12.0, g.y - 2.0, 28.0, 28.0);
            let hover = ui.hovered(b);
            if hover {
                ui.hot = true;
            }
            let c = Color::white(if hover { 0.95 } else { 0.55 });
            let (px, py) = (b.cx(), b.cy());
            let d = dir as f32;
            ui.p.line(px - d * 2.5, py - 5.5, px + d * 2.5, py, 1.9, c);
            ui.p.line(px + d * 2.5, py, px - d * 2.5, py + 5.5, 1.9, c);
            if ui.clicked(b) {
                self.step(dir);
            }
        }
        let cw = g.w / 7.0;
        let head_y = g.y + 30.0;
        let letters = ["M", "T", "W", "T", "F", "S", "S"];
        for (i, l) in letters.iter().enumerate() {
            let dim = if i >= 5 { 0.3 } else { 0.5 };
            ui.p.text(
                l,
                Rect::new(g.x + cw * i as f32, head_y, cw, 18.0),
                &TextStyle::new(12.0, Color::white(dim)).semibold().center(),
            );
        }
        let first = weekday((vy, vm, 1));
        let rows_y = head_y + 22.0;
        let rh = ((g.bottom() - rows_y) / 6.0).min(28.0);
        for d in 1..=days_in(vy, vm) {
            let idx = first + d as usize - 1;
            let (col, row) = (idx % 7, idx / 7);
            let cell = Rect::new(g.x + cw * col as f32, rows_y + rh * row as f32, cw, rh);
            let date = (vy, vm, d);
            let is_today = date == now;
            let is_sel = date == sel;
            let hover = ui.hovered(cell);
            if hover {
                ui.hot = true;
            }
            let k = ui.glide(
                gkey("cal-sel", (vy * 400 + vm as i32 * 31 + d as i32) as usize),
                if is_sel { 1.0 } else { 0.0 },
            );
            let rad = rh.min(cw) * 0.5 - 1.0;
            if k > 0.01 {
                ui.p.fill_circle(cell.cx(), cell.cy(), rad * (0.6 + 0.4 * k), Color::white(0.95 * k));
            } else if hover {
                ui.p.fill_circle(cell.cx(), cell.cy(), rad, Color::white(0.08));
            }
            let base = if col >= 5 { 0.4 } else { 0.9 };
            let c = if is_sel {
                palette::INK
            } else if is_today {
                red
            } else {
                Color::white(base)
            };
            let st = TextStyle::new(13.5, c);
            ui.p.text(&d.to_string(), cell, &if is_today || is_sel { st.semibold().center() } else { st.center() });
            if ui.clicked(cell) {
                self.sel = Some(date);
            }
        }
    }
}
