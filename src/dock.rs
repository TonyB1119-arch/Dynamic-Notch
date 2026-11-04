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

use windows::core::{w, Interface, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, WPARAM};
use std::os::windows::process::CommandExt;
use std::sync::OnceLock;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, IShellLinkW, ShellLink, SIGDN_FILESYSPATH};
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
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
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
const T_HIDE: usize = 3;
/// Posted when another window came to the front: put the dock back on top.
const WM_RAISE: u32 = WM_APP + 7;
static DOCK_HWND: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _e: u32, _w: HWND, _o: i32, _c: i32, _t: u32, _ms: u32) {
    let h = DOCK_HWND.load(Ordering::Relaxed);
    if h != 0 {
        let _ = PostMessageW(Some(HWND(h as *mut _)), WM_RAISE, WPARAM(0), LPARAM(0));
    }
}
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
/// The pin picker panel (dips).
const PICK_W: f32 = 340.0;
const PICK_H: f32 = 400.0;
const PICK_ROW: f32 = 38.0;

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
    /// Cache key of its icon (built once, not on every frame).
    key: String,
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
    /// The pin picker: installed apps (Start menu), first visible row, open or not.
    anim_on: bool,
    last_tick: Instant,
    hook: Option<HWINEVENTHOOK>,
    picker: bool,
    apps: Vec<Item>,
    scroll: usize,
    mon: HMONITOR,
    ui_scale: f32,
    scale: f32,
    win_w: f32,
    win_h: f32,
    win_px: (i32, i32, i32, i32),
    region_full: Option<u8>,
    /// Auto-hide: 0 = shown, 1 = slid away; `hide_to` is where it is heading.
    hide_t: f32,
    hide_to: f32,
    mx: f32,
    my: f32,
    mx_to: f32,
    engage: f32,
    engage_to: f32,
    tracking: bool,
    pressed: Option<usize>,
    /// Dragging a pinned icon to a new place: its index, where the press began and where the pointer is.
    drag: Option<usize>,
    press_x: f32,
    drag_x: f32,
    shown: bool,
}

