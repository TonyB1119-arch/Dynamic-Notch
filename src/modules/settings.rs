//! Settings tab: the options people change most, without opening config.toml.
//!
//! Every control writes through `Effects::save_config`, which edits one value
//! in config.toml (keeping comments) and lets the normal hot reload apply it.
//! The Gemini key goes to `%APPDATA%\DynamicNotch\.env`, never to the config.

use std::any::Any;
use std::time::{Duration, Instant};

use super::gemini;
use super::widgets::{chip, segmented, slider, switch, toml_str, TextField};
use super::{Cx, Key, Module, ModuleId, SystemEvent};
use crate::config::{self, Config};
use crate::gfx::{Icon, TextStyle};
use crate::sys::{dotenv, shell_open};
use crate::ui::{ButtonStyle, Ui};
use crate::updater;
use crate::util::{app_dir, palette, Color, Rect, SendHwnd};

const ID: ModuleId = "settings";
const TABS: [&str; 9] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock"];
const ROW_H: f32 = 36.0;
const ROW_GAP: f32 = 4.0;
const TAB_H: f32 = 32.0;
const FOOT_H: f32 = 22.0;
const MAX_HOME: usize = 4;
const MAX_NOTCH: usize = 3;

/// Widgets that can sit on the closed notch: (config id, label).
const NOTCH_CHOICES: [(&str, &str); 7] = [
    ("clock", "Clock"),
    ("date", "Date"),
    ("battery", "Battery"),
    ("weather", "Weather"),
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("notes", "Notes"),
];

/// Widgets that can be placed on the home page: (config id, label).
const HOME_CHOICES: [(&str, &str); 9] = [
    ("weather", "Weather"),
    ("stats", "System stats"),
    ("timer", "Timer"),
    ("control", "Wi-Fi & Bluetooth"),
    ("notes", "Notes"),
    ("clipboard", "Clipboard"),
    ("downloads", "Downloads"),
    ("shelf", "Shelf"),
    ("ports", "Dev servers"),
];

/// Modules that can be switched on and off: (config id, label).
const MODULE_CHOICES: [(&str, &str); 21] = [
    ("media", "Media"),
    ("timer", "Timer"),
    ("downloads", "Downloads"),
    ("clipboard", "Clipboard"),
    ("shelf", "Shelf"),
    ("voice", "Dictation"),
    ("ai", "Ask AI"),
    ("gemini", "Gemini"),
    ("weather", "Weather"),
    ("control", "Wi-Fi & Bluetooth"),
    ("notifications", "Notifications"),
    ("mirror", "Mirror (camera tab)"),
    ("notes", "Notes"),
    ("greeting", "Unlock greeting"),
    ("stats", "System stats"),
    ("ports", "Dev servers"),
    ("privacy", "Mic & camera"),
    ("battery", "Battery"),
    ("locks", "Lock keys"),
    ("osd", "Volume & brightness"),
    ("devices", "USB drives"),
];

/// Tabs that can be reordered: (config id, label), in the default order.
const TAB_CHOICES: [(&str, &str); 14] = [
    ("timer", "Timer"),
    ("stats", "System Stats"),
    ("voice", "Dictation"),
    ("clipboard", "Clipboard"),
    ("downloads", "Downloads"),
    ("shelf", "Shelf"),
    ("ports", "Dev Servers"),
    ("ai", "Ask AI"),
    ("gemini", "Gemini"),
    ("weather", "Weather"),
    ("control", "Wi-Fi & BT"),
    ("notes", "Notes"),
    ("notifications", "Notifications"),
    ("mirror", "Mirror"),
];

const SWATCHES: [u32; 6] = [0x0A84FF, 0xBF5AF2, 0xFF375F, 0xFF9F0A, 0x34C759, 0x64D2FF];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    None,
    City,
    Name,
    Key,
    Model,
}

/// Where the "Check for updates" button stands.
enum Update {
    Idle,
    Checking,
    UpToDate,
    Available { tag: String, installer: Option<String>, page: String, notes: String },
    Installing,
    Failed(String),
}

