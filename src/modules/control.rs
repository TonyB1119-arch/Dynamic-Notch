//! Control Center: Wi-Fi and Bluetooth in one tab.
//!
//! Two buttons at the top switch between the Wi-Fi list and the Bluetooth
//! list; the lists slide sideways with a fade, and the highlight glides
//! between the buttons. Scrolling the wheel over the buttons switches too.
//!
//! The two lists are the `Wifi` and `Bluetooth` modules, kept as parts of
//! this one: their worker threads send their messages to "control", and this
//! module hands each message to whichever part it belongs to.

use std::any::Any;
use std::time::Instant;

use super::bluetooth::Bluetooth;
use super::wifi::Wifi;
use super::{CardSize, Cx, Key, Module, ModuleId, SystemEvent};
use crate::gfx::{Icon, TextStyle};
use crate::ui::Ui;
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "control";
const BAR_H: f32 = 40.0;
const BAR_GAP: f32 = 12.0;

pub struct ControlCenter {
    wifi: Wifi,
    bt: Bluetooth,
    /// 0 = Wi-Fi, 1 = Bluetooth.
    tab: usize,
    /// Animated position between the tabs (0.0 .. 1.0).
    pos: f32,
    last: Instant,
}

impl ControlCenter {
    pub fn new() -> Self {
        Self { wifi: Wifi::new(), bt: Bluetooth::new(), tab: 0, pos: 0.0, last: Instant::now() }
    }
}

impl Module for ControlCenter {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Wi-Fi & Bluetooth"
    }
    fn icon(&self) -> Icon {
        Icon::Control
    }

    fn start(&mut self, cx: &mut Cx) {
        self.wifi.start(cx);
        self.bt.start(cx);
    }

    fn stop(&mut self) {
        self.wifi.stop();
        self.bt.stop();
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        if Wifi::accepts(&msg) {
            self.wifi.on_message(msg, cx);
        } else if Bluetooth::accepts(&msg) {
            self.bt.on_message(msg, cx);
        }
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        self.wifi.on_system(ev, cx);
        self.bt.on_system(ev, cx);
    }

    fn glance(&self) -> Option<(Icon, String)> {
        self.wifi.glance()
    }

    fn glance_for(&self, id: &str) -> Option<(Icon, String)> {
        if id == "bluetooth" {
            self.bt.glance()
        } else {
            self.wifi.glance()
        }
    }

    // -- home card: two halves, each opens its list ---------------------------

    fn card(&self) -> Option<CardSize> {
        Some(CardSize::Small)
    }

    fn draw_card(&mut self, ui: &mut Ui, r: Rect) {
        ui.card(r, false);
        let half = r.w * 0.5;
        let halves = [Rect::new(r.x, r.y, half, r.h), Rect::new(r.x + half, r.y, half, r.h)];
        let info = [self.wifi.glance(), self.bt.glance()];
        for (i, (h, g)) in halves.iter().zip(info.iter()).enumerate() {
            if ui.hovered(*h) {
                ui.hot = true;
                ui.p.fill_rounded(h.inset(3.0), 10.0, Color::white(0.07));
            }
            if let Some((icon, text)) = g {
                let inner = h.inset_xy(10.0, 0.0);
                ui.p.icon(*icon, Rect::new(inner.x, h.cy() - 8.0, 16.0, 16.0), palette::TEXT_DIM);
                ui.p.text(
                    text,
                    Rect::new(inner.x + 22.0, h.y, (inner.w - 22.0).max(0.0), h.h),
                    &TextStyle::new(12.0, palette::TEXT).medium(),
                );
            }
            if ui.clicked(*h) {
                self.tab = i;
                ui.fx.open_page = Some(ID);
            }
        }
        ui.p.line(r.x + half, r.y + 9.0, r.x + half, r.bottom() - 9.0, 1.0, Color::white(0.1));
    }

    // -- page ---------------------------------------------------------------------

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        self.tab == 0 && self.wifi.wants_keyboard()
    }

    fn page_height(&self, w: f32) -> f32 {
        let a = self.wifi.page_height(w);
        let b = self.bt.page_height(w);
        let t = self.pos.clamp(0.0, 1.0);
        BAR_H + BAR_GAP + a + (b - a) * t
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        if self.tab == 0 {
            self.wifi.on_key(key, cx);
        }
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        // glide toward the selected tab
        let dt = ui.now.saturating_duration_since(self.last).as_secs_f32().min(0.05);
        self.last = ui.now;
        let target = self.tab as f32;
        if (self.pos - target).abs() < 0.003 {
            self.pos = target;
        } else {
            self.pos += (target - self.pos) * (1.0 - (-dt * 13.0).exp());
            ui.animate();
            ui.fx.layout_changed = true;
        }
        let settled = self.pos == target;

        // --- the two buttons ---------------------------------------------------
        let bar = Rect::new(r.x, r.y, r.w, BAR_H);
        // minimal: two quiet labels over a hairline, with one blue beam gliding under the open one
        let half = bar.w * 0.5;
        ui.p.fill_rect(Rect::new(bar.x, bar.bottom() - 1.0, bar.w, 1.0), Color::white(0.07));
        let bx = bar.x + self.pos * half + half * 0.5;
        ui.p.fill_rounded(Rect::new(bx - 22.0, bar.bottom() - 2.5, 44.0, 2.5), 1.25, palette::BLUE);
        for (i, (icon, label)) in [(Icon::Wifi, "Wi-Fi"), (Icon::Bluetooth, "Bluetooth")].iter().enumerate() {
            let b = Rect::new(bar.x + i as f32 * half, bar.y, half, bar.h);
            if ui.hovered(b) {
                ui.hot = true;
            }
            let near = (1.0 - (self.pos - i as f32).abs()).clamp(0.0, 1.0);
            let col = palette::TEXT_FAINT.lerp(palette::TEXT, near);
            let st = TextStyle::new(13.0, col).medium();
            let (tw, _) = ui.p.measure(label, &st);
            let x = b.cx() - (16.0 + 8.0 + tw) * 0.5;
            ui.p.icon(*icon, Rect::new(x, b.cy() - 8.0 - 1.0, 16.0, 16.0), col);
            ui.p.text(label, Rect::new(x + 24.0, b.y - 1.0, tw + 4.0, b.h), &st);
            if ui.clicked(b) {
                self.tab = i;
            }
        }
        // wheel over the buttons: down = next, up = previous
        let wheel = ui.wheel(bar);
        if wheel < 0.0 {
            self.tab = 1;
        } else if wheel > 0.0 {
            self.tab = 0;
        }

        // --- the two lists, sliding ------------------------------------------------
        let area = Rect::new(r.x, bar.bottom() + BAR_GAP, r.w, (r.h - BAR_H - BAR_GAP).max(0.0));
        let step = area.w + 28.0;
        ui.p.push_clip(area);
        for i in 0..2usize {
            let d = i as f32 - self.pos;
            let off = d * step;
            if off.abs() >= area.w {
                continue;
            }
            let alpha = ui.p.alpha;
            ui.p.alpha = alpha * (1.0 - 0.85 * d.abs().clamp(0.0, 1.0));
            let was = ui.interactive;
            // only the visible, settled list reacts to the mouse
            ui.interactive = was && settled && i == self.tab;
            let sub = Rect::new(area.x + off, area.y, area.w, area.h);
            if i == 0 {
                self.wifi.draw_page(ui, sub);
            } else {
                self.bt.draw_page(ui, sub);
            }
            ui.interactive = was;
            ui.p.alpha = alpha;
        }
        ui.p.pop_clip();
    }
}
