//! The Tuna look's own notch screens. The open notch is a row of big glass boxes (the open page, the
//! time, the state of the computer) with one floating glass panel under it that holds the page. There
//! are no tabs: the wheel or a swipe turns the page, and so does a click on the first box.
//!
//! Everything is drawn with the same primitives as the other looks; only the layout and the shapes
//! here are specific to Tuna.

use crate::gfx::{icons, Icon, TextStyle};
use crate::modules::{Indicator, Module, ModuleId};
use crate::ui::{rect_key, tuna_rim, tuna_sel, Ui};
use crate::util::{Color, Rect};

/// Space round the boxes, the height of the row of boxes and the gap between the row and the panel.
pub const SIDE: f32 = 12.0;
pub const ROW: f32 = 88.0;
pub const GAP: f32 = 10.0;
/// Space between the panel's edge and the page inside it.
pub const INSET: f32 = 16.0;
/// What the app reserves above, beside and under a page.
pub const TOP_H: f32 = SIDE + ROW + GAP + INSET;
pub const PAD: f32 = SIDE + INSET;
pub const BOTTOM: f32 = SIDE + INSET;
/// Height of the home page.
pub const HOME_H: f32 = 150.0;

const BOX_R: f32 = 24.0;
const PANEL_R: f32 = 28.0;

/// A box of glass: dark and clear with a bright rim. `glow` (0 - 1) turns it the deep blue of what is chosen.
pub fn glass_box(ui: &mut Ui, r: Rect, radius: f32, glow: f32) {
    let base = Color::rgba(0.03, 0.04, 0.07, 0.32);
    ui.p.fill_rounded(r, radius, base.lerp(tuna_sel(), glow));
    if glow < 0.99 {
        ui.p.fill_rounded(r, radius, Color::white(0.05 * (1.0 - glow)));
    }
    let w = 1.0 + 0.6 * glow;
    ui.p.stroke_rounded(r.inset(w * 0.5), radius - w * 0.5, w, Color::white(0.15).lerp(tuna_rim(), glow));
}

/// The floating panel the page sits in. `full` is the whole open notch.
pub fn panel(ui: &mut Ui, full: Rect) {
    let r = Rect::new(full.x + SIDE, full.y + SIDE + ROW + GAP, full.w - 2.0 * SIDE, full.h - (2.0 * SIDE + ROW + GAP));
    if r.h > 8.0 && r.w > 8.0 {
        glass_box(ui, r, PANEL_R, 0.0);
    }
}

/// The page after (or before) this one.
fn step(tabs: &[(ModuleId, Icon, String)], page: ModuleId, back: bool) -> Option<ModuleId> {
    let n = tabs.len();
    if n == 0 {
        return None;
    }
    let next = match tabs.iter().position(|t| t.0 == page) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None => 0,
    };
    Some(tabs[next].0)
}

