//! Tiny immediate-mode UI layer on top of `Painter`.
//!
//! Modules describe their UI every frame; interaction is resolved against
//! the input snapshot (hover position, press and click positions).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use crate::bus::Bus;
use crate::config::Config;
use crate::gfx::{Icon, ImageData, Painter, TextStyle};
use crate::modules::Effects;
use crate::util::{palette, Color, Rect};

#[derive(Default, Clone, Debug)]
pub struct Input {
    /// Cursor position (logical, window-relative) when over the notch.
    pub mouse: Option<(f32, f32)>,
    /// Press position while the left button is held.
    pub down: Option<(f32, f32)>,
    /// Completed click this frame: (press, release).
    pub click: Option<((f32, f32), (f32, f32))>,
    pub right_click: Option<(f32, f32)>,
    /// Wheel notches this frame (positive = up).
    pub wheel: f32,
}

#[allow(dead_code)] // fields are for modules to use
pub struct Ui<'p, 'a> {
    pub p: &'p mut Painter<'a>,
    pub input: &'p Input,
    pub fx: &'p mut Effects,
    pub cfg: &'p Config,
    pub bus: &'p Bus,
    pub now: Instant,
    /// Current accent color (album art driven or configured).
    pub accent: Color,
    /// Set when the cursor is over something interactive.
    pub hot: bool,
    /// Interaction is disabled while content is fading (prevents misclicks).
    pub interactive: bool,
}

thread_local! {
    /// Values that are gliding towards a target (see `Ui::glide`): key → (shown, last touched).
    static GLIDE: RefCell<HashMap<u64, (f32, Instant)>> = RefCell::new(HashMap::new());
    /// Flip clock digits: slot → (digit shown, previous digit, when it changed, last drawn).
    static FLIP: RefCell<HashMap<u32, (u8, u8, Instant, Instant)>> = RefCell::new(HashMap::new());
}

/// An instant safely in the past (never underflows right after boot).
fn long_ago(now: Instant) -> Instant {
    now.checked_sub(std::time::Duration::from_secs(60)).unwrap_or(now)
}

/// How long one flip takes (seconds): slow enough to read, quick enough to keep up.
const FLIP_SECS: f32 = 0.6;

/// How long one rolling digit takes (seconds).
const ROLL_SECS: f32 = 0.55;

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// How quickly gauges and numbers settle on a new value (seconds).
const GLIDE_TAU: f32 = 0.16;

/// Stable key for a value that should glide, e.g. `gkey("cpu", 0)`.
pub fn gkey(tag: &str, i: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    i.hash(&mut h);
    h.finish()
}

pub fn rect_key(r: Rect, tag: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    (r.x.round() as i32, r.y.round() as i32, r.w.round() as i32).hash(&mut h);
    h.finish()
}

