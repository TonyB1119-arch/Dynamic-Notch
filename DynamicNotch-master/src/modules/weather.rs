//! Weather: current conditions and a five day forecast from open-meteo.com
//! (free, no account, no key). The city comes from the config or from
//! Settings → Weather; it is looked up once with Open-Meteo's geocoder.
//!
//! Requests happen on a worker thread: at start, when the city or units
//! change, and when the notch opens while the data is older than
//! `refresh_minutes`. Nothing runs in the background otherwise.

use std::any::Any;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{CardSize, Cx, Module, ModuleId, SystemEvent};
use crate::bus::Bus;
use crate::gfx::{Icon, Painter, TextStyle};
use crate::sys::http::{self, Timeouts};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "weather";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cond {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Storm,
}

impl Cond {
    /// WMO weather interpretation codes, as used by Open-Meteo.
    fn from_code(code: u32) -> Cond {
        match code {
            0 => Cond::Clear,
            1 | 2 => Cond::PartlyCloudy,
            3 => Cond::Cloudy,
            45 | 48 => Cond::Fog,
            51..=57 => Cond::Drizzle,
            61..=67 | 80..=82 => Cond::Rain,
            71..=77 | 85 | 86 => Cond::Snow,
            95..=99 => Cond::Storm,
            _ => Cond::Cloudy,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Cond::Clear => "Clear",
            Cond::PartlyCloudy => "Partly cloudy",
            Cond::Cloudy => "Cloudy",
            Cond::Fog => "Fog",
            Cond::Drizzle => "Drizzle",
            Cond::Rain => "Rain",
            Cond::Snow => "Snow",
            Cond::Storm => "Thunderstorm",
        }
    }
}

#[derive(Clone, Debug)]
struct Day {
    label: String,
    cond: Cond,
    hi: f32,
    lo: f32,
    rain: Option<f32>,
}

#[derive(Clone, Debug)]
struct Forecast {
    place: String,
    temp: f32,
    feels: f32,
    humidity: f32,
    wind: f32,
    cond: Cond,
    night: bool,
    days: Vec<Day>,
    imperial: bool,
}

enum WMsg {
    Data(Forecast),
    Error(String),
}

enum Cmd {
    Fetch { city: String, imperial: bool },
    Quit,
}

pub struct Weather {
    tx: Option<Sender<Cmd>>,
    data: Option<Forecast>,
    error: Option<String>,
    loading: bool,
    last_fetch: Option<Instant>,
    /// What the data on screen was fetched for.
    key: (String, bool),
}

impl Weather {
    pub fn new() -> Self {
        Self { tx: None, data: None, error: None, loading: false, last_fetch: None, key: (String::new(), false) }
    }

    fn fetch(&mut self, cx_cfg: &crate::config::Config) {
        let city = cx_cfg.weather.city.trim().to_string();
        let imperial = cx_cfg.weather.units.eq_ignore_ascii_case("imperial");
        if city.is_empty() {
            self.data = None;
            self.error = None;
            self.loading = false;
            return;
        }
        if let Some(tx) = &self.tx {
            self.loading = true;
            self.error = None;
            self.last_fetch = Some(Instant::now());
            self.key = (city.clone(), imperial);
            let _ = tx.send(Cmd::Fetch { city, imperial });
        }
    }

    fn unit(&self) -> &'static str {
        if self.data.as_ref().is_some_and(|d| d.imperial) {
            "°F"
        } else {
            "°C"
        }
    }
}

