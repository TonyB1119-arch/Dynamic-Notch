#!/bin/bash
set -e
cd "$(dirname "$0")"
if [ -f src/dock.rs ]; then echo "src/dock.rs already exists, leaving it alone"; exit 0; fi
cat > src/dock.rs <<'DOCKEOF'
//! The dock: a glass bar at the bottom of the screen with your pinned apps and the apps that are
//! open right now. Icons grow under the pointer; a click launches or brings the app forward.
//!
//! Icons: a .png in `%APPDATA%\DynamicNotch\dock-icons\` named after the item (`Chrome.png`) or
//! after its exe (`chrome.png`) replaces the app's own icon. Items are set in `[dock] items`.
//!
//! Cost: nothing runs while the pointer is away except one light scan of the open windows every
//! few seconds; frames are drawn only while the icons are moving.

use std::collections::HashMap;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CLOAKED, DWMWA_EXCLUDED_FROM_PEEK,
    DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, GetMonitorInfoW, SetWindowRgn, HMONITOR, MONITORINFO};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::config::DockCfg;
use crate::gfx::image::decode;
use crate::gfx::{Gfx, ImageData, Painter, Res, TextStyle};
use crate::host::Host;
use crate::sys::{self, dragdrop};
use crate::util::{app_dir, Color, Rect};

const CLASS: PCWSTR = w!("DynamicNotch.Dock");
const MOUSELEAVE: u32 = 0x02A3;
const T_ANIM: usize = 1;
const T_SCAN: usize = 2;
/// Windows that are not apps in the dock's sense.
const SKIP_EXE: [&str; 8] = [
    "applicationframehost.exe",
    "textinputhost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "lockapp.exe",
    "widgets.exe",
    "systemsettings.exe",
];
/// Space under the bar for its shadow, in dips.
const FOOT: f32 = 10.0;

#[derive(Clone)]
struct Item {
    label: String,
    /// What a click opens (an exe, a shortcut, a folder).
    path: String,
    /// The exe that tells whether the app is running ("" when the path is not an exe).
    exe: String,
    /// An explicit icon file (relative to the dock-icons folder, or absolute).
    png: Option<String>,
    pinned: bool,
    /// A window of the app while it runs (0 = not running).
    hwnd: isize,
}

pub struct Dock {
    hwnd: HWND,
    host: Host,
    res: Res,
    cfg: DockCfg,
    pinned: Vec<Item>,
    items: Vec<Item>,
    /// Row of the bar: an item index, or None for the divider between pinned and open apps.
    cells: Vec<Option<usize>>,
    icons: HashMap<String, Option<ImageData>>,
    run_order: Vec<String>,
    mon: HMONITOR,
    ui_scale: f32,
    scale: f32,
    win_w: f32,
    win_h: f32,
    win_px: (i32, i32, i32, i32),
    region_full: Option<bool>,
    mx: f32,
    my: f32,
    mx_to: f32,
    engage: f32,
    engage_to: f32,
    tracking: bool,
    pressed: Option<usize>,
    shown: bool,
}

impl Dock {
    pub fn new(gfx: &Gfx, compositor: Compositor) -> windows::core::Result<Self> {
        let hwnd = create_window()?;
        let host = Host::new_shared(hwnd, gfx, 64, 64, compositor)?;
        let res = Res::new(gfx)?;
        let _ = std::fs::create_dir_all(icons_dir());
        Ok(Self {
            hwnd,
            host,
            res,
            cfg: DockCfg::default(),
            pinned: Vec::new(),
            items: Vec::new(),
            cells: Vec::new(),
            icons: HashMap::new(),
            run_order: Vec::new(),
            mon: HMONITOR::default(),
            ui_scale: 1.0,
            scale: 1.0,
            win_w: 0.0,
            win_h: 0.0,
            win_px: (0, 0, 0, 0),
            region_full: None,
            mx: 0.0,
            my: 0.0,
            mx_to: 0.0,
            engage: 0.0,
            engage_to: 0.0,
            tracking: false,
            pressed: None,
            shown: false,
        })
    }

