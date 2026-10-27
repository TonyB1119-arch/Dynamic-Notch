//! Calculator: type an expression and the answer appears as you type.
//!
//! Understands + - * / ^ ( ), "mod", percentages ("20% of 85", "200 + 10%"),
//! constants (pi, e), functions (sqrt, sin, cos, tan, ln, log, abs, round …)
//! and unit conversions ("5 km to mi", "72 f to c", "2 gb in mb").

use crate::gfx::{Icon, TextStyle};
use crate::modules::widgets::{scroll, TextField};
use crate::modules::{Cx, Key, Module, ModuleId, Peek};
use crate::sys::clip;
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect, SendHwnd};

const ID: ModuleId = "calc";
const INPUT_H: f32 = 42.0;
const RESULT_H: f32 = 56.0;
const ROW_H: f32 = 32.0;
const LIST_H: f32 = 3.0 * ROW_H;
const MAX_HISTORY: usize = 12;

#[derive(Clone)]
struct Out {
    /// Shown big on the right ("1,024").
    text: String,
    /// Copied to the clipboard and used to continue ("1024").
    plain: String,
}

pub struct Calc {
    input: TextField,
    hwnd: SendHwnd,
    degrees: bool,
    cache_text: String,
    cache_deg: bool,
    cache: Option<Result<Out, String>>,
    history: Vec<(String, Out)>,
    scroll: f32,
}

impl Calc {
    pub fn new() -> Self {
        Self {
            input: TextField::new(120),
            hwnd: SendHwnd(0),
            degrees: true,
            cache_text: String::new(),
            cache_deg: true,
            cache: None,
            history: Vec::new(),
            scroll: 0.0,
        }
    }

    fn refresh(&mut self) {
        let t = self.input.text();
        if t == self.cache_text && self.degrees == self.cache_deg {
            return;
        }
        self.cache = if t.trim().is_empty() { None } else { Some(calculate(&t, self.degrees)) };
        self.cache_text = t;
        self.cache_deg = self.degrees;
    }

    fn commit(&mut self) {
        self.refresh();
        if let Some(Ok(out)) = self.cache.clone() {
            let expr = self.input.text().trim().to_string();
            self.history.insert(0, (expr, out.clone()));
            self.history.truncate(MAX_HISTORY);
            // keep going from the answer, but not for conversions ("3.1 mi")
            if out.plain.parse::<f64>().is_ok() {
                self.input.set(&out.plain);
            }
            self.scroll = 0.0;
        }
    }
}

