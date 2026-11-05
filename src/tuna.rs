//! The Tuna look's own notch screens, after the launcher: a small frame of big square tiles (one per
//! page, the open one lit blue) and a floating panel under it that holds the page. No header, no
//! tabs; click a tile, or scroll or swipe, to change page.

use crate::gfx::{Icon, TextStyle};
use crate::modules::{Module, ModuleId};
use crate::ui::{gkey, tuna_rim, tuna_sel, Ui};
use crate::util::{Color, Rect};

/// Space round the frame and the panel.
const MARGIN: f32 = 10.0;
const TILE: f32 = 76.0;
const TILE_GAP: f32 = 8.0;
const FRAME_PAD: f32 = 10.0;
const FRAME_H: f32 = TILE + 2.0 * FRAME_PAD;
/// Space between the frame and the panel, and between the panel's edge and the page inside it.
const GAP: f32 = 10.0;
const INSET: f32 = 14.0;
/// What the app reserves above, beside and under a page.
pub const TOP_H: f32 = MARGIN + FRAME_H + GAP + INSET;
pub const PAD: f32 = MARGIN + INSET;
pub const BOTTOM: f32 = MARGIN + INSET;
/// Height of the home page.
pub const HOME_H: f32 = 150.0;

/// The floating panel the page sits in. `full` is the whole open notch.
pub fn panel(ui: &mut Ui, full: Rect) {
    let y = full.y + MARGIN + FRAME_H + GAP;
    let r = Rect::new(full.x + MARGIN, y, full.w - 2.0 * MARGIN, full.bottom() - MARGIN - y);
    if r.h > 8.0 && r.w > 8.0 {
        ui.p.fill_rounded(r, 24.0, Color::rgba(0.04, 0.07, 0.12, 0.5));
        ui.p.stroke_rounded(r.inset(0.5), 23.5, 1.0, Color::white(0.1));
    }
}

/// The name under a tile.
fn label(modules: &[Box<dyn Module>], id: ModuleId) -> String {
    match id {
        "home" => "Home".to_string(),
        "settings" => "Settings".to_string(),
        _ => match modules.iter().find(|m| m.id() == id).map(|m| m.title()) {
            Some("Wi-Fi & Bluetooth") => "Network".to_string(),
            Some("Volume & Brightness") => "Sound".to_string(),
            Some(t) => t.to_string(),
            None => String::new(),
        },
    }
}

/// The frame of tiles at the top. It is as wide as its tiles; with more pages than fit, it slides to keep
/// the open one in view.
pub fn strip(
    ui: &mut Ui,
    full: Rect,
    modules: &[Box<dyn Module>],
    started: &[bool],
    tabs: &[(ModuleId, Icon, String)],
    page: ModuleId,
) {
    let mut items: Vec<(ModuleId, Icon)> = tabs.iter().map(|t| (t.0, t.1)).collect();
    if modules.iter().enumerate().any(|(i, m)| started[i] && m.id() == "settings") {
        items.push(("settings", Icon::Settings));
    }
    let n = items.len();
    if n == 0 {
        return;
    }
    let pitch = TILE + TILE_GAP;
    let room = full.w - 2.0 * MARGIN - 2.0 * FRAME_PAD + TILE_GAP;
    let fit = ((room / pitch).floor() as usize).clamp(1, n);
    let fw = fit as f32 * pitch - TILE_GAP + 2.0 * FRAME_PAD;
    let frame = Rect::new(full.cx() - fw * 0.5, full.y + MARGIN, fw, FRAME_H);
    ui.p.fill_rounded(frame, 30.0, Color::rgba(0.04, 0.07, 0.12, 0.5));
    ui.p.stroke_rounded(frame.inset(0.5), 29.5, 1.0, Color::white(0.14));
    ui.p.stroke_rounded(frame.inset(3.5), 26.5, 1.0, Color::white(0.05));

    let cur = items.iter().position(|t| t.0 == page).unwrap_or(0);
    let target = (cur as f32 - (fit as f32 - 1.0) * 0.5).clamp(0.0, (n - fit) as f32);
    let scroll = ui.glide(gkey("tuna-scroll", 0), target);
    let over = ui.hovered(frame);
    ui.p.push_clip(frame.inset(2.0));
    for (i, (id, icon)) in items.iter().enumerate() {
        let x = frame.x + FRAME_PAD + (i as f32 - scroll) * pitch;
        if x + TILE < frame.x || x > frame.right() {
            continue;
        }
        let r = Rect::new(x, frame.y + FRAME_PAD, TILE, TILE);
        let hover = over && ui.hovered(r);
        if hover {
            ui.hot = true;
        }
        let on = ui.glide(gkey(id, 91), if *id == page { 1.0 } else { 0.0 });
        let h = ui.glide(gkey(id, 92), if hover { 1.0 } else { 0.0 });
        let fill = Color::white(0.05 + 0.04 * h).lerp(tuna_sel(), on);
        ui.p.fill_rounded(r, 22.0, fill);
        ui.p.stroke_rounded(r.inset(0.5), 21.5, 1.0, Color::white(0.08 + 0.05 * h).lerp(tuna_rim(), on));
        if on > 0.01 {
            ui.p.stroke_rounded(r.inset(3.5), 18.5, 1.0, Color::white(0.1 * on));
        }
        let fg = Color::white(0.62 + 0.38 * on.max(h));
        ui.p.icon(*icon, Rect::new(r.cx() - 16.0, r.y + 11.0, 32.0, 32.0), fg);
        let name = label(modules, *id);
        ui.p.text(
            &name,
            Rect::new(r.x + 2.0, r.bottom() - 26.0, r.w - 4.0, 18.0),
            &TextStyle::new(11.5, fg).semibold().center(),
        );
        if hover && ui.clicked(r) {
            ui.fx.open_page = Some(*id);
        }
    }
    ui.p.pop_clip();
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
