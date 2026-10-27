//! Focus timer (Pomodoro): work sessions and breaks with a ring of ticks.
//!
//! While it runs, the closed notch shows a small ring and the minutes left,
//! and a peek tells you when a session or break ends. Nothing runs while it
//! is stopped. Lengths come from `[tools]` in config.toml.

use std::time::{Duration, Instant};

use crate::gfx::{Icon, TextStyle};
use crate::modules::{Activity, Cx, Module, ModuleId, Peek, Slot};
use crate::sys::play_sound;
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "focus";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Work,
    Short,
    Long,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Work => "Focus",
            Phase::Short => "Short break",
            Phase::Long => "Long break",
        }
    }
}

pub struct Focus {
    phase: Phase,
    running: bool,
    /// When the running phase ends.
    ends_at: Option<Instant>,
    /// Time left in the phase while paused or not started.
    left: Duration,
    total: Duration,
    /// Work sessions finished (this run of the app).
    done: u32,
    refresh_at: Option<Instant>,
    /// Lengths in minutes (work, short, long), copied from the config.
    lens: [u32; 3],
}

impl Focus {
    pub fn new() -> Self {
        let work = Duration::from_secs(25 * 60);
        Self {
            phase: Phase::Work,
            running: false,
            ends_at: None,
            left: work,
            total: work,
            done: 0,
            refresh_at: None,
            lens: [25, 5, 15],
        }
    }

    fn len_of(&self, p: Phase) -> Duration {
        let mins = match p {
            Phase::Work => self.lens[0],
            Phase::Short => self.lens[1],
            Phase::Long => self.lens[2],
        };
        Duration::from_secs(mins.max(1) as u64 * 60)
    }

    fn set_phase(&mut self, p: Phase, run: bool, now: Instant) {
        self.phase = p;
        self.total = self.len_of(p);
        self.left = self.total;
        self.running = run;
        self.ends_at = run.then(|| now + self.total);
    }

    fn remaining(&self, now: Instant) -> Duration {
        match self.ends_at {
            Some(e) if self.running => e.saturating_duration_since(now),
            _ => self.left,
        }
    }

    fn next_after(&self) -> Phase {
        match self.phase {
            Phase::Work => {
                if (self.done + 1) % 4 == 0 {
                    Phase::Long
                } else {
                    Phase::Short
                }
            }
            _ => Phase::Work,
        }
    }

    fn toggle(&mut self, now: Instant) {
        if self.running {
            self.left = self.remaining(now);
            self.running = false;
            self.ends_at = None;
        } else {
            self.running = true;
            self.ends_at = Some(now + self.left);
        }
    }

    fn skip(&mut self, now: Instant) {
        // a skipped session does not count as done
        let next = self.next_after();
        let run = self.running;
        self.set_phase(next, run, now);
    }

    fn reset(&mut self, now: Instant) {
        self.set_phase(Phase::Work, false, now);
        self.done = 0;
    }
}

fn mmss(d: Duration) -> String {
    let s = d.as_secs() + u64::from(d.subsec_millis() > 0);
    format!("{:02}:{:02}", s / 60, s % 60)
}