/// The row of boxes at the top: the open page, the time and the state of the computer.
pub fn row(
    ui: &mut Ui,
    full: Rect,
    modules: &[Box<dyn Module>],
    started: &[bool],
    tabs: &[(ModuleId, Icon, String)],
    page: ModuleId,
    indicators: &[Indicator],
) {
    let strip = Rect::new(full.x + SIDE, full.y + SIDE, full.w - 2.0 * SIDE, ROW);
    let inner = strip.w - 2.0 * GAP;
    let b1 = Rect::new(strip.x, strip.y, (inner * 0.38).round(), ROW);
    let b2 = Rect::new(b1.right() + GAP, strip.y, (inner * 0.31).round(), ROW);
    let b3 = Rect::new(b2.right() + GAP, strip.y, strip.right() - b2.right() - GAP, ROW);

    // the open page: its icon and name, a pip for every page; a click goes on to the next page
    let cur = tabs.iter().position(|t| t.0 == page);
    let icon = cur.map_or(Icon::Settings, |i| tabs[i].1);
    let title = if page == "home" {
        "Home".to_string()
    } else {
        modules.iter().find(|m| m.id() == page).map(|m| m.title().to_string()).unwrap_or_default()
    };
    let hover = ui.hovered(b1);
    if hover {
        ui.hot = true;
    }
    let h = ui.glide(rect_key(b1, "tuna-page"), if hover { 1.0 } else { 0.0 });
    glass_box(ui, b1, BOX_R, 0.82 + 0.18 * h);
    let d = 44.0;
    let disc = Rect::new(b1.x + 16.0, b1.cy() - d * 0.5, d, d);
    ui.p.fill_circle(disc.cx(), disc.cy(), d * 0.5, Color::white(0.12));
    ui.p.icon(icon, disc.center_square(d * 0.52), Color::white(1.0));
    let tx = disc.right() + 12.0;
    ui.p.text(
        &title,
        Rect::new(tx, b1.y + 17.0, (b1.right() - tx - 10.0).max(0.0), 26.0),
        &TextStyle::new(18.0, Color::white(1.0)).semibold(),
    );
    let mut px = tx;
    for i in 0..tabs.len().min(10) {
        let on = Some(i) == cur;
        let w = if on { 16.0 } else { 5.0 };
        ui.p.fill_rounded(Rect::new(px, b1.y + 54.0, w, 5.0), 2.5, Color::white(if on { 0.95 } else { 0.32 }));
        px += w + 5.0;
    }
    if ui.clicked(b1) {
        ui.fx.open_page = step(tabs, page, false);
    }

    // the time
    glass_box(ui, b2, BOX_R, 0.0);
    let (hh, mm, _s, suffix, date) = crate::app::clock_parts();
    let time = if suffix.is_empty() { format!("{hh:02}:{mm:02}") } else { format!("{hh}:{mm:02} {suffix}") };
    let big = TextStyle::new(30.0, Color::white(1.0)).semibold().center();
    ui.p.text(&time, Rect::new(b2.x + 8.0, b2.y + 12.0, b2.w - 16.0, 40.0), &big);
    ui.p.text(
        &date,
        Rect::new(b2.x + 8.0, b2.y + 54.0, b2.w - 16.0, 20.0),
        &TextStyle::new(12.5, Color::white(0.55)).medium().center(),
    );

    // the state of the computer: battery, load, and the settings gear
    glass_box(ui, b3, BOX_R, 0.0);
    let mut battery = None;
    for ind in indicators {
        if let Some(b) = ind.battery {
            battery = Some(b);
        }
    }
    let has_settings = modules.iter().enumerate().any(|(i, m)| started[i] && m.id() == "settings");
    let right = if has_settings { b3.right() - 52.0 } else { b3.right() - 14.0 };
    let left = b3.x + 18.0;
    let load = |v: Option<i32>| v.map_or("--%".to_string(), |v| format!("{v}%"));
    let small = TextStyle::new(12.0, Color::white(0.62)).medium();
    let (cpu, ram) = (load(crate::modules::stats::cpu_percent()), load(crate::modules::stats::ram_percent()));
    let mut y = b3.y + 15.0;
    if let Some((level, charging)) = battery {
        let br = Rect::new(left, y + 4.0, 28.0, 14.0);
        icons::battery(ui.p, br, level, charging, crate::app::battery_color(level, charging));
        let pct = format!("{:.0}%", level * 100.0);
        ui.p.text(
            &pct,
            Rect::new(br.right() + 8.0, y, (right - br.right() - 8.0).max(0.0), 22.0),
            &TextStyle::new(15.0, Color::white(0.97)).semibold(),
        );
        y += 28.0;
        ui.p.text(&format!("CPU {cpu}   RAM {ram}"), Rect::new(left, y, (right - left).max(0.0), 18.0), &small);
    } else {
        ui.p.text(
            &format!("CPU {cpu}"),
            Rect::new(left, y, (right - left).max(0.0), 22.0),
            &TextStyle::new(15.0, Color::white(0.97)).semibold(),
        );
        y += 28.0;
        ui.p.text(&format!("RAM {ram}"), Rect::new(left, y, (right - left).max(0.0), 18.0), &small);
    }
    if has_settings {
        let gear = Rect::new(b3.right() - 46.0, b3.cy() - 16.0, 32.0, 32.0);
        let hv = ui.hovered(gear);
        if hv {
            ui.hot = true;
        }
        let g = ui.glide(rect_key(gear, "tuna-gear"), if hv { 1.0 } else { 0.0 });
        let on = page == "settings";
        let fill = Color::white(0.06 + 0.08 * g).lerp(tuna_sel(), if on { 1.0 } else { 0.0 });
        ui.p.fill_circle(gear.cx(), gear.cy(), 16.0, fill);
        if on {
            ui.p.stroke_circle(gear.cx(), gear.cy(), 15.5, 1.0, tuna_rim());
        }
        ui.p.icon(Icon::Settings, gear.center_square(17.0), Color::white(0.82 + 0.18 * g));
        if ui.clicked(gear) {
            ui.fx.open_page = Some("settings");
        }
    }
}

/// The home page: the player (when something plays) and the chosen widgets, side by side.
pub fn home(ui: &mut Ui, r: Rect, modules: &mut [Box<dyn Module>], large: Option<usize>, tiles: &[usize]) {
    let mut cards: Vec<(usize, f32)> = Vec::new();
    if let Some(i) = large {
        cards.push((i, 1.7));
    }
    for &i in tiles {
        if cards.len() >= 3 {
            break;
        }
        cards.push((i, 1.0));
    }
    if cards.is_empty() {
        let st = TextStyle::new(13.0, Color::white(0.55)).medium().center();
        ui.p.text("Scroll or swipe to look through the pages", r, &st);
        return;
    }
    let gap = 10.0;
    let total: f32 = cards.iter().map(|c| c.1).sum();
    let room = r.w - gap * (cards.len() as f32 - 1.0);
    let mut x = r.x;
    for (i, weight) in cards {
        let w = room * weight / total;
        modules[i].draw_card(ui, Rect::new(x, r.y, w, r.h));
        x += w + gap;
    }
}
