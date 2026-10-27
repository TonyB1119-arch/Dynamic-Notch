//! Wellness reminders: drink water, rest your eyes (20-20-20) and stretch.
//!
//! Each reminder is off until you switch it on. A switched-on reminder costs
//! one timer wake-up per interval, nothing in between. Intervals are saved to
//! `[tools]` in config.toml.

use std::time::{Duration, Instant};

use super::fmt_duration;
use crate::config::Config;
use crate::gfx::{Icon, TextStyle};
use crate::modules::widgets::switch;
use crate::modules::{Cx, Module, ModuleId, Peek, SystemEvent};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "wellness";
const ROW_H: f32 = 56.0;
const HEAD_H: f32 = 28.0;
const GLASS_GOAL: u32 = 8;

struct Kind {
    key: &'static str,
    name: &'static str,
    icon: Icon,
    default_minutes: u32,
    title: &'static str,
    sub: &'static str,
}

const KINDS: [Kind; 3] = [
    Kind {
        key: "water_minutes",
        name: "Drink water",
        icon: Icon::Cloud,
        default_minutes: 45,
        title: "Time for some water",
        sub: "A glass now keeps you going",
    },
    Kind {
        key: "eyes_minutes",
        name: "Rest your eyes",
        icon: Icon::Sparkle,
        default_minutes: 20,
        title: "Look away for 20 seconds",
        sub: "Focus on something 20 feet away",
    },
    Kind {
        key: "stretch_minutes",
        name: "Stand and stretch",
        icon: Icon::Pulse,
        default_minutes: 60,
        title: "Stand up and stretch",
        sub: "You have been sitting for a while",
    },
];

pub struct Wellness {
    minutes: [u32; 3],
    due: [Option<Instant>; 3],
    glasses: u32,
    refresh_at: Option<Instant>,
}

impl Wellness {
    pub fn new() -> Self {
        Self { minutes: [0; 3], due: [None; 3], glasses: 0, refresh_at: None }
    }

    /// Re-arm one reminder after its interval changed.
    fn arm(&mut self, i: usize, now: Instant) {
        self.due[i] = (self.minutes[i] > 0).then(|| now + Duration::from_secs(self.minutes[i] as u64 * 60));
    }

    fn sync(&mut self, cfg: &Config, now: Instant) {
        let t = &cfg.tools;
        let want = [t.water_minutes, t.eyes_minutes, t.stretch_minutes];
        for i in 0..3 {
            if want[i] != self.minutes[i] || (want[i] > 0 && self.due[i].is_none()) {
                self.minutes[i] = want[i];
                self.arm(i, now);
            }
        }
    }
}

