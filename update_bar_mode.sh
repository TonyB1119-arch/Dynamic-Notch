cd /workspaces/Notch
python3 - <<'PYEOF' && ./release.sh "Menu bar mode: a macOS-style bar with Control Center, status items and drag-to-reorder"
BAR = r"""//! Bar mode: a macOS-style menu bar across the top of the screen instead of the notch.
//!
//! The left side holds the launcher button and the name of the app in front; the right side holds the
//! status items (weather, processor and memory load, battery, Wi-Fi, Bluetooth, sound, the Control
//! Center and the clock). Items can be dragged to a new place (also across the invisible divider, which
//! is how the Control Center moves to the left side), and a right-click removes or adds them. A click
//! opens a popover under the item: the page of the module behind it, or the Control Center.
//!
//! The bar is its own window, drawn like the dock. It does not own any data: the app hands it the
//! modules every time it draws.

use std::mem::size_of;
use std::time::Instant;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, GetMonitorInfoW, SetWindowRgn, HMONITOR, MONITORINFO, RGN_OR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::bus::Bus;
use crate::config::{BarCfg, Config};
use crate::gfx::{icons, Gfx, Icon, Painter, Res, TextStyle};
use crate::host::Host;
use crate::modules::{Effects, Indicator, Module, ModuleId, Peek, Trailing};
use crate::ui::{Input, Ui};
use crate::util::{Color, Rect};

const CLASS: PCWSTR = w!("DynamicNotch.Bar");
const MOUSELEAVE: u32 = 0x02A3;
/// Once a second: the clock, the loads and the app in front are looked at again.
const T_TICK: usize = 1;
/// About sixty times a second, only while something moves (a popover opening, a banner fading).
const T_ANIM: usize = 2;
/// Ten times a second while a popover is open: a click anywhere else puts it away.
const T_POLL: usize = 3;

/// Room under the bar for popovers, in logical pixels.
const POP_ROOM: f32 = 520.0;
const POP_GAP: f32 = 6.0;
const ITEM_PAD: f32 = 7.0;
const ITEM_GAP: f32 = 2.0;
const CC_W: f32 = 332.0;

/// Every item the bar knows: (id, name).
pub const ALL: [(&str, &str); 9] = [
    ("weather", "Weather"),
    ("cpu", "Processor"),
    ("ram", "Memory"),
    ("battery", "Battery"),
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("sound", "Sound"),
    ("control", "Control Center"),
    ("clock", "Date & time"),
];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// What the app should do for the bar.
pub enum Act {
    Launcher,
}

#[derive(Clone, Copy, PartialEq)]
enum Pop {
    Control,
    Page(ModuleId),
}

struct Hud {
    icon: Icon,
    frac: f32,
    text: Option<String>,
    at: Instant,
    secs: f32,
}

struct Banner {
    icon: Icon,
    title: String,
    subtitle: String,
    at: Instant,
    secs: f32,
}

/// What the app lends the bar while it draws.
pub struct Ctx<'a> {
    pub modules: &'a mut [Box<dyn Module>],
    pub started: &'a [bool],
    pub cfg: &'a Config,
    pub bus: &'a Bus,
    pub accent: Color,
    pub indicators: &'a [Indicator],
}

pub struct Bar {
    hwnd: HWND,
    host: Host,
    res: Res,
    cfg: BarCfg,
    mon: HMONITOR,
    ui_scale: f32,
    scale: f32,
    win_w: f32,
    win_px: (i32, i32, i32, i32),
    shown: bool,
    input: Input,
    tracking: bool,
    /// Where everything was drawn last time: "logo", "app" and the items (the divider is "spacer").
    cells: Vec<(String, Rect)>,
    pop: Option<Pop>,
    pop_open: bool,
    pop_t: f32,
    pop_anchor: Rect,
    pop_rect: Rect,
    press: Option<(String, f32)>,
    drag: Option<String>,
    hud: Option<Hud>,
    banner: Option<Banner>,
    dirty: bool,
    sig: u64,
    anim_on: bool,
    last_anim: Instant,
    front_app: String,
    act: Option<Act>,
    region: Vec<(i32, i32, i32, i32)>,
    tick: bool,
    poll_on: bool,
}

impl Bar {
    pub fn new(gfx: &Gfx, compositor: Compositor) -> windows::core::Result<Self> {
        let hwnd = create_window()?;
        let host = Host::new_shared(hwnd, gfx, 64, 64, compositor)?;
        let res = Res::new(gfx)?;
        Ok(Self {
            hwnd,
            host,
            res,
            cfg: BarCfg::default(),
            mon: HMONITOR::default(),
            ui_scale: 1.0,
            scale: 1.0,
            win_w: 0.0,
            win_px: (0, 0, 0, 0),
            shown: false,
            input: Input::default(),
            tracking: false,
            cells: Vec::new(),
            pop: None,
            pop_open: false,
            pop_t: 0.0,
            pop_anchor: Rect::default(),
            pop_rect: Rect::default(),
            press: None,
            drag: None,
            hud: None,
            banner: None,
            dirty: true,
            sig: 0,
            anim_on: false,
            last_anim: Instant::now(),
            front_app: String::new(),
            act: None,
            region: Vec::new(),
            tick: false,
            poll_on: false,
        })
    }

    /// New settings (or a first start).
    pub fn configure(&mut self, cfg: &BarCfg, mon: HMONITOR, ui_scale: f32) {
        // while an item is being dragged the order on screen is the truth
        let items = if self.drag.is_some() { self.cfg.items.clone() } else { cfg.items.clone() };
        self.cfg = cfg.clone();
        self.cfg.items = items;
        self.mon = mon;
        self.ui_scale = ui_scale;
        self.place();
        unsafe {
            SetTimer(Some(self.hwnd), T_TICK, 1000, None);
        }
        self.poll_front();
        self.dirty = true;
        self.sig = 0;
    }

    /// The screen changed (or the bar moved to another monitor).
    pub fn relocate(&mut self, mon: HMONITOR, ui_scale: f32) {
        self.mon = mon;
        self.ui_scale = ui_scale;
        self.place();
        self.dirty = true;
    }

    /// After a graphics device loss.
    pub fn recover(&mut self, gfx: &Gfx) {
        let _ = self.host.set_rendering_device(gfx);
        if let Ok(r) = Res::new(gfx) {
            self.res = r;
        }
        self.dirty = true;
        self.sig = 0;
    }

    fn bar_h(&self) -> f32 {
        self.cfg.height
    }

    fn place(&mut self) {
        if self.mon.is_invalid() {
            return;
        }
        unsafe {
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(self.mon, &mut mi).as_bool() {
                return;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(self.mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let scale = dx as f32 / 96.0 * self.ui_scale;
            let m = mi.rcMonitor;
            let (wpx, hpx) = (m.right - m.left, ((self.bar_h() + POP_ROOM) * scale).ceil() as i32);
            let geom = (m.left, m.top, wpx, hpx);
            if geom != self.win_px || (scale - self.scale).abs() > 0.001 {
                self.scale = scale;
                self.win_w = wpx as f32 / scale;
                self.win_px = geom;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), geom.0, geom.1, wpx, hpx, SWP_NOACTIVATE);
                let _ = self.host.resize(wpx, hpx);
                self.region.clear();
                self.sig = 0;
            }
            if !self.shown {
                self.shown = true;
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Back to the front of the always-on-top windows, without taking focus.
    pub fn raise(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// A fullscreen app is in front: the bar steps aside.
    pub fn set_hidden(&mut self, hidden: bool) {
        if hidden == self.shown {
            self.shown = !hidden;
            unsafe {
                let _ = ShowWindow(self.hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            self.dirty = true;
        }
    }

    // -- what the app can ask ------------------------------------------------------------------------

    pub fn take_act(&mut self) -> Option<Act> {
        self.act.take()
    }

    pub fn wants_draw(&self) -> bool {
        self.dirty || self.tick
    }

    /// Open a module's page as a popover under the bar (anchored where its item is, or at the right).
    pub fn open_page(&mut self, page: ModuleId) {
        let anchor =
            find_cell(&self.cells, page_item(page)).unwrap_or(Rect::new(self.win_w - 40.0, 0.0, 24.0, self.bar_h()));
        self.open(Pop::Page(page), anchor);
    }

    pub fn open_control(&mut self) {
        let anchor = find_cell(&self.cells, "control").unwrap_or(Rect::new(self.win_w - 40.0, 0.0, 24.0, self.bar_h()));
        self.open(Pop::Control, anchor);
    }

    pub fn is_open(&self) -> bool {
        self.pop.is_some()
    }

    pub fn close(&mut self) {
        if self.pop.is_some() {
            self.pop_open = false;
            self.start_anim();
        }
    }

    /// Volume and brightness show as a small pill under the Control Center icon.
    pub fn show_hud(&mut self, pk: &Peek) {
        let Trailing::Level { frac, text, .. } = &pk.trailing else {
            return;
        };
        self.hud = Some(Hud { icon: pk.icon, frac: *frac, text: text.clone(), at: Instant::now(), secs: 1.8 });
        self.dirty = true;
        self.start_anim();
    }

    /// Everything else the notch would have shown for a moment becomes a banner at the top right.
    pub fn show_banner(&mut self, pk: &Peek) {
        let secs = pk.duration.as_secs_f32().clamp(2.0, 6.0);
        self.banner = Some(Banner {
            icon: pk.icon,
            title: pk.title.clone(),
            subtitle: pk.subtitle.clone(),
            at: Instant::now(),
            secs,
        });
        self.dirty = true;
        self.start_anim();
    }

    fn open(&mut self, pop: Pop, anchor: Rect) {
        if self.pop == Some(pop) && self.pop_open {
            self.close();
            return;
        }
        self.pop = Some(pop);
        self.pop_open = true;
        self.pop_anchor = anchor;
        self.dirty = true;
        if !self.poll_on {
            self.poll_on = true;
            unsafe {
                SetTimer(Some(self.hwnd), T_POLL, 100, None);
            }
        }
        self.start_anim();
    }

    fn start_anim(&mut self) {
        if !self.anim_on {
            self.anim_on = true;
            self.last_anim = Instant::now();
            unsafe {
                SetTimer(Some(self.hwnd), T_ANIM, 10, None);
            }
        }
    }

    /// Does something still move?
    fn moving(&self) -> bool {
        let fading = |at: Instant, secs: f32| {
            let t = at.elapsed().as_secs_f32();
            t < 0.2 || t > secs - 1.1
        };
        (self.pop.is_some() && ((self.pop_open && self.pop_t < 1.0) || (!self.pop_open && self.pop_t > 0.0)))
            || self.hud.as_ref().is_some_and(|h| fading(h.at, h.secs))
            || self.banner.as_ref().is_some_and(|b| fading(b.at, b.secs))
    }

    fn step_anim(&mut self) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_anim).as_secs_f32().min(0.05);
        self.last_anim = now;
        if self.pop.is_some() {
            if self.pop_open {
                self.pop_t = (self.pop_t + dt / 0.16).min(1.0);
            } else {
                self.pop_t = (self.pop_t - dt / 0.12).max(0.0);
                if self.pop_t <= 0.0 {
                    self.pop = None;
                    self.poll_on = false;
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_POLL);
                    }
                }
            }
        }
        self.expire();
        self.dirty = true;
        if !self.moving() {
            self.anim_on = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_ANIM);
            }
        }
    }

    /// Put away the pill and the banner when their time is up.
    fn expire(&mut self) {
        if self.hud.as_ref().is_some_and(|h| h.at.elapsed().as_secs_f32() > h.secs) {
            self.hud = None;
            self.dirty = true;
        }
        if self.banner.as_ref().is_some_and(|b| b.at.elapsed().as_secs_f32() > b.secs) {
            self.banner = None;
            self.dirty = true;
        }
    }

    /// The app in front (not this one), by name and window title.
    fn poll_front(&mut self) {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() || fg == self.hwnd {
                return;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid == std::process::id() || pid == 0 {
                return;
            }
            let exe = crate::dock::exe_of(pid);
            let stem =
                std::path::Path::new(&exe).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if !stem.is_empty() && nice(&stem) != self.front_app {
                self.front_app = nice(&stem);
                self.dirty = true;
            }
        }
    }

    // -- input -----------------------------------------------------------------------------------

    fn point(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as f32 / self.scale;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / self.scale;
        (x, y)
    }

    /// The item (or "logo") under a point of the bar.
    fn item_at(&self, x: f32, y: f32) -> Option<String> {
        if y > self.bar_h() {
            return None;
        }
        self.cells.iter().find(|c| c.0 != "app" && c.0 != "spacer" && c.1.contains(x, y)).map(|c| c.0.clone())
    }

    /// The item a right-click at `lp` landed on, and the items that are not on the bar.
    pub fn menu_info(&self, lp: LPARAM) -> (Option<String>, Vec<(usize, &'static str)>) {
        let (x, y) = self.point(lp);
        let target = self.item_at(x, y).filter(|id| id != "logo");
        let missing = ALL
            .iter()
            .enumerate()
            .filter(|(_, a)| !self.cfg.items.iter().any(|s| s == a.0))
            .map(|(k, a)| (k, a.1))
            .collect();
        (target, missing)
    }

    /// A choice from the right-click menu: 1 = remove `target`, 100 + k = add `ALL[k]`, 2 = start over.
    pub fn menu_command(&mut self, cmd: i32, target: Option<String>) {
        match cmd {
            1 => {
                if let Some(t) = target {
                    self.cfg.items.retain(|s| *s != t);
                }
            }
            2 => self.cfg.items = BarCfg::default().items,
            c if c >= 100 && ((c - 100) as usize) < ALL.len() => {
                let id = ALL[(c - 100) as usize].0.to_string();
                if !self.cfg.items.contains(&id) {
                    // a new item joins before the clock when there is one
                    let at = self.cfg.items.iter().position(|s| s == "clock").unwrap_or(self.cfg.items.len());
                    self.cfg.items.insert(at, id);
                }
            }
            _ => return,
        }
        self.save_items();
        self.dirty = true;
    }

    fn save_items(&self) {
        let list: Vec<String> = self.cfg.items.iter().map(|s| format!("'{s}'")).collect();
        crate::config::set_value("bar", "items", &format!("[{}]", list.join(", ")));
    }

    /// Dragging `id`: it takes the place under the pointer, counted in items left of the pointer.
    fn reorder(&mut self, id: &str, x: f32) {
        let Some(from) = self.cfg.items.iter().position(|s| s == id) else {
            return;
        };
        let mut to = 0;
        for s in &self.cfg.items {
            if s == id {
                continue;
            }
            // items that are not shown right now (no data) keep their place and count as left of the pointer
            match self.cells.iter().find(|c| c.0 == *s) {
                Some(c) if c.1.cx() >= x => {}
                _ => to += 1,
            }
        }
        if to != from {
            let it = self.cfg.items.remove(from);
            self.cfg.items.insert(to.min(self.cfg.items.len()), it);
            self.dirty = true;
        }
    }

    fn click_item(&mut self, id: &str) {
        let anchor = find_cell(&self.cells, id).unwrap_or_default();
        match id {
            "logo" => self.act = Some(Act::Launcher),
            "control" | "sound" => self.open(Pop::Control, anchor),
            "wifi" | "bluetooth" => {
                crate::modules::control::WANT_TAB
                    .store(i32::from(id == "bluetooth"), std::sync::atomic::Ordering::Relaxed);
                self.open(Pop::Page("control"), anchor);
            }
            "weather" => self.open(Pop::Page("weather"), anchor),
            "cpu" | "ram" => self.open(Pop::Page("stats"), anchor),
            "battery" => self.open(Pop::Page("battery"), anchor),
            "clock" => self.open(Pop::Page("calendar"), anchor),
            _ => {}
        }
    }

    pub fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = self.point(lp);
                if !self.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    self.tracking = true;
                }
                self.input.mouse = Some((x, y));
                if self.drag.is_none() {
                    if let Some((id, px)) = self.press.clone() {
                        if (x - px).abs() > 6.0 && id != "logo" {
                            self.drag = Some(id);
                        }
                    }
                }
                if let Some(id) = self.drag.clone() {
                    self.reorder(&id, x);
                }
                self.dirty = true;
                Some(LRESULT(0))
            }
            MOUSELEAVE => {
                self.tracking = false;
                if self.drag.is_none() {
                    self.input.mouse = None;
                }
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.point(lp);
                self.input.mouse = Some((x, y));
                self.input.down = Some((x, y));
                self.press = self.item_at(x, y).map(|id| (id, x));
                unsafe {
                    let _ = SetCapture(self.hwnd);
                }
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = self.point(lp);
                self.input.mouse = Some((x, y));
                if let Some(d) = self.input.down.take() {
                    self.input.click = Some((d, (x, y)));
                }
                unsafe {
                    let _ = ReleaseCapture();
                }
                if self.drag.take().is_some() {
                    self.press = None;
                    self.save_items();
                } else if let Some((id, _)) = self.press.take() {
                    if self.item_at(x, y).as_deref() == Some(id.as_str()) {
                        self.click_item(&id);
                    }
                }
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                self.input.wheel += ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_MOUSEHWHEEL => {
                self.input.hwheel += ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
                self.dirty = true;
                Some(LRESULT(0))
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_TIMER if wp.0 == T_ANIM => {
                self.step_anim();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_POLL => {
                // the left button went down somewhere that is not this bar or its popover
                unsafe {
                    if GetAsyncKeyState(0x01) as u16 & 0x8000 != 0 && self.input.down.is_none() {
                        let mut pt = POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        let (x, y) =
                            ((pt.x - self.win_px.0) as f32 / self.scale, (pt.y - self.win_px.1) as f32 / self.scale);
                        if self.pop_open && !self.pop_rect.expand(2.0).contains(x, y) && y > self.bar_h() {
                            self.close();
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_TICK => {
                self.poll_front();
                self.expire();
                if self.shown {
                    self.raise();
                }
                self.tick = true;
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// One status item, ready to be drawn.
struct Cell {
    id: String,
    icon: Option<Icon>,
    text: String,
    battery: Option<(f32, bool)>,
    w: f32,
}

fn ink(a: f32) -> Color {
    Color::white(a)
}

fn page_item(page: ModuleId) -> &'static str {
    match page {
        "weather" => "weather",
        "stats" => "cpu",
        "battery" => "battery",
        "calendar" => "clock",
        "control" => "wifi",
        _ => "control",
    }
}

fn find_cell(cells: &[(String, Rect)], id: &str) -> Option<Rect> {
    cells.iter().find(|c| c.0 == id).map(|c| c.1)
}

fn nice(stem: &str) -> String {
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn clicked(input: &Input, r: Rect) -> bool {
    input.click.is_some_and(|(d, u)| r.contains(d.0, d.1) && r.contains(u.0, u.1))
}

/// A rounded tile of the Control Center: a round icon, a title and a line below it.
fn tile(p: &mut Painter, r: Rect, icon: Icon, title: &str, sub: &str, on: bool, accent: Color, hover: bool) {
    p.fill_rounded(r, 18.0, ink(if hover { 0.16 } else { 0.10 }));
    let d = (r.h - 20.0).min(34.0);
    let c = Rect::new(r.x + 10.0, r.cy() - d * 0.5, d, d);
    p.fill_circle(c.cx(), c.cy(), d * 0.5, if on { accent } else { ink(0.16) });
    p.icon(icon, c.inset(d * 0.24), if on { Color::white(1.0) } else { ink(0.85) });
    let tx = c.right() + 8.0;
    p.text(
        title,
        Rect::new(tx, r.y + 6.0, r.right() - tx - 6.0, r.h * 0.5 - 4.0),
        &TextStyle::new(12.5, ink(0.95)).semibold(),
    );
    p.text(sub, Rect::new(tx, r.cy() - 1.0, r.right() - tx - 6.0, r.h * 0.5 - 4.0), &TextStyle::new(11.0, ink(0.6)));
}

/// A slider in the Control Center. Returns the new value while the pointer drags it.
fn slider(p: &mut Painter, input: &Input, r: Rect, icon: Icon, frac: f32, live: bool) -> Option<f32> {
    p.fill_rounded(r, r.h * 0.5, ink(0.12));
    let fw = (r.w * frac.clamp(0.0, 1.0)).max(r.h);
    p.push_clip(r);
    p.fill_rounded(Rect::new(r.x, r.y, fw, r.h), r.h * 0.5, ink(0.9));
    p.pop_clip();
    p.icon(icon, Rect::new(r.x + 9.0, r.y + 9.0, r.h - 18.0, r.h - 18.0), Color::rgba(0.1, 0.1, 0.12, 0.85));
    let mut out = None;
    if live {
        if let (Some(d), Some(m)) = (input.down, input.mouse) {
            if r.expand(4.0).contains(d.0, d.1) {
                out = Some(((m.0 - r.x - r.h * 0.5) / (r.w - r.h)).clamp(0.0, 1.0));
            }
        }
        if input.wheel != 0.0 && input.mouse.is_some_and(|m| r.contains(m.0, m.1)) {
            out = Some((frac + input.wheel * 0.05).clamp(0.0, 1.0));
        }
    }
    out
}

impl Bar {
    /// The items as they would be drawn right now, without positions.
    fn gather(items: &[String], ctx: &Ctx) -> Vec<Cell> {
        let mut out = Vec::new();
        for id in items {
            let mut c = Cell { id: id.clone(), icon: None, text: String::new(), battery: None, w: 0.0 };
            match id.as_str() {
                "weather" => {
                    let Some(m) = ctx.modules.iter().zip(ctx.started).find(|(m, s)| **s && m.id() == "weather") else {
                        continue;
                    };
                    let Some((ic, t)) = m.0.glance() else { continue };
                    c.icon = Some(ic);
                    c.text = format!("{t}°");
                }
                "cpu" => {
                    let Some(v) = crate::modules::stats::cpu_percent() else { continue };
                    c.icon = Some(Icon::Cpu);
                    c.text = format!("{v}%");
                }
                "ram" => {
                    let Some(v) = crate::modules::stats::ram_percent() else { continue };
                    c.icon = Some(Icon::Ram);
                    c.text = format!("{v}%");
                }
                "battery" => {
                    let Some(b) = ctx.indicators.iter().find_map(|i| i.battery) else { continue };
                    c.battery = Some(b);
                    c.text = format!("{}%", (b.0 * 100.0).round() as i32);
                }
                "wifi" => c.icon = Some(Icon::Wifi),
                "bluetooth" => c.icon = Some(Icon::Bluetooth),
                "sound" => {
                    c.icon = Some(if crate::modules::osd::volume_now().is_some_and(|v| v < 0.01) {
                        Icon::Mute
                    } else {
                        Icon::Volume
                    })
                }
                "control" => c.icon = Some(Icon::Control),
                "clock" => {
                    let (h, m, _, suffix, _) = crate::app::clock_parts();
                    let t = unsafe { GetLocalTime() };
                    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                    let sfx = if suffix.trim().is_empty() { String::new() } else { format!(" {}", suffix.trim()) };
                    c.text = format!(
                        "{} {} {}   {}:{:02}{}",
                        days[(t.wDayOfWeek as usize).min(6)],
                        MONTHS[(t.wMonth as usize).clamp(1, 12) - 1],
                        t.wDay,
                        h,
                        m,
                        sfx
                    );
                }
                _ => continue,
            }
            out.push(c);
        }
        out
    }

    /// How wide each item is.
    fn measure(cells: &mut [Cell], p: &mut Painter) {
        let st = TextStyle::new(13.0, ink(0.95)).medium();
        for c in cells {
            let tw = if c.text.is_empty() { 0.0 } else { p.measure(&c.text, &st).0 };
            let iw = if c.battery.is_some() {
                24.0
            } else if c.icon.is_some() {
                16.0
            } else {
                0.0
            };
            c.w = ITEM_PAD * 2.0 + iw + tw + if iw > 0.0 && tw > 0.0 { 5.0 } else { 0.0 };
        }
    }

    /// Paint the bar (and whatever hangs under it). Called by the app when `wants_draw`.
    pub fn draw(&mut self, gfx: &Gfx, ctx: &mut Ctx, fx: &mut Effects) {
        let forced = self.dirty;
        self.dirty = false;
        self.tick = false;
        if !self.shown || self.win_px.2 == 0 {
            return;
        }
        let sc = self.scale;
        let bh = self.bar_h();
        let bold = TextStyle::new(13.0, ink(0.97)).semibold();
        let plain = TextStyle::new(13.0, ink(0.95)).medium();
        let input = self.input.clone();
        let hover_id = if self.drag.is_some() { None } else { input.mouse.and_then(|m| self.item_at(m.0, m.1)) };

        let mut cells = Self::gather(&self.cfg.items, ctx);
        // content signature: nothing is painted again when the clock, loads and hover are the same
        let mut h: u64 = 1469598103934665603;
        let mut mix = |b: &[u8]| {
            for x in b {
                h = (h ^ *x as u64).wrapping_mul(1099511628211);
            }
        };
        for c in &cells {
            mix(c.id.as_bytes());
            mix(c.text.as_bytes());
            if let Some((l, ch)) = c.battery {
                mix(&[(l * 100.0) as u8, ch as u8]);
            }
        }
        mix(self.front_app.as_bytes());
        mix(hover_id.as_deref().unwrap_or("").as_bytes());
        mix(&[self.drag.is_some() as u8, self.pop.is_some() as u8]);
        if !forced && h == self.sig {
            return;
        }
        self.sig = h;

        let (dc, _) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(e) => {
                crate::log!("bar: begin_draw failed: {e}");
                self.dirty = true;
                return;
            }
        };
        let mut next_pop: Option<Pop> = None;
        let mut close_after = false;
        let mut region: Vec<(i32, i32, i32, i32)> = Vec::new();
        let mut pop_px: Option<(Rect, f32)> = None;
        {
            let mut p = Painter::new(dc, gfx, &mut self.res, sc, 0.0, 0.0);
            p.clear();
            Self::measure(&mut cells, &mut p);

            // -- the strip -----------------------------------------------------------------------
            let mut cell_rects: Vec<(String, Rect)> = Vec::new();
            if self.cfg.background {
                p.fill_rounded(Rect::new(0.0, 0.0, self.win_w, bh), 0.0, Color::rgba(0.07, 0.08, 0.10, 0.38));
                p.line(0.0, bh - 0.5, self.win_w, bh - 0.5, 1.0, ink(0.08));
            }
            // left: launcher button and the name of the app in front
            let logo = Rect::new(6.0, 2.0, bh - 4.0, bh - 4.0);
            if hover_id.as_deref() == Some("logo") {
                p.fill_rounded(logo, 7.0, ink(0.14));
            }
            p.icon(Icon::Grid, logo.inset(5.5), ink(0.95));
            cell_rects.push(("logo".into(), logo));
            let mut x = logo.right() + 8.0;
            let app = if self.front_app.is_empty() { "Desktop".to_string() } else { self.front_app.clone() };
            let aw = p.measure(&app, &bold).0.min(220.0);
            p.text(&app, Rect::new(x, 0.0, aw + 2.0, bh), &bold);
            cell_rects.push(("app".into(), Rect::new(x, 0.0, aw, bh)));
            x += aw + 14.0;

            // the two groups of items: before and after the divider
            let div = self.cfg.items.iter().position(|s| s == "spacer");
            let (left, right): (Vec<&Cell>, Vec<&Cell>) = match div {
                Some(d) => {
                    let mut l = Vec::new();
                    let mut r = Vec::new();
                    for c in &cells {
                        let at = self.cfg.items.iter().position(|s| *s == c.id).unwrap_or(0);
                        if at < d {
                            l.push(c);
                        } else {
                            r.push(c);
                        }
                    }
                    (l, r)
                }
                None => (Vec::new(), cells.iter().collect()),
            };
            let rtotal: f32 = right.iter().map(|c| c.w + ITEM_GAP).sum::<f32>();
            let rstart = (self.win_w - 8.0 - rtotal + ITEM_GAP).max(x);
            let mut place: Vec<(&Cell, f32)> = Vec::new();
            let mut lx = x;
            for c in &left {
                place.push((c, lx));
                lx += c.w + ITEM_GAP;
            }
            let mut rx = rstart;
            for c in &right {
                place.push((c, rx));
                rx += c.w + ITEM_GAP;
            }
            cell_rects.push(("spacer".into(), Rect::new((lx + rstart) * 0.5, 0.0, 0.0, bh)));
            for (c, cx) in &place {
                let r = Rect::new(*cx, 2.0, c.w, bh - 4.0);
                let open = self.pop.is_some()
                    && self.pop_open
                    && self.pop_anchor.cx() >= r.x
                    && self.pop_anchor.cx() <= r.right();
                let dragged = self.drag.as_deref() == Some(c.id.as_str());
                if open || dragged {
                    p.fill_rounded(r, 7.0, ink(if dragged { 0.22 } else { 0.18 }));
                } else if hover_id.as_deref() == Some(c.id.as_str()) {
                    p.fill_rounded(r, 7.0, ink(0.12));
                }
                let mut ix = r.x + ITEM_PAD;
                if let Some((lvl, ch)) = c.battery {
                    let br = Rect::new(ix, r.cy() - 6.0, 22.0, 12.0);
                    icons::battery(&mut p, br, lvl, ch, crate::app::battery_color(lvl, ch));
                    ix += 29.0;
                } else if let Some(ic) = c.icon {
                    p.icon(ic, Rect::new(ix, r.cy() - 8.0, 16.0, 16.0), ink(0.95));
                    ix += 21.0;
                }
                if !c.text.is_empty() {
                    p.text(&c.text, Rect::new(ix, 0.0, r.right() - ix + 4.0, bh), &plain);
                }
                cell_rects.push((c.id.clone(), r));
            }
            self.cells = cell_rects;
            region.push((0, 0, self.win_px.2, (bh * sc).ceil() as i32));

            // -- the popover ---------------------------------------------------------------------
            if let Some(pop) = self.pop {
                let want = match pop {
                    Pop::Page(pg) => page_item(pg),
                    Pop::Control => "control",
                };
                let anchor = if self.pop_anchor.w > 0.0 {
                    self.pop_anchor
                } else {
                    find_cell(&self.cells, want).unwrap_or(Rect::new(self.win_w - 40.0, 0.0, 24.0, bh))
                };
                let (pw, ph) = match pop {
                    Pop::Control => (CC_W, 372.0),
                    Pop::Page(id) => {
                        let w = 380.0;
                        let hh =
                            ctx.modules.iter().find(|m| m.id() == id).map(|m| m.page_height(w - 24.0)).unwrap_or(160.0);
                        (w, (hh + 24.0).clamp(120.0, POP_ROOM - POP_GAP - 8.0))
                    }
                };
                let px = (anchor.cx() - pw * 0.5).clamp(8.0, (self.win_w - pw - 8.0).max(8.0));
                let t = self.pop_t;
                let ease = 1.0 - (1.0 - t) * (1.0 - t);
                let pr = Rect::new(px, bh + POP_GAP - (1.0 - ease) * 8.0, pw, ph);
                self.pop_rect = pr;
                let old = p.alpha;
                p.alpha = old * ease;
                p.fill_rounded(pr, 18.0, Color::rgba(0.07, 0.08, 0.10, 0.5));
                p.stroke_rounded(pr, 18.0, 1.0, ink(0.14));
                let inner = pr.inset(12.0);
                let live = self.pop_open && t > 0.9;
                match pop {
                    Pop::Control => {
                        let gap = 8.0;
                        let half = (inner.w - gap) * 0.5;
                        let (mx, my) = input.mouse.unwrap_or((-1.0, -1.0));
                        let over = |r: Rect| live && r.contains(mx, my);
                        let wifi = Rect::new(inner.x, inner.y, half, 62.0);
                        let blue = Rect::new(inner.x + half + gap, inner.y, half, 62.0);
                        let g = |id: &str| {
                            ctx.modules.iter().find(|m| m.id() == "control").and_then(|m| m.glance_for(id)).map(|g| g.1)
                        };
                        tile(
                            &mut p,
                            wifi,
                            Icon::Wifi,
                            "Wi-Fi",
                            &g("wifi").unwrap_or_else(|| "Networks".into()),
                            true,
                            ctx.accent,
                            over(wifi),
                        );
                        tile(
                            &mut p,
                            blue,
                            Icon::Bluetooth,
                            "Bluetooth",
                            &g("bluetooth").unwrap_or_else(|| "Devices".into()),
                            true,
                            ctx.accent,
                            over(blue),
                        );
                        if live && clicked(&input, wifi) {
                            crate::modules::control::WANT_TAB.store(0, std::sync::atomic::Ordering::Relaxed);
                            next_pop = Some(Pop::Page("control"));
                        }
                        if live && clicked(&input, blue) {
                            crate::modules::control::WANT_TAB.store(1, std::sync::atomic::Ordering::Relaxed);
                            next_pop = Some(Pop::Page("control"));
                        }
                        let dnd = crate::bloom::dnd();
                        let focus = Rect::new(inner.x, wifi.bottom() + gap, half, 62.0);
                        let apps = Rect::new(inner.x + half + gap, focus.y, half, 62.0);
                        tile(
                            &mut p,
                            focus,
                            Icon::Moon,
                            "Focus",
                            if dnd { "Do Not Disturb" } else { "Off" },
                            dnd,
                            ctx.accent,
                            over(focus),
                        );
                        tile(&mut p, apps, Icon::Grid, "Launcher", "Apps & search", false, ctx.accent, over(apps));
                        if live && clicked(&input, focus) {
                            crate::bloom::set_dnd(!dnd);
                            self.sig = 0;
                        }
                        if live && clicked(&input, apps) {
                            self.act = Some(Act::Launcher);
                            close_after = true;
                        }
                        let label = TextStyle::new(12.0, ink(0.8)).semibold();
                        let dl = Rect::new(inner.x + 4.0, focus.bottom() + 10.0, inner.w, 16.0);
                        p.text("Display", dl, &label);
                        let ds = Rect::new(inner.x, dl.bottom() + 2.0, inner.w, 36.0);
                        let bright = crate::modules::osd::brightness_now().unwrap_or(0.5);
                        if let Some(v) = slider(&mut p, &input, ds, Icon::Brightness, bright, live) {
                            if (v - bright).abs() > 0.004 {
                                fx.set_brightness = Some((v * 100.0).round() as u8);
                            }
                        }
                        let sl = Rect::new(inner.x + 4.0, ds.bottom() + 8.0, inner.w, 16.0);
                        p.text("Sound", sl, &label);
                        let ss = Rect::new(inner.x, sl.bottom() + 2.0, inner.w, 36.0);
                        let vol = crate::modules::osd::volume_now().unwrap_or(0.0);
                        let vic = if vol < 0.01 { Icon::Mute } else { Icon::Volume };
                        if let Some(v) = slider(&mut p, &input, ss, vic, vol, live) {
                            if (v - vol).abs() > 0.004 {
                                fx.set_volume = Some(v);
                            }
                        }
                        // what is playing
                        let np = Rect::new(inner.x, ss.bottom() + 10.0, inner.w, inner.bottom() - ss.bottom() - 10.0);
                        p.fill_rounded(np, 18.0, ink(0.08));
                        if let Some((i, m)) = ctx.modules.iter_mut().enumerate().find(|(_, m)| m.id() == "media") {
                            if ctx.started[i] {
                                let mut ui = Ui {
                                    p: &mut p,
                                    input: &input,
                                    fx,
                                    cfg: ctx.cfg,
                                    bus: ctx.bus,
                                    now: Instant::now(),
                                    accent: ctx.accent,
                                    hot: false,
                                    interactive: live,
                                };
                                m.draw_card(&mut ui, np.inset(6.0));
                            }
                        }
                    }
                    Pop::Page(id) => {
                        if let Some((i, m)) = ctx.modules.iter_mut().enumerate().find(|(_, m)| m.id() == id) {
                            if ctx.started[i] {
                                p.push_clip(pr);
                                let mut ui = Ui {
                                    p: &mut p,
                                    input: &input,
                                    fx,
                                    cfg: ctx.cfg,
                                    bus: ctx.bus,
                                    now: Instant::now(),
                                    accent: ctx.accent,
                                    hot: false,
                                    interactive: live,
                                };
                                m.draw_page(&mut ui, inner);
                                ui.p.pop_clip();
                            }
                        }
                    }
                }
                p.alpha = old;
                pop_px = Some((pr, ease));
                let e = pr.expand(2.0);
                region.push((
                    (e.x * sc).floor().max(0.0) as i32,
                    (e.y * sc).floor().max(0.0) as i32,
                    (e.right() * sc).ceil() as i32,
                    (e.bottom() * sc).ceil() as i32,
                ));
            } else {
                self.pop_rect = Rect::default();
            }

            // -- the volume and brightness pill, and the banner -----------------------------------
            let fade = |at: Instant, secs: f32| {
                let t = at.elapsed().as_secs_f32();
                (t / 0.15).min(1.0).min(((secs - t) / 0.25).max(0.0))
            };
            let mut y = bh + 6.0;
            if self.pop.is_none() {
                if let Some(hd) = &self.hud {
                    let a = fade(hd.at, hd.secs);
                    let anchor =
                        find_cell(&self.cells, "control").unwrap_or(Rect::new(self.win_w - 40.0, 0.0, 24.0, bh));
                    let r =
                        Rect::new((anchor.right() - 230.0).clamp(8.0, (self.win_w - 238.0).max(8.0)), y, 230.0, 42.0);
                    let old = p.alpha;
                    p.alpha = old * a;
                    p.fill_rounded(r, 21.0, Color::rgba(0.07, 0.08, 0.10, 0.72));
                    p.stroke_rounded(r, 21.0, 1.0, ink(0.14));
                    p.icon(hd.icon, Rect::new(r.x + 14.0, r.cy() - 9.0, 18.0, 18.0), ink(0.95));
                    let tr = Rect::new(r.x + 42.0, r.cy() - 3.0, r.w - 42.0 - 56.0, 6.0);
                    p.fill_rounded(tr, 3.0, ink(0.2));
                    p.fill_rounded(
                        Rect::new(tr.x, tr.y, (tr.w * hd.frac.clamp(0.0, 1.0)).max(6.0), tr.h),
                        3.0,
                        ink(0.95),
                    );
                    let txt = hd.text.clone().unwrap_or_else(|| format!("{}%", (hd.frac * 100.0).round() as i32));
                    p.text(&txt, Rect::new(r.right() - 52.0, r.y, 40.0, r.h), &TextStyle::new(12.0, ink(0.9)).medium());
                    p.alpha = old;
                    let e = r.expand(2.0);
                    region.push((
                        (e.x * sc).floor().max(0.0) as i32,
                        (e.y * sc).floor() as i32,
                        (e.right() * sc).ceil() as i32,
                        (e.bottom() * sc).ceil() as i32,
                    ));
                    y = r.bottom() + 6.0;
                }
            }
            if let Some(b) = &self.banner {
                let a = fade(b.at, b.secs);
                let r = Rect::new(self.win_w - 8.0 - 310.0, y, 310.0, 58.0);
                let old = p.alpha;
                p.alpha = old * a;
                p.fill_rounded(r, 16.0, Color::rgba(0.07, 0.08, 0.10, 0.74));
                p.stroke_rounded(r, 16.0, 1.0, ink(0.14));
                let ic = Rect::new(r.x + 12.0, r.cy() - 17.0, 34.0, 34.0);
                p.fill_circle(ic.cx(), ic.cy(), 17.0, ink(0.14));
                p.icon(b.icon, ic.inset(8.0), ink(0.95));
                p.text(
                    &b.title,
                    Rect::new(r.x + 56.0, r.y + 8.0, r.w - 68.0, 20.0),
                    &TextStyle::new(13.0, ink(0.97)).semibold(),
                );
                p.text(
                    &b.subtitle,
                    Rect::new(r.x + 56.0, r.y + 28.0, r.w - 68.0, 20.0),
                    &TextStyle::new(12.0, ink(0.65)),
                );
                p.alpha = old;
                let e = r.expand(2.0);
                region.push((
                    (e.x * sc).floor().max(0.0) as i32,
                    (e.y * sc).floor() as i32,
                    (e.right() * sc).ceil() as i32,
                    (e.bottom() * sc).ceil() as i32,
                ));
            }
        }
        if let Err(e) = self.host.end_draw() {
            crate::log!("bar: end_draw failed: {e}");
        }
        self.res.end_frame();

        // glass behind the strip and the popover
        let wpx = self.win_px.2 as f32;
        let strip = if self.cfg.background { 0.9 } else { 0.0 };
        self.host.set_backdrop_rounded(Rect::new(0.0, 0.0, wpx, (bh * sc).ceil()), 0.0, strip);
        match pop_px {
            Some((r, e)) => {
                self.host.set_backdrop2_rounded(Rect::new(r.x * sc, r.y * sc, r.w * sc, r.h * sc), 18.0 * sc, 0.9 * e)
            }
            None => self.host.set_backdrop2_rounded(Rect::default(), 0.0, 0.0),
        }
        self.set_region(region);

        // one frame's worth of input has been used
        self.input.click = None;
        self.input.right_click = None;
        self.input.wheel = 0.0;
        self.input.hwheel = 0.0;
        if close_after {
            self.close();
        }
        if let Some(pop) = next_pop {
            let a = self.pop_anchor;
            self.pop = Some(pop);
            self.pop_open = true;
            self.pop_anchor = a;
            self.dirty = true;
            self.start_anim();
        }
    }

    /// Only the strip, the popover, the pill and the banner take the mouse; the rest lets it through.
    fn set_region(&mut self, region: Vec<(i32, i32, i32, i32)>) {
        if region == self.region || region.is_empty() {
            return;
        }
        unsafe {
            let (a, b, c, d) = region[0];
            let rgn = CreateRectRgn(a, b, c, d);
            for &(a, b, c, d) in &region[1..] {
                let t = CreateRectRgn(a, b, c, d);
                CombineRgn(Some(rgn), Some(rgn), Some(t), RGN_OR);
                let _ = DeleteObject(t.into());
            }
            SetWindowRgn(self.hwnd, Some(rgn), true);
        }
        self.region = region;
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_TICK);
            let _ = KillTimer(Some(self.hwnd), T_ANIM);
            let _ = KillTimer(Some(self.hwnd), T_POLL);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

extern "system" fn bar_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_RBUTTONUP {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| context_menu(hwnd, lp)));
        return LRESULT(0);
    }
    let r = std::panic::catch_unwind(|| crate::window::with_app(|a| a.bar_message(msg, wp, lp)).flatten());
    match r {
        Ok(Some(v)) => v,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Right-click: remove the item under the pointer, add one that is missing, or start over.
fn context_menu(hwnd: HWND, lp: LPARAM) {
    let (target, missing) = crate::window::with_app(|a| a.bar_menu_info(lp)).unwrap_or_default();
    let cmd = unsafe {
        let menu = CreatePopupMenu().unwrap_or_default();
        if let Some(t) = &target {
            let name = ALL.iter().find(|a| a.0 == t).map(|a| a.1).unwrap_or("item");
            let s = wstr(&format!("Remove {name} from the bar"));
            let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR(s.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        let labels: Vec<(usize, Vec<u16>)> = missing.iter().map(|(k, n)| (*k, wstr(&format!("Add {n}")))).collect();
        for (k, s) in &labels {
            let _ = AppendMenuW(menu, MF_STRING, 100 + *k, PCWSTR(s.as_ptr()));
        }
        if !labels.is_empty() {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        let reset = wstr("Reset the bar");
        let _ = AppendMenuW(menu, MF_STRING, 2, PCWSTR(reset.as_ptr()));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let c = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        c.0
    };
    if cmd != 0 {
        crate::window::with_app(|a| a.bar_menu_command(cmd, target));
    }
}

fn create_window() -> windows::core::Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS,
            w!("Dynamic Notch Bar"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;
        let no_round = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &no_round as *const _ as _,
            std::mem::size_of_val(&no_round) as u32,
        );
        let on = BOOL(1);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_EXCLUDED_FROM_PEEK, &on as *const _ as _, 4);
        let none: u32 = 0xFFFF_FFFE;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        Ok(hwnd)
    }
}
"""
import pathlib, re

