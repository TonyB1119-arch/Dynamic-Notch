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
    /// Sideways wheel / touchpad swipe this frame (positive = right).
    pub hwheel: f32,
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
    /// Set when something on the open page used the wheel this frame (a list that scrolls).
    static WHEEL_TAKEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Did a part of the page use the wheel this frame?
pub fn wheel_taken() -> bool {
    WHEEL_TAKEN.with(|t| t.get())
}

pub fn reset_wheel_taken() {
    WHEEL_TAKEN.with(|t| t.set(false));
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

thread_local! {
    /// True while a caller has asked for the beam look (see `Ui::beams`). Everywhere else the same
    /// gauges, sweeps and rings are drawn as plain smooth lines.
    static BEAMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Overall look: 0 = beams everywhere, 1 = calm, 2 = minimal, 3 = nothing, 4 = bloom, 5 = alcove.
static LOOK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Applies `[appearance] look`. Called when the settings load or change.
pub fn set_look(name: &str) {
    let v = match name.trim().to_ascii_lowercase().as_str() {
        "calm" => 1,
        "minimal" => 2,
        "nothing" => 3,
        "bloom" => 4,
        "alcove" => 5,
        _ => 0,
    };
    LOOK.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// True in the minimal look: thin plain lines, no glow, no swell.
pub fn look_minimal() -> bool {
    LOOK.load(std::sync::atomic::Ordering::Relaxed) == 2
}

/// True in the Nothing look: monochrome, dot-matrix "glyph" lights with a single red accent.
pub fn look_nothing() -> bool {
    LOOK.load(std::sync::atomic::Ordering::Relaxed) == 3
}

/// True in the Bloom look: soft capsule gauges, bordered translucent cards, a clear blue accent.
pub fn look_bloom() -> bool {
    matches!(LOOK.load(std::sync::atomic::Ordering::Relaxed), 4 | 5)
}

/// True in the Alcove look: Bloom's capsule gauges inside Alcove's island layouts.
pub fn look_alcove() -> bool {
    LOOK.load(std::sync::atomic::Ordering::Relaxed) == 5
}

/// Monochrome icons (`[appearance] mono_icons`): any coloured icon is drawn white.
static MONO_ICONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_mono_icons(on: bool) {
    MONO_ICONS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn mono_icons() -> bool {
    MONO_ICONS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Icon badges are rounded squares (`[appearance] icon_square`) instead of circles.
static ICON_SQUARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_icon_square(on: bool) {
    ICON_SQUARE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn icon_square() -> bool {
    ICON_SQUARE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The accent of the Bloom look (iOS blue).
pub const BLOOM_BLUE: Color = Color::hex(0x0A84FF);

/// The one accent colour of the Nothing look.
pub const NOTHING_RED: Color = Color::hex(0xD71921);

fn look_beams() -> bool {
    LOOK.load(std::sync::atomic::Ordering::Relaxed) == 0
}

fn beams_on() -> bool {
    look_beams() || BEAMS.with(|b| b.get())
}

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
/// Bloom gauges are white; a colour that carries meaning (a warning red, a charging green) tints them.
fn bloom_tint(c: Color) -> Color {
    let a = c.a.clamp(0.0, 1.0);
    let sat = (c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b)).clamp(0.0, 1.0);
    Color::white(0.97).lerp(c, (sat * 0.8).min(0.75)).with_a(a)
}

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
            self.fx.nudge = true;
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
            let w = self.input.wheel;
            if w != 0.0 {
                WHEEL_TAKEN.with(|t| t.set(true));
            }
            w
        } else {
            0.0
        }
    }

    pub fn animate(&mut self) {
        self.fx.animate = true;
    }

    /// Like `animate`, for slow, gentle motion: drawn at about 30 frames a second.
    pub fn animate_slow(&mut self) {
        self.fx.animate_slow = true;
    }

    // -- widgets --------------------------------------------------------------

    /// Eased hover and press amounts (0..1) for a control, so highlights fade and presses settle
    /// instead of snapping. Also marks the pointer as over something clickable.
    pub fn touch(&mut self, r: Rect, tag: &str) -> (f32, f32) {
        let (hover, pressed) = (self.hovered(r), self.pressed(r));
        if hover {
            self.hot = true;
        }
        let key = rect_key(r, tag);
        let h = self.glide(key, hover as u8 as f32);
        let p = self.glide(key ^ 0x9e37, pressed as u8 as f32);
        (h, p)
    }

    /// Round icon button. Returns true when clicked.
    pub fn icon_button(&mut self, r: Rect, icon: Icon, style: ButtonStyle) -> bool {
        let (h, p) = self.touch(r, "icon-button");
        let bg = style.bg.lerp(style.hover_bg, h);
        let rr = r.inset(r.w * 0.07 * p);
        self.p.fill_rounded(rr, style.radius, bg);
        let s = r.w.min(r.h) * style.icon_scale * (1.0 - 0.14 * p) * (1.0 + 0.03 * h);
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
        let rr = if pressed { r.inset(2.0) } else { r };
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
        if !beams_on() {
            return self.line_ring(cx, cy, diameter, stroke, frac, color);
        }
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
        if !beams_on() {
            return self.line_ring_spin(cx, cy, diameter, stroke, color);
        }
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

    /// Run `f` with the beam look switched on (it is on everywhere by default; the non-beam looks
    /// below are kept for a future "beams: data only" option).
    pub fn beams<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        if look_minimal() || look_nothing() || look_bloom() {
            return f(self);
        }
        let old = BEAMS.with(|b| b.replace(true));
        let r = f(self);
        BEAMS.with(|b| b.set(old));
        r
    }

    // ---- minimal look: thin plain lines, one colour, no glow or swell ----

    /// A thin line from `from` to `to` (0..1) over a faint rail.
    fn min_bar(&mut self, r: Rect, from: f32, to: f32, color: Color) {
        let t = 2.0;
        let y = r.cy() - t * 0.5;
        self.p.fill_rounded_live(Rect::new(r.x, y, r.w, t), t * 0.5, Color::white(0.14));
        let (x0, x1) = (r.x + r.w * from.clamp(0.0, 1.0), r.x + r.w * to.clamp(0.0, 1.0));
        if x1 - x0 > 0.5 {
            let a = 0.92 * color.a.clamp(0.0, 1.0);
            self.p.fill_rounded_live(Rect::new(x0, y, (x1 - x0).max(t), t), t * 0.5, Color::white(a));
        }
    }

    /// "Working on it": one short line gliding back and forth along the rail with eased ends.
    fn min_sweep(&mut self, r: Rect) {
        let t = 2.0;
        let y = r.cy() - t * 0.5;
        self.p.fill_rounded_live(Rect::new(r.x, y, r.w, t), t * 0.5, Color::white(0.14));
        let phase = ((self.now.elapsed().as_secs_f32() + epoch_secs()) % 2.0) / 2.0;
        let e = 0.5 - 0.5 * (std::f32::consts::PI * phase).cos();
        let seg = (r.w * 0.28).max(8.0);
        let x = r.x + (r.w - seg) * e;
        self.p.fill_rounded_live(Rect::new(x, y, seg, t), t * 0.5, Color::white(0.85));
        self.fx.animate = true;
    }

    /// A thin ring that fills clockwise from 12 o'clock.
    fn min_ring(&mut self, cx: f32, cy: f32, diameter: f32, frac: f32, color: Color) {
        let w = 1.6;
        let rad = (diameter * 0.5 - w).max(1.0);
        self.p.stroke_circle(cx, cy, rad, w, Color::white(0.14));
        if frac > 0.003 {
            let a = 0.92 * color.a.clamp(0.0, 1.0);
            self.p.arc(cx, cy, rad, 0.0, std::f32::consts::TAU * frac.clamp(0.0, 1.0), w, Color::white(a));
        }
    }

    /// A thin ring with one short arc turning steadily.
    fn min_ring_spin(&mut self, cx: f32, cy: f32, diameter: f32) {
        let w = 1.6;
        let rad = (diameter * 0.5 - w).max(1.0);
        self.p.stroke_circle(cx, cy, rad, w, Color::white(0.14));
        let start = (epoch_secs() * 0.8).fract() * std::f32::consts::TAU;
        self.p.arc(cx, cy, rad, start, 1.3, w, Color::white(0.85));
        self.fx.animate = true;
    }

    // ---- Bloom look: soft capsules, thick rounded rings, white on a faint track ----

    /// A capsule from `from` to `to` (0..1) over a faint track.
    fn bloom_bar(&mut self, r: Rect, from: f32, to: f32, color: Color) {
        let t = (r.h * 0.55).clamp(5.0, 9.0).min(r.h.max(5.0));
        let track = Rect::new(r.x, r.cy() - t * 0.5, r.w, t);
        self.p.fill_rounded_live(track, t * 0.5, Color::white(0.18));
        let (x0, x1) = (r.x + r.w * from.clamp(0.0, 1.0), r.x + r.w * to.clamp(0.0, 1.0));
        if x1 - x0 > 0.5 {
            let w = (x1 - x0).max(t.min(r.w));
            let c = bloom_tint(color);
            self.p.fill_rounded_live(Rect::new(x0, track.y, w, t), t * 0.5, c);
        }
    }

    /// "Working on it": a soft capsule gliding to and fro along the track.
    fn bloom_sweep(&mut self, r: Rect) {
        let t = (r.h * 0.55).clamp(5.0, 9.0).min(r.h.max(5.0));
        let track = Rect::new(r.x, r.cy() - t * 0.5, r.w, t);
        self.p.fill_rounded_live(track, t * 0.5, Color::white(0.14));
        let seg = (r.w * 0.3).max(t * 2.0).min(r.w);
        let ping = 0.5 - 0.5 * ((epoch_secs() % 1.8) / 1.8 * std::f32::consts::TAU).cos();
        let x = r.x + ping * (r.w - seg);
        self.p.fill_rounded_live(Rect::new(x, track.y, seg, t), t * 0.5, Color::white(0.92));
        self.fx.animate = true;
    }

    /// A thick ring with round ends, filling clockwise from 12 o'clock.
    fn bloom_ring(&mut self, cx: f32, cy: f32, diameter: f32, frac: f32, color: Color) {
        let w = (diameter * 0.14).clamp(2.5, 5.0);
        let rad = (diameter * 0.5 - w * 0.5).max(1.0);
        self.p.stroke_circle(cx, cy, rad, w, Color::white(0.14));
        if frac > 0.004 {
            let sweep = (std::f32::consts::TAU * frac.clamp(0.0, 1.0)).min(std::f32::consts::TAU - 0.002);
            self.p.arc(cx, cy, rad, 0.0, sweep, w, bloom_tint(color));
        }
    }

    /// A thick ring with one soft arc turning around it.
    fn bloom_ring_spin(&mut self, cx: f32, cy: f32, diameter: f32) {
        let w = (diameter * 0.14).clamp(2.5, 5.0);
        let rad = (diameter * 0.5 - w * 0.5).max(1.0);
        self.p.stroke_circle(cx, cy, rad, w, Color::white(0.14));
        let start = (epoch_secs() * 0.8).fract() * std::f32::consts::TAU;
        self.p.arc(cx, cy, rad, start, 1.5, w, Color::white(0.92));
        self.fx.animate = true;
    }

    // ---- Nothing look: dot-matrix "glyph" lights, white on black with one red accent ----

    /// A row of LED dots. `from`..`to` (0..1) are lit white; the leading dot is red.
    fn glyph_bar(&mut self, r: Rect, from: f32, to: f32, color: Color) {
        let pitch = if r.w < 60.0 { 4.6 } else { 6.0 };
        let n = ((r.w / pitch).floor() as usize).max(3);
        let step = r.w / n as f32;
        let dot = (step * 0.26).clamp(1.1, 1.8);
        let (from, to) = (from.clamp(0.0, 1.0) * n as f32, to.clamp(0.0, 1.0) * n as f32);
        let a = color.a.clamp(0.0, 1.0);
        let head = if to > from + 0.01 && to < n as f32 - 0.01 { Some((to - 0.001).floor() as usize) } else { None };
        for i in 0..n {
            let t = (to - i as f32).clamp(0.0, 1.0) * (i as f32 + 1.0 - from).clamp(0.0, 1.0);
            let lit = if Some(i) == head { NOTHING_RED } else { Color::white(0.95) };
            let c = Color::white(0.13).lerp(lit, t).with_a((0.13 + 0.82 * t) * if t > 0.0 { a } else { 1.0 });
            self.p.fill_circle(r.x + step * (i as f32 + 0.5), r.cy(), dot, c);
        }
    }

    /// "Working on it": a short comet of LED dots that steps along the row, red at the head.
    fn glyph_sweep(&mut self, r: Rect) {
        let pitch = if r.w < 60.0 { 4.6 } else { 6.0 };
        let n = ((r.w / pitch).floor() as usize).max(3);
        let step = r.w / n as f32;
        let dot = (step * 0.26).clamp(1.1, 1.8);
        let head = ((epoch_secs() * 14.0) as usize % (n + 5)) as i32;
        for i in 0..n {
            let back = head - i as i32;
            let c = match back {
                0 => NOTHING_RED,
                1 => Color::white(0.9),
                2 => Color::white(0.45),
                3 => Color::white(0.22),
                _ => Color::white(0.13),
            };
            self.p.fill_circle(r.x + step * (i as f32 + 0.5), r.cy(), dot, c);
        }
        self.fx.animate = true;
    }

    /// A circle of LED dots that light clockwise from 12 o'clock, red at the head.
    fn glyph_ring(&mut self, cx: f32, cy: f32, diameter: f32, frac: f32, color: Color) {
        let (n, rad, dot) = Self::dot_ring_geom(diameter);
        let lit_n = frac.clamp(0.0, 1.0) * n as f32;
        let head = if lit_n > 0.01 && lit_n < n as f32 - 0.01 { Some((lit_n - 0.001).floor() as usize) } else { None };
        let a = color.a.clamp(0.0, 1.0);
        for i in 0..n {
            let t = (lit_n - i as f32).clamp(0.0, 1.0);
            let lit = if Some(i) == head { NOTHING_RED } else { Color::white(0.95) };
            let c = Color::white(0.13).lerp(lit, t).with_a((0.13 + 0.82 * t) * if t > 0.0 { a } else { 1.0 });
            let ang = std::f32::consts::TAU * i as f32 / n as f32;
            self.p.fill_circle(cx + rad * ang.sin(), cy - rad * ang.cos(), dot, c);
        }
    }

    /// A ring of LED dots with a red head and a short white tail, stepping around.
    fn glyph_ring_spin(&mut self, cx: f32, cy: f32, diameter: f32) {
        let (n, rad, dot) = Self::dot_ring_geom(diameter);
        let head = ((epoch_secs() * 0.9).fract() * n as f32) as i32;
        for i in 0..n as i32 {
            let back = (head - i).rem_euclid(n as i32);
            let c = match back {
                0 => NOTHING_RED,
                1 => Color::white(0.9),
                2 => Color::white(0.5),
                3 => Color::white(0.25),
                _ => Color::white(0.13),
            };
            let ang = std::f32::consts::TAU * i as f32 / n as f32;
            self.p.fill_circle(cx + rad * ang.sin(), cy - rad * ang.cos(), dot, c);
        }
        self.fx.animate = true;
    }

    /// Tab or page marker: a short line (beams in the beam look, a plain line otherwise).
    pub fn underline(&mut self, r: Rect, c: Color) {
        if look_nothing() {
            self.p.fill_circle(r.cx(), r.cy(), 2.0, NOTHING_RED.with_a(c.a));
        } else if beams_on() {
            self.ticks(r, 1.0, c);
        } else {
            let h = if look_minimal() { 2.0 } else { r.h };
            self.p.fill_rounded_live(Rect::new(r.x, r.y + (r.h - h) * 0.5, r.w, h), h * 0.5, c);
        }
    }

    /// Gauge without beams: pill segments for a small meter and a glow bar for anything wider.
    /// The lit part shines white at its leading edge and swells a little there.
    fn line_bar(&mut self, r: Rect, frac: f32, color: Color) {
        if look_nothing() {
            return self.glyph_bar(r, 0.0, frac, color);
        }
        if look_minimal() {
            return self.min_bar(r, 0.0, frac, color);
        }
        if look_bloom() {
            return self.bloom_bar(r, 0.0, frac, color);
        }
        let frac = frac.clamp(0.0, 1.0);
        if r.w <= 80.0 {
            let n = ((r.w / 10.0).round() as usize).clamp(3, 8);
            self.draw_pills(r, frac, color, n);
        } else {
            self.glow_span(r, 0.0, frac, color);
        }
    }

    /// A row of pill segments that fill in; the leading ones shine white and stand taller.
    fn draw_pills(&mut self, r: Rect, frac: f32, color: Color, n: usize) {
        let lit = themed(color);
        let dim = Color::white(0.13);
        let gap = 3.0;
        let sw = (r.w - gap * (n as f32 - 1.0)) / n as f32;
        let h = (r.h * 0.55).clamp(4.0, 7.0);
        for i in 0..n {
            let behind = frac * n as f32 - i as f32;
            let t = smooth01(behind);
            let s = Self::lead(behind, 3.5);
            let c = dim.lerp(lit, t).lerp(Color::white(1.0).with_a(lit.a), s);
            let ph = h + 5.0 * s;
            self.p.fill_rounded_live(Rect::new(r.x + i as f32 * (sw + gap), r.cy() - ph * 0.5, sw, ph), ph * 0.5, c);
        }
    }

    /// Pill segments with a chosen count (used where a bar should read as chunky steps).
    pub fn pills(&mut self, r: Rect, frac: f32, color: Color, n: usize) {
        self.draw_pills(r, frac.clamp(0.0, 1.0), color, n.max(2));
    }

    /// A soft glow bar: a rail, the lit stretch from `from` to `to` (0..1 of the width) with a faint
    /// halo, and a leading end that shines white and swells.
    fn glow_span(&mut self, r: Rect, from: f32, to: f32, color: Color) {
        let t = (r.h * 0.5).clamp(3.5, 6.0);
        let y = r.cy() - t * 0.5;
        self.p.fill_rounded_live(Rect::new(r.x, y, r.w, t), t * 0.5, Color::white(0.10));
        let (x0, x1) = (r.x + r.w * from.clamp(0.0, 1.0), r.x + r.w * to.clamp(0.0, 1.0));
        if x1 - x0 > 0.5 {
            let fw = (x1 - x0).max(t);
            let end = x0 + fw;
            let lit = themed(color);
            self.p.fill_rounded_live(
                Rect::new(x0 - 1.0, y - 2.0, fw + 2.0, t + 4.0),
                (t + 4.0) * 0.5,
                lit.with_a(lit.a * 0.16),
            );
            self.p.fill_rounded_live(Rect::new(x0, y, fw, t), t * 0.5, lit);
            // the leading end: one tapered shape with a round cap, shaded by a real gradient from
            // the bar colour to white, so it is perfectly smooth
            let hmax = t + 3.0;
            let reach = (fw * 0.85).min(90.0).max(hmax * 1.4);
            let (xs, rc) = (end - reach, hmax * 0.5);
            let xc = end - rc;
            let mut pts = PolyBuf::new();
            for i in 0..=14 {
                let u = i as f32 / 14.0;
                let e = u.powf(2.6);
                pts.push((xs + (xc - xs) * u, r.cy() - (t + (hmax - t) * e) * 0.5));
            }
            for i in 1..10 {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 10.0;
                pts.push((xc + rc * a.cos(), r.cy() + rc * a.sin()));
            }
            for i in (0..=14).rev() {
                let u = i as f32 / 14.0;
                let e = u.powf(2.6);
                pts.push((xs + (xc - xs) * u, r.cy() + (t + (hmax - t) * e) * 0.5));
            }
            let white = Color::white(1.0).with_a(lit.a);
            let stops =
                [(0.0, lit.with_a(0.0)), (0.45, lit.lerp(white, 0.25)), (0.8, lit.lerp(white, 0.75)), (1.0, white)];
            self.p.fill_gradient_poly(pts.get(), &stops, xs, end);
        }
    }

    /// "Working on it" without beams: a soft shimmer sliding along a rail, whitest and largest in
    /// the middle of the stretch.
    fn line_sweep(&mut self, r: Rect, color: Color) {
        if look_nothing() {
            return self.glyph_sweep(r);
        }
        if look_minimal() {
            return self.min_sweep(r);
        }
        if look_bloom() {
            return self.bloom_sweep(r);
        }
        let t = (r.h * 0.3).clamp(2.5, 3.5);
        let y = r.cy() - t * 0.5;
        self.p.fill_rounded_live(Rect::new(r.x, y, r.w, t), t * 0.5, Color::white(0.12));
        let phase = (self.now.elapsed().as_secs_f32() + epoch_secs()) % 1.6 / 1.6;
        let seg = r.w * 0.34;
        let mid = r.x - seg * 0.5 + phase * (r.w + seg);
        let base = themed(color);
        // a soft lens-shaped glow with a gradient from the bar colour to white at its centre
        let half = seg * 0.5;
        let (lo, hi) = ((mid - half).max(r.x), (mid + half).min(r.right()));
        if hi - lo > 1.0 {
            let n = 16;
            let bell = |x: f32| {
                let k = (1.0 - (x - mid).abs() / half).clamp(0.0, 1.0);
                (t + 3.0 * k * k * (3.0 - 2.0 * k)) * 0.5
            };
            let mut pts = PolyBuf::new();
            for i in 0..=n {
                let x = lo + (hi - lo) * i as f32 / n as f32;
                pts.push((x, r.cy() - bell(x)));
            }
            for i in (0..=n).rev() {
                let x = lo + (hi - lo) * i as f32 / n as f32;
                pts.push((x, r.cy() + bell(x)));
            }
            let white = Color::white(1.0).with_a(base.a);
            let stops = [
                (0.0, base.with_a(0.0)),
                (0.25, base.with_a(base.a * 0.75)),
                (0.5, base.lerp(white, 0.9)),
                (0.75, base.with_a(base.a * 0.75)),
                (1.0, base.with_a(0.0)),
            ];
            self.p.fill_gradient_poly(pts.get(), &stops, mid - half, mid + half);
        }
        self.fx.animate = true;
    }

    /// A range without beams: a glow range with a white, swollen leading end.
    fn line_range(&mut self, r: Rect, from: f32, to: f32, color: Color) {
        if look_nothing() {
            return self.glyph_bar(r, from, to, color);
        }
        if look_minimal() {
            return self.min_bar(r, from, to, color);
        }
        if look_bloom() {
            return self.bloom_bar(r, from, to, color);
        }
        self.glow_span(r, from, to, color);
    }

    /// Ring without beams: a circle of dots that light up clockwise from 12 o'clock. The leading
    /// dots brighten toward white.
    fn line_ring(&mut self, cx: f32, cy: f32, diameter: f32, _stroke: f32, frac: f32, color: Color) {
        if look_nothing() {
            return self.glyph_ring(cx, cy, diameter, frac, color);
        }
        if look_minimal() {
            return self.min_ring(cx, cy, diameter, frac, color);
        }
        if look_bloom() {
            return self.bloom_ring(cx, cy, diameter, frac, color);
        }
        let frac = frac.clamp(0.0, 1.0);
        let (n, rad, dot) = Self::dot_ring_geom(diameter);
        let lit = themed(color);
        let dim = Color::white(0.13);
        for i in 0..n {
            let behind = frac * n as f32 - i as f32;
            let t = smooth01(behind);
            let s = Self::lead(behind, 5.0);
            let c = dim.lerp(lit, t).lerp(Color::white(1.0).with_a(lit.a), s);
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            self.p.fill_circle(cx + rad * a.sin(), cy - rad * a.cos(), dot, c);
        }
    }

    /// Dot count, ring radius and dot radius for a dot ring of this diameter.
    fn dot_ring_geom(diameter: f32) -> (usize, f32, f32) {
        let spacing = (diameter * 0.16).clamp(4.2, 7.0);
        let dot = spacing * 0.3;
        let rad = (diameter * 0.5 - dot - 0.5).max(2.0);
        let n = ((std::f32::consts::TAU * rad / spacing).round() as usize).clamp(10, 48);
        (n, rad, dot)
    }

    /// Spinning ring without beams: a dot ring with a bright head and a fading tail, circling.
    fn line_ring_spin(&mut self, cx: f32, cy: f32, diameter: f32, _stroke: f32, color: Color) {
        if look_nothing() {
            return self.glyph_ring_spin(cx, cy, diameter);
        }
        if look_minimal() {
            return self.min_ring_spin(cx, cy, diameter);
        }
        if look_bloom() {
            return self.bloom_ring_spin(cx, cy, diameter);
        }
        let turn = (epoch_secs() * 0.9).fract();
        let (n, rad, dot) = Self::dot_ring_geom(diameter);
        let lit = themed(color);
        let dim = Color::white(0.13);
        for i in 0..n {
            // distance of this dot behind the head, in dots
            let behind = (turn * n as f32 - i as f32).rem_euclid(n as f32);
            let t = (1.0 - behind / (n as f32 * 0.35)).clamp(0.0, 1.0);
            let c = dim.lerp(lit, t).lerp(Color::white(1.0).with_a(lit.a), t * t);
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            self.p.fill_circle(cx + rad * a.sin(), cy - rad * a.cos(), dot, c);
        }
        self.fx.animate = true;
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
    /// Shine for an element `behind` steps behind the moving edge: full on the newest lit one,
    /// fading smoothly over `span` steps, and zero for anything not lit yet.
    fn lead(behind: f32, span: f32) -> f32 {
        let k = (1.0 - (behind - 1.0).max(0.0) / span).clamp(0.0, 1.0);
        k * k * smooth01(behind)
    }

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
        self.beams(|ui| ui.ticks_ex(r, frac, Color::white(1.0), true));
    }

    /// Like `ticks_ex` but draws `frac` exactly as given (the caller smooths it).
    fn ticks_raw(&mut self, r: Rect, frac: f32, color: Color, _warm: bool) {
        if !beams_on() {
            return self.line_bar(r, frac, color);
        }
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
        if !beams_on() {
            return self.line_range(r, from, to, color);
        }
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
        if !beams_on() {
            return self.line_sweep(r, color);
        }
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
        if look_nothing() {
            // columns of LED dots, one per sample, newest in red at the top of its column
            let n = values.len().max(1);
            let step = r.w / n as f32;
            let dot = (step * 0.28).clamp(0.9, 1.6);
            let pitch = dot * 3.2;
            let rows = ((r.h / pitch).floor() as usize).max(2);
            for (i, v) in values.iter().enumerate() {
                let v = self.glide(rect_key(r, "graph") ^ gkey("g", i), v.clamp(0.0, 1.0));
                let lit = (v * rows as f32).round() as usize;
                let x = r.x + step * (i as f32 + 0.5);
                for k in 0..rows {
                    let y = r.bottom() - pitch * (k as f32 + 0.5);
                    let c = if k < lit {
                        if i + 1 == n && k + 1 == lit {
                            NOTHING_RED
                        } else {
                            Color::white(0.3 + 0.65 * (i + 1) as f32 / n as f32)
                        }
                    } else {
                        Color::white(0.08)
                    };
                    self.p.fill_circle(x, y, dot, c);
                }
            }
            return;
        }
        if look_bloom() {
            let n = values.len().max(1);
            let step = r.w / n as f32;
            let w = (step * 0.62).max(1.8);
            let color = bloom_tint(color);
            for (i, v) in values.iter().enumerate() {
                let v = self.glide(rect_key(r, "graph") ^ gkey("g", i), v.clamp(0.0, 1.0));
                let h = (v * r.h).max(w);
                let age = (i + 1) as f32 / n as f32;
                let c = Color::white(0.2).lerp(color, 0.3 + 0.7 * age);
                self.p.fill_rounded(
                    Rect::new(r.x + i as f32 * step + (step - w) * 0.5, r.bottom() - h, w, h),
                    w * 0.5,
                    c,
                );
            }
            return;
        }
        let n = values.len().max(1);
        let step = r.w / n as f32;
        let minimal = look_minimal();
        let w = if minimal { (step * 0.3).max(1.2) } else { (step * 0.56).max(1.4) };
        let color = themed(color);
        for (i, v) in values.iter().enumerate() {
            let v = self.glide(rect_key(r, "graph") ^ gkey("g", i), v.clamp(0.0, 1.0));
            let h = (v * r.h).max(2.0);
            let age = (i + 1) as f32 / n as f32;
            let mut c = TICK_DIM.lerp(color, 0.25 + 0.75 * age);
            if minimal {
                c = Color::white(0.2 + 0.7 * age);
            } else if white_beams() {
                // white style: the newest samples are blue
                c = c.lerp(palette::BLUE.with_a(c.a), ((age - 0.55) / 0.45).clamp(0.0, 1.0));
            }
            if !minimal && !white_beams() {
                // the newest samples shine white, like the leading edge of a gauge
                c = c.lerp(Color::white(1.0).with_a(c.a), ((age - 0.8) / 0.2).clamp(0.0, 1.0).powi(2) * 0.8);
            }
            self.p.fill_rounded(Rect::new(r.x + i as f32 * step + (step - w) * 0.5, r.bottom() - h, w, h), w * 0.5, c);
        }
    }

    /// A short faint run of ticks, used as a section rule or header accent.
    pub fn tick_rule(&mut self, r: Rect, alpha: f32) {
        if look_nothing() {
            let n = ((r.w / 6.0) as usize).max(2);
            for i in 0..n {
                self.p.fill_circle(r.x + r.w * (i as f32 + 0.5) / n as f32, r.cy(), 1.0, Color::white(alpha * 1.4));
            }
            return;
        }
        if !beams_on() {
            self.p.fill_rounded(r, r.h * 0.5, Color::white(alpha));
            return;
        }
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
        if look_nothing() {
            return self.glyph_sweep(r);
        }
        if look_minimal() && !beams_on() {
            return self.min_sweep(r);
        }
        if look_bloom() {
            return self.bloom_sweep(Rect::new(r.x, r.cy() - 3.0, r.w, 6.0));
        }
        if !beams_on() {
            // a single dot that breathes
            let pulse = 0.5 + 0.5 * (epoch_secs() * std::f32::consts::TAU / 1.4).sin();
            let base = themed(color);
            self.p.fill_circle(r.cx(), r.cy(), 4.5 + 2.0 * pulse, base.with_a(base.a * 0.18));
            self.p.fill_circle(r.cx(), r.cy(), 3.0 + 1.0 * pulse, base.with_a(base.a * (0.55 + 0.45 * pulse)));
            self.p.fill_circle(r.cx(), r.cy(), 1.4 + 0.8 * pulse, Color::white(0.4 + 0.5 * pulse));
            self.fx.animate = true;
            return;
        }
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
        if look_nothing() {
            let frac = self.glide(rect_key(r, "scrub"), frac.clamp(0.0, 1.0));
            let grow = self.glide(rect_key(r, "scrub-hover"), if hover { 1.0 } else { 0.0 });
            self.glyph_bar(Rect::new(r.x, r.cy() - 3.0, r.w, 6.0), 0.0, frac, Color::white(1.0));
            if grow > 0.02 {
                self.p.fill_circle(r.x + r.w * frac, r.cy(), 2.2 + 1.8 * grow, NOTHING_RED.with_a(grow));
            }
            return;
        }
        if look_minimal() {
            let frac = self.glide(rect_key(r, "scrub"), frac.clamp(0.0, 1.0));
            let grow = self.glide(rect_key(r, "scrub-hover"), if hover { 1.0 } else { 0.0 });
            self.min_bar(Rect::new(r.x, r.cy() - 3.0, r.w, 6.0), 0.0, frac, Color::white(1.0));
            if grow > 0.02 {
                let px = r.x + r.w * frac;
                self.p.fill_circle(px, r.cy(), 2.0 + 2.5 * grow, Color::white(grow));
            }
            return;
        }
        if look_bloom() {
            let frac = self.glide(rect_key(r, "scrub"), frac.clamp(0.0, 1.0));
            let grow = self.glide(rect_key(r, "scrub-hover"), if hover { 1.0 } else { 0.0 });
            let t = 5.0 + 2.0 * grow;
            let track = Rect::new(r.x, r.cy() - t * 0.5, r.w, t);
            self.p.fill_rounded_live(track, t * 0.5, Color::white(0.2));
            let w = (r.w * frac).max(t.min(r.w));
            self.p.fill_rounded_live(Rect::new(r.x, track.y, w, t), t * 0.5, Color::white(0.96));
            if grow > 0.02 {
                let px = r.x + (r.w * frac).clamp(0.0, r.w);
                self.p.fill_circle(px, r.cy() + 1.0, 6.5 * grow, Color::black(0.3 * grow));
                self.p.fill_circle(px, r.cy(), 6.5 * grow, Color::white(grow));
            }
            return;
        }
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
        if look_alcove() {
            // a soft grey tile, no border
            self.p.fill_rounded(r, 20.0, Color::white(if hover { 0.11 } else { 0.07 }));
            return interactive && self.clicked(r);
        }
        if look_bloom() {
            // a bordered, translucent group card
            self.p.fill_rounded(r, 18.0, Color::white(if hover { 0.08 } else { 0.04 }));
            self.p.stroke_rounded(r.inset(0.5), 18.0, 1.0, Color::white(0.1));
            return interactive && self.clicked(r);
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
        if look_alcove() {
            let fill = match tint {
                Some(t) => t,
                None => Color::white(if hover { 0.11 } else { 0.07 }),
            };
            self.p.fill_rounded(row, 14.0, fill);
            return;
        }
        if look_bloom() {
            let fill = match tint {
                Some(t) => t,
                None => Color::white(if hover { 0.08 } else { 0.04 }),
            };
            self.p.fill_rounded(row, 14.0, fill);
            self.p.stroke_rounded(row.inset(0.5), 14.0, 1.0, Color::white(0.08));
            return;
        }
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
        self.p.icon(icon, Rect::new(r.cx() - is * 0.5, top, is, is), Color::white(0.7));
        self.p.text(msg, Rect::new(r.x, top + is + 8.0, r.w, 18.0), &TextStyle::new(12.5, Color::white(0.86)).center());
    }
}

/// Seconds since process start, used to keep looping animations continuous.
pub fn epoch_secs() -> f32 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// 0..1 with eased ends, so a segment fills in softly instead of snapping.
fn smooth01(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Fixed-size point list for the glow shapes, so drawing them never touches the heap.
struct PolyBuf {
    p: [(f32, f32); 48],
    n: usize,
}

impl PolyBuf {
    fn new() -> Self {
        Self { p: [(0.0, 0.0); 48], n: 0 }
    }
    fn push(&mut self, pt: (f32, f32)) {
        if self.n < self.p.len() {
            self.p[self.n] = pt;
            self.n += 1;
        }
    }
    fn get(&self) -> &[(f32, f32)] {
        &self.p[..self.n]
    }
}
