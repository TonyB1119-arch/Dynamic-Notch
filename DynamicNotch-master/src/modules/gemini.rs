//! Gemini chat tab, separate from the OpenRouter-based "Ask AI".
//!
//! Paste your key right on the page (or in Settings → Gemini). It is saved in
//! `%APPDATA%\DynamicNotch\.env` as GEMINI_API_KEY, never in config.toml; a
//! GEMINI_API_KEY from the environment or another `.env` works too.
//! Answers stream in over SSE (`streamGenerateContent?alt=sse`).

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use super::widgets::TextField;
use super::{Cx, Key, Module, ModuleId, SystemEvent};
use crate::anim::ease_out;
use crate::bus::Bus;
use crate::config::GeminiCfg;
use crate::gfx::{Icon, TextStyle};
use crate::sys::{clip, dotenv, http, shell_open};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect, SendHwnd};

const ID: ModuleId = "gemini";
pub const KEY_VAR: &str = "GEMINI_API_KEY";
const INPUT_H: f32 = 40.0;
const FOOTER_H: f32 = 24.0;
const MAX_RESPONSE_H: f32 = 230.0;
const KEY_PAGE_H: f32 = 150.0;
const BLUE: Color = Color::hex(0x4285F4);

/// Sent by the settings page after it saved or removed the API key.
pub struct KeyChanged;

enum GMsg {
    Delta(u64, String),
    Done(u64),
    Error(u64, String),
}

#[derive(Clone)]
struct Turn {
    user: String,
    assistant: String,
}

pub struct Gemini {
    input: TextField,
    key_field: TextField,
    has_key: bool,
    /// Showing the key form even though a key exists (Change key).
    editing_key: bool,
    key_error: Option<String>,
    history: Vec<Turn>,
    current: Option<Turn>,
    streaming: bool,
    error: Option<String>,
    gen: u64,
    cancel: Option<Arc<AtomicBool>>,
    scroll: f32,
    stick_bottom: bool,
    text_h: f32,
    asked_at: Instant,
    hwnd: SendHwnd,
    bus: Option<Bus>,
    cfg: GeminiCfg,
}

impl Gemini {
    pub fn new() -> Self {
        Self {
            input: TextField::new(4000),
            key_field: TextField::new(200).masked(),
            has_key: false,
            editing_key: false,
            key_error: None,
            history: Vec::new(),
            current: None,
            streaming: false,
            error: None,
            gen: 0,
            cancel: None,
            scroll: 0.0,
            stick_bottom: true,
            text_h: 0.0,
            asked_at: Instant::now(),
            hwnd: SendHwnd(0),
            bus: None,
            cfg: GeminiCfg::default(),
        }
    }

    fn key_form(&self) -> bool {
        !self.has_key || self.editing_key
    }

    fn save_key(&mut self) -> bool {
        let key: String = self.key_field.text().chars().filter(|c| !c.is_whitespace()).collect();
        if key.len() < 20 {
            self.key_error = Some("That doesn't look like a Gemini API key".into());
            return false;
        }
        if !dotenv::set_saved(KEY_VAR, &key) {
            self.key_error = Some("Couldn't save the key (is %APPDATA% writable?)".into());
            return false;
        }
        self.key_field.clear();
        self.key_error = None;
        self.has_key = true;
        self.editing_key = false;
        true
    }

    fn stop_stream(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        if self.streaming {
            self.streaming = false;
            if let Some(t) = self.current.take() {
                if !t.assistant.is_empty() {
                    self.history.push(t);
                }
            }
        }
    }

    fn new_chat(&mut self) {
        self.stop_stream();
        self.history.clear();
        self.current = None;
        self.error = None;
        self.scroll = 0.0;
    }

    fn submit(&mut self) {
        let prompt = self.input.text().trim().to_string();
        if prompt.is_empty() || self.streaming {
            return;
        }
        let Some(key) = dotenv::get_saved_first(KEY_VAR) else {
            self.has_key = false;
            return;
        };
        let Some(bus) = self.bus.clone() else { return };
        if let Some(t) = self.current.take() {
            self.history.push(t);
        }
        self.error = None;
        self.input.clear();
        self.gen += 1;
        self.streaming = true;
        self.scroll = 0.0;
        self.stick_bottom = true;
        self.asked_at = Instant::now();

        let mut contents = Vec::new();
        for t in self.history.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev() {
            contents.push(json!({ "role": "user", "parts": [{ "text": t.user }] }));
            contents.push(json!({ "role": "model", "parts": [{ "text": t.assistant }] }));
        }
        contents.push(json!({ "role": "user", "parts": [{ "text": prompt }] }));
        self.current = Some(Turn { user: prompt, assistant: String::new() });

        let body = json!({
            "systemInstruction": { "parts": [{ "text": self.cfg.system_prompt }] },
            "contents": contents,
            "generationConfig": { "maxOutputTokens": self.cfg.max_tokens },
        });
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.cfg.endpoint.trim_end_matches('/'),
            self.cfg.model.trim()
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let gen = self.gen;
        let _ = std::thread::Builder::new()
            .name("gemini-request".into())
            .spawn(move || request(url, key, body, gen, cancel, bus));
    }

