//! World clock: the time in a handful of cities, each with a tick bar for
//! how far through its day it is. Cities come from `[tools] zones` in
//! config.toml as "Name=UTC offset in hours".

use std::time::{Duration, Instant};

use super::{local_offset_secs, utc_secs, Local};
use crate::gfx::{Icon, TextStyle};
use crate::modules::{Cx, Module, ModuleId};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "worldclock";
const ROW_H: f32 = 44.0;
const HEAD_H: f32 = 30.0;
const MAX_ZONES: usize = 6;

pub struct WorldClock {
    twelve_hour: bool,
    refresh_at: Option<Instant>,
    /// Cached parse of the config list: (name, offset in seconds).
    zones: Vec<(String, i64)>,
    zones_src: Vec<String>,
}

impl WorldClock {
    pub fn new() -> Self {
        Self { twelve_hour: true, refresh_at: None, zones: Vec::new(), zones_src: Vec::new() }
    }

    fn sync(&mut self, src: &[String]) {
        if self.zones_src == src {
            return;
        }
        self.zones = src.iter().filter_map(|s| parse_zone(s)).take(MAX_ZONES).collect();
        self.zones_src = src.to_vec();
    }
}

fn parse_zone(s: &str) -> Option<(String, i64)> {
    let (name, off) = s.split_once('=')?;
    let hours: f64 = off.trim().trim_start_matches('+').parse().ok()?;
    if !(-14.0..=14.0).contains(&hours) {
        return None;
    }
    Some((name.trim().to_string(), (hours * 3600.0).round() as i64))
}

fn fmt_clock(sod: i64, twelve: bool) -> String {
    let (h, m) = (sod / 3600, (sod % 3600) / 60);
    if twelve {
        let ap = if h < 12 { "AM" } else { "PM" };
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        format!("{h12}:{m:02} {ap}")
    } else {
        format!("{h:02}:{m:02}")
    }
}

fn fmt_diff(secs: i64) -> String {
    let h = secs as f64 / 3600.0;
    if h.fract() == 0.0 {
        format!("{h:+.0}h")
    } else {
        format!("{h:+.1}h")
    }
}

impl Module for WorldClock {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "World Clock"
    }
    fn icon(&self) -> Icon {
        Icon::Globe
    }

    fn start(&mut self, cx: &mut Cx) {
        self.sync(&cx.cfg.tools.zones);
    }

    fn on_system(&mut self, ev: &crate::modules::SystemEvent, cx: &mut Cx) {
        if matches!(ev, crate::modules::SystemEvent::ConfigReloaded) {
            self.sync(&cx.cfg.tools.zones);
        }
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let first = self.zones.first()?;
        let sod = (utc_secs() + first.1).rem_euclid(86_400);
        Some((Icon::Globe, fmt_clock(sod, self.twelve_hour)))
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
        HEAD_H + 6.0 + (1 + self.zones.len().max(1)) as f32 * ROW_H
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        self.sync(&ui.cfg.tools.zones);
        self.refresh_at = Some(ui.now + Duration::from_secs(30));

        ui.caption("WORLD CLOCK", Rect::new(r.x + 2.0, r.y + 4.0, 200.0, 18.0));
        let tgl = Rect::new(r.right() - 70.0, r.y + 2.0, 70.0, 26.0);
        let label = if self.twelve_hour { "12 hour" } else { "24 hour" };
        if ui.pill_button(tgl, label, None, ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM)) {
            self.twelve_hour = !self.twelve_hour;
        }

        let utc = utc_secs();
        let here = local_offset_secs();
        let now = Local::now();
        let mut rows: Vec<(String, i64)> = vec![("Here".to_string(), here)];
        rows.extend(self.zones.iter().cloned());
        if self.zones.is_empty() {
            ui.p.text(
                "Add cities under [tools] zones in config.toml",
                Rect::new(r.x, r.y + HEAD_H + 6.0 + ROW_H, r.w, ROW_H),
                &TextStyle::new(12.0, palette::TEXT_FAINT).center(),
            );
        }
        for (i, (name, off)) in rows.iter().enumerate() {
            let row = Rect::new(r.x, r.y + HEAD_H + 6.0 + i as f32 * ROW_H, r.w, ROW_H - 4.0);
            let hover = ui.hovered(row);
            ui.row_bg(row, hover, None);
            let secs = utc + off;
            let day = secs.div_euclid(86_400);
            let sod = secs.rem_euclid(86_400);
            let when = match day - now.days() {
                0 => "Today",
                1 => "Tomorrow",
                -1 => "Yesterday",
                _ => "",
            };
            let sub = if i == 0 { when.to_string() } else { format!("{when} · {}", fmt_diff(off - here)) };
            ui.p.text(
                name,
                Rect::new(row.x + 14.0, row.y + 5.0, row.w * 0.30, 18.0),
                &TextStyle::new(13.5, palette::TEXT).semibold(),
            );
            ui.p.text(
                &sub,
                Rect::new(row.x + 14.0, row.y + 23.0, row.w * 0.30, 14.0),
                &TextStyle::new(11.0, palette::TEXT_FAINT),
            );
            // how far through the day it is there
            let bar = Rect::new(row.x + row.w * 0.34, row.cy() - 6.0, row.w * 0.34, 12.0);
            ui.ticks(bar, sod as f32 / 86_400.0, Color::white(0.9));
            ui.p.text(
                &fmt_clock(sod, self.twelve_hour),
                Rect::new(row.right() - row.w * 0.28 - 12.0, row.y, row.w * 0.28, row.h),
                &TextStyle::new(17.0, palette::TEXT).semibold().display().right(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones() {
        assert_eq!(parse_zone("Tokyo=+9"), Some(("Tokyo".into(), 32_400)));
        assert_eq!(parse_zone("Delhi = 5.5"), Some(("Delhi".into(), 19_800)));
        assert_eq!(parse_zone("Nowhere=99"), None);
        assert_eq!(parse_zone("nonsense"), None);
    }

    #[test]
    fn clocks() {
        assert_eq!(fmt_clock(0, true), "12:00 AM");
        assert_eq!(fmt_clock(13 * 3600 + 5 * 60, true), "1:05 PM");
        assert_eq!(fmt_clock(13 * 3600 + 5 * 60, false), "13:05");
        assert_eq!(fmt_diff(9 * 3600), "+9h");
        assert_eq!(fmt_diff(-19_800), "-5.5h");
    }
}
