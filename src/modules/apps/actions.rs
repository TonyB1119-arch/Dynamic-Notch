//! Quick actions: lock, sleep, screenshot, show desktop, empty the recycle
//! bin, switch dark/light mode, open Downloads, Do Not Disturb, and
//! restart / shut down / sign out (those ask for a second tap).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{run_hidden, spawn_hidden};
use crate::gfx::{Icon, TextStyle};
use crate::modules::{Cx, Indicator, Module, ModuleId, DND};
use crate::sys::shell_open;
use crate::ui::Ui;
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "actions";
const COLS: usize = 4;
const CELL_H: f32 = 62.0;
const GAP: f32 = 8.0;
/// How long a risky action stays armed after the first tap.
const ARM: Duration = Duration::from_secs(3);
/// How long the check mark stays on a tile after it ran.
const FLASH: Duration = Duration::from_millis(1400);

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Lock,
    Sleep,
    Screenshot,
    Desktop,
    Recycle,
    Theme,
    Downloads,
    Dnd,
    Restart,
    Shutdown,
    SignOut,
}

const ACTS: [(Act, &str, Icon); 11] = [
    (Act::Lock, "Lock", Icon::Lock),
    (Act::Sleep, "Sleep", Icon::Pause),
    (Act::Screenshot, "Screenshot", Icon::Camera),
    (Act::Desktop, "Show desktop", Icon::Image),
    (Act::Recycle, "Empty bin", Icon::Trash),
    (Act::Theme, "Dark mode", Icon::Brightness),
    (Act::Downloads, "Downloads", Icon::Download),
    (Act::Dnd, "Do not disturb", Icon::Bell),
    (Act::Restart, "Restart", Icon::Refresh),
    (Act::Shutdown, "Shut down", Icon::Bolt),
    (Act::SignOut, "Sign out", Icon::Unlock),
];

pub struct Actions {
    armed: Option<(Act, Instant)>,
    flash: Option<(Act, Instant)>,
    /// Whether apps currently use the dark theme (read when the page first opens).
    dark: Option<bool>,
}

impl Actions {
    pub fn new() -> Self {
        Self { armed: None, flash: None, dark: None }
    }

    fn read_dark() -> bool {
        let out = run_hidden(
            "reg",
            &["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"],
        )
        .unwrap_or_default();
        // "AppsUseLightTheme    REG_DWORD    0x0" means dark
        out.split_whitespace().last().map_or(true, |v| v == "0x0")
    }

    fn set_dark(dark: bool) {
        let v = if dark { "0" } else { "1" };
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
        for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
            spawn_hidden("reg", &["add", key, "/v", name, "/t", "REG_DWORD", "/d", v, "/f"]);
        }
    }

    /// Runs an action. Returns true when the notch should close afterwards.
    fn run(&mut self, act: Act) -> bool {
        match act {
            Act::Lock => {
                spawn_hidden("rundll32.exe", &["user32.dll,LockWorkStation"]);
                true
            }
            Act::Sleep => {
                spawn_hidden("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]);
                true
            }
            Act::Screenshot => {
                shell_open("ms-screenclip:");
                true
            }
            Act::Desktop => {
                shell_open("shell:::{3080F90D-D7AD-11D9-BD98-0000947B0257}");
                true
            }
            Act::Recycle => {
                spawn_hidden(
                    "powershell.exe",
                    &["-NoProfile", "-Command", "Clear-RecycleBin -Force -ErrorAction SilentlyContinue"],
                );
                false
            }
            Act::Theme => {
                let dark = !self.dark.unwrap_or_else(Self::read_dark);
                Self::set_dark(dark);
                self.dark = Some(dark);
                false
            }
            Act::Downloads => {
                shell_open("shell:Downloads");
                true
            }
            Act::Dnd => {
                DND.fetch_xor(true, Ordering::Relaxed);
                false
            }
            Act::Restart => {
                spawn_hidden("shutdown.exe", &["/r", "/t", "0"]);
                false
            }
            Act::Shutdown => {
                spawn_hidden("shutdown.exe", &["/s", "/t", "0"]);
                false
            }
            Act::SignOut => {
                spawn_hidden("shutdown.exe", &["/l"]);
                false
            }
        }
    }
}