    /// New settings (or a first start): re-read the items and icons, then lay the bar out.
    pub fn configure(&mut self, gfx: &Gfx, cfg: &DockCfg, mon: HMONITOR, ui_scale: f32) {
        self.cfg = cfg.clone();
        self.pinned = cfg.items.iter().filter(|s| !s.trim().is_empty()).map(|s| parse_item(s)).collect();
        self.icons.clear();
        self.mon = mon;
        self.ui_scale = ui_scale;
        self.refresh_items();
        self.place(gfx);
        unsafe {
            SetTimer(Some(self.hwnd), T_SCAN, 2500, None);
        }
        self.draw(gfx);
    }

    /// The screen changed (or the notch moved to another monitor).
    pub fn relocate(&mut self, gfx: &Gfx, mon: HMONITOR, ui_scale: f32) {
        self.mon = mon;
        self.ui_scale = ui_scale;
        self.place(gfx);
        self.draw(gfx);
    }

    /// After a graphics device loss.
    pub fn recover(&mut self, gfx: &Gfx) {
        let _ = self.host.set_rendering_device(gfx);
        if let Ok(r) = Res::new(gfx) {
            self.res = r;
        }
        self.draw(gfx);
    }

    // -- geometry ---------------------------------------------------------------------------------

    fn sizes(&self) -> (f32, f32, f32, f32) {
        let base = self.cfg.icon_size;
        (base, self.cfg.magnify, (base * 0.16).round(), (base * 0.22).round())
    }

    fn place(&mut self, gfx: &Gfx) {
        let _ = gfx;
        if self.mon.is_invalid() {
            return;
        }
        let (base, mag, gap, pad) = self.sizes();
        unsafe {
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(self.mon, &mut mi).as_bool() {
                return;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(self.mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let scale = dx as f32 / 96.0 * self.ui_scale;
            let work = mi.rcWork;
            let n = self.cells.len() as f32;
            let rest = n * base + (n - 1.0).max(0.0) * gap;
            let room = (work.right - work.left) as f32 / scale - 8.0;
            let win_w = (rest + 2.0 * pad + base * (mag - 1.0) * 2.4 + 24.0).min(room).max(120.0);
            let win_h = base * mag + 2.0 * pad + 44.0;
            let (wpx, hpx) = ((win_w * scale).ceil() as i32, (win_h * scale).ceil() as i32);
            let x = (work.left + work.right) / 2 - wpx / 2;
            let y = work.bottom - hpx - (self.cfg.bottom_margin * scale) as i32;
            let geom = (x, y, wpx, hpx);
            if geom != self.win_px || (scale - self.scale).abs() > 0.001 {
                self.scale = scale;
                self.win_w = win_w;
                self.win_h = win_h;
                self.win_px = geom;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, wpx, hpx, SWP_NOACTIVATE);
                let _ = self.host.resize(wpx, hpx);
                self.region_full = None;
            }
            if !self.shown {
                self.shown = true;
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVE);
            }
        }
        self.set_region(self.engage_to > 0.0);
    }

    /// Clicks only land on the bar while the pointer is away, and on the whole window (room for the
    /// grown icons and the label) while it is over the dock.
    fn set_region(&mut self, full: bool) {
        if self.region_full == Some(full) || self.win_px.2 == 0 {
            return;
        }
        self.region_full = Some(full);
        let (_, _, wpx, hpx) = self.win_px;
        let s = self.scale;
        let (l, t, r, b) = if full {
            (0, 0, wpx, hpx)
        } else {
            let (divider, _) = self.divider_flags();
            let lay = layout(&divider, self.win_w, self.sizes().0, self.sizes().1, self.sizes().2, 0.0, 0.0);
            let bar = self.bar_rect(&lay);
            ((bar.x * s).floor() as i32, (bar.y * s).floor() as i32, (bar.right() * s).ceil() as i32, hpx)
        };
        unsafe {
            let rgn = CreateRectRgn(l.max(0), t.max(0), r.min(wpx), b.min(hpx));
            SetWindowRgn(self.hwnd, Some(rgn), false);
        }
    }