impl Module for Focus {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Focus"
    }
    fn icon(&self) -> Icon {
        Icon::Timer
    }

    fn start(&mut self, cx: &mut Cx) {
        self.sync_cfg(cx.cfg, cx.now);
    }

    fn on_system(&mut self, ev: &crate::modules::SystemEvent, cx: &mut Cx) {
        if matches!(ev, crate::modules::SystemEvent::ConfigReloaded) {
            self.sync_cfg(cx.cfg, cx.now);
        }
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let text = if self.running {
            mmss(self.remaining(Instant::now()))
        } else if self.done > 0 {
            format!("{} done", self.done)
        } else {
            "Ready".to_string()
        };
        Some((Icon::Timer, text))
    }

    fn next_tick(&self) -> Option<Instant> {
        match (self.ends_at.filter(|_| self.running), self.refresh_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn on_tick(&mut self, cx: &mut Cx) {
        self.refresh_at = None;
        if let Some(e) = self.ends_at.filter(|_| self.running) {
            if cx.now >= e {
                let was = self.phase;
                if was == Phase::Work {
                    self.done += 1;
                }
                let next = if was == Phase::Work {
                    if self.done % 4 == 0 {
                        Phase::Long
                    } else {
                        Phase::Short
                    }
                } else {
                    Phase::Work
                };
                self.set_phase(next, true, cx.now);
                let (title, sub) = match was {
                    Phase::Work => ("Focus session complete", format!("Take a {}", next.label().to_lowercase())),
                    _ => ("Break is over", "Ready to focus?".to_string()),
                };
                cx.fx.peek(
                    Peek::new(Icon::Timer, palette::BLUE, title, sub).key("focus").duration_ms(5000).page("tools"),
                );
                play_sound("SystemAsterisk");
            }
        }
        // keep the minutes in the closed notch fresh while it runs
        if self.running {
            self.refresh_at = Some(cx.now + Duration::from_secs(60));
        }
        cx.fx.redraw = true;
    }

    fn activity(&self) -> Option<Activity> {
        if !self.running {
            return None;
        }
        let left = self.remaining(Instant::now());
        let frac = 1.0 - left.as_secs_f32() / self.total.as_secs_f32().max(1.0);
        Some(Activity {
            priority: 38,
            left: Slot::Ring(Some(frac.clamp(0.0, 1.0)), palette::BLUE),
            right: Slot::Text(format!("{}m", left.as_secs().div_ceil(60)), palette::TEXT),
            wing: 1.5,
            page: Some("tools"),
        })
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        196.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let now = ui.now;
        if self.running {
            // once a second while you are looking at it
            if self.refresh_at.is_none_or(|t| t > now + Duration::from_secs(1)) {
                self.refresh_at = Some(now + Duration::from_secs(1));
            }
        }
        let left = self.remaining(now);
        let frac = 1.0 - left.as_secs_f32() / self.total.as_secs_f32().max(1.0);

        // --- the ring -------------------------------------------------------
        let d = 150.0_f32.min(r.h - 8.0);
        let (cx, cy) = (r.x + 8.0 + d * 0.5, r.cy());
        ui.ring(cx, cy, d, 7.0, frac.clamp(0.0, 1.0), palette::BLUE);
        ui.p.text(
            &mmss(left),
            Rect::new(cx - d * 0.5, cy - 22.0, d, 32.0),
            &TextStyle::new(30.0, palette::TEXT).semibold().display().center(),
        );
        ui.p.text(
            self.phase.label(),
            Rect::new(cx - d * 0.5, cy + 12.0, d, 18.0),
            &TextStyle::new(12.5, palette::TEXT_DIM).center(),
        );

        // --- controls -------------------------------------------------------
        let x = r.x + d + 40.0;
        let w = (r.right() - x).max(120.0);
        ui.caption("FOCUS TIMER", Rect::new(x, r.y + 8.0, w, 16.0));
        ui.p.text(
            &format!("{} session{} done", self.done, if self.done == 1 { "" } else { "s" }),
            Rect::new(x, r.y + 26.0, w, 20.0),
            &TextStyle::new(15.0, palette::TEXT).semibold(),
        );
        let main = Rect::new(x, r.y + 60.0, 132.0, 40.0);
        let label = if self.running { "Pause" } else { "Start" };
        let icon = if self.running { Icon::Pause } else { Icon::Play };
        if ui.pill_button(main, label, Some(icon), ButtonStyle::inverted()) {
            self.toggle(now);
            ui.fx.redraw = true;
        }
        let skip = Rect::new(main.right() + 10.0, main.y + 3.0, 34.0, 34.0);
        if ui.icon_button(skip, Icon::Next, ButtonStyle::filled(Color::white(0.08)).scale(0.5)) {
            self.skip(now);
            ui.fx.redraw = true;
        }
        let reset = Rect::new(skip.right() + 8.0, main.y + 3.0, 34.0, 34.0);
        if ui.icon_button(reset, Icon::Refresh, ButtonStyle::filled(Color::white(0.08)).scale(0.5)) {
            self.reset(now);
            ui.fx.redraw = true;
        }

        // progress towards the long break
        let cycle = (self.done % 4) as f32 / 4.0;
        ui.p.text(
            "Until the long break",
            Rect::new(x, r.y + 120.0, w, 16.0),
            &TextStyle::new(11.5, palette::TEXT_FAINT),
        );
        ui.ticks(Rect::new(x, r.y + 142.0, w.min(260.0), 12.0), cycle, Color::white(0.9));
    }
}

impl Focus {
    fn sync_cfg(&mut self, cfg: &crate::config::Config, now: Instant) {
        let t = &cfg.tools;
        let lens = [t.focus_work, t.focus_break, t.focus_long_break];
        if lens != self.lens {
            self.lens = lens;
            if !self.running {
                let p = self.phase;
                self.set_phase(p, false, now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_text() {
        assert_eq!(mmss(Duration::from_secs(1500)), "25:00");
        assert_eq!(mmss(Duration::from_millis(59_100)), "01:00");
        assert_eq!(mmss(Duration::from_secs(0)), "00:00");
    }

    #[test]
    fn long_break_every_fourth() {
        let mut f = Focus::new();
        f.done = 3;
        assert_eq!(f.next_after(), Phase::Long);
        f.done = 1;
        assert_eq!(f.next_after(), Phase::Short);
        f.phase = Phase::Short;
        assert_eq!(f.next_after(), Phase::Work);
    }
}