impl Module for Actions {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Quick Actions"
    }
    fn icon(&self) -> Icon {
        Icon::Bolt
    }

    fn glance(&self) -> Option<(Icon, String)> {
        Some((Icon::Bolt, if DND.load(Ordering::Relaxed) { "DND on".into() } else { "11 actions".into() }))
    }

    fn indicators(&self, out: &mut Vec<Indicator>) {
        if DND.load(Ordering::Relaxed) {
            out.push(Indicator {
                color: palette::ORANGE,
                icon: Some(Icon::Bell),
                label: Some("Do not disturb".into()),
                battery: None,
                dot: false,
            });
        }
    }

    fn on_tick(&mut self, _cx: &mut Cx) {}

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        3.0 * (CELL_H + GAP) - GAP + 4.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let dark = *self.dark.get_or_insert_with(Self::read_dark);
        let dnd = DND.load(Ordering::Relaxed);
        if self.armed.is_some_and(|(_, t)| ui.now.saturating_duration_since(t) > ARM) {
            self.armed = None;
        }
        if self.flash.is_some_and(|(_, t)| ui.now.saturating_duration_since(t) > FLASH) {
            self.flash = None;
        }
        if self.armed.is_some() || self.flash.is_some() {
            ui.animate();
        }

        let cw = (r.w - GAP * (COLS as f32 - 1.0)) / COLS as f32;
        let mut clicked: Option<Act> = None;
        for (i, (act, label, icon)) in ACTS.iter().enumerate() {
            let (c, row) = (i % COLS, i / COLS);
            let cell = Rect::new(r.x + c as f32 * (cw + GAP), r.y + row as f32 * (CELL_H + GAP), cw, CELL_H);
            let hover = ui.hovered(cell);
            let armed = self.armed.filter(|(a, _)| a == act);
            let flashed = self.flash.is_some_and(|(a, _)| a == *act);
            let on = *act == Act::Dnd && dnd;
            let risky = matches!(act, Act::Restart | Act::Shutdown | Act::SignOut);

            let bg = if on {
                Color::white(0.94)
            } else if armed.is_some() {
                palette::RED.with_a(0.22)
            } else if hover {
                Color::white(0.09)
            } else {
                Color::white(0.05)
            };
            ui.p.fill_rounded(cell, 16.0, bg);
            if hover {
                ui.hot = true;
            }
            let fg = if on {
                palette::INK
            } else if armed.is_some() {
                palette::RED
            } else {
                palette::TEXT
            };
            let ico = if flashed { Icon::Check } else { *icon };
            ui.p.icon(
                ico,
                Rect::new(cell.cx() - 10.0, cell.y + 9.0, 20.0, 20.0),
                if flashed { palette::GREEN } else { fg },
            );
            let text = match (*act, armed.is_some()) {
                (_, true) => "Tap again".to_string(),
                (Act::Theme, _) => (if dark { "Light mode" } else { "Dark mode" }).to_string(),
                _ => label.to_string(),
            };
            ui.p.text(
                &text,
                Rect::new(cell.x + 4.0, cell.y + 34.0, cell.w - 8.0, 16.0),
                &TextStyle::new(11.5, if on { palette::INK } else { fg }).medium().center(),
            );
            if let Some((_, t)) = armed {
                // the ticks drain while the second tap is still possible
                let left = 1.0 - ui.now.saturating_duration_since(t).as_secs_f32() / ARM.as_secs_f32();
                ui.ticks(
                    Rect::new(cell.x + 12.0, cell.bottom() - 9.0, cell.w - 24.0, 5.0),
                    left.clamp(0.0, 1.0),
                    palette::RED,
                );
            }
            if ui.clicked(cell) {
                clicked = Some(*act);
                let _ = risky;
            }
        }

        if let Some(act) = clicked {
            let risky = matches!(act, Act::Restart | Act::Shutdown | Act::SignOut);
            if risky && self.armed.map(|(a, _)| a) != Some(act) {
                self.armed = Some((act, ui.now));
            } else {
                self.armed = None;
                self.flash = Some((act, ui.now));
                if self.run(act) {
                    ui.fx.collapse = true;
                }
            }
            ui.fx.redraw = true;
        }
    }
}