impl Module for Wellness {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Wellness"
    }
    fn icon(&self) -> Icon {
        Icon::Sparkle
    }

    fn start(&mut self, cx: &mut Cx) {
        self.sync(cx.cfg, cx.now);
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        if matches!(ev, SystemEvent::ConfigReloaded) {
            self.sync(cx.cfg, cx.now);
        }
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let on = self.minutes.iter().filter(|m| **m > 0).count();
        Some((Icon::Sparkle, if on == 0 { "All off".to_string() } else { format!("{on} on") }))
    }

    fn next_tick(&self) -> Option<Instant> {
        self.due.iter().flatten().copied().chain(self.refresh_at).min()
    }

    fn on_tick(&mut self, cx: &mut Cx) {
        self.refresh_at = None;
        for i in 0..3 {
            if self.due[i].is_some_and(|d| cx.now >= d) {
                let k = &KINDS[i];
                let sub =
                    if i == 0 { format!("{} of {GLASS_GOAL} glasses so far", self.glasses) } else { k.sub.to_string() };
                cx.fx.peek(
                    Peek::new(k.icon, palette::BLUE, k.title, sub).key("wellness").duration_ms(6000).page("tools"),
                );
                self.arm(i, cx.now);
            }
        }
        cx.fx.redraw = true;
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        HEAD_H + 3.0 * ROW_H + 58.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let now = ui.now;
        if self.refresh_at.is_none_or(|t| t > now + Duration::from_secs(30)) {
            self.refresh_at = Some(now + Duration::from_secs(30));
        }
        ui.caption("REMINDERS", Rect::new(r.x + 2.0, r.y + 4.0, 200.0, 18.0));

        for (i, k) in KINDS.iter().enumerate() {
            let row = Rect::new(r.x, r.y + HEAD_H + i as f32 * ROW_H, r.w, ROW_H - 6.0);
            ui.row_bg(row, false, None);
            let on = self.minutes[i] > 0;
            ui.p.icon(
                k.icon,
                Rect::new(row.x + 14.0, row.cy() - 10.0, 20.0, 20.0),
                if on { palette::TEXT } else { palette::TEXT_DIM },
            );
            ui.p.text(
                k.name,
                Rect::new(row.x + 46.0, row.y + 6.0, 150.0, 18.0),
                &TextStyle::new(13.5, palette::TEXT).semibold(),
            );
            let sub = if on { format!("every {} min", self.minutes[i]) } else { "Off".to_string() };
            ui.p.text(
                &sub,
                Rect::new(row.x + 46.0, row.y + 25.0, 150.0, 16.0),
                &TextStyle::new(11.5, palette::TEXT_FAINT),
            );

            // right side: switch, then the interval buttons and the countdown
            let sw = Rect::new(row.right() - 56.0, row.y, 48.0, row.h);
            if switch(ui, sw, on, palette::BLUE) {
                let next = if on { 0 } else { k.default_minutes };
                self.minutes[i] = next;
                self.arm(i, now);
                ui.fx.save_config.push(("tools".into(), k.key.into(), next.to_string()));
            }
            if on {
                let plus = Rect::new(sw.x - 34.0, row.cy() - 14.0, 28.0, 28.0);
                let minus = Rect::new(plus.x - 32.0, row.cy() - 14.0, 28.0, 28.0);
                let style = ButtonStyle::filled(Color::white(0.08)).scale(0.5);
                for (b, icon, delta) in [(minus, Icon::Minus, -5i32), (plus, Icon::Plus, 5)] {
                    if ui.icon_button(b, icon, style) {
                        let next = (self.minutes[i] as i32 + delta).clamp(5, 480) as u32;
                        self.minutes[i] = next;
                        self.arm(i, now);
                        ui.fx.save_config.push(("tools".into(), k.key.into(), next.to_string()));
                    }
                }
                if let Some(d) = self.due[i] {
                    let total = self.minutes[i] as f32 * 60.0;
                    let left = d.saturating_duration_since(now);
                    let frac = 1.0 - left.as_secs_f32() / total.max(1.0);
                    let bar =
                        Rect::new(row.x + 210.0, row.cy() - 6.0, (minus.x - row.x - 210.0 - 70.0).max(40.0), 12.0);
                    ui.ticks(bar, frac.clamp(0.0, 1.0), Color::white(0.9));
                    ui.p.text(
                        &format!("in {}", fmt_duration(left.as_secs())),
                        Rect::new(bar.right() + 8.0, row.y, 62.0, row.h),
                        &TextStyle::new(11.5, palette::TEXT_DIM),
                    );
                }
            }
        }

        // glasses of water today
        let y = r.y + HEAD_H + 3.0 * ROW_H + 6.0;
        ui.p.text(
            &format!("{} of {GLASS_GOAL} glasses today", self.glasses),
            Rect::new(r.x + 14.0, y, 180.0, 36.0),
            &TextStyle::new(13.0, palette::TEXT).medium(),
        );
        ui.ticks(
            Rect::new(r.x + 200.0, y + 12.0, (r.w - 200.0 - 190.0).max(60.0), 12.0),
            self.glasses as f32 / GLASS_GOAL as f32,
            Color::white(0.9),
        );
        let add = Rect::new(r.right() - 176.0, y + 3.0, 110.0, 30.0);
        if ui.pill_button(add, "+ Glass", Some(Icon::Cloud), ButtonStyle::inverted()) {
            self.glasses += 1;
        }
        let reset = Rect::new(r.right() - 58.0, y + 3.0, 58.0, 30.0);
        if ui.pill_button(reset, "Reset", None, ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
            self.glasses = 0;
        }
    }
}