    fn divider_flags(&self) -> (Vec<bool>, usize) {
        (self.cells.iter().map(|c| c.is_none()).collect(), self.cells.len())
    }

    /// The bar behind the icons, from the row's current layout.
    fn bar_rect(&self, lay: &[(f32, f32)]) -> Rect {
        let (base, _, _, pad) = self.sizes();
        let bottom = self.win_h - FOOT;
        let (x0, x1) = match (lay.first(), lay.last()) {
            (Some(f), Some(l)) => (f.0, l.0 + l.1),
            _ => (self.win_w * 0.5, self.win_w * 0.5),
        };
        Rect::new(x0 - pad, bottom - base - 2.0 * pad, x1 - x0 + 2.0 * pad, base + 2.0 * pad)
    }

    // -- the list of items --------------------------------------------------------------------------

    /// Pinned apps first, then the other open apps in the order they appeared. True when the set of
    /// shown apps changed (the bar then needs a new size).
    fn refresh_items(&mut self) -> bool {
        let run = if self.cfg.show_running { running_apps() } else { Vec::new() };
        let mut used = vec![false; run.len()];
        let mut items: Vec<Item> = Vec::new();
        for p in &self.pinned {
            let mut it = p.clone();
            if !it.exe.is_empty() {
                if let Some(k) = run.iter().position(|r| r.0.eq_ignore_ascii_case(&it.exe)) {
                    it.hwnd = run[k].1;
                    used[k] = true;
                }
            }
            items.push(it);
        }
        let np = items.len();
        let mut order: Vec<String> = Vec::new();
        for o in &self.run_order {
            if run.iter().enumerate().any(|(k, r)| !used[k] && r.0.eq_ignore_ascii_case(o)) {
                order.push(o.clone());
            }
        }
        for (k, r) in run.iter().enumerate() {
            if !used[k] && !order.iter().any(|o| o.eq_ignore_ascii_case(&r.0)) {
                order.push(r.0.clone());
            }
        }
        for o in &order {
            if let Some(r) = run.iter().find(|r| r.0.eq_ignore_ascii_case(o)) {
                items.push(Item {
                    label: nice_name(&r.0),
                    path: r.0.clone(),
                    exe: r.0.clone(),
                    png: None,
                    pinned: false,
                    hwnd: r.1,
                });
            }
        }
        self.run_order = order;
        let changed = items.len() != self.items.len()
            || items.iter().zip(&self.items).any(|(a, b)| a.path != b.path || (a.hwnd != 0) != (b.hwnd != 0));
        self.cells.clear();
        for i in 0..items.len() {
            if i == np && np > 0 {
                self.cells.push(None);
            }
            self.cells.push(Some(i));
        }
        self.items = items;
        changed
    }

    fn load_icons(&mut self) {
        let dir = icons_dir();
        for it in &self.items {
            let key = icon_key(it);
            if !self.icons.contains_key(&key) {
                let img = load_icon(it, &dir);
                self.icons.insert(key, img);
            }
        }
    }

    // -- drawing -----------------------------------------------------------------------------------