impl Dock {
    pub fn new(gfx: &Gfx, compositor: Compositor) -> windows::core::Result<Self> {
        let hwnd = create_window()?;
        let host = Host::new_shared(hwnd, gfx, 64, 64, compositor)?;
        let res = Res::new(gfx)?;
        let _ = std::fs::create_dir_all(icons_dir());
        // ask Windows for the Store apps in the background so the picker opens instantly
        std::thread::spawn(|| {
            uwp_apps();
        });
        DOCK_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let hook = unsafe {
            SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT)
        };
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
            anim_on: false,
            last_tick: Instant::now(),
            hook: Some(hook),
            picker: false,
            apps: Vec::new(),
            scroll: 0,
            mon: HMONITOR::default(),
            ui_scale: 1.0,
            scale: 1.0,
            win_w: 0.0,
            win_h: 0.0,
            win_px: (0, 0, 0, 0),
            region_full: None,
            hide_t: 0.0,
            hide_to: 0.0,
            mx: 0.0,
            my: 0.0,
            mx_to: 0.0,
            engage: 0.0,
            engage_to: 0.0,
            tracking: false,
            pressed: None,
            drag: None,
            press_x: 0.0,
            drag_x: 0.0,
            shown: false,
        })
    }

    /// New settings (or a first start): re-read the items and icons, then lay the bar out.
    pub fn configure(&mut self, gfx: &Gfx, cfg: &DockCfg, mon: HMONITOR, ui_scale: f32) {
        // Settings reload on every change (a slider drag sends many): only redo what actually changed.
        let old = std::mem::replace(&mut self.cfg, cfg.clone());
        self.cfg.items.retain(|s| !s.trim().is_empty());
        if old.auto_hide != cfg.auto_hide {
            self.region_full = None;
            if cfg.auto_hide {
                self.hide_to = 1.0;
                self.start_anim();
            } else {
                self.hide_t = 0.0;
                self.hide_to = 0.0;
            }
        }
        if old.melt != cfg.melt {
            self.region_full = None;
        }
        if old.hide_taskbar != cfg.hide_taskbar {
            if cfg.hide_taskbar {
                crate::taskbar::hide_taskbar();
            } else {
                crate::taskbar::recover_taskbar();
            }
        }
        let items_changed = old.items != cfg.items || old.show_running != cfg.show_running || old.hide_apps != cfg.hide_apps;
        if items_changed || self.items.is_empty() {
            self.pinned = cfg.items.iter().filter(|s| !s.trim().is_empty()).map(|s| parse_item(s)).collect();
        }
        self.mon = mon;
        self.ui_scale = ui_scale;
        if items_changed || self.items.is_empty() {
            self.refresh_items();
            crate::log!("dock: {} items, monitor ok: {}", self.items.len(), !mon.is_invalid());
        }
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

    /// Room under the bar: the shadow, plus (with auto-hide) the gap to the screen edge, so the
    /// window can touch the edge and catch the pointer there.
    fn foot(&self) -> f32 {
        if self.cfg.melt {
            0.0
        } else if self.cfg.auto_hide {
            FOOT + self.cfg.bottom_margin
        } else {
            FOOT
        }
    }

    fn place(&mut self, gfx: &Gfx) {
        let _ = gfx;
        if self.mon.is_invalid() {
            crate::log!("dock: no monitor");
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
            let rest = n * base + (n - 1.0).max(0.0) * gap + base * 0.5;
            let room = (work.right - work.left) as f32 / scale - 8.0;
            let win_w = (rest + 2.0 * pad + base * (mag - 1.0) * 2.4 + 24.0).min(room).max(120.0);
            let win_w = if self.picker { win_w.max(PICK_W + 16.0).min(room.max(PICK_W + 16.0)) } else { win_w };
            let win_h = base * mag + 2.0 * pad + 44.0 + self.foot().max(FOOT) - FOOT + if self.picker { PICK_H + 12.0 } else { 0.0 };
            let (wpx, hpx) = ((win_w * scale).ceil() as i32, (win_h * scale).ceil() as i32);
            let x = (work.left + work.right) / 2 - wpx / 2;
            let lift = if self.cfg.auto_hide || self.cfg.melt { 0 } else { (self.cfg.bottom_margin * scale) as i32 };
            let y = work.bottom - hpx - lift;
            let geom = (x, y, wpx, hpx);
            if geom != self.win_px || (scale - self.scale).abs() > 0.001 {
                self.scale = scale;
                self.win_w = win_w;
                self.win_h = win_h;
                self.win_px = geom;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, wpx, hpx, SWP_NOACTIVATE);
                let _ = self.host.resize(wpx, hpx);
                crate::log!("dock: window {wpx}x{hpx} at {x},{y} scale {scale:.2}");
                self.region_full = None;
            }
            if !self.shown {
                self.shown = true;
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        self.set_region(self.engage_to > 0.0);
    }

    /// Back to the front of the always-on-top windows, without taking focus.
    fn raise(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    fn hidden(&self) -> bool {
        self.cfg.auto_hide && self.hide_t >= 0.999
    }

    /// Clicks only land on the bar while the pointer is away, and on the whole window (room for the
    /// grown icons and the label) while it is over the dock.
    fn set_region(&mut self, full: bool) {
        let mode = if self.picker {
            1u8
        } else if self.hidden() && !full {
            2u8
        } else if full {
            1
        } else {
            0
        };
        if self.region_full == Some(mode) || self.win_px.2 == 0 {
            return;
        }
        self.region_full = Some(mode);
        let (_, _, wpx, hpx) = self.win_px;
        let s = self.scale;
        let e = self.ear();
        let (l, t, r, b) = if mode == 1 {
            (0, 0, wpx, hpx)
        } else if mode == 2 {
            // only a thin strip on the screen edge wakes the dock
            let (divider, _) = self.divider_flags();
            let lay = layout(&divider, self.win_w, self.sizes().0, self.sizes().1, self.sizes().2, 0.0, 0.0);
            let bar = self.bar_rect(&lay);
            (((bar.x - e) * s).floor() as i32, hpx - (4.0 * s).ceil() as i32, ((bar.right() + e) * s).ceil() as i32, hpx)
        } else {
            let (divider, _) = self.divider_flags();
            let lay = layout(&divider, self.win_w, self.sizes().0, self.sizes().1, self.sizes().2, 0.0, 0.0);
            let bar = self.bar_rect(&lay);
            (((bar.x - e) * s).floor() as i32, (bar.y * s).floor() as i32, ((bar.right() + e) * s).ceil() as i32, hpx)
        };
        unsafe {
            let rgn = CreateRectRgn(l.max(0), t.max(0), r.min(wpx), b.min(hpx));
            SetWindowRgn(self.hwnd, Some(rgn), false);
        }
    }

    fn divider_flags(&self) -> (Vec<bool>, usize) {
        (self.cells.iter().map(|c| c.is_none()).collect(), self.cells.len())
    }

    /// How far the curved feet of the melted dock reach out to each side (0 when it is off).
    fn ear(&self) -> f32 {
        if !self.cfg.melt {
            return 0.0;
        }
        let (base, _, _, pad) = self.sizes();
        ((base + 2.0 * pad) * 0.26).clamp(10.0, 26.0)
    }

    /// The bar behind the icons, from the row's current layout.
    fn bar_rect(&self, lay: &[(f32, f32)]) -> Rect {
        let (base, _, _, pad) = self.sizes();
        let bottom = self.win_h - self.foot();
        let (x0, x1) = match (lay.first(), lay.last()) {
            (Some(f), Some(l)) => (f.0, l.0 + l.1),
            _ => (self.win_w * 0.5, self.win_w * 0.5),
        };
        let hp = pad + (base + 2.0 * pad) * 0.12;
        Rect::new(x0 - hp, bottom - base - 2.0 * pad, x1 - x0 + 2.0 * hp, base + 2.0 * pad)
    }

    // -- the list of items --------------------------------------------------------------------------

    /// Pinned apps first, then the other open apps in the order they appeared. True when the set of
    /// shown apps changed (the bar then needs a new size).
    fn refresh_items(&mut self) -> bool {
        let run = if self.cfg.show_running { running_apps(&self.cfg.hide_apps) } else { Vec::new() };
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
                items.push(with_key(Item {
                    label: nice_name(&r.0),
                    path: r.0.clone(),
                    exe: r.0.clone(),
                    png: None,
                    pinned: false,
                    hwnd: r.1,
                    key: String::new(),
                }));
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
            if !self.icons.contains_key(&it.key) {
                let img = load_icon(it, &dir);
                self.icons.insert(it.key.clone(), img);
            }
        }
    }

    // -- drawing -----------------------------------------------------------------------------------

    fn draw(&mut self, gfx: &Gfx) {
        if !self.shown || self.win_px.2 == 0 {
            return;
        }
        self.load_icons();
        if self.picker {
            self.load_picker_icons();
        }
        let (base, mag, gap, pad) = self.sizes();
        let (divider, _) = self.divider_flags();
        let engage = if self.drag.is_some() { 0.0 } else { self.engage };
        let lay = layout(&divider, self.win_w, base, mag, gap, self.mx, engage);
        let bar = self.bar_rect(&lay);
        let radius = bar.h * 0.36;
        let sc = self.scale;
        let dy = self.hide_t * (bar.h + self.foot() + 6.0);
        let op = self.cfg.opacity;
        self.host.set_backdrop_rounded(
            Rect::new(bar.x * sc, (bar.y + dy) * sc, bar.w * sc, (bar.h + if self.cfg.melt { radius } else { 0.0 }) * sc),
            radius * sc,
            if self.hide_t > 0.98 || !self.cfg.glass { 0.0 } else { op },
        );
        let hover =
            if self.engage_to > 0.5 && self.hide_t < 0.3 && self.drag.is_none() { hit(&lay, &self.cells, bar, base, mag, self.mx_to, self.my) } else { None };
        let (dc, _) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(e) => {
                crate::log!("dock: begin_draw failed: {e}");
                return;
            }
        };
        {
            let panel = self.panel_rect();
            let ear = self.ear();
            let mut p = Painter::new(dc, gfx, &mut self.res, sc, 0.0, 0.0);
            p.clear();
            p.set_local(1.0, 0.0, 0.0, 0.0, dy);
            if !self.cfg.glass {
                // liquid glass off: a plain solid bar
                p.fill_rounded(bar, radius, Color::rgba(0.06, 0.06, 0.08, 0.94));
            }
            // plain blur: the blurred screen behind it plus one flat tint (the slider sets its strength)
            // no tint: the clear blur behind it is the whole background, `op` sets how strong it is
            // the white rim: thickness and brightness come from the settings
            let rw = self.cfg.rim_width;
            if self.cfg.glass && rw > 0.05 && self.cfg.rim_bright > 0.005 {
                if self.cfg.melt {
                    if let Some(g) = melt_ring(&p, bar, radius, ear, rw) {
                        p.fill_geometry(&g, Color::white(self.cfg.rim_bright));
                    }
                } else {
                    p.stroke_rounded(bar.inset(rw * 0.5), (radius - rw * 0.5).max(0.0), rw, Color::white(self.cfg.rim_bright));
                }
            }
            let icon_bottom = bar.bottom() - pad;
            let mut label: Option<(f32, f32, String)> = None;
            for (k, &(x, wd)) in lay.iter().enumerate() {
                let Some(i) = self.cells[k] else {
                    p.fill_rect(Rect::new(x, bar.y + pad * 0.9, 1.0, base * 0.95), Color::white(0.22));
                    continue;
                };
                let it = &self.items[i];
                // a press squeezes the icon a little
                let dragged = self.drag == Some(i);
                let sq = if dragged { 1.06 } else if self.pressed == Some(i) { 0.9 } else { 1.0 };
                let s = wd * sq;
                let mut r = Rect::new(x + (wd - s) * 0.5, icon_bottom - s, s, s);
                if dragged {
                    // follows the pointer, a little lifted
                    r = Rect::new(self.drag_x - s * 0.5, icon_bottom - s - 8.0, s, s);
                }
                match self.icons.get(&it.key).and_then(|o| o.as_ref()) {
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
            if self.picker {
                p.set_local(1.0, 0.0, 0.0, 0.0, 0.0);
                let pw = panel;
                p.fill_rounded(pw, 20.0, Color::rgba(0.09, 0.09, 0.11, 0.96));
                let title = TextStyle::new(15.0, Color::white(0.95)).semibold();
                p.text("Pin apps", Rect::new(pw.x + 16.0, pw.y + 10.0, 100.0, 28.0), &title);
                let dim = TextStyle::new(11.0, Color::white(0.45));
                p.text("scroll for more", Rect::new(pw.x + 96.0, pw.y + 10.0, 130.0, 28.0), &dim);
                let done = Rect::new(pw.right() - 76.0, pw.y + 10.0, 64.0, 28.0);
                p.fill_rounded(done, 14.0, Color::white(0.14));
                p.text("Done", done, &TextStyle::new(12.0, Color::white(0.95)).semibold().center());
                let vis = ((PICK_H - 56.0) / PICK_ROW) as usize;
                if self.apps.is_empty() {
                    p.text("No apps found", Rect::new(pw.x, pw.y + 60.0, pw.w, 30.0), &TextStyle::new(11.0, Color::white(0.45)).center());
                }
                let name_st = TextStyle::new(13.0, Color::white(0.92));
                let pill_st = TextStyle::new(11.5, Color::white(0.9)).semibold().center();
                let pill_on = TextStyle::new(11.5, Color::rgba(0.08, 0.08, 0.1, 1.0)).semibold().center();
                for (k, it) in self.apps.iter().skip(self.scroll).take(vis).enumerate() {
                    let row = Rect::new(pw.x + 8.0, pw.y + 52.0 + k as f32 * PICK_ROW, pw.w - 16.0, PICK_ROW - 2.0);
                    if row.contains(self.mx_to, self.my) {
                        p.fill_rounded(row, 10.0, Color::white(0.07));
                    }
                    let ir = Rect::new(row.x + 8.0, row.y + (row.h - 26.0) * 0.5, 26.0, 26.0);
                    match self.icons.get(&it.key).and_then(|o| o.as_ref()) {
                        Some(img) => p.image(img, ir, 0.0, 1.0),
                        None => p.fill_rounded(ir, 7.0, Color::white(0.14)),
                    }
                    p.text(&it.label, Rect::new(row.x + 46.0, row.y, row.w - 46.0 - 80.0, row.h), &name_st);
                    let on = self.pinned.iter().any(|q| {
                        q.label.eq_ignore_ascii_case(&it.label) || q.path.eq_ignore_ascii_case(&it.path)
                    });
                    let pill = Rect::new(row.right() - 72.0, row.y + (row.h - 24.0) * 0.5, 64.0, 24.0);
                    if on {
                        p.fill_rounded(pill, 12.0, Color::white(0.92));
                        p.text("Pinned", pill, &pill_on);
                    } else {
                        p.fill_rounded(pill, 12.0, Color::white(0.12));
                        p.text("Pin", pill, &pill_st);
                    }
                }
            }
        }
        if let Err(e) = self.host.end_draw() {
            crate::log!("dock: end_draw failed: {e}");
        }
        self.res.end_frame();
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
            crate::log!("dock: drew a frame ({} cells, bar {:.0}x{:.0})", self.cells.len(), bar.w, bar.h);
        }
    }

    // -- input and timers -----------------------------------------------------------------------------

    pub fn handle(&mut self, gfx: &Gfx, fs_hidden: bool, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_RAISE => {
                if self.shown {
                    self.raise();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL if self.picker => {
                let d = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                let max = self.apps.len().saturating_sub(self.visible_rows()) as i32;
                self.scroll = (self.scroll as i32 - d / 120 * 2).clamp(0, max) as usize;
                self.draw(gfx);
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN if self.picker => Some(LRESULT(0)),
            WM_LBUTTONUP if self.picker => {
                let (x, y) = self.point(lp);
                self.picker_click(gfx, x, y);
                Some(LRESULT(0))
            }
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
                if let (Some(i), None) = (self.pressed, self.drag) {
                    if (x - self.press_x).abs() > 6.0 && i < self.pinned.len() {
                        self.drag = Some(i);
                        unsafe {
                            let _ = SetCapture(self.hwnd);
                        }
                    }
                }
                if let Some(i) = self.drag {
                    self.drag_x = x;
                    let to = self.drop_slot(i, x);
                    if to != i {
                        self.move_pinned(i, to);
                        self.drag = Some(to);
                        self.pressed = Some(to);
                    }
                }
                if self.cfg.auto_hide {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_HIDE);
                    }
                    if self.hide_to != 0.0 {
                        self.hide_to = 0.0;
                    }
                }
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
                if self.cfg.auto_hide {
                    unsafe {
                        SetTimer(Some(self.hwnd), T_HIDE, 700, None);
                    }
                }
                self.start_anim();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_HIDE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_HIDE);
                }
                if self.cfg.auto_hide && self.engage_to == 0.0 && !self.picker {
                    self.hide_to = 1.0;
                    self.start_anim();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.point(lp);
                self.pressed = self.item_at(x, y);
                self.press_x = x;
                self.draw(gfx);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = self.point(lp);
                let was = self.pressed.take();
                if self.drag.take().is_some() {
                    // dropped: keep the new order
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    crate::config::set_value("dock", "items", &items_value(&self.cfg.items));
                    if x < 0.0 || y < 0.0 || x > self.win_w || y > self.win_h {
                        self.tracking = false;
                        self.engage_to = 0.0;
                        self.start_anim();
                    }
                    self.draw(gfx);
                    return Some(LRESULT(0));
                }
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
                if self.shown {
                    self.raise();
                }
                if self.cfg.hide_taskbar {
                    crate::taskbar::hide_taskbar();
                    self.place(gfx);
                }
                if fs_hidden == self.shown {
                    self.shown = !fs_hidden;
                    unsafe {
                        let _ = ShowWindow(self.hwnd, if fs_hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
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

    /// What a right-click at `lp` landed on: (label, path, pinned).
    pub fn menu_target(&self, lp: LPARAM) -> Option<(String, String, bool)> {
        let (x, y) = self.point(lp);
        let it = self.items.get(self.item_at(x, y)?)?;
        Some((it.label.clone(), it.path.clone(), it.pinned))
    }

    // -- dragging pinned icons --------------------------------------------------------------------

    /// Where the dragged pinned icon `i` belongs for a pointer at `x`: its place among the pinned
    /// icons, counted by how many of the others lie to its left (read from the resting layout).
    fn drop_slot(&self, i: usize, x: f32) -> usize {
        let (base, mag, gap, _) = self.sizes();
        let (divider, _) = self.divider_flags();
        let lay = layout(&divider, self.win_w, base, mag, gap, 0.0, 0.0);
        let np = self.pinned.len();
        let mut to = 0;
        for (k, c) in self.cells.iter().enumerate() {
            if let Some(j) = *c {
                if j < np && j != i && lay[k].0 + lay[k].1 * 0.5 < x {
                    to += 1;
                }
            }
        }
        to
    }

    /// Move pinned icon `from` to place `to`, in the shown list and in the saved list.
    fn move_pinned(&mut self, from: usize, to: usize) {
        let n = self.pinned.len();
        if from >= n || to >= n || self.cfg.items.len() != n || self.items.len() < n {
            return;
        }
        let it = self.pinned.remove(from);
        self.pinned.insert(to, it);
        let raw = self.cfg.items.remove(from);
        self.cfg.items.insert(to, raw);
        let shown = self.items.remove(from);
        self.items.insert(to, shown);
    }

    // -- the pin picker -----------------------------------------------------------------------------

    fn panel_rect(&self) -> Rect {
        let w = PICK_W.min(self.win_w - 16.0);
        Rect::new((self.win_w - w) * 0.5, 4.0, w, PICK_H)
    }

    fn visible_rows(&self) -> usize {
        ((PICK_H - 56.0) / PICK_ROW) as usize
    }

    pub fn open_picker(&mut self, gfx: &Gfx) {
        self.apps = scan_start_menu();
        self.scroll = 0;
        self.picker = true;
        self.hide_t = 0.0;
        self.hide_to = 0.0;
        self.region_full = None;
        self.place(gfx);
        self.draw(gfx);
    }

    fn close_picker(&mut self, gfx: &Gfx) {
        self.picker = false;
        self.apps = Vec::new();
        let items = &self.items;
        self.icons.retain(|k, _| items.iter().any(|i| &i.key == k));
        self.region_full = None;
        self.place(gfx);
        if self.cfg.auto_hide {
            unsafe {
                SetTimer(Some(self.hwnd), T_HIDE, 700, None);
            }
        }
        self.draw(gfx);
    }

    fn load_picker_icons(&mut self) {
        let dir = icons_dir();
        let vis = self.visible_rows();
        for it in self.apps.iter().skip(self.scroll).take(vis) {
            if !self.icons.contains_key(&it.key) {
                let img = load_icon(it, &dir);
                self.icons.insert(it.key.clone(), img);
            }
        }
    }

    fn picker_click(&mut self, gfx: &Gfx, x: f32, y: f32) {
        let pw = self.panel_rect();
        if !pw.contains(x, y) || Rect::new(pw.right() - 76.0, pw.y + 10.0, 64.0, 28.0).contains(x, y) {
            self.close_picker(gfx);
            return;
        }
        let ly = pw.y + 52.0;
        if y >= ly {
            let k = self.scroll + ((y - ly) / PICK_ROW) as usize;
            if (y - ly) / PICK_ROW < self.visible_rows() as f32 && k < self.apps.len() {
                self.toggle_pin(gfx, k);
            }
        }
    }

    fn toggle_pin(&mut self, gfx: &Gfx, k: usize) {
        let (name, path) = (self.apps[k].label.clone(), self.apps[k].path.clone());
        let mut items = self.cfg.items.clone();
        let before = items.len();
        items.retain(|s| {
            let p = parse_item(s);
            !(p.label.eq_ignore_ascii_case(&name) || p.path.eq_ignore_ascii_case(&path))
        });
        if items.len() == before {
            let target =
                resolve_lnk(&path).filter(|t| t.to_ascii_lowercase().ends_with(".exe")).unwrap_or(path);
            items.push(format!("{name}|{target}"));
        }
        self.cfg.items = items;
        crate::config::set_value("dock", "items", &items_value(&self.cfg.items));
        self.pinned = self.cfg.items.iter().filter(|s| !s.trim().is_empty()).map(|s| parse_item(s)).collect();
        self.refresh_items();
        self.place(gfx);
        self.draw(gfx);
    }

    /// A new icon file was chosen (or removed): load every icon again.
    pub fn reload_icons(&mut self, gfx: &Gfx) {
        self.icons.clear();
        self.draw(gfx);
    }

    /// The pinned list as the dock currently has it.
    pub fn items_cfg(&self) -> Vec<String> {
        self.cfg.items.clone()
    }

    fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.hide_t > 0.3 {
            return None;
        }
        let (base, mag, gap, _) = self.sizes();
        let (divider, _) = self.divider_flags();
        let lay = layout(&divider, self.win_w, base, mag, gap, self.mx, self.engage);
        hit(&lay, &self.cells, self.bar_rect(&lay), base, mag, x, y)
    }

    fn start_anim(&mut self) {
        // Only start the timer once: restarting it on every mouse move would keep it from ever firing.
        if !self.anim_on {
            self.anim_on = true;
            self.last_tick = Instant::now();
            unsafe {
                SetTimer(Some(self.hwnd), T_ANIM, 10, None);
            }
        }
    }

    /// One animation tick: the pointer and the grow factor ease toward where they should be.
    fn step(&mut self, gfx: &Gfx) {
        // ease by elapsed time so the speed is the same whatever the timer's rhythm
        let now = Instant::now();
        let f = (now.duration_since(self.last_tick).as_secs_f32() * 60.0).clamp(0.5, 4.0);
        self.last_tick = now;
        let ease = |k: f32| 1.0 - (1.0 - k).powf(f);
        self.mx += (self.mx_to - self.mx) * ease(0.32);
        self.engage += (self.engage_to - self.engage) * ease(0.22);
        if self.hide_t != self.hide_to {
            let k = ease(if self.hide_to > self.hide_t { 0.2 } else { 0.3 });
            self.hide_t += (self.hide_to - self.hide_t) * k;
            if (self.hide_t - self.hide_to).abs() < 0.01 {
                self.hide_t = self.hide_to;
                self.region_full = None;
                self.set_region(self.engage_to > 0.0);
            }
        }
        if (self.mx - self.mx_to).abs() < 0.15 {
            self.mx = self.mx_to;
        }
        if (self.engage - self.engage_to).abs() < 0.004 {
            self.engage = self.engage_to;
        }
        self.draw(gfx);
        if self.mx == self.mx_to && self.engage == self.engage_to && self.hide_t == self.hide_to {
            self.anim_on = false;
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
            let _ = KillTimer(Some(self.hwnd), T_HIDE);
            if let Some(h) = self.hook.take() {
                let _ = UnhookWinEvent(h);
            }
            DOCK_HWND.store(0, Ordering::Relaxed);
            if self.cfg.hide_taskbar {
                crate::taskbar::recover_taskbar();
            }
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
    with_key(Item { label, path, exe, png: (!png.is_empty()).then(|| png.to_string()), pinned: true, hwnd: 0, key: String::new() })
}

fn with_key(mut it: Item) -> Item {
    it.key = icon_key(&it);
    it
}

pub fn expand_env(s: &str) -> String {
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

/// The picture the dock shows for an app called `label` that opens `path` (so the radial menu matches).
pub fn icon_for(label: &str, path: &str) -> Option<ImageData> {
    let it = with_key(Item {
        label: label.to_string(),
        path: path.to_string(),
        exe: String::new(),
        png: None,
        pinned: false,
        hwnd: 0,
        key: String::new(),
    });
    load_icon(&it, &icons_dir())
}

/// The white rim of the melted dock as one filled outline: round top corners, straight sides and
/// concave feet that flare out and end flat on the screen edge (the notch's shape, upside down).
/// The outline continues below the window, so no line shows along the screen edge.
fn melt_ring(p: &Painter, bar: Rect, r: f32, e: f32, w: f32) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Geometry> {
    let (x0, x1, t, b) = (bar.x, bar.right(), bar.y, bar.bottom());
    let h = w * 0.5;
    let e = e.min(bar.h * 0.5).max(h + 0.5);
    let r = r.min(bar.w * 0.5).min(bar.h - e).max(h + 0.5);
    p.path(|g| {
        g.move_to(x0 - e, b - h);
        g.arc_to(x0 - h, b - e, e - h, false);
        g.line_to(x0 - h, t + r);
        g.arc_to(x0 + r, t - h, r + h, true);
        g.line_to(x1 - r, t - h);
        g.arc_to(x1 + h, t + r, r + h, true);
        g.line_to(x1 + h, b - e);
        g.arc_to(x1 + e, b - h, e - h, false);
        g.line_to(x1 + e, b + h);
        g.arc_to(x1 - h, b - e, e + h, true);
        g.line_to(x1 - h, t + r);
        g.arc_to(x1 - r, t + h, r - h, false);
        g.line_to(x0 + r, t + h);
        g.arc_to(x0 + h, t + r, r - h, false);
        g.line_to(x0 + h, b - e);
        g.arc_to(x0 - e, b + h, e + h, true);
        g.close();
    })
}

// -- open windows -------------------------------------------------------------------------------------

/// The apps with a normal window right now: (exe path, one of its windows), topmost first.
thread_local! {
    /// pid -> exe path, kept between scans so each pass doesn't reopen every process.
    static EXE_CACHE: std::cell::RefCell<HashMap<u32, String>> = std::cell::RefCell::new(HashMap::new());
}

fn running_apps(hide: &[String]) -> Vec<(String, isize)> {
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
    let mut paths: HashMap<u32, String> = EXE_CACHE.with(|c| std::mem::take(&mut *c.borrow_mut()));
    paths.retain(|pid, p| !p.is_empty() && wins.iter().any(|w| w.1 == *pid));
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
        if SKIP_EXE.contains(&file.as_str()) || hide.iter().any(|h| h.trim().eq_ignore_ascii_case(&file)) || out.iter().any(|o| o.0.eq_ignore_ascii_case(&path)) {
            continue;
        }
        out.push((path, h));
    }
    EXE_CACHE.with(|c| *c.borrow_mut() = paths);
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

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn same_path(entry: &str, path: &str) -> bool {
    parse_item(entry).path.eq_ignore_ascii_case(path)
}

fn save_items(items: &[String]) {
    crate::config::set_value("dock", "items", &items_value(items));
    crate::window::with_app(|a| a.on_menu(crate::tray::CMD_RELOAD));
}

fn items_value(items: &[String]) -> String {
    let arr: Vec<String> = items.iter().map(|s| format!("'{}'", s.replace('\'', ""))).collect();
    format!("[{}]", arr.join(", "))
}

/// Where a shortcut points to (None for Store apps), for the launcher's quit / restart.
pub fn lnk_target(path: &str) -> Option<String> {
    resolve_lnk(path)
}

/// The program a shortcut points to ("" when it has none, as with Store apps).
fn resolve_lnk(path: &str) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let wp = wstr(path);
        pf.Load(PCWSTR(wp.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 520];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
    }
}

/// Store apps (Camera, Settings, Microsoft Store...) have no shortcut file: ask Windows for them.
/// Returns (name, app id); asked only once per run.
fn uwp_apps() -> &'static Vec<(String, String)> {
    static APPS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    APPS.get_or_init(|| {
        let script = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-StartApps | ForEach-Object { $_.Name + '|' + $_.AppID }";
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x0800_0000)
            .output();
        let mut v: Vec<(String, String)> = Vec::new();
        if let Ok(o) = out {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Some((name, id)) = line.trim().rsplit_once('|') {
                    // Store apps have an id like Package_family!App
                    if id.contains('!') && !name.is_empty() {
                        v.push((name.trim().to_string(), id.trim().to_string()));
                    }
                }
            }
        }
        crate::log!("dock: {} store apps", v.len());
        v
    })
}

/// A file picker for an icon image.
fn pick_image(owner: HWND) -> Option<String> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let spec = [COMDLG_FILTERSPEC { pszName: w!("Images"), pszSpec: w!("*.png;*.jpg;*.jpeg;*.bmp;*.ico") }];
        let _ = dlg.SetFileTypes(&spec);
        let _ = dlg.SetTitle(w!("Choose an icon"));
        dlg.Show(Some(owner)).ok()?;
        let item = dlg.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let out = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        out
    }
}

/// Every app shortcut in the Start menu, sorted by name.
fn scan_start_menu() -> Vec<Item> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<(String, String)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 3 {
                    walk(&p, depth + 1, out);
                }
            } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("lnk")) {
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let l = name.to_ascii_lowercase();
                if ["uninstall", "readme", "documentation", "release notes", "license", " help"]
                    .iter()
                    .any(|w| l.contains(w))
                {
                    continue;
                }
                out.push((name, p.to_string_lossy().to_string()));
            }
        }
    }
    let mut found: Vec<(String, String)> = Vec::new();
    for (var, tail) in [("ProgramData", r"Microsoft\Windows\Start Menu\Programs"), ("APPDATA", r"Microsoft\Windows\Start Menu\Programs")] {
        if let Ok(base) = std::env::var(var) {
            walk(&Path::new(&base).join(tail), 0, &mut found);
        }
    }
    for (name, id) in uwp_apps() {
        found.push((name.clone(), format!(r"shell:AppsFolder\{id}")));
    }
    found.sort_by_key(|f| f.0.to_ascii_lowercase());
    found.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    found
        .into_iter()
        .map(|(name, path)| {
            with_key(Item { label: name, path, exe: String::new(), png: None, pinned: false, hwnd: 0, key: String::new() })
        })
        .collect()
}