pub struct Settings {
    tab: usize,
    update: Update,
    cfg: Config,
    city: TextField,
    name: TextField,
    key: TextField,
    model: TextField,
    focus: Focus,
    /// How far the current tab is scrolled (a whole number of rows) when it is taller than the page.
    scroll: f32,
    has_key: bool,
    status: Option<(String, Instant, bool)>,
    hwnd: SendHwnd,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            tab: 0,
            update: Update::Idle,
            cfg: Config::default(),
            city: TextField::new(80),
            name: TextField::new(24),
            key: TextField::new(200).masked(),
            model: TextField::new(80),
            focus: Focus::None,
            scroll: 0.0,
            has_key: false,
            status: None,
            hwnd: SendHwnd(0),
        }
    }

    fn say(&mut self, text: &str, good: bool) {
        self.status = Some((text.to_string(), Instant::now(), good));
    }

    fn sync_fields(&mut self) {
        if self.focus != Focus::City {
            self.city.set(&self.cfg.weather.city);
        }
        if self.focus != Focus::Name {
            self.name.set(&self.cfg.greeting.name);
        }
        if self.focus != Focus::Model {
            self.model.set(&self.cfg.gemini.model);
        }
    }

    fn save_key(&mut self, bus: &crate::bus::Bus) {
        let key: String = self.key.text().chars().filter(|c| !c.is_whitespace()).collect();
        if key.len() < 20 {
            self.say("That doesn't look like a Gemini API key", false);
        } else if dotenv::set_saved(gemini::KEY_VAR, &key) {
            self.key.clear();
            self.has_key = true;
            bus.to_module("gemini", gemini::KeyChanged);
            self.say("Key saved", true);
        } else {
            self.say("Couldn't save the key", false);
        }
    }
}

fn put(ui: &mut Ui, section: &str, key: &str, value: String) {
    ui.fx.save_config.push((section.to_string(), key.to_string(), value));
}

fn list_value(ids: &[String]) -> String {
    format!("[{}]", ids.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", "))
}