    fn draw(&mut self, gfx: &Gfx) {
        if !self.shown || self.win_px.2 == 0 {
            return;
        }
        self.load_icons();
        let (base, mag, gap, pad) = self.sizes();
        let (divider, _) = self.divider_flags();
        let lay = layout(&divider, self.win_w, base, mag, gap, self.mx, self.engage);
        let bar = self.bar_rect(&lay);
        let radius = bar.h * 0.32;
        let sc = self.scale;
        self.host.set_backdrop_rounded(Rect::new(bar.x * sc, bar.y * sc, bar.w * sc, bar.h * sc), radius * sc, 1.0);
        let hover =
            if self.engage_to > 0.5 { hit(&lay, &self.cells, bar, base, mag, self.mx_to, self.my) } else { None };
        let Ok((dc, _)) = self.host.begin_draw() else { return };
        {
            let mut p = Painter::new(dc, gfx, &mut self.res, sc, 0.0, 0.0);
            p.clear();
            p.shadow(bar, radius, 0.6);
            p.fill_rounded(bar, radius, Color::rgba(0.07, 0.07, 0.09, 0.42));
            let sheen = Rect::new(bar.x + radius * 0.6, bar.y + 1.0, bar.w - radius * 1.2, 1.0);
            p.fill_rounded(sheen, 0.5, Color::white(0.26));
            p.stroke_rounded(bar.inset(0.5), radius - 0.5, 1.0, Color::white(0.20));
            p.stroke_rounded(bar.inset(1.5), radius - 1.5, 1.0, Color::white(0.05));
            let icon_bottom = bar.bottom() - pad;
            let mut label: Option<(f32, f32, String)> = None;
            for (k, &(x, wd)) in lay.iter().enumerate() {
                let Some(i) = self.cells[k] else {
                    p.fill_rect(Rect::new(x, bar.y + pad * 0.9, 1.0, base * 0.95), Color::white(0.22));
                    continue;
                };
                let it = &self.items[i];
                // a press squeezes the icon a little
                let sq = if self.pressed == Some(i) { 0.9 } else { 1.0 };
                let s = wd * sq;
                let r = Rect::new(x + (wd - s) * 0.5, icon_bottom - s, s, s);
                match self.icons.get(&icon_key(it)).and_then(|o| o.as_ref()) {
                    Some(img) => p.image(img, r, 0.0, 1.0),
                    None => {
                        p.fill_rounded(r, s * 0.24, Color::white(0.16));
                        let initial: String = it.label.chars().next().map(|c| c.to_string()).unwrap_or_default();
                        let st = TextStyle::new(s * 0.46, Color::white(0.92)).semibold().center();
                        p.text(&initial, r, &st);
                    }
                }
                if it.hwnd != 0 {
                    p.fill_circle(r.cx(), bar.bottom() - pad * 0.38, 2.0, Color::white(0.85));
                }
                if hover == Some(i) {
                    label = Some((r.cx(), r.y, it.label.clone()));
                }
            }
            if let Some((cx, top, text)) = label {
                let st = TextStyle::new(13.0, Color::white(0.96)).semibold().center();
                let (tw, _) = p.measure(&text, &st);
                let (lw, lh) = (tw + 22.0, 26.0);
                let lx = (cx - lw * 0.5).clamp(4.0, (self.win_w - lw - 4.0).max(4.0));
                let pill = Rect::new(lx, (top - lh - 8.0).max(2.0), lw, lh);
                p.fill_rounded(pill, lh * 0.5, Color::rgba(0.06, 0.06, 0.08, 0.80));
                p.stroke_rounded(pill.inset(0.5), lh * 0.5 - 0.5, 1.0, Color::white(0.16));
                p.text(&text, pill, &st);
            }
        }
        let _ = self.host.end_draw();
        self.res.end_frame();
    }

    // -- input and timers -----------------------------------------------------------------------------