impl Module for Calc {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Calculator"
    }
    fn icon(&self) -> Icon {
        Icon::Plus
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
    }

    fn glance(&self) -> Option<(Icon, String)> {
        Some((Icon::Plus, "= ?".into()))
    }

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        INPUT_H + 10.0 + RESULT_H + 12.0 + LIST_H
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        if self.input.key(key, self.hwnd) {
            self.commit();
        }
        cx.fx.layout_changed = true;
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let accent = palette::BLUE;
        let toggle = Rect::new(r.right() - 72.0, r.y + 5.0, 72.0, 32.0);
        let field = Rect::new(r.x, r.y, r.w - 80.0, INPUT_H);
        if self.input.draw(ui, field, "Try 2+2, 20% of 85 or 5 km to mi", accent, true) {
            ui.fx.keyboard_focus = Some(true);
        }
        let mode = if self.degrees { "DEG" } else { "RAD" };
        if ui.pill_button(toggle, mode, None, ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
            self.degrees = !self.degrees;
        }
        self.refresh();

        // --- answer -------------------------------------------------------
        let res = Rect::new(r.x, field.bottom() + 10.0, r.w, RESULT_H);
        let mut copy: Option<String> = None;
        match &self.cache {
            Some(Ok(out)) => {
                let cb = Rect::new(res.right() - 34.0, res.cy() - 14.0, 28.0, 28.0);
                if ui.icon_button(cb, Icon::Copy, ButtonStyle::default().scale(0.6).fg(palette::TEXT_DIM)) {
                    copy = Some(out.plain.clone());
                }
                ui.p.text(
                    &format!("= {}", out.text),
                    Rect::new(res.x + 6.0, res.y, res.w - 48.0, res.h),
                    &TextStyle::new(30.0, palette::TEXT).semibold().display(),
                );
            }
            Some(Err(e)) => {
                ui.p.text(
                    e,
                    Rect::new(res.x + 6.0, res.y, res.w - 12.0, res.h),
                    &TextStyle::new(13.0, palette::TEXT_DIM),
                );
            }
            None => {
                ui.p.text(
                    "Press Enter to keep a calculation",
                    Rect::new(res.x + 6.0, res.y, res.w - 12.0, res.h),
                    &TextStyle::new(13.0, palette::TEXT_FAINT),
                );
            }
        }
        ui.tick_rule(Rect::new(r.x + 4.0, res.bottom() + 2.0, r.w - 8.0, 4.0), 0.08);

        // --- history ------------------------------------------------------
        let area = Rect::new(r.x, res.bottom() + 12.0, r.w, LIST_H);
        if self.history.is_empty() {
            ui.p.text(
                "Your calculations show up here. Click one to copy the answer.",
                area,
                &TextStyle::new(12.0, palette::TEXT_FAINT).center(),
            );
        } else {
            scroll(ui, area, &mut self.scroll, self.history.len() as f32 * ROW_H);
            ui.p.push_clip(area);
            for (i, (expr, out)) in self.history.iter().enumerate() {
                let row = Rect::new(area.x, area.y + i as f32 * ROW_H - self.scroll, area.w, ROW_H - 4.0);
                if row.bottom() < area.y || row.y > area.bottom() {
                    continue;
                }
                let hover = ui.hovered(row);
                ui.row_bg(row, hover, None);
                ui.p.text(
                    expr,
                    Rect::new(row.x + 12.0, row.y, row.w * 0.55, row.h),
                    &TextStyle::new(12.5, palette::TEXT_DIM),
                );
                ui.p.text(
                    &format!("= {}", out.text),
                    Rect::new(row.x + row.w * 0.55, row.y, row.w * 0.45 - 12.0, row.h),
                    &TextStyle::new(13.0, palette::TEXT).semibold().display().right(),
                );
                if ui.clicked(row) {
                    copy = Some(out.plain.clone());
                }
            }
            ui.p.pop_clip();
        }

        if let Some(t) = copy {
            clip::set_text(self.hwnd.hwnd(), &t);
            ui.fx.peek(Peek::new(Icon::Check, palette::GREEN, "Copied", t).key("calc").duration_ms(1300));
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

fn calculate(text: &str, degrees: bool) -> Result<Out, String> {
    let t = text.trim().to_lowercase().replace('×', "*").replace('÷', "/").replace('−', "-");
    if let Some(out) = percent_of(&t, degrees)? {
        return Ok(out);
    }
    if let Some(out) = convert(&t, degrees)? {
        return Ok(out);
    }
    let v = eval(&t, degrees)?;
    Ok(Out { text: fmt_num(v), plain: plain_num(v) })
}

/// "20% of 85"
fn percent_of(t: &str, degrees: bool) -> Result<Option<Out>, String> {
    let Some(pos) = t.find(" of ") else { return Ok(None) };
    let left = t[..pos].trim();
    let Some(pct) = left.strip_suffix('%') else { return Ok(None) };
    let a = eval(pct, degrees)?;
    let b = eval(&t[pos + 4..], degrees)?;
    let v = a / 100.0 * b;
    Ok(Some(Out { text: fmt_num(v), plain: plain_num(v) }))
}

fn eval(text: &str, degrees: bool) -> Result<f64, String> {
    let mut p = Parser { s: text.chars().filter(|c| *c != ',' && *c != '_').collect(), i: 0, deg: degrees, pct: false };
    let v = p.expr()?;
    p.skip_ws();
    if p.i < p.s.len() {
        return Err(format!("Unexpected \"{}\"", p.s[p.i]));
    }
    if v.is_finite() {
        Ok(v)
    } else {
        Err("That doesn't have an answer".into())
    }
}

struct Parser {
    s: Vec<char>,
    i: usize,
    deg: bool,
    /// The last thing parsed was a percentage ("10%").
    pct: bool,
}

impl Parser {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            let op = match self.peek() {
                Some(c @ ('+' | '-')) => c,
                _ => return Ok(v),
            };
            self.i += 1;
            let mut rhs = self.term()?;
            // "200 + 10%" means 200 plus 10 percent of 200
            if self.pct {
                rhs *= v;
            }
            v = if op == '+' { v + rhs } else { v - rhs };
        }
    }

    fn term(&mut self) -> Result<f64, String> {
        self.pct = false;
        let mut v = self.unary()?;
        loop {
            self.skip_ws();
            let rest: String = self.s[self.i.min(self.s.len())..].iter().take(3).collect();
            if rest.starts_with('*') {
                self.i += 1;
                v *= self.unary()?;
            } else if rest.starts_with('/') {
                self.i += 1;
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("Can't divide by zero".into());
                }
                v /= d;
            } else if rest == "mod" {
                self.i += 3;
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("Can't divide by zero".into());
                }
                v %= d;
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.eat('-') {
            return Ok(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.postfix()?;
        if self.eat('^') {
            let exp = self.unary()?;
            self.pct = false;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<f64, String> {
        let mut v = self.primary()?;
        loop {
            if self.eat('%') {
                v /= 100.0;
                self.pct = true;
            } else if self.eat('!') {
                if !(0.0..=170.0).contains(&v) || v.fract() != 0.0 {
                    return Err("Factorial needs a whole number from 0 to 170".into());
                }
                v = (1..=v as u64).fold(1.0, |a, b| a * b as f64);
                self.pct = false;
            } else {
                return Ok(v);
            }
        }
    }

    fn primary(&mut self) -> Result<f64, String> {
        match self.peek() {
            None => Err("Finish the expression".into()),
            Some('(') => {
                self.i += 1;
                let v = self.expr()?;
                if !self.eat(')') {
                    return Err("Missing )".into());
                }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) if c.is_alphabetic() || c == 'π' => self.name(),
            Some(c) => Err(format!("Unexpected \"{c}\"")),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == '.') {
            self.i += 1;
        }
        // 1e5, 2.5e-3 (but not "2e" or "2 e")
        if self.i < self.s.len() && self.s[self.i] == 'e' {
            let mut j = self.i + 1;
            if j < self.s.len() && (self.s[j] == '+' || self.s[j] == '-') {
                j += 1;
            }
            if j < self.s.len() && self.s[j].is_ascii_digit() {
                while j < self.s.len() && self.s[j].is_ascii_digit() {
                    j += 1;
                }
                self.i = j;
            }
        }
        let t: String = self.s[start..self.i].iter().collect();
        t.parse::<f64>().map_err(|_| format!("\"{t}\" isn't a number"))
    }

    fn name(&mut self) -> Result<f64, String> {
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_alphanumeric() || self.s[self.i] == 'π') {
            self.i += 1;
        }
        let word: String = self.s[start..self.i].iter().collect();
        match word.as_str() {
            "pi" | "π" => return Ok(std::f64::consts::PI),
            "tau" => return Ok(std::f64::consts::TAU),
            "e" => return Ok(std::f64::consts::E),
            _ => {}
        }
        const FUNCS: [&str; 16] = [
            "sqrt", "cbrt", "sin", "cos", "tan", "asin", "acos", "atan", "ln", "log", "log10", "log2", "exp", "abs",
            "round", "floor",
        ];
        if !FUNCS.contains(&word.as_str()) && word != "ceil" {
            return Err(format!("Don't know \"{word}\""));
        }
        // a function: sqrt(16) or sqrt 16
        let arg = if self.peek() == Some('(') {
            self.i += 1;
            let v = self.expr()?;
            if !self.eat(')') {
                return Err("Missing )".into());
            }
            v
        } else {
            self.unary()?
        };
        self.pct = false;
        let to_rad = |x: f64| if self.deg { x.to_radians() } else { x };
        let from_rad = |x: f64| if self.deg { x.to_degrees() } else { x };
        Ok(match word.as_str() {
            "sqrt" => {
                if arg < 0.0 {
                    return Err("No real square root".into());
                }
                arg.sqrt()
            }
            "cbrt" => arg.cbrt(),
            "sin" => to_rad(arg).sin(),
            "cos" => to_rad(arg).cos(),
            "tan" => to_rad(arg).tan(),
            "asin" => from_rad(arg.asin()),
            "acos" => from_rad(arg.acos()),
            "atan" => from_rad(arg.atan()),
            "ln" => arg.ln(),
            "log" | "log10" => arg.log10(),
            "log2" => arg.log2(),
            "exp" => arg.exp(),
            "abs" => arg.abs(),
            "round" => arg.round(),
            "floor" => arg.floor(),
            "ceil" => arg.ceil(),
            _ => return Err(format!("Don't know \"{word}\"")),
        })
    }
}

// ---------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Cat {
    Length,
    Mass,
    Volume,
    Data,
    Speed,
    Time,
    Temp,
}

/// (category, size of one unit in the category's base unit).
fn unit(name: &str) -> Option<(Cat, f64)> {
    let n = name.trim().trim_start_matches('°');
    let n = n.strip_suffix('s').filter(|_| n.len() > 3).unwrap_or(n);
    Some(match n {
        "mm" => (Cat::Length, 0.001),
        "cm" => (Cat::Length, 0.01),
        "m" | "meter" | "metre" => (Cat::Length, 1.0),
        "km" => (Cat::Length, 1000.0),
        "in" | "inch" | "inche" => (Cat::Length, 0.0254),
        "ft" | "foot" | "feet" => (Cat::Length, 0.3048),
        "yd" | "yard" => (Cat::Length, 0.9144),
        "mi" | "mile" => (Cat::Length, 1609.344),
        "mg" => (Cat::Mass, 0.001),
        "g" | "gram" => (Cat::Mass, 1.0),
        "kg" => (Cat::Mass, 1000.0),
        "lb" | "lbs" | "pound" => (Cat::Mass, 453.592_37),
        "oz" | "ounce" => (Cat::Mass, 28.349_523_125),
        "t" | "tonne" => (Cat::Mass, 1_000_000.0),
        "ml" => (Cat::Volume, 1.0),
        "l" | "liter" | "litre" => (Cat::Volume, 1000.0),
        "tsp" => (Cat::Volume, 4.928_921_593_75),
        "tbsp" => (Cat::Volume, 14.786_764_781_25),
        "cup" => (Cat::Volume, 236.588_236_5),
        "floz" => (Cat::Volume, 29.573_529_562_5),
        "pt" | "pint" => (Cat::Volume, 473.176_473),
        "qt" | "quart" => (Cat::Volume, 946.352_946),
        "gal" | "gallon" => (Cat::Volume, 3785.411_784),
        "b" | "byte" => (Cat::Data, 1.0),
        "kb" => (Cat::Data, 1024.0),
        "mb" => (Cat::Data, 1_048_576.0),
        "gb" => (Cat::Data, 1_073_741_824.0),
        "tb" => (Cat::Data, 1_099_511_627_776.0),
        "mps" => (Cat::Speed, 1.0),
        "kph" | "kmh" => (Cat::Speed, 1.0 / 3.6),
        "mph" => (Cat::Speed, 0.447_04),
        "kn" | "knot" => (Cat::Speed, 0.514_444),
        "ms" => (Cat::Time, 0.001),
        "sec" | "second" => (Cat::Time, 1.0),
        "min" | "minute" => (Cat::Time, 60.0),
        "h" | "hr" | "hour" => (Cat::Time, 3600.0),
        "d" | "day" => (Cat::Time, 86_400.0),
        "wk" | "week" => (Cat::Time, 604_800.0),
        "c" | "celsius" => (Cat::Temp, 1.0),
        "f" | "fahrenheit" => (Cat::Temp, 2.0),
        "k" | "kelvin" => (Cat::Temp, 3.0),
        _ => return None,
    })
}

fn to_celsius(v: f64, code: f64) -> f64 {
    match code as i32 {
        2 => (v - 32.0) / 1.8,
        3 => v - 273.15,
        _ => v,
    }
}

fn from_celsius(c: f64, code: f64) -> f64 {
    match code as i32 {
        2 => c * 1.8 + 32.0,
        3 => c + 273.15,
        _ => c,
    }
}

/// "5 km to mi", "72 f in c"
fn convert(t: &str, degrees: bool) -> Result<Option<Out>, String> {
    let Some((pos, sep)) = [" to ", " in ", " as "].iter().filter_map(|s| t.rfind(s).map(|p| (p, s.len()))).max()
    else {
        return Ok(None);
    };
    let (left, to_name) = (t[..pos].trim(), t[pos + sep..].trim());
    // the unit is the trailing letters of the left side
    let unit_start = left
        .rfind(|c: char| !(c.is_alphabetic() || c == '°'))
        .map_or(0, |i| i + left[i..].chars().next().map_or(1, char::len_utf8));
    let (num, from_name) = (left[..unit_start].trim(), &left[unit_start..]);
    if num.is_empty() {
        return Ok(None);
    }
    let (Some((cat_a, fa)), Some((cat_b, fb))) = (unit(from_name), unit(to_name)) else {
        return Ok(None);
    };
    if cat_a != cat_b {
        return Err(format!("Can't convert {from_name} to {to_name}"));
    }
    let v = eval(num, degrees)?;
    let out = if cat_a == Cat::Temp { from_celsius(to_celsius(v, fa), fb) } else { v * fa / fb };
    let shown = to_name.trim_start_matches('°');
    Ok(Some(Out { text: format!("{} {shown}", fmt_num(out)), plain: plain_num(out) }))
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

fn plain_num(v: f64) -> String {
    if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-9) {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn fmt_num(v: f64) -> String {
    let plain = plain_num(v);
    if plain.contains('e') {
        return plain;
    }
    let (int, frac) = match plain.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (plain, None),
    };
    let (neg, digits) = match int.strip_prefix('-') {
        Some(d) => (true, d.to_string()),
        None => (false, int),
    };
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut out = if neg { format!("-{grouped}") } else { grouped };
    if let Some(f) = frac {
        out.push('.');
        out.push_str(&f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(s: &str) -> String {
        match calculate(s, true) {
            Ok(o) => o.plain,
            Err(e) => format!("err:{e}"),
        }
    }

    #[test]
    fn arithmetic() {
        assert_eq!(calc("2+2*3"), "8");
        assert_eq!(calc("(2+2)*3"), "12");
        assert_eq!(calc("2^3^2"), "512");
        assert_eq!(calc("-3 + 5"), "2");
        assert_eq!(calc("10 mod 4"), "2");
        assert_eq!(calc("5!"), "120");
        assert_eq!(calc("1,000 + 1"), "1001");
    }

    #[test]
    fn percentages() {
        assert_eq!(calc("20% of 85"), "17");
        assert_eq!(calc("200 + 10%"), "220");
        assert_eq!(calc("50%"), "0.5");
    }

    #[test]
    fn functions() {
        assert_eq!(calc("sqrt(16)"), "4");
        assert_eq!(calc("sin(90)"), "1");
        assert_eq!(calc("log(1000)"), "3");
        assert!(calc("1/0").starts_with("err:"));
    }

    #[test]
    fn units() {
        assert_eq!(calc("1 km to m"), "1000");
        assert_eq!(calc("32 f to c"), "0");
        assert_eq!(calc("1 gb in mb"), "1024");
        assert!(calc("1 kg to m").starts_with("err:"));
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_num(1234567.5), "1,234,567.5");
        assert_eq!(fmt_num(-1000.0), "-1,000");
        assert_eq!(plain_num(0.1 + 0.2), "0.3");
    }
}