/// "Order" grid for a list of items: each one is a capsule with ‹ › arrows to
/// move it earlier or later. `row` is the first row (its height is the row
/// height). Returns the new order when something moved.
fn order_row(ui: &mut Ui, row: Rect, ids: &[String], names: &[(&str, &str)], cols: usize) -> Option<Vec<String>> {
    if ids.len() < 2 {
        return None;
    }
    let gap = 8.0;
    let cols = cols.max(1);
    let w = (row.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let mut result = None;
    for (i, id) in ids.iter().enumerate() {
        let r = Rect::new(row.x + (i % cols) as f32 * (w + gap), row.y + (i / cols) as f32 * (row.h + gap), w, row.h);
        ui.p.fill_rounded(r, r.h * 0.5, Color::white(0.07));
        let label = names.iter().find(|(k, _)| *k == id.as_str()).map(|(_, n)| *n).unwrap_or(id.as_str());
        ui.p.text(
            label,
            Rect::new(r.x + 28.0, r.y, r.w - 56.0, r.h),
            &TextStyle::new(12.0, palette::TEXT).medium().center(),
        );
        for (dir, can) in [(-1i32, i > 0), (1i32, i + 1 < ids.len())] {
            let a = if dir < 0 {
                Rect::new(r.x + 3.0, r.y + 3.0, 24.0, r.h - 6.0)
            } else {
                Rect::new(r.right() - 27.0, r.y + 3.0, 24.0, r.h - 6.0)
            };
            let hover = can && ui.hovered(a);
            if hover {
                ui.hot = true;
                ui.p.fill_rounded(a, a.h * 0.5, Color::white(0.14));
            }
            let col = if can { palette::TEXT } else { palette::TEXT_FAINT.with_a(0.4) };
            ui.p.text(if dir < 0 { "‹" } else { "›" }, a, &TextStyle::new(17.0, col).medium().center());
            if can && ui.clicked(a) {
                let mut next = ids.to_vec();
                let j = (i as i32 + dir) as usize;
                next.swap(i, j);
                result = Some(next);
            }
        }
    }
    result
}

/// The tab order to show: the saved order (known ids only), then anything missing.
fn tab_order(saved: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in saved {
        if TAB_CHOICES.iter().any(|(k, _)| *k == id.as_str()) && !out.contains(id) {
            out.push(id.clone());
        }
    }
    for (k, _) in TAB_CHOICES {
        if !out.iter().any(|o| o == k) {
            out.push(k.to_string());
        }
    }
    out
}

/// How many rows each tab holds (the page grows to fit, then scrolls once the window is full).
fn tab_rows(tab: usize) -> f32 {
    match tab {
        0 => 14.0,
        1 => 14.0,
        2 => 5.6,
        3 => 5.5,
        4 => 4.0,
        5 => 4.0,
        6 => 5.6,
        7 => 5.9,
        _ => 8.6,
    }
}

fn row_rect(body: Rect, i: usize) -> Rect {
    Rect::new(body.x, body.y + i as f32 * (ROW_H + ROW_GAP), body.w, ROW_H)
}

fn label(ui: &mut Ui, row: Rect, text: &str) {
    ui.row_bg(row, false, None);
    ui.p.text(text, Rect::new(row.x + 12.0, row.y, row.w * 0.5, row.h), &TextStyle::new(13.0, palette::TEXT));
}

fn toggle_row(ui: &mut Ui, row: Rect, text: &str, on: bool, section: &str, key: &str) {
    label(ui, row, text);
    let sw = Rect::new(row.right() - 56.0, row.y, 48.0, row.h);
    let accent = ui.accent;
    if switch(ui, sw, on, accent) {
        put(ui, section, key, (!on).to_string());
    }
}

#[allow(clippy::too_many_arguments)]
fn slider_row(
    ui: &mut Ui,
    row: Rect,
    text: &str,
    val: f32,
    (lo, hi): (f32, f32),
    shown: String,
    section: &str,
    key: &str,
) {
    label(ui, row, text);
    ui.p.text(
        &shown,
        Rect::new(row.right() - 54.0, row.y, 46.0, row.h),
        &TextStyle::new(12.0, palette::TEXT_DIM).right(),
    );
    let sl = Rect::new(row.right() - 54.0 - 176.0, row.cy() - 12.0, 164.0, 24.0);
    let frac = ((val - lo) / (hi - lo)).clamp(0.0, 1.0);
    let accent = ui.accent;
    if let Some(f) = slider(ui, sl, frac, accent) {
        put(ui, section, key, format!("{:.2}", lo + f * (hi - lo)));
    }
}

impl Module for Settings {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Settings"
    }
    fn icon(&self) -> Icon {
        Icon::Settings
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
        self.cfg = cx.cfg.clone();
        self.has_key = dotenv::get_saved_first(gemini::KEY_VAR).is_some();
        self.sync_fields();
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        match ev {
            SystemEvent::ConfigReloaded => {
                self.cfg = cx.cfg.clone();
                self.sync_fields();
                cx.fx.redraw = true;
            }
            SystemEvent::Expanded(false) => self.focus = Focus::None,
            _ => {}
        }
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let Ok(m) = msg.downcast::<updater::Msg>() else { return };
        self.update = match *m {
            updater::Msg::UpToDate => Update::UpToDate,
            updater::Msg::Available { tag, installer, page, notes } => Update::Available { tag, installer, page, notes },
            updater::Msg::Installing => Update::Installing,
            updater::Msg::Failed(why) => Update::Failed(why),
        };
        cx.fx.redraw = true;
    }

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        self.focus != Focus::None
    }

    fn page_height(&self, _w: f32) -> f32 {
        TAB_H + 10.0 + tab_rows(self.tab) * (ROW_H + ROW_GAP) + FOOT_H
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        let enter = match self.focus {
            Focus::City => self.city.key(key, self.hwnd),
            Focus::Name => self.name.key(key, self.hwnd),
            Focus::Key => self.key.key(key, self.hwnd),
            Focus::Model => self.model.key(key, self.hwnd),
            Focus::None => false,
        };
        if enter {
            match self.focus {
                Focus::City => {
                    let v = self.city.text().trim().to_string();
                    cx.fx.save_config.push(("weather".into(), "city".into(), toml_str(&v)));
                    self.say("City saved", true);
                }
                Focus::Name => {
                    let v = self.name.text().trim().to_string();
                    cx.fx.save_config.push(("greeting".into(), "name".into(), toml_str(&v)));
                    self.say("Name saved", true);
                }
                Focus::Key => self.save_key(cx.bus),
                Focus::Model => {
                    let v = self.model.text().trim().to_string();
                    if !v.is_empty() {
                        cx.fx.save_config.push(("gemini".into(), "model".into(), toml_str(&v)));
                        self.say("Model saved", true);
                    }
                }
                Focus::None => {}
            }
            cx.fx.keyboard_focus = Some(false);
            self.focus = Focus::None;
        }
        cx.fx.layout_changed = true;
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let _accent = ui.accent;

        // --- sections ---------------------------------------------------------
        let gap = 6.0;
        let tw = (r.w - gap * (TABS.len() as f32 - 1.0)) / TABS.len() as f32;
        for (i, name) in TABS.iter().enumerate() {
            let b = Rect::new(r.x + i as f32 * (tw + gap), r.y, tw, TAB_H);
            let on = i == self.tab;
            let style = if on {
                ButtonStyle::inverted()
            } else {
                ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM)
            };
            let hit = if crate::ui::look_alcove() {
                crate::alcove::chip(ui, b, name, None, on)
            } else {
                ui.pill_button(b, name, None, style)
            };
            if hit && !on {
                self.tab = i;
                self.scroll = 0.0;
                self.focus = Focus::None;
                ui.fx.keyboard_focus = Some(false);
                ui.fx.layout_changed = true;
            }
        }
        let body = Rect::new(r.x, r.y + TAB_H + 10.0, r.w, r.h - TAB_H - 10.0 - FOOT_H);

        // a tab taller than the page scrolls with the wheel, a row at a time
        let content_h = tab_rows(self.tab) * (ROW_H + ROW_GAP);
        let long = content_h > body.h + 0.5;
        let (page, was_live) = (body, ui.interactive);
        let body = if long {
            let shown =
                crate::alcove::scroll_rows(ui, page, &mut self.scroll, content_h, ROW_H + ROW_GAP, "settings-scroll");
            ui.p.push_clip(page);
            // rows scrolled out of view must not answer the pointer
            ui.interactive = was_live && ui.input.mouse.is_some_and(|(x, y)| page.contains(x, y));
            Rect::new(page.x, page.y - shown, page.w, content_h)
        } else {
            body
        };
        match self.tab {
            0 => self.general(ui, body),
            1 => self.look(ui, body),
            2 => self.home(ui, body),
            3 => self.modules(ui, body),
            4 => self.weather(ui, body),
            5 => self.gemini(ui, body),
            6 => self.notch(ui, body),
            7 => self.tabs(ui, body),
            _ => self.dock(ui, body),
        }
        if long {
            ui.p.pop_clip();
            ui.interactive = was_live;
        }

        // --- status line ------------------------------------------------------
        if let Some((text, at, good)) = self.status.clone() {
            if at.elapsed() < Duration::from_secs(4) {
                let col = if good { palette::GREEN } else { palette::ORANGE };
                ui.p.text(&text, Rect::new(r.x, r.bottom() - FOOT_H, r.w, FOOT_H), &TextStyle::new(11.5, col).right());
            } else {
                self.status = None;
            }
        } else {
            ui.p.text(
                "Build v3.0.1 · Changes apply right away",
                Rect::new(r.x, r.bottom() - FOOT_H, r.w, FOOT_H),
                &TextStyle::new(11.0, palette::TEXT_FAINT).right(),
            );
        }
    }
}