    pub fn handle(&mut self, gfx: &Gfx, fs_hidden: bool, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
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
                self.my = y;
                self.mx_to = x;
                if self.engage_to < 0.5 {
                    // start right under the pointer instead of sweeping in from the side
                    self.mx = x;
                    self.engage_to = 1.0;
                    self.set_region(true);
                }
                self.start_anim();
                Some(LRESULT(0))
            }
            MOUSELEAVE => {
                self.tracking = false;
                self.pressed = None;
                self.engage_to = 0.0;
                self.start_anim();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.point(lp);
                self.pressed = self.item_at(x, y);
                self.draw(gfx);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = self.point(lp);
                let was = self.pressed.take();
                if let (Some(a), Some(b)) = (was, self.item_at(x, y)) {
                    if a == b {
                        self.activate(a);
                    }
                }
                self.draw(gfx);
                Some(LRESULT(0))
            }
            WM_RBUTTONUP | WM_RBUTTONDOWN => Some(LRESULT(0)),
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_TIMER if wp.0 == T_ANIM => {
                self.step(gfx);
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_SCAN => {
                let changed = self.refresh_items();
                if fs_hidden == self.shown {
                    self.shown = !fs_hidden;
                    unsafe {
                        let _ = ShowWindow(self.hwnd, if fs_hidden { SW_HIDE } else { SW_SHOWNOACTIVE });
                    }
                }
                if changed {
                    self.place(gfx);
                    self.draw(gfx);
                }
                unsafe {
                    SetTimer(Some(self.hwnd), T_SCAN, 2500, None);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn point(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as f32 / self.scale;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / self.scale;
        (x, y)
    }

    fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        let (base, mag, gap, _) = self.sizes();
        let (divider, _) = self.divider_flags();
        let lay = layout(&divider, self.win_w, base, mag, gap, self.mx, self.engage);
        hit(&lay, &self.cells, self.bar_rect(&lay), base, mag, x, y)
    }

    fn start_anim(&mut self) {
        unsafe {
            SetTimer(Some(self.hwnd), T_ANIM, 16, None);
        }
    }

    /// One animation tick: the pointer and the grow factor ease toward where they should be.
    fn step(&mut self, gfx: &Gfx) {
        self.mx += (self.mx_to - self.mx) * 0.32;
        self.engage += (self.engage_to - self.engage) * 0.22;
        if (self.mx - self.mx_to).abs() < 0.15 {
            self.mx = self.mx_to;
        }
        if (self.engage - self.engage_to).abs() < 0.004 {
            self.engage = self.engage_to;
        }
        self.draw(gfx);
        if self.mx == self.mx_to && self.engage == self.engage_to {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_ANIM);
            }
            if self.engage_to == 0.0 {
                self.set_region(false);
            }
        }
    }