/// Beam style: false = blue with a hint of white, true = white with a hint of blue.
static WHITE_BEAMS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Applies `[appearance] white_beams`. Called when the settings load or change.
pub fn set_white_beams(on: bool) {
    WHITE_BEAMS.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn white_beams() -> bool {
    WHITE_BEAMS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Every lit beam in the app has this colour (the callers' colours only carry opacity).
fn themed(c: Color) -> Color {
    let base = if white_beams() { Color::hex(0xF4F8FF) } else { palette::BLUE };
    base.with_a(c.a)
}

/// Leading edge of a beam: light blue in the blue style, a soft blue touch in the white one.
pub fn beam_lead() -> Color {
    if white_beams() {
        Color::hex(0x5BAEFF)
    } else {
        Color::hex(0x8CC8FF)
    }
}

/// Colour of a tick that is not lit.
const TICK_DIM: Color = Color::white(0.16);

/// Layout of a tick row: (tick count, distance between ticks, tick width).
fn tick_geom(w: f32) -> (usize, f32, f32) {
    let n = ((w / 4.6).round() as usize).max(2);
    let step = w / n as f32;
    (n, step, (step * 0.56).max(1.4))
}

/// Layout of a tick ring: (outer radius, tick length, tick width, tick count).
fn ring_geom(diameter: f32, stroke: f32) -> (f32, f32, f32, usize) {
    let outer = diameter * 0.5;
    let len = (stroke * 1.7).clamp(3.5, 12.0);
    let tw = (stroke * 0.42).clamp(1.5, 3.0);
    let n = ((std::f32::consts::TAU * outer) / (tw * 2.1)).round().clamp(18.0, 72.0) as usize;
    (outer, len, tw, n)
}

#[derive(Clone, Copy)]
pub struct ButtonStyle {
    pub fg: Color,
    pub bg: Color,
    pub hover_bg: Color,
    pub radius: f32,
    pub icon_scale: f32,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        Self { fg: palette::TEXT, bg: Color::white(0.0), hover_bg: Color::white(0.12), radius: 999.0, icon_scale: 0.62 }
    }
}

#[allow(dead_code)]
impl ButtonStyle {
    /// White capsule with dark text: the "active" / primary look.
    pub fn inverted() -> Self {
        Self { fg: palette::INK, bg: Color::white(0.95), hover_bg: Color::white(1.0), ..Default::default() }
    }
    pub fn filled(bg: Color) -> Self {
        Self { bg, hover_bg: bg.lerp(Color::white(1.0), 0.15).with_a(bg.a.max(0.2)), ..Default::default() }
    }
    pub fn fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn scale(mut self, s: f32) -> Self {
        self.icon_scale = s;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
}

#[allow(dead_code)]
impl<'p, 'a> Ui<'p, 'a> {
    pub fn hovered(&self, r: Rect) -> bool {
        self.interactive && self.input.mouse.is_some_and(|(x, y)| r.contains(x, y))
    }

    pub fn pressed(&self, r: Rect) -> bool {
        self.interactive
            && self.input.down.is_some_and(|(x, y)| r.contains(x, y))
            && self.input.mouse.is_some_and(|(x, y)| r.contains(x, y))
    }

    pub fn clicked(&mut self, r: Rect) -> bool {
        if !self.interactive {
            return false;
        }
        let hit = self.input.click.is_some_and(|((px, py), (ux, uy))| r.contains(px, py) && r.contains(ux, uy));
        if hit {
            self.fx.redraw = true;
        }
        hit
    }

    /// Click position inside `r` as a 0..1 fraction of its width.
    pub fn clicked_at(&mut self, r: Rect) -> Option<f32> {
        if self.clicked(r) {
            self.input.click.map(|(_, (ux, _))| ((ux - r.x) / r.w).clamp(0.0, 1.0))
        } else {
            None
        }
    }

    pub fn right_clicked(&self, r: Rect) -> bool {
        self.interactive && self.input.right_click.is_some_and(|(x, y)| r.contains(x, y))
    }

    pub fn wheel(&self, r: Rect) -> f32 {
        if self.hovered(r) {
            self.input.wheel
        } else {
            0.0
        }
    }

    pub fn animate(&mut self) {
        self.fx.animate = true;
    }

    // -- widgets --------------------------------------------------------------

    /// Round icon button. Returns true when clicked.
    pub fn icon_button(&mut self, r: Rect, icon: Icon, style: ButtonStyle) -> bool {
        let hover = self.hovered(r);
        let pressed = self.pressed(r);
        if hover {
            self.hot = true;
        }
        let bg = if hover { style.hover_bg } else { style.bg };
        let rr = if pressed { r.inset(r.w * 0.04) } else { r };
        self.p.fill_rounded(rr, style.radius, bg);
        let s = r.w.min(r.h) * style.icon_scale * if pressed { 0.92 } else { 1.0 };
        self.p.icon(icon, r.center_square(s), style.fg);
        self.clicked(r)
    }

    /// Capsule button with a text label (and optional leading icon).
    pub fn pill_button(&mut self, r: Rect, label: &str, icon: Option<Icon>, style: ButtonStyle) -> bool {
        let hover = self.hovered(r);
        let pressed = self.pressed(r);
        if hover {
            self.hot = true;
        }
        let bg = if hover { style.hover_bg } else { style.bg };
        let rr = if pressed { r.inset(1.0) } else { r };
        self.p.fill_rounded(rr, style.radius.min(r.h * 0.5), bg);
        let ts = TextStyle::new((r.h * 0.42).clamp(10.0, 15.0), style.fg).medium().center();
        match icon {
            Some(ic) => {
                let (tw, _) = self.p.measure(label, &ts);
                let is = r.h * 0.5;
                let total = is + 6.0 + tw;
                let x = r.cx() - total * 0.5;
                self.p.icon(ic, Rect::new(x, r.cy() - is * 0.5, is, is), style.fg);
                let tr = Rect::new(x + is + 6.0, r.y, tw + 2.0, r.h);
                self.p.text(label, tr, &TextStyle { align: crate::gfx::Align::Start, ..ts });
            }
            None => self.p.text(label, r, &ts),
        }
        self.clicked(r)
    }

    /// Eases a value towards `target` instead of jumping: call it every frame
    /// with the same key and show the result. Keeps frames coming until it has
    /// arrived, then costs nothing.
    pub fn glide(&mut self, key: u64, target: f32) -> f32 {
        let now = self.now;
        let (v, moving) = GLIDE.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() > 512 {
                m.retain(|_, e| now.saturating_duration_since(e.1).as_secs() < 10);
            }
            let e = m.entry(key).or_insert((target, now));
            // a long gap (idle) still starts the glide gently instead of snapping
            let dt = now.saturating_duration_since(e.1).as_secs_f32().min(1.0 / 30.0);
            e.1 = now;
            if (target - e.0).abs() < 0.0015 {
                e.0 = target;
            } else {
                e.0 += (target - e.0) * (1.0 - (-dt / GLIDE_TAU).exp());
            }
            (e.0, (target - e.0).abs() >= 0.0015)
        });
        if moving {
            self.fx.animate = true;
        }
        v
    }

    /// One flip-clock digit: a dark tile split in two halves. When the digit
    /// changes the upper half folds down and the new lower half folds in.
    /// `slot` identifies the digit between frames.
    pub fn flip_digit(&mut self, r: Rect, slot: u32, digit: u8, size: f32) {
        let now = self.now;
        let (cur, prev, t) = FLIP.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.entry(slot).or_insert((digit, digit, long_ago(now), now));
            // coming back after a while (page was closed): no flip for a stale digit
            if now.saturating_duration_since(e.3).as_secs_f32() > 2.0 {
                *e = (digit, digit, long_ago(now), now);
            }
            if e.0 != digit {
                e.1 = e.0;
                e.0 = digit;
                e.2 = now;
            }
            e.3 = now;
            (e.0, e.1, now.saturating_duration_since(e.2).as_secs_f32() / FLIP_SECS)
        });
        let radius = (r.h * 0.14).min(10.0);
        let mid = r.cy();
        let top = Rect::new(r.x, r.y, r.w, r.h * 0.5);
        let bot = Rect::new(r.x, mid, r.w, r.h * 0.5);
        let style = TextStyle::new(size, palette::TEXT).semibold().display().center();
        let face = Color::white(0.05);
        let digit_text = |ui: &mut Self, d: u8, shade: f32| {
            ui.p.fill_rounded(r, radius, face.lerp(Color::black(1.0), shade));
            ui.p.text(&d.to_string(), r, &style);
        };
        if t >= 1.0 || prev == cur {
            // at rest: one tile, one text draw
            digit_text(self, cur, 0.0);
            return;
        }
        // static halves: the new top and the bottom (old one until the flap lands)
        self.p.push_clip(top);
        digit_text(self, cur, 0.0);
        self.p.pop_clip();
        let flipping = t < 1.0 && prev != cur;
        let bottom_digit = if flipping { prev } else { cur };
        self.p.push_clip(bot);
        digit_text(self, bottom_digit, 0.0);
        self.p.pop_clip();
        if flipping {
            self.fx.animate = true;
            let k = |x: f32| x * x * (3.0 - 2.0 * x);
            if t < 0.5 {
                // the old upper half falls forward and darkens
                let f = k((t * 2.0).clamp(0.0, 1.0));
                self.p.push_clip(top);
                let saved = self.p.push_scale_y(1.0 - f, mid);
                digit_text(self, prev, 0.35 * f);
                self.p.pop_local(saved);
                self.p.pop_clip();
            } else {
                // the new lower half folds down into place
                let f = k(((t - 0.5) * 2.0).clamp(0.0, 1.0));
                self.p.push_clip(bot);
                let saved = self.p.push_scale_y(f, mid);
                digit_text(self, cur, 0.35 * (1.0 - f));
                self.p.pop_local(saved);
                self.p.pop_clip();
            }
        }
    }

    /// The home clock in the style chosen under Settings → Look.
    #[allow(clippy::too_many_arguments)]
    pub fn clock(&mut self, x: f32, y: f32, tile_w: f32, tile_h: f32, h: u8, m: u8, s: u8, slot0: u32) -> f32 {
        if self.cfg.appearance.clock_style.eq_ignore_ascii_case("flip") {
            self.flip_clock(x, y, tile_w, tile_h, h, m, s, slot0)
        } else {
            self.roll_clock(x, y, tile_w, tile_h, h, m, s, slot0)
        }
    }

    /// One rolling digit: when it changes, the old one slides up and fades while the new
    /// one rises in from below. Bare text, no tile: two text draws only while it moves.
    fn roll_digit(&mut self, r: Rect, slot: u32, digit: u8, size: f32, color: Color) {
        let now = self.now;
        let (cur, prev, t) = FLIP.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.entry(slot).or_insert((digit, digit, long_ago(now), now));
            if now.saturating_duration_since(e.3).as_secs_f32() > 2.0 {
                *e = (digit, digit, long_ago(now), now);
            }
            if e.0 != digit {
                e.1 = e.0;
                e.0 = digit;
                e.2 = now;
            }
            e.3 = now;
            (e.0, e.1, now.saturating_duration_since(e.2).as_secs_f32() / ROLL_SECS)
        });
        let style = TextStyle::new(size, color).semibold().display().center();
        if t >= 1.0 || prev == cur {
            self.p.text(DIGITS[(cur % 10) as usize], r, &style);
            return;
        }
        self.fx.animate = true;
        let e = 1.0 - (1.0 - t).powi(3); // ease out
        let travel = r.h * 0.85;
        let a0 = self.p.alpha;
        self.p.push_clip(Rect::new(r.x - 2.0, r.y - r.h * 0.3, r.w + 4.0, r.h * 1.6));
        self.p.alpha = a0 * (1.0 - (t / 0.7).min(1.0));
        self.p.text(DIGITS[(prev % 10) as usize], r.translate(0.0, (-travel * e).round()), &style);
        self.p.alpha = a0 * ((t - 0.15) / 0.6).clamp(0.0, 1.0);
        self.p.text(DIGITS[(cur % 10) as usize], r.translate(0.0, (travel * (1.0 - e)).round()), &style);
        self.p.alpha = a0;
        self.p.pop_clip();
    }

    /// Rolling clock "HH:MM:SS" in bare digits; the seconds are dimmer. Same footprint
    /// and return value as `flip_clock`.
    #[allow(clippy::too_many_arguments)]
    pub fn roll_clock(&mut self, x: f32, y: f32, tile_w: f32, tile_h: f32, h: u8, m: u8, s: u8, slot0: u32) -> f32 {
        let size = tile_h * 0.86;
        let cell = tile_w * 0.74;
        let pair_gap = (tile_w * 0.42).max(9.0);
        let mut cx = x;
        for (i, v) in [h, m, s].into_iter().enumerate() {
            let col = if i == 2 { Color::white(0.86) } else { palette::TEXT };
            for (j, d) in [v / 10, v % 10].into_iter().enumerate() {
                self.roll_digit(Rect::new(cx, y, cell, tile_h), slot0 + (i * 2 + j) as u32, d, size, col);
                cx += cell;
                if j == 0 {
                    cx += 1.0;
                }
            }
            if i < 2 {
                let dx = cx + pair_gap * 0.5;
                let dot = (tile_w * 0.05).max(1.5);
                self.p.fill_circle(dx, y + tile_h * 0.38, dot, Color::white(0.45));
                self.p.fill_circle(dx, y + tile_h * 0.62, dot, Color::white(0.45));
            }
            cx += pair_gap;
        }
        cx - pair_gap - x
    }

    /// A flip clock "HH MM SS": six digit tiles in pairs. `slot0` keeps the
    /// digits apart from other clocks. Returns the width used.
    #[allow(clippy::too_many_arguments)]
    pub fn flip_clock(&mut self, x: f32, y: f32, tile_w: f32, tile_h: f32, h: u8, m: u8, s: u8, slot0: u32) -> f32 {
        let gap = (tile_w * 0.09).max(2.0);
        let pair_gap = (tile_w * 0.36).max(8.0);
        let size = tile_h * 0.66;
        let mut cx = x;
        for (i, v) in [h, m, s].into_iter().enumerate() {
            for (j, d) in [v / 10, v % 10].into_iter().enumerate() {
                self.flip_digit(Rect::new(cx, y, tile_w, tile_h), slot0 + (i * 2 + j) as u32, d, size);
                cx += tile_w + if j == 0 { gap } else { 0.0 };
            }
            if i < 2 {
                // a colon between hours, minutes and seconds
                let dx = cx + pair_gap * 0.5;
                let dot = (tile_w * 0.07).max(1.6);
                self.p.fill_circle(dx, y + tile_h * 0.36, dot, Color::white(0.7));
                self.p.fill_circle(dx, y + tile_h * 0.64, dot, Color::white(0.7));
            }
            cx += pair_gap;
        }
        cx - pair_gap - x
    }

    /// Circular gauge made of ticks, filling clockwise from 12 o'clock.
    pub fn ring(&mut self, cx: f32, cy: f32, diameter: f32, stroke: f32, frac: f32, color: Color) {
        let (outer, len, tw, n) = ring_geom(diameter, stroke);
        let color = themed(color);
        let key = rect_key(Rect::new(cx, cy, diameter, 0.0), "ring");
        let frac = self.glide2(key, frac.clamp(0.0, 1.0));
        let pos = frac.clamp(0.0, 1.0) * n as f32;
        let white = white_beams();
        let span = (n as f32 * 0.28).max(4.0);
        for i in 0..n {
            let behind = pos - i as f32;
            let t = behind.clamp(0.0, 1.0);
            let mut c = TICK_DIM.lerp(color, t);
            let mut s = 0.0;
            if t > 0.0 {
                s = Self::shine(behind, span) * t;
                let tint = if white { palette::BLUE } else { Color::white(1.0) };
                c = c.lerp(tint.with_a(c.a), s * 0.9);
            }
            self.radial_tick(cx, cy, outer, len, tw, i, n, c, s);
        }
    }

    /// The same ring for "working on it": a bright stretch of ticks that circles.
    pub fn ring_spin(&mut self, cx: f32, cy: f32, diameter: f32, stroke: f32, color: Color) {
        let (outer, len, tw, n) = ring_geom(diameter, stroke);
        let color = themed(color);
        let head = (epoch_secs() * 0.9).fract() * n as f32;
        for i in 0..n {
            let behind = (head - i as f32).rem_euclid(n as f32);
            let t = (1.0 - behind / (n as f32 * 0.35)).clamp(0.0, 1.0);
            let mut c = TICK_DIM.lerp(color, t);
            let s = t * t;
            let tint = if white_beams() { palette::BLUE } else { Color::white(1.0) };
            c = c.lerp(tint.with_a(c.a), s * 0.9);
            self.radial_tick(cx, cy, outer, len, tw, i, n, c, s);
        }
        self.fx.animate = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn radial_tick(
        &mut self,
        cx: f32,
        cy: f32,
        outer: f32,
        len: f32,
        tw: f32,
        i: usize,
        n: usize,
        c: Color,
        shine: f32,
    ) {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / n as f32;
        let (s, co) = a.sin_cos();
        // the whiter a beam shines, the longer and thicker it is
        let (len, tw) = (len + shine * 3.0, tw + shine * 0.7);
        self.p.line(
            cx + co * (outer - len + shine),
            cy + s * (outer - len + shine),
            cx + co * (outer + shine),
            cy + s * (outer + shine),
            tw,
            c,
        );
    }

    /// A row of thin vertical ticks that fills left to right (the app's gauge look).
    pub fn ticks(&mut self, r: Rect, frac: f32, color: Color) {
        self.ticks_ex(r, frac, color, false);
    }

    /// A value that eases in and out: two gentle glides in a row, so it starts and stops softly.
    fn glide2(&mut self, key: u64, target: f32) -> f32 {
        let first = self.glide(key, target);
        self.glide(key ^ 0x9E37_79B9, first)
    }

    /// One vertical beam. It sits on whole device pixels with a whole-pixel width, so every beam
    /// in a row is equally crisp and bright, and it grows by `shine` (0..1): the whiter a beam
    /// shines, the larger it is.
    fn beam(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color, shine: f32) {
        let sc = self.p.scale.max(0.5);
        let px = |v: f32| (v * sc).round() / sc;
        let bw = ((w * sc).round().max(2.0)) / sc;
        let bx = px(x + (w - bw) * 0.5);
        let grow = shine * 2.2;
        self.p.fill_rounded(Rect::new(bx, px(y) - grow, bw, h + 2.0 * grow), bw * 0.5, c);
    }

    /// How brightly a beam shines, 0..1: strongest at the leading edge, fading over `span` beams.
    fn shine(behind: f32, span: f32) -> f32 {
        let k = (1.0 - behind / span).clamp(0.0, 1.0);
        k * k
    }

    /// `warm` is kept for callers; every gauge now shines white at its leading edge.
    pub fn ticks_ex(&mut self, r: Rect, frac: f32, color: Color, _warm: bool) {
        let frac = self.glide2(rect_key(r, "ticks"), frac.clamp(0.0, 1.0));
        self.ticks_raw(r, frac, color, false);
    }

    /// The volume / brightness bar (the same look as every gauge).
    pub fn ticks_hud(&mut self, r: Rect, frac: f32) {
        self.ticks_ex(r, frac, Color::white(1.0), true);
    }

    /// Like `ticks_ex` but draws `frac` exactly as given (the caller smooths it).
    fn ticks_raw(&mut self, r: Rect, frac: f32, color: Color, _warm: bool) {
        let (n, step, w) = tick_geom(r.w);
        let color = themed(color);
        let pos = frac.clamp(0.0, 1.0) * n as f32;
        let white = white_beams();
        for i in 0..n {
            let behind = pos - i as f32; // how far this tick is behind the leading edge
            let t = behind.clamp(0.0, 1.0);
            let mut c = TICK_DIM.lerp(color, t);
            let mut s = 0.0;
            if t > 0.0 {
                s = Self::shine(behind, 7.0) * t;
                let tint = if white { palette::BLUE } else { Color::white(1.0) };
                c = c.lerp(tint.with_a(c.a), s * 0.9);
            }
            self.beam(r.x + i as f32 * step + (step - w) * 0.5, r.y, w, r.h, c, s);
        }
    }

    /// A row of ticks with only the stretch from `from` to `to` (0..1 of the width) lit,
    /// like a temperature range.
    pub fn tick_range(&mut self, r: Rect, from: f32, to: f32, color: Color) {
        let (n, step, w) = tick_geom(r.w);
        let color = themed(color);
        let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0).max(from));
        let white = white_beams();
        for i in 0..n {
            let pos = (i as f32 + 0.5) / n as f32;
            let lit = pos >= from - 0.5 / n as f32 && pos <= to + 0.5 / n as f32;
            let mut c = if lit { color } else { TICK_DIM };
            let mut s = 0.0;
            if lit {
                // the middle of the lit stretch shines white and stands a little taller
                let t = if to > from { ((pos - from) / (to - from)).clamp(0.0, 1.0) } else { 0.5 };
                s = (1.0 - (2.0 * t - 1.0).abs()).powi(2);
                let tint = if white { palette::BLUE } else { Color::white(1.0) };
                c = c.lerp(tint.with_a(c.a), s * 0.9);
            }
            self.beam(r.x + i as f32 * step + (step - w) * 0.5, r.y, w, r.h, c, s);
        }
    }

    /// Ticks with a bright stretch sliding along them ("working on it").
    pub fn ticks_sweep(&mut self, r: Rect, color: Color) {
        let (n, step, w) = tick_geom(r.w);
        let color = themed(color);
        let phase = (self.now.elapsed().as_secs_f32() + epoch_secs()) % 1.6 / 1.6;
        let head = phase * (n as f32 + 8.0) - 4.0;
        for i in 0..n {
            let t = (1.0 - (head - i as f32).abs() / 4.0).clamp(0.0, 1.0);
            let mut c = TICK_DIM.lerp(color, t);
            let shine = t * t;
            let tint = if white_beams() { palette::BLUE } else { Color::white(1.0) };
            c = c.lerp(tint.with_a(c.a), shine * 0.9);
            self.beam(r.x + i as f32 * step + (step - w) * 0.5, r.y, w, r.h, c, shine);
        }
        self.fx.animate = true;
    }

    /// Mini bar graph of recent values (0..1, oldest first): one tick per
    /// sample, taller for higher values, older ones fading out. New values
    /// glide into place.
    pub fn tick_graph(&mut self, r: Rect, values: &[f32], color: Color) {
        let n = values.len().max(1);
        let step = r.w / n as f32;
        let w = (step * 0.56).max(1.4);
        let color = themed(color);
        for (i, v) in values.iter().enumerate() {
            let v = self.glide(rect_key(r, "graph") ^ gkey("g", i), v.clamp(0.0, 1.0));
            let h = (v * r.h).max(2.0);
            let age = (i + 1) as f32 / n as f32;
            let mut c = TICK_DIM.lerp(color, 0.25 + 0.75 * age);
            if white_beams() {
                // white style: the newest samples are blue
                c = c.lerp(palette::BLUE.with_a(c.a), ((age - 0.55) / 0.45).clamp(0.0, 1.0));
            }
            if !white_beams() {
                // the newest samples shine white, like the leading edge of a gauge
                c = c.lerp(Color::white(1.0).with_a(c.a), ((age - 0.8) / 0.2).clamp(0.0, 1.0).powi(2) * 0.8);
            }
            self.p.fill_rounded(Rect::new(r.x + i as f32 * step + (step - w) * 0.5, r.bottom() - h, w, h), w * 0.5, c);
        }
    }

    /// A short faint run of ticks, used as a section rule or header accent.
    pub fn tick_rule(&mut self, r: Rect, alpha: f32) {
        let (n, step, w) = tick_geom(r.w);
        let c = Color::white(alpha);
        for i in 0..n {
            self.p.fill_rounded(Rect::new(r.x + i as f32 * step + (step - w) * 0.5, r.y, w, r.h), w * 0.5, c);
        }
    }

    /// Progress bar, drawn as ticks (the rect's height only sets how tall they are).
    pub fn progress(&mut self, r: Rect, frac: f32, color: Color) {
        let h = (r.h * 2.6).clamp(8.0, 14.0);
        self.ticks(Rect::new(r.x, r.cy() - h * 0.5, r.w, h), frac, color);
    }

    /// Indeterminate progress: a bright stretch of ticks sliding along.
    pub fn progress_indeterminate(&mut self, r: Rect, color: Color) {
        let h = (r.h * 2.6).clamp(8.0, 14.0);
        self.ticks_sweep(Rect::new(r.x, r.cy() - h * 0.5, r.w, h), color);
    }

    /// A record: dark disc, faint grooves, two sheen arcs that turn with it
    /// and a coloured label carrying the music note.
    pub fn vinyl(&mut self, r: Rect, angle: f32, label: Color, art: Option<&ImageData>) {
        let (cx, cy) = (r.cx(), r.cy());
        let rad = r.w.min(r.h) * 0.5;
        self.p.fill_circle(cx, cy, rad, Color::hex(0x0C0C0E));
        self.p.stroke_circle(cx, cy, rad - 0.5, 1.0, Color::white(0.16));
        for k in 1..=4 {
            self.p.stroke_circle(cx, cy, rad * (0.40 + 0.12 * k as f32), 0.6, Color::white(0.05));
        }
        let sheen = Color::white(0.08);
        self.p.arc(cx, cy, rad * 0.70, angle, 0.75, rad * 0.30, sheen);
        self.p.arc(cx, cy, rad * 0.70, angle + std::f32::consts::PI, 0.75, rad * 0.30, sheen);
        match art {
            // the cover is the record's label and turns with it
            Some(img) => {
                let lr = rad * 0.46;
                self.p.image_disc(img, cx, cy, lr, angle);
                self.p.stroke_circle(cx, cy, lr, 1.0, Color::black(0.55));
                self.p.fill_circle(cx, cy, rad * 0.055, Color::hex(0x0C0C0E));
                self.p.stroke_circle(cx, cy, rad * 0.055, 0.8, Color::white(0.25));
            }
            None => {
                let lr = rad * 0.38;
                self.p.fill_circle(cx, cy, lr, label.lerp(Color::black(1.0), 0.55));
                self.p.fill_circle(
                    cx + lr * 0.72 * angle.cos(),
                    cy + lr * 0.72 * angle.sin(),
                    rad * 0.035,
                    Color::white(0.45),
                );
                self.p.icon(Icon::Music, Rect::new(cx - lr * 0.5, cy - lr * 0.5, lr, lr), Color::white(0.95));
            }
        }
    }

    /// Scrubber for the media player: a row of ticks that fills as the track
    /// plays (the last few warm towards yellow), with a tall playhead on the edge.
    pub fn scrubber(&mut self, r: Rect, frac: f32, hover: bool) {
        let h = if hover { 16.0 } else { 14.0 };
        let frac = self.glide(rect_key(r, "scrub"), frac.clamp(0.0, 1.0));
        self.ticks_raw(Rect::new(r.x, r.cy() - h * 0.5, r.w, h), frac, Color::white(0.92), true);
        let px = r.x + 2.0 + (r.w - 4.0) * frac.clamp(0.0, 1.0);
        let ph = h + 8.0;
        self.p.fill_rounded(Rect::new(px - 2.2, r.cy() - ph * 0.5 + 1.0, 4.4, ph), 2.2, Color::black(0.35));
        self.p.fill_rounded(Rect::new(px - 1.8, r.cy() - ph * 0.5, 3.6, ph), 1.8, Color::white(0.97));
    }

    /// Rounded card background, highlighted on hover when `interactive`.
    pub fn card(&mut self, r: Rect, interactive: bool) -> bool {
        let hover = interactive && self.hovered(r);
        if hover {
            self.hot = true;
        }
        // flat on black: no background until hovered
        if hover {
            self.p.fill_rounded(r, 18.0, Color::white(0.05));
        }
        interactive && self.clicked(r)
    }

    /// Flat list row: no background, a hairline underneath and a soft
    /// highlight when hovered, or `tint` for the row that is in use.
    pub fn row_bg(&mut self, row: Rect, hover: bool, tint: Option<Color>) {
        match tint {
            Some(t) => self.p.fill_rounded(row, 14.0, t),
            None => {
                if hover {
                    self.p.fill_rounded(row, 14.0, Color::white(0.05));
                }
                self.p.fill_rect(
                    Rect::new(row.x + 12.0, row.bottom() - 0.5, (row.w - 24.0).max(0.0), 1.0),
                    Color::white(0.06),
                );
            }
        }
    }

    pub fn label(&mut self, s: &str, r: Rect, style: TextStyle) {
        self.p.text(s, r, &style);
    }

    /// Section header: small dim caps text.
    pub fn caption(&mut self, _s: &str, _r: Rect) {
        // section labels are gone: the content speaks for itself
    }

    /// Empty-state placeholder: icon + message centered in `r`.
    pub fn empty_state(&mut self, r: Rect, icon: Icon, msg: &str) {
        let is = 26.0;
        let total = is + 8.0 + 18.0;
        let top = r.cy() - total * 0.5;
        self.p.icon(icon, Rect::new(r.cx() - is * 0.5, top, is, is), palette::TEXT_FAINT);
        self.p.text(msg, Rect::new(r.x, top + is + 8.0, r.w, 18.0), &TextStyle::new(12.5, palette::TEXT_DIM).center());
    }
}

/// Seconds since process start, used to keep looping animations continuous.
pub fn epoch_secs() -> f32 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}