def rep(path, old, new):
    p = pathlib.Path(path)
    s = p.read_text()
    if new in s:
        return
    assert s.count(old) == 1, (path, old[:70], s.count(old))
    p.write_text(s.replace(old, new))

pathlib.Path("src/bar.rs").write_text(BAR)

rep("src/main.rs", "mod bloom;\n", "mod bar;\nmod bloom;\n")

rep("src/bloom.rs", "pub fn dnd() -> bool {\n    DND.load(Ordering::Relaxed)\n}\n",
    "pub fn dnd() -> bool {\n    DND.load(Ordering::Relaxed)\n}\n\n/// Turn do not disturb on or off (the menu bar's Control Center).\npub fn set_dnd(on: bool) {\n    DND.store(on, Ordering::Relaxed);\n}\n")

CFG = '''/// The menu bar that can take the notch's place.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BarCfg {
    /// "notch" or "bar".
    pub mode: String,
    /// Height of the bar in logical pixels.
    pub height: f32,
    /// A tinted glass strip behind the items (off: just the items on the desktop).
    pub background: bool,
    /// What is on the bar, left to right; "spacer" splits the left side from the right side.
    pub items: Vec<String>,
}

impl Default for BarCfg {
    fn default() -> Self {
        Self {
            mode: "notch".to_string(),
            height: 28.0,
            background: true,
            items: ["spacer", "weather", "cpu", "ram", "battery", "wifi", "bluetooth", "sound", "control", "clock"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl BarCfg {
    fn clamped(&mut self) {
        self.height = self.height.clamp(22.0, 44.0);
        if self.mode.trim().to_ascii_lowercase() != "bar" {
            self.mode = "notch".to_string();
        } else {
            self.mode = "bar".to_string();
        }
        // only things the bar knows, each once, and a single divider
        let mut seen: Vec<String> = Vec::new();
        for s in std::mem::take(&mut self.items) {
            let s = s.trim().to_ascii_lowercase();
            if (s == "spacer" || crate::bar::ALL.iter().any(|a| a.0 == s)) && !seen.contains(&s) {
                seen.push(s);
            }
        }
        self.items = seen;
    }
}

'''
rep("src/config.rs", "/// The dock at the bottom of the screen.\n", CFG + "/// The dock at the bottom of the screen.\n")
rep("src/config.rs", "    pub launcher: LauncherCfg,\n", "    pub launcher: LauncherCfg,\n    pub bar: BarCfg,\n")
rep("src/config.rs", "        self.launcher.clamped();\n", "        self.launcher.clamped();\n        self.bar.clamped();\n")

