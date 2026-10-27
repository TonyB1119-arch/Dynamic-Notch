//! System info: uptime, memory, battery (with time left) and free space on
//! every drive, each with a tick gauge. Drive sizes are read once when the
//! page opens (and on Refresh) by a short-lived helper process.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};

use super::{fmt_duration, run_hidden};
use crate::gfx::{Icon, TextStyle};
use crate::modules::{Cx, Module, ModuleId};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "sysinfo";
const ROW_H: f32 = 36.0;
const HEAD_H: f32 = 30.0;
const MAX_DRIVES: usize = 5;

#[derive(Clone, Debug, PartialEq)]
struct Drive {
    name: String,
    used: u64,
    free: u64,
}

#[derive(Default)]
struct Shared {
    drives: Vec<Drive>,
    loading: bool,
    loaded: bool,
}

pub struct SysInfo {
    shared: Arc<Mutex<Shared>>,
    refresh_at: Option<Instant>,
}

impl SysInfo {
    pub fn new() -> Self {
        Self { shared: Arc::new(Mutex::new(Shared::default())), refresh_at: None }
    }

    fn scan(&self, bus: &crate::bus::Bus) {
        {
            let Ok(mut s) = self.shared.lock() else { return };
            if s.loading {
                return;
            }
            s.loading = true;
        }
        let shared = Arc::clone(&self.shared);
        let bus = bus.clone();
        let _ = std::thread::Builder::new().name("sysinfo".into()).spawn(move || {
            let text = run_hidden(
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-Command",
                    "Get-PSDrive -PSProvider FileSystem | Where-Object { $_.Used -ne $null } | ForEach-Object { '{0}|{1}|{2}' -f $_.Name,$_.Used,$_.Free }",
                ],
            )
            .unwrap_or_default();
            let drives = parse_drives(&text);
            if let Ok(mut s) = shared.lock() {
                s.drives = drives;
                s.loading = false;
                s.loaded = true;
            }
            bus.to_module("tools", ());
        });
    }
}

fn parse_drives(text: &str) -> Vec<Drive> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.trim().split('|');
            let name = it.next()?.trim();
            let used: u64 = it.next()?.trim().parse().ok()?;
            let free: u64 = it.next()?.trim().parse().ok()?;
            (!name.is_empty() && used + free > 0).then(|| Drive { name: format!("{name}:"), used, free })
        })
        .take(MAX_DRIVES)
        .collect()
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
}

fn power() -> Option<SYSTEM_POWER_STATUS> {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return None;
    }
    Some(s)
}

/// (fraction 0..1, text) for the battery row.
fn battery_line() -> (Option<f32>, String) {
    let Some(s) = power() else { return (None, "Unknown".into()) };
    if s.BatteryFlag == 128 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
        return (None, "No battery".into());
    }
    let pct = s.BatteryLifePercent as f32 / 100.0;
    let plugged = s.ACLineStatus == 1;
    let text = if plugged {
        if s.BatteryLifePercent >= 100 {
            "Full · plugged in".to_string()
        } else {
            format!("{}% · charging", s.BatteryLifePercent)
        }
    } else if s.BatteryLifeTime != u32::MAX {
        format!("{}% · {} left", s.BatteryLifePercent, fmt_duration(s.BatteryLifeTime as u64))
    } else {
        format!("{}%", s.BatteryLifePercent)
    };
    (Some(pct), text)
}

impl Module for SysInfo {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "System Info"
    }
    fn icon(&self) -> Icon {
        Icon::Pulse
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let up = unsafe { GetTickCount64() } / 1000;
        Some((Icon::Pulse, format!("up {}", fmt_duration(up))))
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
        let drives = self.shared.lock().map_or(1, |s| s.drives.len().max(1));
        HEAD_H + 6.0 + (3 + drives) as f32 * ROW_H
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        self.refresh_at = Some(ui.now + Duration::from_secs(5));
        let (needs_scan, drives, loading) = match self.shared.lock() {
            Ok(s) => (!s.loaded && !s.loading, s.drives.clone(), s.loading),
            Err(_) => (false, Vec::new(), false),
        };
        if needs_scan {
            self.scan(ui.bus);
        }

        let user = std::env::var("USERNAME").unwrap_or_default();
        let pc = std::env::var("COMPUTERNAME").unwrap_or_default();
        ui.caption(&format!("{pc}  ·  {user}").to_uppercase(), Rect::new(r.x + 2.0, r.y + 4.0, r.w - 100.0, 18.0));
        let rb = Rect::new(r.right() - 90.0, r.y + 2.0, 90.0, 26.0);
        if ui.pill_button(
            rb,
            "Refresh",
            Some(Icon::Refresh),
            ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM),
        ) {
            self.scan(ui.bus);
        }
        if loading {
            ui.ticks_sweep(Rect::new(r.x + r.w * 0.45, r.y + 10.0, r.w * 0.2, 8.0), Color::white(0.9));
        }

        let mut rows: Vec<(String, Option<f32>, String)> = Vec::new();
        let up = unsafe { GetTickCount64() } / 1000;
        // uptime gauge: a full week fills the bar
        rows.push(("Uptime".into(), Some((up as f32 / 604_800.0).min(1.0)), fmt_duration(up)));
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() && m.ullTotalPhys > 0 {
            let used = m.ullTotalPhys - m.ullAvailPhys;
            rows.push((
                "Memory".into(),
                Some(used as f32 / m.ullTotalPhys as f32),
                format!("{:.1} of {:.1} GB", gb(used), gb(m.ullTotalPhys)),
            ));
        } else {
            rows.push(("Memory".into(), None, "Unknown".into()));
        }
        let (bat, bat_text) = battery_line();
        rows.push(("Battery".into(), bat, bat_text));
        if drives.is_empty() {
            rows.push(("Drives".into(), None, if loading { "Reading…".into() } else { "None found".into() }));
        }
        for d in &drives {
            let total = d.used + d.free;
            rows.push((
                format!("Drive {}", d.name),
                Some(d.used as f32 / total as f32),
                format!("{:.0} GB free of {:.0}", gb(d.free), gb(total)),
            ));
        }

        for (i, (label, frac, value)) in rows.iter().enumerate() {
            let row = Rect::new(r.x, r.y + HEAD_H + 6.0 + i as f32 * ROW_H, r.w, ROW_H - 4.0);
            ui.row_bg(row, false, None);
            ui.p.text(label, Rect::new(row.x + 14.0, row.y, 100.0, row.h), &TextStyle::new(12.5, palette::TEXT_DIM));
            let bar = Rect::new(row.x + 120.0, row.cy() - 6.0, row.w * 0.32, 12.0);
            match frac {
                Some(f) => ui.ticks(bar, *f, Color::white(0.9)),
                None => ui.tick_rule(Rect::new(bar.x, bar.cy() - 3.0, bar.w, 6.0), 0.10),
            }
            ui.p.text(
                value,
                Rect::new(bar.right() + 12.0, row.y, row.right() - bar.right() - 24.0, row.h),
                &TextStyle::new(13.0, palette::TEXT).semibold().right(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_lines() {
        let d = parse_drives("C|100|300\nD|50|50\nbad\nE|0|0\n");
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], Drive { name: "C:".into(), used: 100, free: 300 });
    }
}
