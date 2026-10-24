//! Settings tab: the options people change most, without opening config.toml.
//!
//! Every control writes through `Effects::save_config`, which edits one value
//! in config.toml (keeping comments) and lets the normal hot reload apply it.
//! The Gemini key goes to `%APPDATA%\DynamicNotch\.env`, never to the config.

use std::time::{Duration, Instant};

use super::gemini;
use super::widgets::{chip, segmented, slider, switch, toml_str, TextField};
use super::{Cx, Key, Module, ModuleId, SystemEvent};
use crate::config::{self, Config};
use crate::gfx::{Icon, TextStyle};
use crate::sys::{dotenv, shell_open};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{app_dir, palette, Color, Rect, SendHwnd};

const ID: ModuleId = "settings";
const TABS: [&str; 6] = ["General", "Look", "Home", "Modules", "Weather", "Gemini"];
const ROW_H: f32 = 32.0;
const ROW_GAP: f32 = 3.0;
const TAB_H: f32 = 28.0;
const FOOT_H: f32 = 18.0;
const MAX_HOME: usize = 4;

/// Widgets that can be placed on the home page: (config id, label).
const HOME_CHOICES: [(&str, &str); 11] = [
    ("weather", "Weather"),
    ("stats", "System stats"),
    ("timer", "Timer"),
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("notes", "Notes"),
    ("clock", "Clock"),
    ("clipboard", "Clipboard"),
    ("downloads", "Downloads"),
    ("shelf", "Shelf"),
    ("ports", "Dev servers"),
];

/// Modules that can be switched on and off: (config id, label).
const MODULE_CHOICES: [(&str, &str); 18] = [
    ("media", "Media"),
    ("timer", "Timer"),
    ("downloads", "Downloads"),
    ("clipboard", "Clipboard"),
    ("shelf", "Shelf"),
    ("voice", "Dictation"),
    ("ai", "Ask AI"),
    ("gemini", "Gemini"),
    ("weather", "Weather"),
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("notes", "Notes"),
    ("stats", "System stats"),
    ("ports", "Dev servers"),
    ("privacy", "Mic & camera"),
    ("battery", "Battery"),
    ("locks", "Lock keys"),
    ("osd", "Volume & brightness"),
];

const SWATCHES: [u32; 6] = [0x0A84FF, 0xBF5AF2, 0xFF375F, 0xFF9F0A, 0x34C759, 0x64D2FF];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    None,
    City,
    Key,
    Model,
}

pub struct Settings {
    tab: usize,
    cfg: Config,
    city: TextField,
    key: TextField,
    model: TextField,
    focus: Focus,
    has_key: bool,
    status: Option<(String, Instant, bool)>,
    hwnd: SendHwnd,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            tab: 0,
            cfg: Config::default(),
            city: TextField::new(80),
            key: TextField::new(200).masked(),
            model: TextField::new(80),
            focus: Focus::None,
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

fn row_rect(body: Rect, i: usize) -> Rect {
    Rect::new(body.x, body.y + i as f32 * (ROW_H + ROW_GAP), body.w, ROW_H)
}

fn label(ui: &mut Ui, row: Rect, text: &str) {
    ui.p.fill_rounded(row, 10.0, palette::CARD);
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

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        self.focus != Focus::None
    }

