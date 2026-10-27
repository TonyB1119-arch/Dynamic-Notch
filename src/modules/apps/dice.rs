//! Randomizer: roll dice, flip a coin or generate a password.

use std::time::{Duration, Instant};

use super::Rng;
use crate::gfx::{Icon, TextStyle};
use crate::modules::widgets::{segmented, slider, switch};
use crate::modules::{Cx, Module, ModuleId};
use crate::sys::clip;
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect, SendHwnd};

const ID: ModuleId = "dice";
const SIDES: [u32; 6] = [4, 6, 8, 12, 20, 100];
const ROLL: Duration = Duration::from_millis(650);
const PW_MIN: usize = 8;
const PW_MAX: usize = 32;

pub struct Dice {
    mode: usize,
    rng: Rng,
    hwnd: SendHwnd,
    // dice
    sides: usize,
    value: u32,
    rolling_since: Option<Instant>,
    recent: Vec<u32>,
    // coin
    heads: bool,
    flipping_since: Option<Instant>,
    flips: [u32; 2],
    // password
    length: usize,
    digits: bool,
    symbols: bool,
    password: String,
    copied_at: Option<Instant>,
}

impl Dice {
    pub fn new() -> Self {
        Self {
            mode: 0,
            rng: Rng::new(),
            hwnd: SendHwnd(0),
            sides: 1,
            value: 0,
            rolling_since: None,
            recent: Vec::new(),
            heads: true,
            flipping_since: None,
            flips: [0, 0],
            length: 16,
            digits: true,
            symbols: true,
            password: String::new(),
            copied_at: None,
        }
    }

    fn roll(&mut self, now: Instant) {
        self.value = self.rng.below(SIDES[self.sides]) + 1;
        self.rolling_since = Some(now);
        self.recent.insert(0, self.value);
        self.recent.truncate(8);
    }

    fn flip(&mut self, now: Instant) {
        self.heads = self.rng.below(2) == 0;
        self.flips[usize::from(!self.heads)] += 1;
        self.flipping_since = Some(now);
    }

    fn generate(&mut self) {
        let mut set: Vec<char> = ('a'..='z').chain('A'..='Z').collect();
        if self.digits {
            set.extend('0'..='9');
        }
        if self.symbols {
            set.extend("!@#$%^&*-_=+?".chars());
        }
        self.password = (0..self.length).map(|_| set[self.rng.below(set.len() as u32) as usize]).collect();
    }
}

impl Module for Dice {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Randomizer"
    }
    fn icon(&self) -> Icon {
        Icon::Gamepad
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
    }

    fn glance(&self) -> Option<(Icon, String)> {
        Some((Icon::Gamepad, "Dice · Coin · Password".into()))
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        214.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let now = ui.now;
        let seg = Rect::new(r.x, r.y, 300.0_f32.min(r.w), 34.0);
        if let Some(i) = segmented(ui, seg, &["Dice", "Coin", "Password"], Some(self.mode), palette::BLUE) {
            self.mode = i;
            if i == 2 && self.password.is_empty() {
                self.generate();
            }
        }
        let body = Rect::new(r.x, r.y + 48.0, r.w, r.h - 48.0);
        match self.mode {
            0 => self.draw_dice(ui, body, now),
            1 => self.draw_coin(ui, body, now),
            _ => self.draw_password(ui, body, now),
        }
    }
}

