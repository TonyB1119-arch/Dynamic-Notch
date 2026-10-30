//! Battery + charging state. Purely event driven: Windows sends
//! WM_POWERBROADCAST when the power source or percentage changes.

use std::time::{Duration, Instant};

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use super::{Cx, Indicator, Module, ModuleId, Peek, SystemEvent, Trailing};
use crate::gfx::Icon;
use crate::util::{palette, Color};

const ID: ModuleId = "battery";

#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct State {
    has_battery: bool,
    plugged: bool,
    percent: u8,
    saver: bool,
    /// Minutes of charge left on battery (rounded to 5), 0 when Windows can't tell.
    mins: u32,
}

fn read() -> State {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return State::default();
    }
    State {
        has_battery: s.BatteryFlag != 128 && s.BatteryFlag != 255 && s.BatteryLifePercent <= 100,
        plugged: s.ACLineStatus == 1,
        percent: s.BatteryLifePercent.min(100),
        saver: s.SystemStatusFlag == 1,
        mins: if s.BatteryLifeTime == u32::MAX { 0 } else { (s.BatteryLifeTime / 60 + 2) / 5 * 5 },
    }
}

pub struct Battery {
    state: State,
    warned_low: bool,
    warned_full: bool,
    /// Where the charge stood when we last started running on battery: (percent, when).
    drain_from: Option<(u8, Instant)>,
    /// The text next to the percentage, worked out when the state changes, not on every frame.
    label: Option<String>,
}

impl Battery {
    pub fn new() -> Self {
        Self { state: State::default(), warned_low: false, warned_full: false, drain_from: None, label: None }
    }

    fn level(&self) -> f32 {
        self.state.percent as f32 / 100.0
    }

    /// Minutes left on battery: Windows' own estimate when it has one, otherwise
    /// worked out from how fast the charge has been falling since unplugging.
    fn minutes_left(&self) -> Option<u32> {
        if self.state.mins > 0 {
            return Some(self.state.mins);
        }
        let (from, at) = self.drain_from?;
        let used = from.saturating_sub(self.state.percent) as f32;
        let elapsed = at.elapsed().as_secs_f32() / 60.0;
        if used < 2.0 || elapsed < 2.0 {
            return None;
        }
        let left = self.state.percent as f32 / (used / elapsed);
        Some(((left / 5.0).round() as u32 * 5).clamp(5, 99 * 60))
    }

    fn refresh(&mut self) {
        self.track_drain();
        self.label = self.extra();
    }

    fn track_drain(&mut self) {
        if self.state.plugged || !self.state.has_battery {
            self.drain_from = None;
        } else if self.drain_from.is_none() {
            self.drain_from = Some((self.state.percent, Instant::now()));
        }
    }

    /// Shown next to the percentage: an infinity sign while plugged in,
    /// otherwise the time left on battery (when Windows has an estimate).
    fn extra(&self) -> Option<String> {
        if self.state.plugged {
            Some("\u{221E}".to_string())
        } else if let Some(mins) = self.minutes_left() {
            let (h, m) = (mins / 60, mins % 60);
            Some(if h > 0 { format!("{h}h {m:02}m") } else { format!("{m}m") })
        } else {
            None
        }
    }
}

impl Module for Battery {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Battery"
    }
    fn icon(&self) -> Icon {
        Icon::Bolt
    }

    fn start(&mut self, _cx: &mut Cx) {
        self.state = read();
        self.refresh();
    }

    fn next_tick(&self) -> Option<Instant> {
        self.state.has_battery.then(|| Instant::now() + Duration::from_secs(45))
    }

    fn on_tick(&mut self, _cx: &mut Cx) {
        self.state = read();
        self.refresh();
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        if !matches!(ev, SystemEvent::PowerChanged) {
            return;
        }
        let new = read();
        let old = self.state;
        self.state = new;
        self.refresh();
        if !new.has_battery || new == old {
            return;
        }
        cx.fx.redraw = true;
        let level = self.level();
        let pct = format!("{}%", new.percent);
        let hud = |color: Color, text: String, ms: u64| {
            Peek::new(Icon::Bolt, color, "", "")
                .trailing(Trailing::Level { frac: level, color, text: Some(text) })
                .key("battery")
                .duration_ms(ms)
        };
        if new.plugged && !old.plugged && cx.cfg.battery.peek_on_plug {
            let text = if new.percent >= 100 { "Fully charged".to_string() } else { format!("Charging · {pct}") };
            cx.fx.peek(hud(palette::GREEN, text, 2600));
            self.warned_low = false;
        } else if !new.plugged && old.plugged && cx.cfg.battery.peek_on_plug {
            cx.fx.peek(hud(Color::white(0.92), format!("On battery · {pct}"), 2000));
        }
        let low = cx.cfg.battery.low_threshold;
        if !new.plugged && new.percent <= low && !self.warned_low {
            self.warned_low = true;
            cx.fx.peek(hud(palette::RED, format!("Low battery · {pct}"), 5000));
        }
        if new.plugged && new.percent >= 100 && old.percent < 100 && !self.warned_full {
            self.warned_full = true;
            cx.fx.peek(hud(palette::GREEN, "Fully charged".to_string(), 2600));
        }
        if new.percent < 95 {
            self.warned_full = false;
        }
    }

    fn indicators(&self, out: &mut Vec<Indicator>) {
        if self.state.has_battery {
            out.push(Indicator {
                color: if self.state.saver { palette::YELLOW } else { Color::white(0.9) },
                icon: None,
                label: self.label.clone(),
                battery: Some((self.level(), self.state.plugged)),
                dot: false,
            });
        }
    }
}