/// Name and launch path of every installed app (Start menu shortcuts and Store apps), for the launcher.
pub fn installed_apps() -> Vec<(String, String)> {
    scan_start_menu().into_iter().map(|i| (i.label, i.path)).collect()
}

/// The right-click menu: keep / remove the app under the pointer, or add any app. Runs outside the
/// app borrow because the menu and the picker wait for the user.
fn context_menu(hwnd: HWND, lp: LPARAM) {
    let target = crate::window::with_app(|a| a.dock_menu_target(lp)).flatten();
    let cmd = unsafe {
        let menu = CreatePopupMenu().unwrap_or_default();
        if let Some((label, _, pinned)) = &target {
            let t = wstr(&if *pinned { format!("Remove {label} from the dock") } else { format!("Keep {label} in the dock") });
            let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR(t.as_ptr()));
            let ci = wstr("Change icon…");
            let di = wstr("Use the default icon");
            let _ = AppendMenuW(menu, MF_STRING, 3, PCWSTR(ci.as_ptr()));
            let _ = AppendMenuW(menu, MF_STRING, 4, PCWSTR(di.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        let add = wstr("Add or remove apps…");
        let _ = AppendMenuW(menu, MF_STRING, 2, PCWSTR(add.as_ptr()));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let c = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0, pt.x, pt.y, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        c.0
    };
    let mut items = crate::window::with_app(|a| a.dock_items()).unwrap_or_default();
    match (cmd, target) {
        (1, Some((_, path, pinned))) => {
            if pinned {
                items.retain(|s| !same_path(s, &path));
            } else if !items.iter().any(|s| same_path(s, &path)) {
                items.push(format!("{}|{}", nice_name(&path), path));
            }
            save_items(&items);
        }
        (3, Some((label, _, _))) => {
            if let Some(src) = pick_image(hwnd) {
                let _ = std::fs::create_dir_all(icons_dir());
                if std::fs::copy(&src, icons_dir().join(format!("{label}.png"))).is_ok() {
                    crate::window::with_app(|a| a.dock_reload_icons());
                }
            }
        }
        (4, Some((label, _, _))) => {
            let _ = std::fs::remove_file(icons_dir().join(format!("{label}.png")));
            let _ = std::fs::remove_file(icons_dir().join(format!("{}.png", label.to_ascii_lowercase())));
            crate::window::with_app(|a| a.dock_reload_icons());
        }
        (2, _) => {
            crate::window::with_app(|a| a.dock_open_picker());
        }
        _ => {}
    }
}

extern "system" fn dock_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_RBUTTONUP {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| context_menu(hwnd, lp)));
        return LRESULT(0);
    }
    let r = std::panic::catch_unwind(|| crate::window::with_app(|a| a.dock_message(msg, wp, lp)).flatten());
    match r {
        Ok(Some(v)) => v,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