impl Dice {
    fn draw_dice(&mut self, ui: &mut Ui, body: Rect, now: Instant) {
        let labels: Vec<String> = SIDES.iter().map(|s| format!("d{s}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let seg = Rect::new(body.x, body.y, 330.0_f32.min(body.w), 30.0);
        if let Some(i) = segmented(ui, seg, &refs, Some(self.sides), palette::BLUE) {
            self.sides = i;
        }
        let rolling = self.rolling_since.is_some_and(|t| now.saturating_duration_since(t) < ROLL);
        let shown = if rolling {
            ui.animate();
            self.rng.below(SIDES[self.sides]) + 1
        } else {
            self.value
        };
        let big = Rect::new(body.x, body.y + 42.0, 150.0, 80.0);
        ui.p.text(
            &if shown == 0 { "–".to_string() } else { shown.to_string() },
            big,
            &TextStyle::new(56.0, palette::TEXT).semibold().display().center(),
        );
        let btn = Rect::new(body.x + 170.0, body.y + 60.0, 130.0, 40.0);
        if ui.pill_button(btn, "Roll", Some(Icon::Refresh), ButtonStyle::inverted()) {
            self.roll(now);
            ui.fx.redraw = true;
        }
        // the last rolls as little tick bars, taller = higher
        if !self.recent.is_empty() {
            let max = SIDES[self.sides] as f32;
            let vals: Vec<f32> = self.recent.iter().rev().map(|v| (*v as f32 / max).min(1.0)).collect();
            ui.p.text(
                "Recent rolls",
                Rect::new(body.x + 330.0, body.y + 44.0, 150.0, 16.0),
                &TextStyle::new(11.0, palette::TEXT_FAINT),
            );
            ui.tick_graph(
                Rect::new(body.x + 330.0, body.y + 64.0, (vals.len() as f32 * 10.0).max(20.0), 50.0),
                &vals,
                Color::white(0.9),
            );
        }
    }

    fn draw_coin(&mut self, ui: &mut Ui, body: Rect, now: Instant) {
        let flipping = self.flipping_since.is_some_and(|t| now.saturating_duration_since(t) < ROLL);
        let (cx, cy) = (body.x + 80.0, body.y + 62.0);
        let side_heads = if flipping {
            ui.animate();
            // the coin "spins": shows each side in turn, squashing like it turns over
            let t =
                now.saturating_duration_since(self.flipping_since.unwrap_or(now)).as_secs_f32() / ROLL.as_secs_f32();
            let phase = (t * 6.0).floor() as i32 % 2 == 0;
            let squash = ((t * 6.0).fract() * std::f32::consts::PI).sin().abs().max(0.15);
            ui.p.fill_rounded(Rect::centered(cx, cy, 70.0 * squash, 70.0), 35.0 * squash.min(1.0), Color::white(0.92));
            phase
        } else {
            ui.p.fill_circle(cx, cy, 35.0, Color::white(0.94));
            self.heads
        };
        let label = if self.flips[0] + self.flips[1] == 0 && !flipping {
            "?"
        } else if side_heads {
            "H"
        } else {
            "T"
        };
        ui.p.text(label, Rect::centered(cx, cy, 70.0, 70.0), &TextStyle::new(28.0, palette::INK).semibold().center());
        let word = if flipping || self.flips[0] + self.flips[1] == 0 {
            ""
        } else if self.heads {
            "Heads"
        } else {
            "Tails"
        };
        ui.p.text(
            word,
            Rect::new(body.x + 140.0, body.y + 20.0, 140.0, 28.0),
            &TextStyle::new(20.0, palette::TEXT).semibold(),
        );
        let btn = Rect::new(body.x + 140.0, body.y + 58.0, 130.0, 40.0);
        if ui.pill_button(btn, "Flip", Some(Icon::Refresh), ButtonStyle::inverted()) {
            self.flip(now);
            ui.fx.redraw = true;
        }
        // heads against tails over all flips
        let total = (self.flips[0] + self.flips[1]).max(1) as f32;
        ui.p.text(
            &format!("Heads {} · Tails {}", self.flips[0], self.flips[1]),
            Rect::new(body.x + 300.0, body.y + 38.0, 200.0, 18.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
        ui.ticks(
            Rect::new(body.x + 300.0, body.y + 62.0, 200.0_f32.min(body.w - 300.0), 12.0),
            self.flips[0] as f32 / total,
            Color::white(0.9),
        );
    }

    fn draw_password(&mut self, ui: &mut Ui, body: Rect, now: Instant) {
        // the password itself; click to copy
        let box_r = Rect::new(body.x, body.y, body.w - 130.0, 44.0);
        let hover = ui.hovered(box_r);
        ui.p.fill_rounded(box_r, 14.0, Color::white(if hover { 0.10 } else { 0.06 }));
        if hover {
            ui.hot = true;
        }
        let copied = self.copied_at.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(1400));
        if copied {
            ui.animate();
        }
        let text = if copied { "Copied to clipboard".to_string() } else { self.password.clone() };
        let col = if copied { palette::GREEN } else { palette::TEXT };
        ui.p.text(&text, box_r.inset_xy(14.0, 0.0), &TextStyle::new(16.0, col).mono());
        if ui.clicked(box_r) && !self.password.is_empty() {
            clip::set_text(self.hwnd.hwnd(), &self.password);
            self.copied_at = Some(now);
            ui.fx.redraw = true;
        }
        let gen = Rect::new(box_r.right() + 10.0, box_r.y + 2.0, 120.0, 40.0);
        if ui.pill_button(gen, "New", Some(Icon::Refresh), ButtonStyle::inverted()) {
            self.generate();
        }

        // length: a row of ticks you click or drag
        let y = body.y + 62.0;
        ui.p.text(
            &format!("Length {}", self.length),
            Rect::new(body.x + 4.0, y, 100.0, 24.0),
            &TextStyle::new(13.0, palette::TEXT_DIM),
        );
        let sl = Rect::new(body.x + 110.0, y, body.w - 110.0, 24.0);
        let frac = (self.length - PW_MIN) as f32 / (PW_MAX - PW_MIN) as f32;
        if let Some(f) = slider(ui, sl, frac, palette::BLUE) {
            self.length = PW_MIN + (f * (PW_MAX - PW_MIN) as f32).round() as usize;
            self.generate();
        }
        for (k, (label, on)) in [("Numbers", self.digits), ("Symbols", self.symbols)].into_iter().enumerate() {
            let row = Rect::new(body.x + k as f32 * 200.0, y + 36.0, 190.0, 30.0);
            ui.p.text(label, Rect::new(row.x + 4.0, row.y, 100.0, row.h), &TextStyle::new(13.0, palette::TEXT_DIM));
            if switch(ui, Rect::new(row.x + 100.0, row.y, 52.0, row.h), on, palette::BLUE) {
                if k == 0 {
                    self.digits = !self.digits;
                } else {
                    self.symbols = !self.symbols;
                }
                self.generate();
            }
        }
    }
}