    fn response(&self) -> Option<&Turn> {
        self.current.as_ref().or(self.history.last())
    }
}

fn request(url: String, key: String, body: Value, gen: u64, cancel: Arc<AtomicBool>, bus: Bus) {
    let headers = [
        ("x-goog-api-key", key),
        ("Content-Type", "application/json".to_string()),
        ("Accept", "text/event-stream".to_string()),
    ];
    let payload = body.to_string().into_bytes();
    let mut parser = http::SseParser::default();
    let mut raw = Vec::new();
    let mut stream_error: Option<String> = None;
    let res = http::post_stream(&url, &headers, &payload, &cancel, |chunk| {
        if raw.len() < 16 * 1024 {
            raw.extend_from_slice(chunk);
        }
        parser.push(chunk, |data| {
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            if let Some(e) = v.get("error") {
                stream_error = Some(e.get("message").and_then(Value::as_str).unwrap_or("stream error").to_string());
                return;
            }
            if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(Value::as_array) {
                let text: String = parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect();
                if !text.is_empty() {
                    bus.to_module(ID, GMsg::Delta(gen, text));
                }
            }
            if let Some(reason) = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
                stream_error = Some(format!("Gemini blocked the request ({reason})"));
            }
        });
        !cancel.load(Ordering::Relaxed)
    });
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    match res {
        Ok(200) if stream_error.is_none() => bus.to_module(ID, GMsg::Done(gen)),
        Ok(status) => {
            let msg = stream_error.unwrap_or_else(|| {
                serde_json::from_slice::<Value>(&raw)
                    .ok()
                    .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_else(|| format!("HTTP {status}"))
            });
            let hint = match status {
                400 | 401 | 403 => " (check your API key in Settings → Gemini)",
                404 => " (check the model name in Settings → Gemini)",
                429 => " (rate limit or quota reached, try again shortly)",
                _ => "",
            };
            bus.to_module(ID, GMsg::Error(gen, format!("{msg}{hint}")));
        }
        Err(e) => bus.to_module(ID, GMsg::Error(gen, e)),
    }
}

impl Module for Gemini {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Gemini"
    }
    fn icon(&self) -> Icon {
        Icon::Chat
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
        self.bus = Some(cx.bus.clone());
        self.cfg = cx.cfg.gemini.clone();
        self.has_key = dotenv::get_saved_first(KEY_VAR).is_some();
    }

    fn stop(&mut self) {
        self.stop_stream();
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let msg = match msg.downcast::<KeyChanged>() {
            Ok(_) => {
                self.has_key = dotenv::get_saved_first(KEY_VAR).is_some();
                self.editing_key = false;
                cx.fx.layout_changed = true;
                return;
            }
            Err(m) => m,
        };
        let Ok(m) = msg.downcast::<GMsg>() else { return };
        match *m {
            GMsg::Delta(g, t) if g == self.gen => {
                if let Some(cur) = self.current.as_mut() {
                    cur.assistant.push_str(&t);
                }
            }
            GMsg::Done(g) if g == self.gen => {
                self.streaming = false;
                self.cancel = None;
            }
            GMsg::Error(g, e) if g == self.gen => {
                self.streaming = false;
                self.cancel = None;
                self.error = Some(e);
                if self.current.as_ref().is_some_and(|c| c.assistant.is_empty()) {
                    if let Some(c) = self.current.take() {
                        self.input.set(&c.user);
                    }
                }
            }
            _ => {}
        }
        cx.fx.layout_changed = true;
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        match ev {
            SystemEvent::ConfigReloaded => {
                self.cfg = cx.cfg.gemini.clone();
                // a key saved from the settings page
                self.has_key = dotenv::get_saved_first(KEY_VAR).is_some();
            }
            SystemEvent::Expanded(false) => self.editing_key = false,
            _ => {}
        }
    }

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        if self.key_form() {
            return KEY_PAGE_H;
        }
        let body = if self.response().is_some() || self.error.is_some() {
            self.text_h.clamp(24.0, MAX_RESPONSE_H) + 30.0
        } else {
            34.0
        };
        INPUT_H + 10.0 + body + FOOTER_H
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        if self.key_form() {
            if self.key_field.key(key, self.hwnd) && self.save_key() {
                cx.fx.layout_changed = true;
            }
        } else {
            match key {
                Key::Up => {
                    self.scroll = (self.scroll - 40.0).max(0.0);
                    self.stick_bottom = false;
                }
                Key::Down => self.scroll += 40.0,
                Key::Copy => {
                    if let Some(t) = self.response() {
                        clip::set_text(self.hwnd.hwnd(), &t.assistant.clone());
                    }
                }
                _ => {
                    if self.input.key(key, self.hwnd) {
                        self.submit();
                    }
                }
            }
        }
        cx.fx.layout_changed = true;
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        if self.key_form() {
            self.draw_key_form(ui, r);
        } else {
            self.draw_chat(ui, r);
        }
    }
}