impl Settings {
    fn general(&mut self, ui: &mut Ui, body: Rect) {
        let g = self.cfg.general.clone();
        let accent = ui.accent;
        toggle_row(
            ui,
            row_rect(body, 0),
            "Open when I hover the notch",
            g.expand_on_hover,
            "general",
            "expand_on_hover",
        );
        toggle_row(
            ui,
            row_rect(body, 1),
            "Hide for fullscreen apps",
            g.hide_in_fullscreen,
            "general",
            "hide_in_fullscreen",
        );
        toggle_row(ui, row_rect(body, 10), "Show bubbles beside the notch", g.bubbles, "general", "bubbles");
        toggle_row(ui, row_rect(body, 11), "Quiet sound with bubbles", g.bubble_sounds, "general", "bubble_sounds");
        toggle_row(ui, row_rect(body, 12), "Round dial for volume and brightness", g.hud_ring, "general", "hud_ring");
        toggle_row(ui, row_rect(body, 13), "Round dials fill clockwise", g.hud_clockwise, "general", "hud_clockwise");
        toggle_row(ui, row_rect(body, 2), "Start with Windows", g.start_with_windows, "general", "start_with_windows");

        let row = row_rect(body, 3);
        label(ui, row, "When idle");
        let sel = ["notch", "pill", "hidden"].iter().position(|s| *s == g.idle_style);
        let seg = Rect::new(row.right() - 232.0, row.y + 3.0, 224.0, row.h - 6.0);
        if let Some(i) = segmented(ui, seg, &["Notch", "Pill", "Hidden"], sel, accent) {
            put(ui, "general", "idle_style", toml_str(["notch", "pill", "hidden"][i]));
        }

        let row = row_rect(body, 4);
        label(ui, row, "Show on");
        let sel = match g.monitor.as_str() {
            "primary" => Some(0),
            "active" => Some(1),
            _ => None,
        };
        let seg = Rect::new(row.right() - 232.0, row.y + 3.0, 224.0, row.h - 6.0);
        if let Some(i) = segmented(ui, seg, &["Main screen", "Active window"], sel, accent) {
            put(ui, "general", "monitor", toml_str(["primary", "active"][i]));
        }

        toggle_row(ui, row_rect(body, 5), "Greet me when I unlock", self.cfg.modules.greeting, "modules", "greeting");

        // the name the unlock animation says hello to
        let row = row_rect(body, 6);
        label(ui, row, "Say hello to");
        let save = Rect::new(row.right() - 70.0, row.y + 2.0, 64.0, row.h - 4.0);
        let field = Rect::new(save.x - 8.0 - 190.0, row.y + 2.0, 190.0, row.h - 4.0);
        if self.name.draw(ui, field, "Your name", accent, self.focus == Focus::Name) {
            self.focus = Focus::Name;
            ui.fx.keyboard_focus = Some(true);
        }
        if ui.pill_button(save, "Save", None, ButtonStyle::inverted()) {
            let v = self.name.text().trim().to_string();
            put(ui, "greeting", "name", toml_str(&v));
            self.say("Name saved", true);
            self.focus = Focus::None;
            ui.fx.keyboard_focus = Some(false);
        }

        // the handwriting the unlock hello is written in
        let row = row_rect(body, 7);
        label(ui, row, "Hello font");
        let cur = self.cfg.greeting.font_name();
        let sel = if cur == "Yellowtail" {
            Some(0)
        } else if cur == "Script Regular" {
            Some(1)
        } else {
            None
        };
        let seg = Rect::new(row.right() - 232.0, row.y + 3.0, 224.0, row.h - 6.0);
        if let Some(i) = segmented(ui, seg, &["Yellowtail", "Script Regular"], sel, accent) {
            put(ui, "greeting", "font", toml_str(["Yellowtail", "Script Regular"][i]));
        }

        let row = row_rect(body, 8);
        let w = (row.w - 16.0) / 3.0;
        let style = ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM);
        let b = |k: f32| Rect::new(row.x + k * (w + 8.0), row.y + 2.0, w, row.h - 4.0);
        if ui.pill_button(b(0.0), "Edit config file", Some(Icon::File), style) {
            let path = config::config_path();
            if !path.exists() {
                let _ = std::fs::write(&path, config::DEFAULT_TEMPLATE);
            }
            shell_open(&path.to_string_lossy());
        }
        if ui.pill_button(b(1.0), "Open data folder", Some(Icon::Folder), style) {
            shell_open(&app_dir().to_string_lossy());
        }
        if ui.pill_button(b(2.0), "Reset home", Some(Icon::Refresh), style) {
            let d = crate::config::HomeCfg::default();
            put(ui, "home", "widgets", list_value(&d.widgets));
        }