A = "src/app.rs"
rep(A, "    dock: Option<crate::dock::Dock>,\n", "    dock: Option<crate::dock::Dock>,\n    bar: Option<crate::bar::Bar>,\n")
rep(A, "            dock: None,\n", "            dock: None,\n            bar: None,\n")
rep(A, "        self.dock_apply();\n        self.sync_taskbar();\n        self.tray = ", "        self.dock_apply();\n        self.bar_apply();\n        self.sync_taskbar();\n        self.tray = ")
rep(A, "                self.dock_apply();\n                self.sync_taskbar();\n                if autostart_changed {", "                self.dock_apply();\n                self.bar_apply();\n                self.sync_taskbar();\n                if autostart_changed {")
rep(A, "        self.dock_relocate();\n        self.check_fullscreen();", "        self.dock_relocate();\n        self.bar_relocate();\n        self.check_fullscreen();")
rep(A, "            if let Some(d) = self.dock.as_mut() {\n                d.recover(&self.gfx);\n            }\n", "            if let Some(d) = self.dock.as_mut() {\n                d.recover(&self.gfx);\n            }\n            if let Some(b) = self.bar.as_mut() {\n                b.recover(&self.gfx);\n            }\n")
rep(A, "            self.fs_hidden = hide;\n            if hide {\n                self.collapse();\n            }\n", "            self.fs_hidden = hide;\n            if let Some(b) = self.bar.as_mut() {\n                b.set_hidden(hide);\n            }\n            if hide {\n                self.collapse();\n            }\n")
rep(A, "            shown: !(self.user_hidden || self.fs_hidden),\n", "            shown: !(self.user_hidden || self.fs_hidden || self.bar_on()),\n")
rep(A, "        let want = if self.fs_hidden {\n            None\n        } else if settled {", "        let want = if self.fs_hidden || self.bar_on() {\n            None\n        } else if settled {")
rep(A, "        if crate::bloom::dnd() && !p.is_hud() {\n            return;\n        }\n", "        if crate::bloom::dnd() && !p.is_hud() {\n            return;\n        }\n        // the menu bar shows these itself: a pill for volume and brightness, a banner for the rest\n        if self.bar_on() {\n            self.bar_peek(&p);\n            return;\n        }\n")
rep(A, "        if let Some(page) = fx.open_page {\n            self.expand(Some(page));\n        }\n        if fx.expand && self.mode != Mode::Expanded {\n            self.expand(None);\n        }\n",
       "        if let Some(page) = fx.open_page {\n            self.expand(Some(page));\n        }\n        if fx.expand && (self.mode != Mode::Expanded || self.bar_on()) {\n            self.expand(None);\n        }\n")