impl Gemini {
    fn draw_key_form(&mut self, ui: &mut Ui, r: Rect) {
        ui.p.icon(Icon::Chat, Rect::new(r.x + 2.0, r.y + 4.0, 20.0, 20.0), BLUE);
        ui.p.text(
            "Connect Gemini",
            Rect::new(r.x + 30.0, r.y, r.w - 30.0, 28.0),
            &TextStyle::new(15.0, palette::TEXT).semibold(),
        );
        ui.p.text(
            "Paste your API key. It is stored only on this PC, in your DynamicNotch folder.",
            Rect::new(r.x, r.y + 32.0, r.w, 18.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        let field = Rect::new(r.x, r.y + 58.0, r.w - 84.0, 38.0);
        let save = Rect::new(field.right() + 8.0, field.y + 4.0, 76.0, 30.0);
        if self.key_field.draw(ui, field, "AIza…", BLUE, true) {
            ui.fx.keyboard_focus = Some(true);
        }
        let can = !self.key_field.is_empty();
        let style = if can {
            ButtonStyle::filled(BLUE.with_a(0.9)).fg(Color::white(1.0))
        } else {
            ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT)
        };
        if ui.pill_button(save, "Save", None, style) && can && self.save_key() {
            ui.fx.layout_changed = true;
        }
        if let Some(e) = &self.key_error {
            ui.p.text(e, Rect::new(r.x, r.y + 100.0, r.w, 16.0), &TextStyle::new(12.0, palette::RED));
        }
        let get = Rect::new(r.x, r.bottom() - 28.0, 170.0, 26.0);
        if ui.pill_button(get, "Get a free key", Some(Icon::Link), ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
            shell_open("https://aistudio.google.com/apikey");
        }
        if self.has_key {
            let back = Rect::new(r.right() - 90.0, r.bottom() - 28.0, 90.0, 26.0);
            if ui.pill_button(back, "Cancel", None, ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
                self.editing_key = false;
                self.key_field.clear();
                ui.fx.layout_changed = true;
            }
        }
    }

    fn draw_chat(&mut self, ui: &mut Ui, r: Rect) {
        // --- input ----------------------------------------------------------
        let field = Rect::new(r.x, r.y, r.w, INPUT_H);
        let btn = Rect::new(field.right() - 34.0, field.cy() - 14.0, 28.0, 28.0);
        let inner = Rect::new(field.x, field.y, field.w - 40.0, field.h);
        let hint = if self.streaming { "Answering…" } else { "Ask Gemini…" };
        if self.input.draw(ui, inner, hint, BLUE, true) {
            ui.fx.keyboard_focus = Some(true);
        }
        if self.streaming {
            if ui.icon_button(btn, Icon::Stop, ButtonStyle::filled(Color::white(0.14)).scale(0.5)) {
                self.stop_stream();
            }
        } else {
            let can = !self.input.is_empty();
            let style = if can {
                ButtonStyle::filled(BLUE.with_a(0.9))
            } else {
                ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT)
            };
            if ui.icon_button(btn, Icon::Send, style.scale(0.48)) && can {
                self.submit();
            }
        }

        // --- response ---------------------------------------------------------
        let body_top = field.bottom() + 10.0;
        let footer = Rect::new(r.x, r.bottom() - FOOTER_H, r.w, FOOTER_H);
        let area = Rect::new(r.x, body_top, r.w, (footer.y - body_top - 4.0).max(0.0));
        if let Some(err) = self.error.clone() {
            let es = TextStyle::new(12.5, palette::RED).wrap().top();
            self.text_h = ui.p.measure_wrapped(&err, &es, area.w - 8.0);
            ui.p.text(&err, Rect::new(area.x + 4.0, area.y + 4.0, area.w - 8.0, area.h), &es);
        } else if let Some(turn) = self.response().cloned() {
            let reveal = ease_out(ui.now.saturating_duration_since(self.asked_at).as_secs_f32() / 0.3);
            let alpha = ui.p.alpha;
            ui.p.alpha = alpha * reveal;
            let q = TextStyle::new(12.0, palette::TEXT_FAINT);
            ui.p.text(&format!("› {}", turn.user), Rect::new(area.x + 4.0, area.y, area.w - 8.0, 18.0), &q);
            let resp = Rect::new(area.x + 4.0, area.y + 24.0, area.w - 8.0, (area.h - 24.0).max(0.0));
            if turn.assistant.is_empty() && self.streaming {
                self.text_h = 20.0;
                ui.p.text("Thinking…", Rect::new(resp.x, resp.y, resp.w, 20.0), &TextStyle::new(13.0, palette::TEXT_DIM));
                ui.animate();
            } else {
                let rs = TextStyle::new(13.5, palette::TEXT).wrap().top();
                let layout = ui.p.layout(&turn.assistant, &rs, resp.w, 100_000.0);
                let h = layout.as_ref().map_or(0.0, |l| l.height);
                self.text_h = h;
                let max_scroll = (h - resp.h).max(0.0);
                let wheel = ui.wheel(resp);
                if wheel != 0.0 {
                    self.scroll -= wheel * 40.0;
                    self.stick_bottom = self.scroll >= max_scroll - 1.0;
                }
                if self.stick_bottom && self.streaming {
                    self.scroll = max_scroll;
                }
                self.scroll = self.scroll.clamp(0.0, max_scroll);
                ui.p.push_clip(resp);
                if let Some(l) = &layout {
                    ui.p.draw_layout(l, resp.x, resp.y - self.scroll, rs.color);
                }
                ui.p.pop_clip();
                if max_scroll > 0.0 {
                    let th = (resp.h * resp.h / h).max(16.0);
                    let ty = resp.y + (resp.h - th) * (self.scroll / max_scroll);
                    ui.p.fill_rounded(Rect::new(resp.right() + 1.0, ty, 3.0, th), 1.5, Color::white(0.25));
                }
            }
            ui.p.alpha = alpha;
            if reveal < 1.0 {
                ui.animate();
            }
        } else {
            self.text_h = 0.0;
            ui.p.text(
                "Enter to send · Esc to close · Ctrl+V to paste",
                Rect::new(area.x + 4.0, area.y, area.w, 22.0),
                &TextStyle::new(12.0, palette::TEXT_FAINT),
            );
        }

        // --- footer -----------------------------------------------------------
        let fs = TextStyle::new(11.0, palette::TEXT_FAINT);
        ui.p.text(&self.cfg.model, Rect::new(footer.x + 4.0, footer.y, footer.w * 0.3, footer.h), &fs);
        let mut x = footer.right();
        let key_btn = Rect::new(x - 92.0, footer.y, 92.0, footer.h);
        if ui.pill_button(key_btn, "Change key", Some(Icon::Lock), ButtonStyle::filled(Color::white(0.06)).fg(palette::TEXT_FAINT)) {
            self.editing_key = true;
            self.key_field.clear();
            ui.fx.layout_changed = true;
        }
        x = key_btn.x - 6.0;
        if self.response().is_some() {
            let nb = Rect::new(x - 92.0, footer.y, 92.0, footer.h);
            if ui.pill_button(nb, "New chat", Some(Icon::Refresh), ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
                self.new_chat();
                ui.fx.layout_changed = true;
            }
            if let Some(t) = self.response() {
                if !t.assistant.is_empty() && !self.streaming {
                    let cb = Rect::new(nb.x - 28.0, footer.y, footer.h, footer.h);
                    let text = t.assistant.clone();
                    if ui.icon_button(cb, Icon::Copy, ButtonStyle::default().scale(0.62).fg(palette::TEXT_DIM)) {
                        clip::set_text(self.hwnd.hwnd(), &text);
                    }
                }
            }
        }
    }
}