    fn activate(&mut self, i: usize) {
        let Some(it) = self.items.get(i) else { return };
        if it.hwnd != 0 {
            sys::activate_window(it.hwnd);
        } else {
            sys::shell_open(&it.path);
            // look again soon, the new window should show its dot
            unsafe {
                SetTimer(Some(self.hwnd), T_SCAN, 1000, None);
            }
        }
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_ANIM);
            let _ = KillTimer(Some(self.hwnd), T_SCAN);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// -- layout --------------------------------------------------------------------------------------------

/// Where each cell of the dock sits for a pointer at `mx` (x in dips inside the window).
/// `divider[k]` marks a thin separator; icons grow in a bell curve around the pointer (`engage`
/// 0..1 fades the effect in and out). Returns (x, width) per cell with the whole row centred.
fn layout(divider: &[bool], win_w: f32, base: f32, mag: f32, gap: f32, mx: f32, engage: f32) -> Vec<(f32, f32)> {
    let n = divider.len();
    let rest = |k: usize| if divider[k] { 2.0 } else { base };
    let total0: f32 = (0..n).map(rest).sum::<f32>() + gap * n.saturating_sub(1) as f32;
    let mut x = (win_w - total0) * 0.5;
    let sigma = base * 1.15;
    let mut centres = Vec::with_capacity(n);
    for k in 0..n {
        centres.push(x + rest(k) * 0.5);
        x += rest(k) + gap;
    }
    let widths: Vec<f32> = (0..n)
        .map(|k| {
            if divider[k] {
                2.0
            } else {
                let d = (centres[k] - mx) / sigma;
                base * (1.0 + (mag - 1.0) * (-d * d).exp() * engage)
            }
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * n.saturating_sub(1) as f32;
    let mut x = (win_w - total) * 0.5;
    widths
        .into_iter()
        .map(|w| {
            let r = (x, w);
            x += w + gap;
            r
        })
        .collect()
}

/// The item under (x, y): its column, from just above the grown icons down to the bar's bottom.
fn hit(lay: &[(f32, f32)], cells: &[Option<usize>], bar: Rect, base: f32, mag: f32, x: f32, y: f32) -> Option<usize> {
    let top = bar.y - base * (mag - 1.0) - 6.0;
    if y < top || y > bar.bottom() + 6.0 {
        return None;
    }
    lay.iter().enumerate().find_map(|(k, &(cx, w))| if x >= cx - 3.0 && x <= cx + w + 3.0 { cells[k] } else { None })
}

// -- items and icons ---------------------------------------------------------------------------------

fn icons_dir() -> PathBuf {
    app_dir().join("dock-icons")
}

/// `path`, `Name|path` or `Name|path|icon.png`; `%VARS%` in the path are expanded.
fn parse_item(s: &str) -> Item {
    let parts: Vec<&str> = s.split('|').map(|p| p.trim()).collect();
    let (label, path, png) = match parts.len() {
        1 => ("", parts[0], ""),
        2 => (parts[0], parts[1], ""),
        _ => (parts[0], parts[1], parts[2]),
    };
    let path = expand_env(path);
    let exe = if path.to_ascii_lowercase().ends_with(".exe") { path.clone() } else { String::new() };
    let label = if label.is_empty() { nice_name(&path) } else { label.to_string() };
    Item { label, path, exe, png: (!png.is_empty()).then(|| png.to_string()), pinned: true, hwnd: 0 }
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let name = &after[..b];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push('%');
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn nice_name(path: &str) -> String {
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn icon_key(it: &Item) -> String {
    format!("{}|{}|{}", it.path.to_ascii_lowercase(), it.label.to_ascii_lowercase(), it.png.as_deref().unwrap_or(""))
}

/// A .png from the dock-icons folder if there is one, else the icon the shell shows for the file.
fn load_icon(it: &Item, dir: &Path) -> Option<ImageData> {
    let stem = Path::new(&it.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(p) = &it.png {
        let p = Path::new(p);
        cands.push(if p.is_absolute() { p.to_path_buf() } else { dir.join(p) });
    }
    for name in [&it.label, &stem] {
        cands.push(dir.join(format!("{name}.png")));
        cands.push(dir.join(format!("{}.png", name.to_ascii_lowercase())));
    }
    for c in cands {
        if let Ok(bytes) = std::fs::read(&c) {
            if let Some(img) = decode(&bytes, 256) {
                return Some(img);
            }
        }
    }
    dragdrop::thumbnail(&it.path, 128)
}

// -- open windows -------------------------------------------------------------------------------------

/// The apps with a normal window right now: (exe path, one of its windows), topmost first.
fn running_apps() -> Vec<(String, isize)> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<(isize, u32)>);
        if is_app_window(h) {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            out.push((h.0 as isize, pid));
        }
        true.into()
    }
    let mut wins: Vec<(isize, u32)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut wins as *mut _ as isize));
    }
    let me = std::process::id();
    let mut paths: HashMap<u32, String> = HashMap::new();
    let mut out: Vec<(String, isize)> = Vec::new();
    for (h, pid) in wins {
        if pid == me {
            continue;
        }
        let path = paths.entry(pid).or_insert_with(|| exe_of(pid)).clone();
        if path.is_empty() {
            continue;
        }
        let file = path.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
        if SKIP_EXE.contains(&file.as_str()) || out.iter().any(|o| o.0.eq_ignore_ascii_case(&path)) {
            continue;
        }
        out.push((path, h));
    }
    out
}

unsafe fn is_app_window(h: HWND) -> bool {
    if !IsWindowVisible(h).as_bool() {
        return false;
    }
    if !GetWindow(h, GW_OWNER).map_or(true, |o| o.is_invalid()) {
        return false;
    }
    let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_APPWINDOW.0 == 0 {
        return false;
    }
    if GetWindowTextLengthW(h) <= 0 {
        return false;
    }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4).is_ok() && cloaked != 0 {
        return false;
    }
    let mut cls = [0u16; 64];
    let n = GetClassNameW(h, &mut cls).max(0) as usize;
    let class = String::from_utf16_lossy(&cls[..n.min(64)]);
    !matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

fn exe_of(pid: u32) -> String {
    if pid <= 4 {
        return String::new();
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if ok {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        }
    }
}

// -- the window ------------------------------------------------------------------------------------------

fn create_window() -> windows::core::Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(dock_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS,
            w!("Dynamic Notch Dock"),
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
        let none: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        Ok(hwnd)
    }
}

extern "system" fn dock_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| crate::window::with_app(|a| a.dock_message(msg, wp, lp)).flatten());
    match r {
        Ok(Some(v)) => v,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
DOCKEOF
echo "Wrote src/dock.rs ($(wc -l < src/dock.rs) lines)"