impl Module for Weather {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Weather"
    }
    fn icon(&self) -> Icon {
        Icon::Cloud
    }

    fn start(&mut self, cx: &mut Cx) {
        let (tx, rx) = channel();
        let bus = cx.bus.clone();
        let _ = std::thread::Builder::new().name("weather".into()).spawn(move || worker(rx, bus));
        self.tx = Some(tx);
        self.fetch(cx.cfg);
    }

    fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Cmd::Quit);
        }
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let Ok(m) = msg.downcast::<WMsg>() else { return };
        self.loading = false;
        match *m {
            WMsg::Data(d) => {
                self.data = Some(d);
                self.error = None;
            }
            WMsg::Error(e) => self.error = Some(e),
        }
        cx.fx.redraw = true;
        cx.fx.layout_changed = true;
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        match ev {
            SystemEvent::Expanded(true) => {
                let max_age = Duration::from_secs(cx.cfg.weather.refresh_minutes as u64 * 60);
                let stale = self.last_fetch.is_none_or(|t| t.elapsed() > max_age);
                if stale && !self.loading {
                    self.fetch(cx.cfg);
                }
            }
            SystemEvent::ConfigReloaded => {
                let city = cx.cfg.weather.city.trim().to_string();
                let imperial = cx.cfg.weather.units.eq_ignore_ascii_case("imperial");
                if self.key != (city, imperial) {
                    self.fetch(cx.cfg);
                    cx.fx.layout_changed = true;
                }
            }
            _ => {}
        }
    }

    fn card(&self) -> Option<CardSize> {
        Some(CardSize::Small)
    }

    fn draw_card(&mut self, ui: &mut Ui, r: Rect) {
        let open = ui.card(r, true);
        if open {
            ui.fx.open_page = Some(ID);
        }
        let inner = r.inset_xy(12.0, 0.0);
        let Some(d) = self.data.clone() else {
            let msg = if ui.cfg.weather.city.trim().is_empty() {
                "Set a city for weather"
            } else if self.error.is_some() {
                "Weather unavailable"
            } else {
                "Loading weather…"
            };
            ui.p.icon(Icon::Cloud, Rect::new(inner.x, r.cy() - 8.0, 16.0, 16.0), palette::TEXT_FAINT);
            ui.p.text(
                msg,
                Rect::new(inner.x + 24.0, r.y, inner.w - 24.0, r.h),
                &TextStyle::new(12.0, palette::TEXT_DIM),
            );
            return;
        };
        let isz = (r.h - 8.0).clamp(14.0, 26.0);
        draw_condition(ui.p, d.cond, Rect::new(inner.x, r.cy() - isz * 0.5, isz, isz), d.night);
        let tx = inner.x + isz + 8.0;
        let temp = format!("{:.0}°", d.temp);
        let (tw, _) = ui.p.measure(&temp, &TextStyle::new(14.0, palette::TEXT).semibold().display());
        ui.p.text(
            &temp,
            Rect::new(tx, r.y, tw + 4.0, r.h),
            &TextStyle::new(14.0, palette::TEXT).semibold().display(),
        );
        ui.p.text(
            &format!("{} · {}", d.cond.label(), d.place),
            Rect::new(tx + tw + 8.0, r.y, (inner.right() - tx - tw - 8.0).max(0.0), r.h),
            &TextStyle::new(11.5, palette::TEXT_DIM),
        );
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        176.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let city_set = !ui.cfg.weather.city.trim().is_empty();
        if !city_set {
            ui.empty_state(Rect::new(r.x, r.y, r.w, r.h - 40.0), Icon::Cloud, "Choose a city to see the weather");
            let b = Rect::new(r.cx() - 70.0, r.bottom() - 36.0, 140.0, 30.0);
            if ui.pill_button(b, "Open settings", Some(Icon::Settings), ButtonStyle::filled(Color::white(0.1))) {
                ui.fx.open_page = Some("settings");
            }
            return;
        }
        let Some(d) = self.data.clone() else {
            if let Some(e) = self.error.clone() {
                ui.empty_state(Rect::new(r.x, r.y, r.w, r.h - 40.0), Icon::Cloud, &e);
                let b = Rect::new(r.cx() - 50.0, r.bottom() - 36.0, 100.0, 30.0);
                if ui.pill_button(b, "Retry", Some(Icon::Refresh), ButtonStyle::filled(Color::white(0.1))) {
                    self.fetch(ui.cfg);
                }
            } else {
                ui.empty_state(r, Icon::Cloud, "Loading weather…");
                ui.animate();
            }
            return;
        };

        // left: now
        let (left, right) = r.split_left((r.w * 0.40).round(), 14.0);
        ui.card(left, false);
        let li = left.inset_xy(16.0, 12.0);
        draw_condition(ui.p, d.cond, Rect::new(li.x, li.y, 52.0, 52.0), d.night);
        ui.p.text(
            &format!("{:.0}{}", d.temp, self.unit()),
            Rect::new(li.x + 62.0, li.y, li.w - 62.0, 40.0),
            &TextStyle::new(34.0, palette::TEXT).semibold().display(),
        );
        ui.p.text(
            d.cond.label(),
            Rect::new(li.x + 62.0, li.y + 38.0, li.w - 62.0, 18.0),
            &TextStyle::new(12.5, palette::TEXT_DIM),
        );
        ui.p.text(&d.place, Rect::new(li.x, li.y + 62.0, li.w, 18.0), &TextStyle::new(13.0, palette::TEXT).medium());
        let wind_unit = if d.imperial { "mph" } else { "km/h" };
        ui.p.text(
            &format!("Feels {:.0}°  ·  {:.0}% humidity  ·  {:.0} {}", d.feels, d.humidity, d.wind, wind_unit),
            Rect::new(li.x, li.y + 82.0, li.w, 16.0),
            &TextStyle::new(11.0, palette::TEXT_FAINT),
        );

        // refresh
        let rb = Rect::new(left.right() - 30.0, left.y + 6.0, 24.0, 24.0);
        if self.loading {
            ui.animate();
        }
        if ui.icon_button(rb, Icon::Refresh, ButtonStyle::default().scale(0.6).fg(palette::TEXT_DIM)) && !self.loading {
            self.fetch(ui.cfg);
        }

        // right: forecast
        let n = d.days.len().max(1) as f32;
        let gap = 6.0;
        let cw = (right.w - gap * (n - 1.0)) / n;
        for (i, day) in d.days.iter().enumerate() {
            let c = Rect::new(right.x + i as f32 * (cw + gap), right.y, cw, right.h);
            ui.card(c, false);
            ui.p.text(
                &day.label,
                Rect::new(c.x, c.y + 8.0, c.w, 16.0),
                &TextStyle::new(11.5, palette::TEXT_DIM).semibold().center(),
            );
            draw_condition(ui.p, day.cond, c.center_square(28.0).translate(0.0, -6.0), false);
            ui.p.text(
                &format!("{:.0}°", day.hi),
                Rect::new(c.x, c.bottom() - 52.0, c.w, 16.0),
                &TextStyle::new(13.0, palette::TEXT).semibold().center(),
            );
            ui.p.text(
                &format!("{:.0}°", day.lo),
                Rect::new(c.x, c.bottom() - 36.0, c.w, 16.0),
                &TextStyle::new(12.0, palette::TEXT_FAINT).center(),
            );
            if let Some(p) = day.rain.filter(|p| *p >= 20.0) {
                ui.p.text(
                    &format!("{p:.0}%"),
                    Rect::new(c.x, c.bottom() - 20.0, c.w, 14.0),
                    &TextStyle::new(10.5, palette::TEAL).center(),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Condition icons (drawn as shapes, so no icon font glyphs are needed)
// ---------------------------------------------------------------------------

fn draw_condition(p: &mut Painter, cond: Cond, r: Rect, night: bool) {
    let s = r.w.min(r.h);
    let (cx, cy) = (r.cx(), r.cy());
    let sun = Color::hex(0xFFD60A);
    let moon = Color::hex(0xE6E1C8);
    let cloud = Color::white(0.82);
    let dark = Color::white(0.55);
    let cloud_shape = |p: &mut Painter, ox: f32, oy: f32, k: f32, c: Color| {
        let u = s * k;
        p.fill_circle(cx + ox - u * 0.22, cy + oy + u * 0.02, u * 0.26, c);
        p.fill_circle(cx + ox + u * 0.08, cy + oy - u * 0.10, u * 0.32, c);
        p.fill_circle(cx + ox + u * 0.34, cy + oy + u * 0.06, u * 0.22, c);
        p.fill_rounded(Rect::new(cx + ox - u * 0.46, cy + oy + u * 0.04, u * 0.92, u * 0.24), u * 0.12, c);
    };
    let sun_shape = |p: &mut Painter, ox: f32, oy: f32, k: f32| {
        let u = s * k;
        if night {
            p.fill_circle(cx + ox, cy + oy, u * 0.30, moon);
        } else {
            p.fill_circle(cx + ox, cy + oy, u * 0.24, sun);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (sn, cs) = a.sin_cos();
                p.line(
                    cx + ox + cs * u * 0.34,
                    cy + oy + sn * u * 0.34,
                    cx + ox + cs * u * 0.46,
                    cy + oy + sn * u * 0.46,
                    (u * 0.06).max(1.2),
                    sun,
                );
            }
        }
    };
    match cond {
        Cond::Clear => sun_shape(p, 0.0, 0.0, 1.0),
        Cond::PartlyCloudy => {
            sun_shape(p, -s * 0.16, -s * 0.16, 0.8);
            cloud_shape(p, s * 0.06, s * 0.12, 0.78, cloud);
        }
        Cond::Cloudy => {
            cloud_shape(p, s * 0.10, -s * 0.06, 0.62, dark);
            cloud_shape(p, -s * 0.04, s * 0.08, 0.80, cloud);
        }
        Cond::Fog => {
            cloud_shape(p, 0.0, -s * 0.14, 0.74, cloud);
            for (i, w) in [0.8, 0.6, 0.7].iter().enumerate() {
                let y = cy + s * (0.16 + 0.12 * i as f32);
                p.line(cx - s * w * 0.5, y, cx + s * w * 0.5, y, (s * 0.06).max(1.2), Color::white(0.45));
            }
        }
        Cond::Drizzle | Cond::Rain => {
            cloud_shape(p, 0.0, -s * 0.12, 0.78, dark);
            let n = if cond == Cond::Rain { 3 } else { 2 };
            for i in 0..n {
                let x = cx + s * (-0.2 + 0.2 * i as f32 + if n == 2 { 0.1 } else { 0.0 });
                p.line(x, cy + s * 0.22, x - s * 0.06, cy + s * 0.40, (s * 0.07).max(1.2), palette::TEAL);
            }
        }
        Cond::Snow => {
            cloud_shape(p, 0.0, -s * 0.12, 0.78, dark);
            for i in 0..3 {
                let x = cx + s * (-0.2 + 0.2 * i as f32);
                p.fill_circle(x, cy + s * (0.30 + if i == 1 { 0.06 } else { 0.0 }), (s * 0.05).max(1.2), Color::white(0.95));
            }
        }
        Cond::Storm => {
            cloud_shape(p, 0.0, -s * 0.14, 0.78, dark);
            p.line(cx + s * 0.04, cy + s * 0.14, cx - s * 0.08, cy + s * 0.30, (s * 0.08).max(1.4), sun);
            p.line(cx - s * 0.08, cy + s * 0.30, cx + s * 0.08, cy + s * 0.30, (s * 0.08).max(1.4), sun);
            p.line(cx + s * 0.08, cy + s * 0.30, cx - s * 0.02, cy + s * 0.46, (s * 0.08).max(1.4), sun);
        }
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

struct Place {
    name: String,
    lat: f64,
    lon: f64,
}

fn worker(rx: Receiver<Cmd>, bus: Bus) {
    let mut place: Option<(String, Place)> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Quit => return,
            Cmd::Fetch { city, imperial } => {
                // only the newest request matters
                let (mut city, mut imperial) = (city, imperial);
                while let Ok(c) = rx.try_recv() {
                    match c {
                        Cmd::Quit => return,
                        Cmd::Fetch { city: c2, imperial: i2 } => {
                            city = c2;
                            imperial = i2;
                        }
                    }
                }
                let res = (|| {
                    if place.as_ref().is_none_or(|(c, _)| *c != city) {
                        place = Some((city.clone(), geocode(&city)?));
                    }
                    let p = &place.as_ref().ok_or("no place")?.1;
                    forecast(p, imperial)
                })();
                match res {
                    Ok(f) => bus.to_module(ID, WMsg::Data(f)),
                    Err(e) => bus.to_module(ID, WMsg::Error(e)),
                }
            }
        }
    }
}

fn get_json(url: &str) -> Result<Value, String> {
    let cancel = AtomicBool::new(false);
    let headers = [("User-Agent", "DynamicNotch/0.1".to_string())];
    let (status, body) = http::fetch("GET", url, &headers, &[], Timeouts::DEFAULT, 512 * 1024, &cancel)?;
    if status != 200 {
        return Err(format!("weather service returned HTTP {status}"));
    }
    serde_json::from_slice(&body).map_err(|_| "unexpected answer from the weather service".to_string())
}

fn url_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// "Springfield, IL" → search "Springfield", prefer the hit whose region or country mentions "IL".
fn geocode(city: &str) -> Result<Place, String> {
    let (name, hint) = match city.split_once(',') {
        Some((n, h)) => (n.trim(), h.trim().to_lowercase()),
        None => (city.trim(), String::new()),
    };
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=en&format=json",
        url_encode(name)
    );
    let v = get_json(&url)?;
    let results = v.get("results").and_then(Value::as_array).ok_or_else(|| format!("couldn't find “{city}”"))?;
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
    let pick = results
        .iter()
        .find(|r| {
            !hint.is_empty()
                && (text(r, "admin1").contains(&hint)
                    || text(r, "country").contains(&hint)
                    || text(r, "country_code") == hint)
        })
        .or_else(|| results.first())
        .ok_or_else(|| format!("couldn't find “{city}”"))?;
    Ok(Place {
        name: pick.get("name").and_then(Value::as_str).unwrap_or(name).to_string(),
        lat: pick.get("latitude").and_then(Value::as_f64).ok_or("bad location")?,
        lon: pick.get("longitude").and_then(Value::as_f64).ok_or("bad location")?,
    })
}

fn forecast(p: &Place, imperial: bool) -> Result<Forecast, String> {
    let units = if imperial { "temperature_unit=fahrenheit&wind_speed_unit=mph" } else { "wind_speed_unit=kmh" };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
         &timezone=auto&forecast_days=5&{units}",
        p.lat, p.lon
    );
    let v = get_json(&url)?;
    let num = |path: &str| v.pointer(path).and_then(Value::as_f64).map(|x| x as f32);
    let cur = |k: &str| num(&format!("/current/{k}"));
    let temp = cur("temperature_2m").ok_or("no temperature in the answer")?;
    let times = v.pointer("/daily/time").and_then(Value::as_array).ok_or("no forecast in the answer")?;
    let arr = |k: &str| v.pointer(&format!("/daily/{k}")).and_then(Value::as_array).cloned().unwrap_or_default();
    let (codes, his, los, rains) = (
        arr("weather_code"),
        arr("temperature_2m_max"),
        arr("temperature_2m_min"),
        arr("precipitation_probability_max"),
    );
    let mut days = Vec::new();
    for (i, t) in times.iter().enumerate() {
        let date = t.as_str().unwrap_or("");
        let f = |a: &Vec<Value>| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
        days.push(Day {
            label: if i == 0 { "Today".into() } else { weekday_label(date) },
            cond: Cond::from_code(codes.get(i).and_then(Value::as_u64).unwrap_or(3) as u32),
            hi: f(&his).unwrap_or(temp),
            lo: f(&los).unwrap_or(temp),
            rain: f(&rains),
        });
    }
    Ok(Forecast {
        place: p.name.clone(),
        temp,
        feels: cur("apparent_temperature").unwrap_or(temp),
        humidity: cur("relative_humidity_2m").unwrap_or(0.0),
        wind: cur("wind_speed_10m").unwrap_or(0.0),
        cond: Cond::from_code(cur("weather_code").unwrap_or(3.0) as u32),
        night: cur("is_day").is_some_and(|d| d < 0.5),
        days,
        imperial,
    })
}

/// "2026-10-24" → "Sat" (Sakamoto's day-of-week algorithm).
fn weekday_label(date: &str) -> String {
    let mut it = date.split('-').filter_map(|x| x.parse::<i32>().ok());
    let (Some(y), Some(m), Some(d)) = (it.next(), it.next(), it.next()) else { return String::new() };
    if !(1..=12).contains(&m) {
        return String::new();
    }
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let w = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d).rem_euclid(7);
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][w as usize].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays() {
        assert_eq!(weekday_label("2026-10-23"), "Fri");
        assert_eq!(weekday_label("2000-01-01"), "Sat");
        assert_eq!(weekday_label("junk"), "");
    }

    #[test]
    fn codes() {
        assert_eq!(Cond::from_code(0), Cond::Clear);
        assert_eq!(Cond::from_code(63), Cond::Rain);
        assert_eq!(Cond::from_code(95), Cond::Storm);
    }

    #[test]
    fn encoding() {
        assert_eq!(url_encode("São Paulo"), "S%C3%A3o%20Paulo");
    }
}