        // updates: only ever checked when this button is pressed
        let row = row_rect(body, 9);
        label(ui, row, "Updates");
        let btn = Rect::new(row.right() - 150.0, row.y + 2.0, 142.0, row.h - 4.0);
        let (note, action) = match &self.update {
            Update::Idle => (format!("Version {}", updater::VERSION), "Check for updates".to_string()),
            Update::Checking => ("Looking…".to_string(), String::new()),
            Update::UpToDate => ("You're up to date".to_string(), "Check again".to_string()),
            Update::Available { tag, installer, notes, .. } => {
                let verb = if installer.is_some() { "Install" } else { "Open page" };
                // the first real line of the release notes: what's new
                let line = notes
                    .lines()
                    .map(|l| l.trim().trim_start_matches(['#', '-', '*', ' ']).trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or("");
                let short: String = if line.chars().count() > 44 {
                    line.chars().take(43).collect::<String>() + "…"
                } else {
                    line.to_string()
                };
                let what = if short.is_empty() { format!("{tag} is available") } else { format!("{tag}: {short}") };
                (what, verb.to_string())
            }
            Update::Installing => ("Installing, the notch will restart".to_string(), String::new()),
            Update::Failed(why) => (why.clone(), "Try again".to_string()),
        };
        ui.p.text(
            &note,
            Rect::new(row.x + 90.0, row.y, (btn.x - row.x - 100.0).max(40.0), row.h),
            &TextStyle::new(11.5, palette::TEXT_DIM).right(),
        );
        if !action.is_empty() && ui.pill_button(btn, &action, None, ButtonStyle::inverted()) {
            match &self.update {
                Update::Available { installer: Some(url), .. } => {
                    updater::install(url, ui.bus);
                    self.update = Update::Installing;
                }
                Update::Available { page, .. } => {
                    shell_open(page);
                }
                _ => {
                    updater::check(&self.cfg.update.repo, ui.bus);
                    self.update = Update::Checking;
                }
            }
        }
    }

    fn look(&mut self, ui: &mut Ui, body: Rect) {
        let a = self.cfg.appearance.clone();
        let accent = ui.accent;
        toggle_row(ui, row_rect(body, 0), "Frosted glass", a.blur, "appearance", "blur");
        toggle_row(ui, row_rect(body, 1), "Shadow", a.shadow, "appearance", "shadow");
        slider_row(
            ui,
            row_rect(body, 2),
            "Panel opacity",
            a.expanded_opacity,
            (0.4, 1.0),
            format!("{:.0}%", a.expanded_opacity * 100.0),
            "appearance",
            "expanded_opacity",
        );
        slider_row(
            ui,
            row_rect(body, 3),
            "Panel width",
            a.expanded_width,
            (520.0, 960.0),
            format!("{:.0}", a.expanded_width),
            "appearance",
            "expanded_width",
        );
        slider_row(
            ui,
            row_rect(body, 4),
            "Size",
            a.scale,
            (0.8, 1.5),
            format!("{:.0}%", a.scale * 100.0),
            "appearance",
            "scale",
        );
        slider_row(
            ui,
            row_rect(body, 5),
            "Bounce",
            a.bounce,
            (0.0, 1.0),
            format!("{:.0}%", a.bounce * 100.0),
            "appearance",
            "bounce",
        );
        slider_row(
            ui,
            row_rect(body, 6),
            "Animation speed",
            a.animation_speed,
            (0.5, 2.0),
            format!("{:.1}×", a.animation_speed),
            "appearance",
            "animation_speed",
        );

        let row = row_rect(body, 7);
        label(ui, row, "Accent color");
        let auto_sel = a.accent.eq_ignore_ascii_case("auto");
        let mut x = row.right() - 12.0 - 6.0 * 26.0 - 64.0;
        let auto = Rect::new(x, row.y + 4.0, 56.0, row.h - 8.0);
        let style = if auto_sel {
            ButtonStyle::inverted()
        } else {
            ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)
        };
        if ui.pill_button(auto, "Auto", None, style) {
            put(ui, "appearance", "accent", toml_str("auto"));
        }
        x += 64.0;
        for hex in SWATCHES {
            let c = Color::hex(hex);
            let cr = Rect::new(x, row.cy() - 10.0, 20.0, 20.0);
            let value = format!("#{hex:06X}");
            let sel = a.accent.eq_ignore_ascii_case(&value);
            if ui.hovered(cr) {
                ui.hot = true;
            }
            ui.p.fill_circle(cr.cx(), cr.cy(), 8.0, c);
            if sel {
                ui.p.stroke_circle(cr.cx(), cr.cy(), 10.5, 1.8, Color::white(0.95));
            }
            if ui.clicked(cr) {
                put(ui, "appearance", "accent", toml_str(&value));
            }
            x += 26.0;
        }