    fn page_height(&self, _w: f32) -> f32 {
        let rows = match self.tab {
            0 => 6.0,
            1 => 7.0,
            2 => 6.0,
            3 => 5.5,
            4 => 4.0,
            _ => 4.0,
        };
        TAB_H + 10.0 + rows * (ROW_H + ROW_GAP) + FOOT_H
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        let enter = match self.focus {
            Focus::City => self.city.key(key, self.hwnd),
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
        let accent = ui.accent;

        // --- sections ---------------------------------------------------------
        let gap = 6.0;
        let tw = (r.w - gap * (TABS.len() as f32 - 1.0)) / TABS.len() as f32;
        for (i, name) in TABS.iter().enumerate() {
            let b = Rect::new(r.x + i as f32 * (tw + gap), r.y, tw, TAB_H);
            let on = i == self.tab;
            let style = if on {
                ButtonStyle::filled(accent.with_a(0.85)).fg(Color::white(1.0))
            } else {
                ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM)
            };
            if ui.pill_button(b, name, None, style) && !on {
                self.tab = i;
                self.focus = Focus::None;
                ui.fx.keyboard_focus = Some(false);
                ui.fx.layout_changed = true;
            }
        }
        let body = Rect::new(r.x, r.y + TAB_H + 10.0, r.w, r.h - TAB_H - 10.0 - FOOT_H);

        match self.tab {
            0 => self.general(ui, body),
            1 => self.look(ui, body),
            2 => self.home(ui, body),
            3 => self.modules(ui, body),
            4 => self.weather(ui, body),
            _ => self.gemini(ui, body),
        }

        // --- status line ------------------------------------------------------
        if let Some((text, at, good)) = self.status.clone() {
            if at.elapsed() < Duration::from_secs(4) {
                let col = if good { palette::GREEN } else { palette::ORANGE };
                ui.p.text(
                    &text,
                    Rect::new(r.x, r.bottom() - FOOT_H, r.w, FOOT_H),
                    &TextStyle::new(11.5, col).right(),
                );
            } else {
                self.status = None;
            }
        } else {
            ui.p.text(
                "Changes apply right away",
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
        toggle_row(ui, row_rect(body, 0), "Open when I hover the notch", g.expand_on_hover, "general", "expand_on_hover");
        toggle_row(ui, row_rect(body, 1), "Hide for fullscreen apps", g.hide_in_fullscreen, "general", "hide_in_fullscreen");
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

        let row = row_rect(body, 5);
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
            "Size",
            a.scale,
            (0.8, 1.5),
            format!("{:.0}%", a.scale * 100.0),
            "appearance",
            "scale",
        );
        slider_row(ui, row_rect(body, 4), "Bounce", a.bounce, (0.0, 1.0), format!("{:.0}%", a.bounce * 100.0), "appearance", "bounce");
        slider_row(
            ui,
            row_rect(body, 5),
            "Animation speed",
            a.animation_speed,
            (0.5, 2.0),
            format!("{:.1}×", a.animation_speed),
            "appearance",
            "animation_speed",
        );

        let row = row_rect(body, 6);
        label(ui, row, "Accent color");
        let auto_sel = a.accent.eq_ignore_ascii_case("auto");
        let mut x = row.right() - 12.0 - 6.0 * 26.0 - 64.0;
        let auto = Rect::new(x, row.y + 4.0, 56.0, row.h - 8.0);
        let style = if auto_sel {
            ButtonStyle::filled(accent.with_a(0.85)).fg(Color::white(1.0))
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
    }

    fn home(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let chosen = self.cfg.home.widgets.clone();
        ui.p.text(
            &format!("Pick up to {MAX_HOME} widgets for the home page, in the order you want them."),
            Rect::new(body.x + 2.0, body.y, body.w, 18.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        let cols = 4;
        let gap = 8.0;
        let cw = (body.w - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = 32.0;
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
    }

    fn modules(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let cols = 4;
        let gap = 8.0;
        let cw = (body.w - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = 32.0;
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
        if ui.pill_button(save, "Save", None, ButtonStyle::filled(accent.with_a(0.85)).fg(Color::white(1.0))) {
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

    fn gemini(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
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
        let style = if can {
            ButtonStyle::filled(blue.with_a(0.9)).fg(Color::white(1.0))
        } else {
            ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT)
        };
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
        if ui.pill_button(save, "Save", None, ButtonStyle::filled(accent.with_a(0.85)).fg(Color::white(1.0))) {
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
        if ui.pill_button(Rect::new(row.x, row.y + 2.0, 170.0, row.h - 4.0), "Get a free key", Some(Icon::Link), style) {
            shell_open("https://aistudio.google.com/apikey");
        }
        if self.has_key
            && ui.pill_button(Rect::new(row.x + 178.0, row.y + 2.0, 150.0, row.h - 4.0), "Remove saved key", Some(Icon::Trash), style)
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