rep(A, "    fn expand(&mut self, page: Option<ModuleId>) {\n", "    fn expand(&mut self, page: Option<ModuleId>) {\n        if self.bar_on() {\n            self.bar_open(page);\n            return;\n        }\n")
rep(A, "    fn toggle(&mut self) {\n", "    fn toggle(&mut self) {\n        if self.bar_on() {\n            if let Some(b) = self.bar.as_mut() {\n                if b.is_open() {\n                    b.close();\n                } else {\n                    b.open_control();\n                }\n            }\n            return;\n        }\n")

METHODS = '''    /// Is the menu bar standing in for the notch?
    fn bar_on(&self) -> bool {
        self.bar.is_some() && self.cfg.bar.mode == "bar"
    }

    /// Messages for the menu bar's own window.
    pub fn bar_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let r = self.bar.as_mut()?.handle(msg, wp, lp);
        if r.is_some() {
            self.bar_pump();
        }
        r
    }

    /// What a right-click at `lp` landed on in the menu bar, and the items that are not on it.
    pub fn bar_menu_info(&self, lp: LPARAM) -> (Option<String>, Vec<(usize, &'static str)>) {
        self.bar.as_ref().map(|b| b.menu_info(lp)).unwrap_or_default()
    }

    /// A choice from the menu bar's right-click menu.
    pub fn bar_menu_command(&mut self, cmd: i32, target: Option<String>) {
        if let Some(b) = self.bar.as_mut() {
            b.menu_command(cmd, target);
        }
        self.bar_pump();
    }

    fn bar_peek(&mut self, p: &Peek) {
        if let Some(b) = self.bar.as_mut() {
            if p.is_hud() {
                b.show_hud(p);
            } else if p.key != Some("media") {
                b.show_banner(p);
            }
        }
    }

    /// The notch's shortcuts (hotkey, tray, tapping the edge) open the Control Center or a page.
    fn bar_open(&mut self, page: Option<ModuleId>) {
        if let Some(b) = self.bar.as_mut() {
            match page {
                Some(pg) if self.modules.iter().any(|m| m.id() == pg && m.has_page()) => b.open_page(pg),
                _ => b.open_control(),
            }
        }
    }

    /// Paint the menu bar if it asked for it, and carry out what it asked the app to do.
    fn bar_pump(&mut self) {
        let Some(mut b) = self.bar.take() else {
            return;
        };
        let mut fx = Effects::default();
        if b.wants_draw() {
            let mut ind = Vec::new();
            for (i, m) in self.modules.iter().enumerate() {
                if self.started[i] {
                    m.indicators(&mut ind);
                }
            }
            let mut ctx = crate::bar::Ctx {
                modules: &mut self.modules,
                started: &self.started,
                cfg: &self.cfg,
                bus: &self.bus,
                accent: self.accent,
                indicators: &ind,
            };
            b.draw(&self.gfx, &mut ctx, &mut fx);
        }
        let act = b.take_act();
        self.bar = Some(b);
        if let Some(crate::bar::Act::Launcher) = act {
            self.launcher_toggle();
        }
        self.apply_effects(fx);
    }

    /// Start, update or remove the menu bar to match the settings.
    fn bar_apply(&mut self) {
        if self.cfg.bar.mode != "bar" {
            if self.bar.take().is_some() {
                self.request_frame();
            }
            return;
        }
        if self.bar.is_none() {
            match crate::bar::Bar::new(&self.gfx, self.host.compositor.clone()) {
                Ok(b) => self.bar = Some(b),
                Err(e) => {
                    log!("menu bar failed to start: {e}");
                    return;
                }
            }
            if self.mode == Mode::Expanded {
                self.collapse();
            }
            self.request_frame();
        }
        let (mon, scale) = (self.monitor, self.cfg.appearance.scale);
        if mon.is_invalid() {
            return;
        }
        let cfg = self.cfg.bar.clone();
        if let Some(b) = self.bar.as_mut() {
            b.configure(&cfg, mon, scale);
        }
        self.bar_pump();
    }

    /// The screen or the monitor changed: put the menu bar where it belongs.
    fn bar_relocate(&mut self) {
        let (mon, scale) = (self.monitor, self.cfg.appearance.scale);
        if mon.is_invalid() {
            return;
        }
        if let Some(b) = self.bar.as_mut() {
            b.relocate(mon, scale);
        }
    }

'''
rep(A, "    /// Messages for the dock's own window.\n", METHODS + "    /// Messages for the dock's own window.\n")

S = "src/modules/settings.rs"
rep(S, "        1 => 18.0,\n", "        1 => 21.0,\n")
rep(S, '        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");\n    }\n',
'''        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");

        // the top of the screen: the notch, or a menu bar like the Mac's
        let row = row_rect(body, 18);
        label(ui, row, "Top of the screen");
        let sel = usize::from(self.cfg.bar.mode == "bar");
        let seg = Rect::new(row.right() - 232.0, row.y + 3.0, 224.0, row.h - 6.0);
        if let Some(i) = segmented(ui, seg, &["Notch", "Menu bar"], Some(sel), accent) {
            put(ui, "bar", "mode", toml_str(["notch", "bar"][i]));
        }
        let b = &self.cfg.bar;
        toggle_row(ui, row_rect(body, 19), "Menu bar: glass strip", b.background, "bar", "background");
        slider_row(
            ui,
            row_rect(body, 20),
            "Menu bar height",
            b.height,
            (22.0, 44.0),
            format!("{:.0}", b.height),
            "bar",
            "height",
        );
    }
''')
PYEOF