        // overall look: beams everywhere, calm (beams only where a level matters) or minimal
        let row = row_rect(body, 8);
        label(ui, row, "Look");
        let sel = match a.look.trim().to_ascii_lowercase().as_str() {
            "calm" => 1,
            "minimal" => 2,
            "nothing" => 3,
            "bloom" => 4,
            "alcove" => 5,
            _ => 0,
        };
        let seg = Rect::new(row.right() - 520.0, row.y + 3.0, 512.0, row.h - 6.0);
        let labels = ["Beams", "Calm", "Minimal", "Nothing", "Bloom", "Alcove"];
        if let Some(i) = segmented(ui, seg, &labels, Some(sel), accent) {
            put(ui, "appearance", "look", toml_str(["beams", "calm", "minimal", "nothing", "bloom", "alcove"][i]));
        }
        toggle_row(ui, row_rect(body, 9), "White beams (a hint of blue)", a.white_beams, "appearance", "white_beams");

        let row = row_rect(body, 10);
        label(ui, row, "Home clock");
        let sel = usize::from(a.clock_style.eq_ignore_ascii_case("flip"));
        let seg = Rect::new(row.right() - 232.0, row.y + 3.0, 224.0, row.h - 6.0);
        if let Some(i) = segmented(ui, seg, &["Rolling digits", "Flip clock"], Some(sel), accent) {
            put(ui, "appearance", "clock_style", toml_str(["roll", "flip"][i]));
        }
        toggle_row(
            ui,
            row_rect(body, 11),
            "Light animations (cached frames)",
            a.motion_cache,
            "appearance",
            "motion_cache",
        );
        toggle_row(
            ui,
            row_rect(body, 12),
            "Soft blur at the bottom edge",
            a.bottom_fade,
            "appearance",
            "bottom_fade",
        );
        toggle_row(
            ui,
            row_rect(body, 13),
            "Glass edge (bends what is behind)",
            a.glass_look,
            "appearance",
            "glass_look",
        );
    }

    fn home(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let chosen = self.cfg.home.widgets.clone();
        ui.p.text(
            &format!("Pick up to {MAX_HOME} widgets for the home page. Use the arrows below to change their order."),
            Rect::new(body.x + 2.0, body.y, body.w, 18.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        let cols = 4;
        let gap = 8.0;
        let cw = (body.w - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = 36.0;
        let top = body.y + 26.0;
        for (i, (id, name)) in HOME_CHOICES.iter().enumerate() {
            let (cx, cy) = (i % cols, i / cols);
            let cr = Rect::new(body.x + cx as f32 * (cw + gap), top + cy as f32 * (ch + gap), cw, ch);
            let pos = chosen.iter().position(|w| w == *id);
            if chip(ui, cr, name, pos.is_some(), pos.map(|p| p + 1), accent) {
                let mut next = chosen.clone();
                match pos {
                    Some(p) => {
                        next.remove(p);
                    }
                    None if next.len() < MAX_HOME => next.push(id.to_string()),
                    None => {
                        self.status = Some((format!("Only {MAX_HOME} fit. Remove one first."), Instant::now(), false));
                        continue;
                    }
                }
                put(ui, "home", "widgets", list_value(&next));
            }
        }
        // reorder what is chosen
        let oy = top + 2.0 * (ch + gap) + 6.0;
        if chosen.len() > 1 {
            ui.p.text(
                "Order",
                Rect::new(body.x + 2.0, oy, 200.0, 16.0),
                &TextStyle::new(11.5, palette::TEXT_FAINT).semibold(),
            );
            if let Some(next) = order_row(ui, Rect::new(body.x, oy + 20.0, body.w, 32.0), &chosen, &HOME_CHOICES, 4) {
                put(ui, "home", "widgets", list_value(&next));
            }
        }
    }

    fn modules(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let cols = 4;
        let gap = 8.0;
        let cw = (body.w - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = 36.0;
        for (i, (id, name)) in MODULE_CHOICES.iter().enumerate() {
            let (cx, cy) = (i % cols, i / cols);
            let cr = Rect::new(body.x + cx as f32 * (cw + gap), body.y + cy as f32 * (ch + gap), cw, ch);
            let on = self.cfg.modules.is_enabled(id);
            if chip(ui, cr, name, on, None, accent) {
                put(ui, "modules", id, (!on).to_string());
            }
        }
    }

    fn weather(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let row = row_rect(body, 0);
        label(ui, row, "City");
        let save = Rect::new(row.right() - 80.0, row.y + 2.0, 74.0, row.h - 4.0);
        let field = Rect::new(save.x - 8.0 - 250.0, row.y + 2.0, 250.0, row.h - 4.0);
        if self.city.draw(ui, field, "e.g. Berlin or Springfield, IL", accent, self.focus == Focus::City) {
            self.focus = Focus::City;
            ui.fx.keyboard_focus = Some(true);
        }
        if ui.pill_button(save, "Save", None, ButtonStyle::inverted()) {
            let v = self.city.text().trim().to_string();
            put(ui, "weather", "city", toml_str(&v));
            self.say("City saved", true);
            self.focus = Focus::None;
            ui.fx.keyboard_focus = Some(false);
        }

        let row = row_rect(body, 1);
        label(ui, row, "Units");
        let sel = if self.cfg.weather.units.eq_ignore_ascii_case("imperial") { 1 } else { 0 };
        let seg = Rect::new(row.right() - 232.0, row.y + 3.0, 224.0, row.h - 6.0);
        if let Some(i) = segmented(ui, seg, &["Metric °C", "Imperial °F"], Some(sel), accent) {
            put(ui, "weather", "units", toml_str(["metric", "imperial"][i]));
        }

        let row = row_rect(body, 2);
        ui.p.text(
            "Forecasts come from open-meteo.com. No account or key needed.",
            Rect::new(row.x + 2.0, row.y, row.w, row.h),
            &TextStyle::new(11.5, palette::TEXT_FAINT),
        );
    }

    fn notch(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let chosen = self.cfg.notch.widgets.clone();
        ui.p.text(
            &format!(
                "Pick up to {MAX_NOTCH} widgets for the closed notch. Use the arrows below to change their order."
            ),
            Rect::new(body.x + 2.0, body.y, body.w, 18.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        let cols = 4;
        let gap = 8.0;
        let cw = (body.w - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = 36.0;
        let top = body.y + 26.0;
        for (i, (id, name)) in NOTCH_CHOICES.iter().enumerate() {
            let (cx, cy) = (i % cols, i / cols);
            let cr = Rect::new(body.x + cx as f32 * (cw + gap), top + cy as f32 * (ch + gap), cw, ch);
            let pos = chosen.iter().position(|w| w == *id);
            if chip(ui, cr, name, pos.is_some(), pos.map(|p| p + 1), accent) {
                let mut next = chosen.clone();
                match pos {
                    Some(p) => {
                        next.remove(p);
                    }
                    None if next.len() < MAX_NOTCH => next.push(id.to_string()),
                    None => {
                        self.status = Some((format!("Only {MAX_NOTCH} fit. Remove one first."), Instant::now(), false));
                        continue;
                    }
                }
                put(ui, "notch", "widgets", list_value(&next));
            }
        }
        // reorder what is chosen
        let oy = top + 2.0 * (ch + gap) + 6.0;
        if chosen.len() > 1 {
            ui.p.text(
                "Order",
                Rect::new(body.x + 2.0, oy, 200.0, 16.0),
                &TextStyle::new(11.5, palette::TEXT_FAINT).semibold(),
            );
            if let Some(next) = order_row(ui, Rect::new(body.x, oy + 20.0, body.w, 32.0), &chosen, &NOTCH_CHOICES, 4) {
                put(ui, "notch", "widgets", list_value(&next));
            }
        }
        ui.p.text(
            "They show while nothing is playing. With the pill style they appear when you hover it.",
            Rect::new(body.x + 2.0, oy + 58.0, body.w, 18.0),
            &TextStyle::new(11.5, palette::TEXT_FAINT),
        );
    }

    fn dock(&mut self, ui: &mut Ui, body: Rect) {
        let d = self.cfg.dock.clone();
        toggle_row(ui, row_rect(body, 0), "Show the dock", d.enabled, "dock", "enabled");
        toggle_row(ui, row_rect(body, 1), "Hide the Windows taskbar", d.hide_taskbar, "dock", "hide_taskbar");
        toggle_row(ui, row_rect(body, 2), "Auto-hide (show on hover)", d.auto_hide, "dock", "auto_hide");
        toggle_row(ui, row_rect(body, 3), "Icons grow near the pointer", d.magnify, "dock", "magnify");
        slider_row(
            ui,
            row_rect(body, 4),
            "Growth",
            d.magnify_amount,
            (0.1, 1.0),
            format!("{:.0}%", d.magnify_amount * 100.0),
            "dock",
            "magnify_amount",
        );
        toggle_row(ui, row_rect(body, 5), "Window previews on hover", d.previews, "dock", "previews");
        slider_row(ui, row_rect(body, 6), "Icon size", d.size, (32.0, 80.0), format!("{:.0}", d.size), "dock", "size");
        ui.p.text(
            "Right-click an icon to pin, unpin or close. Drag icons to reorder. Scroll over an app to step through its windows.",
            Rect::new(body.x + 2.0, row_rect(body, 7).y + 4.0, body.w - 4.0, 34.0),
            &TextStyle::new(11.5, palette::TEXT_FAINT).wrap(),
        );
    }

    fn tabs(&mut self, ui: &mut Ui, body: Rect) {
        let order = tab_order(&self.cfg.tabs.order);
        ui.p.text(
            "Use the arrows to change the order of the tabs in the header. Home always stays first.",
            Rect::new(body.x + 2.0, body.y, body.w, 18.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        let top = body.y + 28.0;
        if let Some(next) = order_row(ui, Rect::new(body.x, top, body.w, 32.0), &order, &TAB_CHOICES, 4) {
            put(ui, "tabs", "order", list_value(&next));
        }
        let rows = order.len().div_ceil(4) as f32;
        let by = top + rows * 40.0 + 6.0;
        let style = ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM);
        if ui.pill_button(Rect::new(body.x, by, 170.0, 30.0), "Reset order", Some(Icon::Refresh), style) {
            put(ui, "tabs", "order", "[]".to_string());
        }
        ui.p.text(
            "Tabs for modules you switched off are hidden.",
            Rect::new(body.x + 182.0, by, body.w - 182.0, 30.0),
            &TextStyle::new(11.5, palette::TEXT_FAINT),
        );
    }

    fn gemini(&mut self, ui: &mut Ui, body: Rect) {
        let _accent = ui.accent;
        let blue = Color::hex(0x4285F4);

        let row = row_rect(body, 0);
        label(ui, row, "API key");
        let save = Rect::new(row.right() - 80.0, row.y + 2.0, 74.0, row.h - 4.0);
        let field = Rect::new(save.x - 8.0 - 250.0, row.y + 2.0, 250.0, row.h - 4.0);
        let hint = if self.has_key { "Saved. Paste a new key to replace it" } else { "Paste your key (AIza…)" };
        if self.key.draw(ui, field, hint, blue, self.focus == Focus::Key) {
            self.focus = Focus::Key;
            ui.fx.keyboard_focus = Some(true);
        }
        let can = !self.key.is_empty();
        let style =
            if can { ButtonStyle::inverted() } else { ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT) };
        if ui.pill_button(save, "Save", None, style) && can {
            let bus = ui.bus.clone();
            self.save_key(&bus);
            self.focus = Focus::None;
            ui.fx.keyboard_focus = Some(false);
        }

        let row = row_rect(body, 1);
        label(ui, row, "Model");
        let save = Rect::new(row.right() - 80.0, row.y + 2.0, 74.0, row.h - 4.0);
        let field = Rect::new(save.x - 8.0 - 250.0, row.y + 2.0, 250.0, row.h - 4.0);
        if self.model.draw(ui, field, "gemini-2.5-flash", blue, self.focus == Focus::Model) {
            self.focus = Focus::Model;
            ui.fx.keyboard_focus = Some(true);
        }
        if ui.pill_button(save, "Save", None, ButtonStyle::inverted()) {
            let v = self.model.text().trim().to_string();
            if !v.is_empty() {
                put(ui, "gemini", "model", toml_str(&v));
                self.say("Model saved", true);
            }
            self.focus = Focus::None;
            ui.fx.keyboard_focus = Some(false);
        }

        let row = row_rect(body, 2);
        let style = ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM);
        if ui.pill_button(Rect::new(row.x, row.y + 2.0, 170.0, row.h - 4.0), "Get a free key", Some(Icon::Link), style)
        {
            shell_open("https://aistudio.google.com/apikey");
        }
        if self.has_key
            && ui.pill_button(
                Rect::new(row.x + 178.0, row.y + 2.0, 150.0, row.h - 4.0),
                "Remove saved key",
                Some(Icon::Trash),
                style,
            )
        {
            dotenv::set_saved(gemini::KEY_VAR, "");
            self.has_key = dotenv::get_saved_first(gemini::KEY_VAR).is_some();
            ui.bus.to_module("gemini", gemini::KeyChanged);
            self.say("Saved key removed", true);
        }

        let row = row_rect(body, 3);
        ui.p.text(
            "The key stays on this PC (in your DynamicNotch folder), never in config.toml.",
            Rect::new(row.x + 2.0, row.y, row.w, row.h),
            &TextStyle::new(11.5, palette::TEXT_FAINT),
        );
    }
}
