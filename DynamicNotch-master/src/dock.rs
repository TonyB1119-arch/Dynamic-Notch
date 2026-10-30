//! The dock: a taskbar replacement that lives at the bottom of the screen.
//!
//! It shows your pinned apps and every app that is running, magnifies the icons under the
//! mouse, shows window previews on hover, has a right-click menu and lets you drag icons to
//! reorder them. Optionally it hides the Windows taskbar (and gives it back on exit, and after
//! a crash on the next start).
//!
//! The dock is its own window that shares the app's graphics device and compositor. The app
//! forwards the window's messages here (`App::dock_message`) and draws it once per frame
//! (`App::frame`), so nothing runs while nothing moves.

use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateRectRgn, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromPoint, ReleaseDC, SelectObject, SetWindowRgn, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ,
    MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{SHAppBarMessage, SHDefExtractIconW, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::DockCfg;
use crate::gfx::image::ImageData;
use crate::gfx::{Gfx, Painter, Res, TextStyle};
use crate::host::Host;
use crate::util::{app_dir, wide, Color, Rect};

/// Gap between icons, padding inside the bar, space below the bar (all in logical pixels).
const GAP: f32 = 10.0;
const PAD: f32 = 12.0;
const MARGIN_B: f32 = 10.0;
/// Room above the bar for magnified icons, previews and menus.
const POP_H: f32 = 300.0;
pub const TIMER_FRAME: usize = 0xD0C2;
const TIMER_POLL: usize = 0xD0C1;
const POLL_MS: u32 = 700;
const WM_APPBAR: u32 = WM_APP + 40;
const WM_MOUSELEAVE: u32 = 0x02A3;
const THUMB_W: f32 = 180.0;
const THUMB_H: f32 = 112.0;
const MENU_W: f32 = 210.0;
const MENU_ROW: f32 = 32.0;

static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);

/// The dock window's handle (0 when there is none), so the window procedure can route to it.
pub fn hwnd_value() -> isize {
    HWND_VALUE.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

struct WinRef {
    h: isize,
    title: String,
}

struct Item {
    key: String,
    /// Executable to launch ("" for apps that cannot be launched from a path).
    path: String,
    name: String,
    pinned: bool,
    wins: Vec<WinRef>,
    icon: Option<ImageData>,
    icon_tried: bool,
    /// Animated centre x and icon scale.
    x: f32,
    s: f32,
    appear: f32,
    fresh: bool,
    tint: Color,
}

impl Item {
    fn new(key: String, path: String, name: String, pinned: bool) -> Self {
        let tint = tile_color(&name);
        Self {
            key,
            path,
            name,
            pinned,
            wins: Vec::new(),
            icon: None,
            icon_tried: false,
            x: 0.0,
            s: 1.0,
            appear: 0.0,
            fresh: true,
            tint,
        }
    }
}

struct Thumb {
    img: Option<ImageData>,
    at: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Open,
    Pin,
    Unpin,
    Close,
    CloseAll,
}

struct Menu {
    key: String,
    rect: Rect,
    rows: Vec<(String, Act)>,
}

struct Press {
    idx: usize,
    at: (f32, f32),
}

struct Drag {
    idx: usize,
    off: f32,
}

pub struct Dock {
    pub hwnd: HWND,
    host: Host,
    res: Option<Res>,
    cfg: DockCfg,
    /// Pinned programs as full paths (the saved list, with bare names resolved).
    pins: Vec<String>,
    timer_on: bool,
    ui_scale: f32,
    scale: f32,
    /// Window rectangle in device pixels (x, y, w, h).
    win: (i32, i32, i32, i32),
    items: Vec<Item>,
    sig: u64,
    fg: isize,
    mouse: Option<(f32, f32)>,
    tracking: bool,
    press: Option<Press>,
    drag: Option<Drag>,
    menu: Option<Menu>,
    hover: Option<(String, Instant)>,
    pop_key: Option<String>,
    pop_alpha: f32,
    pop_left: Option<Instant>,
    mag: f32,
    /// 1 = the bar is shown, 0 = slid away (auto-hide).
    reveal: f32,
    away: Option<Instant>,
    thumbs: HashMap<isize, Thumb>,
    region: Option<(i32, i32, i32, i32, i32)>,
    last: Instant,
    need_place: bool,
    appbar: bool,
    hidden: bool,
    taskbar_old: Option<usize>,
    busy: bool,
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

impl Dock {
    pub fn new(
        hwnd: HWND,
        gfx: &Gfx,
        compositor: &windows::UI::Composition::Compositor,
        cfg: &DockCfg,
        ui_scale: f32,
    ) -> Option<Self> {
        let host = match Host::new_shared(hwnd, gfx, 16, 16, compositor.clone()) {
            Ok(h) => h,
            Err(e) => {
                crate::log!("dock surface failed: {e}");
                return None;
            }
        };
        let res = Res::new(gfx).ok()?;
        HWND_VALUE.store(hwnd.0 as isize, Ordering::Relaxed);
        let mut d = Dock {
            hwnd,
            host,
            res: Some(res),
            cfg: cfg.clone(),
            pins: Vec::new(),
            timer_on: false,
            ui_scale,
            scale: 1.0,
            win: (0, 0, 0, 0),
            items: Vec::new(),
            sig: 0,
            fg: 0,
            mouse: None,
            tracking: false,
            press: None,
            drag: None,
            menu: None,
            hover: None,
            pop_key: None,
            pop_alpha: 0.0,
            pop_left: None,
            mag: 0.0,
            reveal: 1.0,
            away: None,
            thumbs: HashMap::new(),
            region: None,
            last: Instant::now(),
            need_place: true,
            appbar: false,
            hidden: false,
            taskbar_old: None,
            busy: true,
        };
        d.rebuild_pins();
        d.refresh();
        d.place();
        unsafe {
            let _ = SetTimer(Some(hwnd), TIMER_POLL, POLL_MS, None);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        d.apply_taskbar();
        d.start_timer();
        Some(d)
    }

    /// New settings (from the config file).
    pub fn set_config(&mut self, cfg: &DockCfg, ui_scale: f32) {
        let relayout = (cfg.size - self.cfg.size).abs() > 0.5
            || cfg.auto_hide != self.cfg.auto_hide
            || (ui_scale - self.ui_scale).abs() > 0.001;
        let pins_changed = self.cfg.pinned != cfg.pinned;
        self.cfg = cfg.clone();
        if pins_changed {
            self.rebuild_pins();
        }
        self.ui_scale = ui_scale;
        if relayout {
            self.need_place = true;
        }
        self.apply_taskbar();
        self.refresh();
        self.busy = true;
    }

    /// Hide the dock while a full-screen app is in front.
    pub fn set_hidden(&mut self, hidden: bool) {
        if hidden != self.hidden {
            self.hidden = hidden;
            unsafe {
                let _ = ShowWindow(self.hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            self.busy = true;
            if hidden {
                self.stop_timer();
            } else {
                self.start_timer();
            }
        }
    }
    #[allow(dead_code)]

    pub fn needs_frames(&self) -> bool {
        self.busy && !self.hidden
    }

    fn rebuild_pins(&mut self) {
        self.pins = self.cfg.pinned.iter().map(|p| resolve_exe(p)).collect();
    }

    /// Start the frame timer (it runs only while something moves).
    pub fn start_timer(&mut self) {
        if !self.timer_on && !self.hidden {
            self.timer_on = true;
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_FRAME, 8, None);
            }
        }
    }

    pub fn stop_timer(&mut self) {
        if self.timer_on {
            self.timer_on = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_FRAME);
            }
        }
    }

    /// Explorer started again: hide its new taskbar if we are meant to.
    pub fn explorer_restarted(&mut self) {
        if self.cfg.hide_taskbar && self.taskbar_old.is_some() {
            set_taskbar(self.hwnd, true, 0);
        }
        self.need_place = true;
        self.busy = true;
        self.start_timer();
    }

    /// The graphics device was lost and recreated.
    pub fn recover(&mut self, gfx: &Gfx) {
        let _ = self.host.set_rendering_device(gfx);
        if let Ok(r) = Res::new(gfx) {
            self.res = Some(r);
        }
        self.busy = true;
        self.start_timer();
    }

    // -- geometry ----------------------------------------------------------

    fn size(&self) -> f32 {
        self.cfg.size.clamp(32.0, 80.0)
    }
    fn slot(&self) -> f32 {
        self.size() + GAP
    }
    fn bar_h(&self) -> f32 {
        self.size() + 2.0 * PAD
    }
    fn bar_w(&self) -> f32 {
        let n = self.items.len().max(1) as f32;
        n * self.slot() - GAP + 2.0 * PAD * 1.2
    }
    fn win_h(&self) -> f32 {
        MARGIN_B + self.bar_h() + POP_H
    }
    fn win_w(&self) -> f32 {
        (self.win.2 as f32 / self.scale).max(1.0)
    }
    fn bar_rect(&self) -> Rect {
        let (w, h) = (self.bar_w(), self.bar_h());
        let slide = (1.0 - self.reveal) * (h + MARGIN_B + 4.0);
        Rect::new((self.win_w() - w) * 0.5, self.win_h() - MARGIN_B - h + slide, w, h)
    }
    /// How far the icons rise toward the pointer (1.0 = double size).
    fn magnify_amount(&self) -> f32 {
        self.cfg.magnify_amount.clamp(0.1, 1.0)
    }
    /// Space between the bar and the panels above it.
    fn pop_gap(&self) -> f32 {
        self.size() * (0.1 + 0.3 * (1.0 + self.magnify_amount()))
    }
    fn target_x(&self, i: usize) -> f32 {
        self.bar_rect().x + PAD * 1.2 + self.size() * 0.5 + i as f32 * self.slot()
    }

    fn place(&mut self) {
        self.need_place = false;
        unsafe {
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let scale = dx as f32 / 96.0 * self.ui_scale;
            let m = mi.rcMonitor;
            // reserve the strip under the dock so maximised windows stop above it
            let reserve = if self.cfg.auto_hide { 0 } else { ((self.bar_h() + 2.0 * MARGIN_B) * scale).ceil() as i32 };
            let mut abd = APPBARDATA {
                cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                hWnd: self.hwnd,
                uCallbackMessage: WM_APPBAR,
                uEdge: 3, // ABE_BOTTOM
                rc: RECT { left: m.left, top: m.bottom - reserve, right: m.right, bottom: m.bottom },
                lParam: LPARAM(0),
            };
            if self.cfg.auto_hide {
                // an auto-hiding dock floats over the windows and reserves nothing
                if self.appbar {
                    SHAppBarMessage(1, &mut abd); // ABM_REMOVE
                    self.appbar = false;
                }
                abd.rc.bottom = m.bottom;
            } else {
                if !self.appbar {
                    SHAppBarMessage(0, &mut abd); // ABM_NEW
                    self.appbar = true;
                }
                abd.rc = RECT { left: m.left, top: m.bottom - reserve, right: m.right, bottom: m.bottom };
                SHAppBarMessage(2, &mut abd); // ABM_QUERYPOS
                abd.rc.top = abd.rc.bottom - reserve;
                SHAppBarMessage(3, &mut abd); // ABM_SETPOS
            }
            let w_px = m.right - m.left;
            self.scale = scale;
            let h_px = (self.win_h() * scale).ceil() as i32;
            let x = m.left;
            let y = abd.rc.bottom - h_px;
            self.win = (x, y, w_px, h_px);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w_px, h_px, SWP_NOACTIVATE);
            if let Err(e) = self.host.resize(w_px, h_px) {
                crate::log!("dock resize failed: {e}");
            }
            self.region = None;
        }
        self.busy = true;
    }

    // -- taskbar -----------------------------------------------------------

    fn apply_taskbar(&mut self) {
        if self.cfg.hide_taskbar {
            if self.taskbar_old.is_none() {
                let old = unsafe {
                    let mut abd = APPBARDATA {
                        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                        hWnd: self.hwnd,
                        ..Default::default()
                    };
                    SHAppBarMessage(4, &mut abd) // ABM_GETSTATE
                };
                self.taskbar_old = Some(old);
                let _ = std::fs::write(marker_path(), old.to_string());
            }
            set_taskbar(self.hwnd, true, 0);
        } else if let Some(old) = self.taskbar_old.take() {
            set_taskbar(self.hwnd, false, old);
            let _ = std::fs::remove_file(marker_path());
        }
    }

    fn release(&mut self) {
        if let Some(old) = self.taskbar_old.take() {
            set_taskbar(self.hwnd, false, old);
            let _ = std::fs::remove_file(marker_path());
        }
        if self.appbar {
            unsafe {
                let mut abd = APPBARDATA {
                    cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                    hWnd: self.hwnd,
                    ..Default::default()
                };
                SHAppBarMessage(1, &mut abd); // ABM_REMOVE
            }
            self.appbar = false;
        }
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        self.release();
        HWND_VALUE.store(0, Ordering::Relaxed);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
            let _ = KillTimer(Some(self.hwnd), TIMER_FRAME);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn marker_path() -> std::path::PathBuf {
    app_dir().join("taskbar-hidden")
}

/// After a crash the Windows taskbar may still be hidden: give it back at the next start.
pub fn recover_taskbar() {
    let p = marker_path();
    if let Ok(text) = std::fs::read_to_string(&p) {
        let old = text.trim().parse::<usize>().unwrap_or(0);
        set_taskbar(HWND::default(), false, old);
        let _ = std::fs::remove_file(&p);
    }
}

fn taskbar_windows() -> Vec<HWND> {
    let mut bars: Vec<HWND> = Vec::new();
    unsafe {
        if let Ok(h) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            bars.push(h);
        }
        let mut prev: Option<HWND> = None;
        for _ in 0..8 {
            match FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), PCWSTR::null()) {
                Ok(h) if !h.is_invalid() => {
                    bars.push(h);
                    prev = Some(h);
                }
                _ => break,
            }
        }
    }
    bars
}

/// Explorer can bring the taskbar back (after a restart, say): hide it again.
fn hide_bars_if_visible() {
    for b in taskbar_windows() {
        unsafe {
            if IsWindowVisible(b).as_bool() {
                let _ = ShowWindow(b, SW_HIDE);
            }
        }
    }
}

/// Hide (`hide`) or show every Windows taskbar. Hiding also switches the taskbar to
/// auto-hide so the screen area it used is free; showing puts the old state back.
fn set_taskbar(owner: HWND, hide: bool, old_state: usize) {
    unsafe {
        let bars = taskbar_windows();
        if hide {
            let mut abd = APPBARDATA {
                cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                hWnd: owner,
                lParam: LPARAM(1), // ABS_AUTOHIDE
                ..Default::default()
            };
            SHAppBarMessage(10, &mut abd); // ABM_SETSTATE
            for b in &bars {
                let _ = ShowWindow(*b, SW_HIDE);
            }
        } else {
            let mut abd = APPBARDATA {
                cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                hWnd: owner,
                lParam: LPARAM(old_state as isize),
                ..Default::default()
            };
            SHAppBarMessage(10, &mut abd);
            for b in &bars {
                let _ = ShowWindow(*b, SW_SHOWNA);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Running windows
// ---------------------------------------------------------------------------

struct RawWin {
    h: isize,
    title: String,
    exe: String,
    uwp: bool,
}

fn enum_windows() -> Vec<RawWin> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<RawWin>);
        if let Some(w) = consider(h) {
            out.push(w);
        }
        true.into()
    }
    let mut out: Vec<RawWin> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

unsafe fn consider(h: HWND) -> Option<RawWin> {
    if !IsWindowVisible(h).as_bool() {
        return None;
    }
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
    if cloaked != 0 {
        return None;
    }
    let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
    let tool = ex & WS_EX_TOOLWINDOW.0 != 0;
    let app = ex & WS_EX_APPWINDOW.0 != 0;
    let unowned = GetWindow(h, GW_OWNER).map_or(true, |o| o.is_invalid());
    if (tool && !app) || (!unowned && !app) {
        return None;
    }
    if GetWindowTextLengthW(h) == 0 {
        return None;
    }
    let mut cls = [0u16; 64];
    let n = GetClassNameW(h, &mut cls);
    let class = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
    if matches!(
        class.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "XamlExplorerHostIslandWindow"
    ) {
        return None;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    if pid == std::process::id() {
        return None;
    }
    let mut tb = [0u16; 256];
    let tn = GetWindowTextW(h, &mut tb);
    let title = String::from_utf16_lossy(&tb[..tn.max(0) as usize]);
    let exe = process_path(pid)?;
    let uwp = exe.to_ascii_lowercase().ends_with("applicationframehost.exe");
    Some(RawWin { h: h.0 as isize, title, exe, uwp })
}

fn process_path(pid: u32) -> Option<String> {
    if pid <= 4 {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// A bare file name ("notepad.exe") becomes a full path when it can be found.
fn resolve_exe(name: &str) -> String {
    if name.contains('\\') || name.contains('/') {
        return name.to_string();
    }
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(win) = std::env::var_os("WINDIR") {
        let win = std::path::PathBuf::from(win);
        dirs.push(win.clone());
        dirs.push(win.join("System32"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    for d in dirs {
        let c = d.join(name);
        if c.is_file() {
            return c.to_string_lossy().into_owned();
        }
    }
    name.to_string()
}

fn pretty_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl Dock {
    /// Re-read the running windows and the pinned list. Returns true when something changed.
    fn refresh(&mut self) -> bool {
        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        let raw = enum_windows();
        let mut groups: Vec<(String, String, bool, Vec<WinRef>)> = Vec::new();
        for w in raw {
            let (key, path) = if w.uwp {
                (format!("uwp:{}", w.h), String::new())
            } else {
                (w.exe.to_ascii_lowercase(), w.exe.clone())
            };
            let wr = WinRef { h: w.h, title: w.title };
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.3.push(wr),
                None => groups.push((key, path, w.uwp, vec![wr])),
            }
        }
        let mut old = std::mem::take(&mut self.items);
        let take = |key: &str, old: &mut Vec<Item>| old.iter().position(|i| i.key == key).map(|p| old.remove(p));
        let mut next: Vec<Item> = Vec::new();
        for path in &self.pins {
            let key = path.to_ascii_lowercase();
            if next.iter().any(|i| i.key == key) {
                continue;
            }
            let mut it =
                take(&key, &mut old).unwrap_or_else(|| Item::new(key.clone(), path.clone(), pretty_name(path), true));
            it.pinned = true;
            it.wins = groups.iter().find(|g| g.0 == key).map(|g| clone_wins(&g.3)).unwrap_or_default();
            next.push(it);
        }
        // running apps that are not pinned: keep their earlier order, then new ones
        let mut rest: Vec<Item> = Vec::new();
        for mut it in old.into_iter() {
            if let Some(g) = groups.iter().find(|g| g.0 == it.key) {
                it.pinned = false;
                it.wins = clone_wins(&g.3);
                rest.push(it);
            }
        }
        for g in &groups {
            if next.iter().any(|i| i.key == g.0) || rest.iter().any(|i| i.key == g.0) {
                continue;
            }
            let name = if g.2 { g.3.first().map(|w| w.title.clone()).unwrap_or_default() } else { pretty_name(&g.1) };
            let mut it = Item::new(g.0.clone(), g.1.clone(), name, false);
            it.wins = clone_wins(&g.3);
            rest.push(it);
        }
        next.extend(rest);
        self.items = next;
        // icons, loaded once per item
        for i in 0..self.items.len() {
            if self.items[i].icon_tried {
                continue;
            }
            let img = if self.items[i].path.is_empty() {
                self.items[i].wins.first().and_then(|w| window_icon(HWND(w.h as *mut _)))
            } else {
                file_icon(&self.items[i].path)
                    .or_else(|| self.items[i].wins.first().and_then(|w| window_icon(HWND(w.h as *mut _))))
            };
            self.items[i].icon = img;
            self.items[i].icon_tried = true;
        }
        // forget previews of windows that are gone
        let live: Vec<isize> = self.items.iter().flat_map(|i| i.wins.iter().map(|w| w.h)).collect();
        self.thumbs.retain(|h, _| live.contains(h));
        // a hide-the-taskbar setting is re-applied if Explorer brought it back
        if self.cfg.hide_taskbar && self.taskbar_old.is_some() {
            hide_bars_if_visible();
        }
        let mut sig: u64 = fg as u64;
        for it in &self.items {
            for b in it.key.bytes() {
                sig = sig.wrapping_mul(1099511628211).wrapping_add(b as u64);
            }
            sig = sig.wrapping_mul(31).wrapping_add(it.wins.len() as u64);
            for w in &it.wins {
                sig = sig.wrapping_mul(31).wrapping_add(w.h as u64);
            }
            sig = sig.wrapping_mul(31).wrapping_add(it.pinned as u64);
        }
        self.fg = fg;
        let changed = sig != self.sig;
        self.sig = sig;
        if changed {
            if self.items.len() != 0 {
                self.need_place = true;
            }
            self.busy = true;
        }
        changed
    }

    fn save_pins(&mut self) {
        self.pins = self.items.iter().filter(|i| i.pinned && !i.path.is_empty()).map(|i| i.path.clone()).collect();
        self.cfg.pinned = self.pins.clone();
        let list: Vec<String> =
            self.pins.iter().map(|p| format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\""))).collect();
        crate::config::set_value("dock", "pinned", &format!("[{}]", list.join(", ")));
    }
}

fn clone_wins(v: &[WinRef]) -> Vec<WinRef> {
    v.iter().map(|w| WinRef { h: w.h, title: w.title.clone() }).collect()
}

// ---------------------------------------------------------------------------
// Icons and previews
// ---------------------------------------------------------------------------

fn tile_color(name: &str) -> Color {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    let hue = (h % 360) as f32 / 60.0;
    let c = 0.55f32;
    let x = c * (1.0 - ((hue % 2.0) - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::rgba(r + 0.25, g + 0.25, b + 0.25, 1.0)
}

/// The icon of an executable at about 96 pixels, when Windows can extract one.
fn file_icon(path: &str) -> Option<ImageData> {
    unsafe {
        let wp = wide(path);
        let mut large = HICON::default();
        let mut small = HICON::default();
        let size = 96u32 | (16u32 << 16);
        if SHDefExtractIconW(
            PCWSTR(wp.as_ptr()),
            0,
            0,
            Some(&mut large as *mut HICON),
            Some(&mut small as *mut HICON),
            size,
        )
        .is_err()
        {
            return None;
        }
        let img = if large.is_invalid() { None } else { hicon_to_image(large, 96) };
        if !large.is_invalid() {
            let _ = DestroyIcon(large);
        }
        if !small.is_invalid() {
            let _ = DestroyIcon(small);
        }
        img
    }
}

/// The icon a window offers (shared, so it is not destroyed).
fn window_icon(h: HWND) -> Option<ImageData> {
    unsafe {
        let mut res = 0usize;
        // WM_GETICON with ICON_BIG (1)
        SendMessageTimeoutW(h, WM_GETICON, WPARAM(1), LPARAM(0), SMTO_ABORTIFHUNG, 40, Some(&mut res as *mut usize));
        let mut hi = HICON(res as *mut _);
        if hi.is_invalid() {
            hi = HICON(GetClassLongPtrW(h, GCLP_HICON) as *mut _);
        }
        if hi.is_invalid() {
            None
        } else {
            hicon_to_image(hi, 64)
        }
    }
}

/// Draw an icon into a 32-bit surface and keep the pixels (premultiplied BGRA).
unsafe fn hicon_to_image(icon: HICON, px: i32) -> Option<ImageData> {
    let screen = GetDC(None);
    let hdc = CreateCompatibleDC(Some(screen));
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: px,
            biHeight: -px,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
    let out = match bmp {
        Ok(bmp) if !bits.is_null() => {
            let old = SelectObject(hdc, HGDIOBJ(bmp.0));
            let len = (px * px * 4) as usize;
            std::ptr::write_bytes(bits as *mut u8, 0, len);
            let _ = DrawIconEx(hdc, 0, 0, icon, px, px, 0, None, DI_NORMAL);
            let mut data = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            // an icon without an alpha channel comes back with alpha 0: make it opaque
            if data.chunks(4).all(|p| p[3] == 0) {
                for p in data.chunks_mut(4) {
                    if p[0] != 0 || p[1] != 0 || p[2] != 0 {
                        p[3] = 255;
                    }
                }
            }
            Some(ImageData::from_pbgra(px as u32, px as u32, data))
        }
        _ => None,
    };
    let _ = DeleteDC(hdc);
    ReleaseDC(None, screen);
    out
}

/// A small picture of a window (about 360 pixels wide at most).
fn capture(h: HWND) -> Option<ImageData> {
    unsafe {
        if IsIconic(h).as_bool() || IsHungAppWindow(h).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let (w, hgt) = ((r.right - r.left).clamp(1, 3840), (r.bottom - r.top).clamp(1, 2160));
        let screen = GetDC(None);
        let hdc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -hgt,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        let out = match bmp {
            Ok(bmp) if !bits.is_null() => {
                let old = SelectObject(hdc, HGDIOBJ(bmp.0));
                let len = (w * hgt * 4) as usize;
                std::ptr::write_bytes(bits as *mut u8, 0, len);
                let ok = PrintWindow(h, hdc, PRINT_WINDOW_FLAGS(2)).as_bool(); // PW_RENDERFULLCONTENT
                let src = std::slice::from_raw_parts(bits as *const u8, len);
                let img = if ok { Some(downscale(src, w as usize, hgt as usize, 360)) } else { None };
                SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                img
            }
            _ => None,
        };
        let _ = DeleteDC(hdc);
        ReleaseDC(None, screen);
        out
    }
}

/// Average blocks of pixels so the longest side is at most `max` (and make it opaque).
fn downscale(src: &[u8], w: usize, h: usize, max: usize) -> ImageData {
    let k = (w.max(h) + max - 1) / max;
    let k = k.max(1);
    let (nw, nh) = ((w / k).max(1), (h / k).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 3];
            let mut n = 0u32;
            for dy in 0..k {
                for dx in 0..k {
                    let (sx, sy) = (x * k + dx, y * k + dy);
                    if sx < w && sy < h {
                        let i = (sy * w + sx) * 4;
                        acc[0] += src[i] as u32;
                        acc[1] += src[i + 1] as u32;
                        acc[2] += src[i + 2] as u32;
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            let o = (y * nw + x) * 4;
            out[o] = (acc[0] / n) as u8;
            out[o + 1] = (acc[1] / n) as u8;
            out[o + 2] = (acc[2] / n) as u8;
            out[o + 3] = 255;
        }
    }
    ImageData::from_pbgra(nw as u32, nh as u32, out)
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

impl Dock {
    fn xy(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        (x / self.scale, y / self.scale)
    }

    /// The item under the pointer (icons grow upwards, so a little room above the bar counts).
    fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        let bar = self.bar_rect();
        if y < bar.y - self.size() * 0.5 || y > bar.bottom() {
            return None;
        }
        let half = self.slot() * 0.5;
        self.items.iter().position(|it| (x - it.x).abs() <= half)
    }

    /// Messages for the dock window. Returns the result (None = default handling) and whether
    /// a new frame is needed.
    pub fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> (Option<LRESULT>, bool) {
        match msg {
            WM_MOUSEACTIVATE => (Some(LRESULT(MA_NOACTIVATE as isize)), false),
            WM_TIMER if wp.0 == TIMER_POLL => {
                let changed = self.refresh();
                if self.menu.is_some() {
                    self.close_menu_if_clicked_away();
                }
                (Some(LRESULT(0)), changed || self.need_place)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                self.need_place = true;
                (Some(LRESULT(0)), true)
            }
            WM_MOUSEMOVE => {
                let (x, y) = self.xy(lp);
                if !self.tracking {
                    self.tracking = true;
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                }
                self.mouse = Some((x, y));
                if let Some(p) = &self.press {
                    if self.drag.is_none() && ((x - p.at.0).abs() > 6.0 || (y - p.at.1).abs() > 6.0) {
                        let idx = p.idx;
                        if let Some(it) = self.items.get(idx) {
                            self.drag = Some(Drag { idx, off: x - it.x });
                            self.pop_key = None;
                            self.menu = None;
                        }
                    }
                }
                if self.drag.is_some() {
                    self.drag_to(x);
                }
                self.busy = true;
                (Some(LRESULT(0)), true)
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                if self.drag.is_none() {
                    self.mouse = None;
                }
                self.busy = true;
                (Some(LRESULT(0)), true)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.xy(lp);
                self.mouse = Some((x, y));
                if self.menu.is_none() && self.hit_popup(x, y).is_none() {
                    if let Some(idx) = self.item_at(x, y) {
                        self.press = Some(Press { idx, at: (x, y) });
                        unsafe {
                            let _ = SetCapture(self.hwnd);
                        }
                    }
                }
                (Some(LRESULT(0)), true)
            }
            WM_LBUTTONUP => {
                let (x, y) = self.xy(lp);
                self.mouse = Some((x, y));
                unsafe {
                    let _ = ReleaseCapture();
                }
                let press = self.press.take();
                if self.drag.take().is_some() {
                    self.save_pins();
                    self.need_place = true;
                } else if self.menu.is_some() {
                    self.click_menu(x, y);
                } else if let Some(act) = self.hit_popup(x, y) {
                    self.click_popup(act);
                } else if let Some(p) = press {
                    if self.item_at(x, y) == Some(p.idx) {
                        self.click_item(p.idx);
                    }
                }
                self.busy = true;
                (Some(LRESULT(0)), true)
            }
            WM_RBUTTONUP => {
                let (x, y) = self.xy(lp);
                self.mouse = Some((x, y));
                self.menu = None;
                if let Some(idx) = self.item_at(x, y) {
                    self.open_menu(idx);
                }
                self.busy = true;
                (Some(LRESULT(0)), true)
            }
            WM_MOUSEWHEEL => {
                // scrolling over an app with several windows steps through them
                if let Some((x, y)) = self.mouse {
                    if let Some(idx) = self.item_at(x, y) {
                        let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16;
                        self.cycle(idx, if delta > 0 { -1 } else { 1 });
                    }
                }
                (Some(LRESULT(0)), true)
            }
            _ => (None, false),
        }
    }

    fn drag_to(&mut self, x: f32) {
        let Some(d) = &self.drag else { return };
        let (idx, off) = (d.idx, d.off);
        let x0 = self.target_x(0);
        let j = (((x - off - x0) / self.slot()).round().max(0.0) as usize).min(self.items.len().saturating_sub(1));
        if j != idx && idx < self.items.len() {
            let it = self.items.remove(idx);
            self.items.insert(j, it);
            if let Some(d) = self.drag.as_mut() {
                d.idx = j;
            }
        }
    }

    fn click_item(&mut self, idx: usize) {
        let Some(it) = self.items.get(idx) else { return };
        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        match it.wins.len() {
            0 => {
                if !it.path.is_empty() {
                    crate::sys::shell_open(&it.path);
                }
            }
            1 => {
                let h = it.wins[0].h;
                let hwnd = HWND(h as *mut _);
                if fg == h && unsafe { !IsIconic(hwnd).as_bool() } {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    }
                } else {
                    crate::sys::activate_window(h);
                }
            }
            _ => self.cycle(idx, 1),
        }
    }

    /// Bring the next (or previous) window of an app forward.
    fn cycle(&self, idx: usize, dir: i32) {
        let Some(it) = self.items.get(idx) else { return };
        let n = it.wins.len();
        if n == 0 {
            return;
        }
        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        let cur = it.wins.iter().position(|w| w.h == fg);
        let next = match cur {
            Some(c) => ((c as i32 + dir).rem_euclid(n as i32)) as usize,
            None => 0,
        };
        crate::sys::activate_window(it.wins[next].h);
    }

    fn open_menu(&mut self, idx: usize) {
        let Some(it) = self.items.get(idx) else { return };
        let mut rows: Vec<(String, Act)> = Vec::new();
        if it.wins.is_empty() {
            rows.push(("Open".into(), Act::Open));
        } else if !it.path.is_empty() {
            rows.push(("Open new window".into(), Act::Open));
        }
        if !it.path.is_empty() {
            if it.pinned {
                rows.push(("Unpin from dock".into(), Act::Unpin));
            } else {
                rows.push(("Pin to dock".into(), Act::Pin));
            }
        }
        match it.wins.len() {
            0 => {}
            1 => rows.push(("Close window".into(), Act::Close)),
            n => rows.push((format!("Close all {n} windows"), Act::CloseAll)),
        }
        if rows.is_empty() {
            return;
        }
        let h = rows.len() as f32 * MENU_ROW + 12.0;
        let bar = self.bar_rect();
        let x = (it.x - MENU_W * 0.5).clamp(8.0, (self.win_w() - MENU_W - 8.0).max(8.0));
        let y = bar.y - self.pop_gap() - h - 8.0;
        self.menu = Some(Menu { key: it.key.clone(), rect: Rect::new(x, y, MENU_W, h), rows });
        self.pop_key = None;
    }

    fn click_menu(&mut self, x: f32, y: f32) {
        let Some(m) = self.menu.take() else { return };
        if !m.rect.contains(x, y) {
            return;
        }
        let row = ((y - m.rect.y - 6.0) / MENU_ROW).floor();
        if row < 0.0 {
            return;
        }
        let Some((_, act)) = m.rows.get(row as usize) else { return };
        let Some(pos) = self.items.iter().position(|i| i.key == m.key) else { return };
        match act {
            Act::Open => {
                let p = self.items[pos].path.clone();
                if !p.is_empty() {
                    crate::sys::shell_open(&p);
                }
            }
            Act::Pin => {
                self.items[pos].pinned = true;
                self.save_pins();
            }
            Act::Unpin => {
                self.items[pos].pinned = false;
                self.save_pins();
            }
            Act::Close | Act::CloseAll => {
                for w in &self.items[pos].wins {
                    unsafe {
                        let _ = PostMessageW(Some(HWND(w.h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
        }
    }

    /// A click anywhere else dismisses the menu (clicks outside our window never reach us).
    fn close_menu_if_clicked_away(&mut self) {
        let down =
            unsafe { (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0 || (GetAsyncKeyState(0x02) as u16 & 0x8000) != 0 };
        if !down {
            return;
        }
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let (lx, ly) = ((p.x - self.win.0) as f32 / self.scale, (p.y - self.win.1) as f32 / self.scale);
        let inside = self.menu.as_ref().is_some_and(|m| m.rect.contains(lx, ly));
        if !inside {
            self.menu = None;
            self.busy = true;
        }
    }

    // -- preview popup -------------------------------------------------------

    /// Rectangle of the preview panel and its thumbnail cells for an item.
    fn popup_geo(&self, key: &str) -> Option<(Rect, Vec<Rect>)> {
        let it = self.items.iter().find(|i| i.key == key)?;
        let n = it.wins.len();
        if n == 0 {
            return None;
        }
        let pw = n as f32 * THUMB_W + (n as f32 - 1.0) * 10.0 + 24.0;
        let ph = THUMB_H + 26.0 + 24.0;
        let pw = pw.min(self.win_w() - 16.0);
        let bar = self.bar_rect();
        let x = (it.x - pw * 0.5).clamp(8.0, (self.win_w() - pw - 8.0).max(8.0));
        let y = bar.y - self.pop_gap() - ph - 8.0;
        let panel = Rect::new(x, y, pw, ph);
        let cells = (0..n)
            .map(|i| Rect::new(x + 12.0 + i as f32 * (THUMB_W + 10.0), y + 12.0, THUMB_W, THUMB_H + 26.0))
            .collect();
        Some((panel, cells))
    }

    /// What a click at (x, y) would hit in the preview panel: (window handle, close button?).
    fn hit_popup(&self, x: f32, y: f32) -> Option<(isize, bool)> {
        let key = self.pop_key.as_ref()?;
        if self.pop_alpha < 0.5 {
            return None;
        }
        let (_, cells) = self.popup_geo(key)?;
        let it = self.items.iter().find(|i| &i.key == key)?;
        for (c, w) in cells.iter().zip(&it.wins) {
            if c.contains(x, y) {
                let close = Rect::new(c.right() - 26.0, c.y + 6.0, 20.0, 20.0);
                return Some((w.h, close.contains(x, y)));
            }
        }
        None
    }

    fn click_popup(&mut self, (h, close): (isize, bool)) {
        if close {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        } else {
            crate::sys::activate_window(h);
            self.pop_key = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

fn glide(v: &mut f32, target: f32, k: f32) -> bool {
    let d = target - *v;
    if d.abs() < 0.002 {
        *v = target;
        return false;
    }
    *v += d * k;
    true
}

impl Dock {
    /// Step the animations and draw. Returns true while more frames are needed.
    pub fn frame(&mut self, gfx: &Gfx, now: Instant) -> bool {
        if self.hidden {
            return false;
        }
        if self.need_place {
            self.place();
        }
        let dt = now.saturating_duration_since(self.last).as_secs_f32().min(0.05);
        self.last = now;
        let k = 1.0 - (-dt / 0.055).exp();
        let k_slow = 1.0 - (-dt / 0.09).exp();
        let mut moving = false;
        let s = self.size();
        let slot = self.slot();
        let mouse = self.mouse;

        // auto-hide: slide away shortly after the pointer leaves, come back when it touches the edge
        let busy_now = mouse.is_some() || self.menu.is_some() || self.drag.is_some() || self.pop_key.is_some();
        let want_show = if !self.cfg.auto_hide || busy_now {
            self.away = None;
            true
        } else {
            let t = *self.away.get_or_insert(now);
            now.saturating_duration_since(t) < Duration::from_millis(450)
        };
        moving |= glide(&mut self.reveal, if want_show { 1.0 } else { 0.0 }, k_slow);
        if self.away.is_some() && self.reveal > 0.002 {
            moving = true;
        }
        let bar = self.bar_rect();

        // magnification follows the pointer while it is near the bar
        let near = mouse.is_some_and(|(x, y)| {
            self.cfg.magnify
                && self.menu.is_none()
                && x >= bar.x - 24.0
                && x <= bar.right() + 24.0
                && y >= bar.y - s * 0.9
        });
        moving |= glide(&mut self.mag, if near { 1.0 } else { 0.0 }, k_slow);

        // hover dwell decides when the previews appear
        let hovered = mouse.and_then(|(x, y)| self.item_at(x, y));
        let hovered_key = hovered.and_then(|i| self.items.get(i)).map(|i| i.key.clone());
        let same = matches!((&self.hover, &hovered_key), (Some((a, _)), Some(b)) if a == b);
        if !same {
            self.hover = hovered_key.clone().map(|k| (k, now));
        }
        let want_preview = self.cfg.previews && self.drag.is_none() && self.menu.is_none();
        if want_preview {
            if let Some((hk, since)) = &self.hover {
                let has = self.items.iter().find(|i| &i.key == hk).is_some_and(|i| !i.wins.is_empty());
                if has && now.saturating_duration_since(*since) > Duration::from_millis(280) {
                    self.pop_key = Some(hk.clone());
                    self.pop_left = None;
                }
            }
            // keep the panel while the pointer is over it or on its icon; close it shortly after
            let over_pop = match (&self.pop_key, mouse) {
                (Some(key), Some((x, y))) => {
                    self.popup_geo(key).is_some_and(|(p, _)| {
                        // include the strip between the panel and the bar
                        Rect::new(p.x, p.y, p.w, bar.y + 4.0 - p.y).contains(x, y)
                    })
                }
                _ => false,
            };
            let on_icon = hovered_key.is_some() && hovered_key == self.pop_key;
            if self.pop_key.is_some() && !(over_pop || on_icon) {
                let left = *self.pop_left.get_or_insert(now);
                if now.saturating_duration_since(left) > Duration::from_millis(220) {
                    self.pop_key = None;
                    self.pop_left = None;
                }
            } else {
                self.pop_left = None;
            }
        } else {
            self.pop_key = None;
        }
        moving |= glide(&mut self.pop_alpha, if self.pop_key.is_some() { 1.0 } else { 0.0 }, k_slow);

        // capture a preview for the open panel (one per frame)
        if let Some(key) = self.pop_key.clone() {
            if let Some(it) = self.items.iter().find(|i| i.key == key) {
                let stale = |t: &Thumb| now.saturating_duration_since(t.at) > Duration::from_secs(4);
                if let Some(w) = it.wins.iter().find(|w| self.thumbs.get(&w.h).map_or(true, stale)) {
                    let h = w.h;
                    let img = capture(HWND(h as *mut _));
                    self.thumbs.insert(h, Thumb { img, at: now });
                }
            }
        }

        // items: slide to their slots, grow near the pointer
        let drag_idx = self.drag.as_ref().map(|d| d.idx);
        let drag_off = self.drag.as_ref().map_or(0.0, |d| d.off);
        let bell = s * 1.15;
        let amount = self.magnify_amount();
        for i in 0..self.items.len() {
            let tx = self.target_x(i);
            let it = &mut self.items[i];
            if it.fresh {
                it.x = tx;
                it.fresh = false;
            }
            if Some(i) == drag_idx {
                if let Some((mx, _)) = mouse {
                    it.x = mx - drag_off;
                }
            } else {
                moving |= glide(&mut it.x, tx, k);
            }
            let mut target_s = 1.0;
            if self.mag > 0.001 {
                if let Some((mx, _)) = mouse {
                    let d = (mx - it.x) / bell;
                    target_s = 1.0 + amount * self.mag * (-d * d).exp();
                }
            }
            if Some(i) == drag_idx {
                target_s = 1.12;
            }
            moving |= glide(&mut it.s, target_s, k);
            moving |= glide(&mut it.appear, 1.0, k_slow);
        }
        let _ = slot;

        // draw
        let Some(mut res) = self.res.take() else { return false };
        match self.host.begin_draw() {
            Ok((dc, off)) => {
                {
                    let mut p = Painter::new(dc, gfx, &mut res, self.scale, off.x as f32, off.y as f32);
                    p.clear();
                    self.paint(&mut p);
                }
                if let Err(e) = self.host.end_draw() {
                    crate::log!("dock draw failed: {e}");
                }
                res.end_frame();
            }
            Err(e) => crate::log!("dock begin draw failed: {e}"),
        }
        self.res = Some(res);

        // the blur behind the bar, and the part of the window that takes mouse clicks
        let sc = self.scale;
        let radius = (bar.h * 0.5).min(30.0);
        self.host.set_backdrop(
            Rect::new(bar.x * sc, (bar.y + radius) * sc, bar.w * sc, (bar.h - radius) * sc),
            radius * sc,
            if self.reveal < 0.02 { 0.0 } else { 1.0 },
        );
        self.update_region();

        self.busy = moving
            || self.mouse.is_some()
            || self.menu.is_some()
            || self.drag.is_some()
            || self.pop_key.is_some()
            || self.pop_alpha > 0.002;
        self.busy
    }

    fn update_region(&mut self) {
        let sc = self.scale;
        let bar = self.bar_rect();
        if self.cfg.auto_hide && self.reveal < 0.02 {
            // hidden: only a thin strip at the screen edge listens for the pointer
            let strip = Rect::new(bar.x - 24.0, self.win_h() - 4.0, bar.w + 48.0, 4.0);
            let px = (
                (strip.x * sc).floor().max(0.0) as i32,
                (strip.y * sc).floor().max(0.0) as i32,
                (strip.right() * sc).ceil() as i32,
                self.win.3,
                1,
            );
            if self.region != Some(px) {
                self.region = Some(px);
                unsafe {
                    let rgn = CreateRectRgn(px.0, px.1, px.2, px.3);
                    SetWindowRgn(self.hwnd, Some(rgn), false);
                }
            }
            return;
        }
        // the bar, and a band above it while icons grow or a panel is open
        let top = if self.mag > 0.01 || self.pop_alpha > 0.01 || self.menu.is_some() { 0.0 } else { bar.y - 14.0 };
        let mut rect = Rect::new(bar.x - 24.0, top.max(0.0), bar.w + 48.0, self.win_h() - top.max(0.0));
        if let Some(key) = &self.pop_key {
            if let Some((p, _)) = self.popup_geo(key) {
                let l = rect.x.min(p.x);
                let r = rect.right().max(p.right());
                rect = Rect::new(l, rect.y, r - l, rect.h);
            }
        }
        if let Some(m) = &self.menu {
            let l = rect.x.min(m.rect.x);
            let r = rect.right().max(m.rect.right());
            rect = Rect::new(l, rect.y, r - l, rect.h);
        }
        let px = (
            (rect.x * sc).floor().max(0.0) as i32,
            (rect.y * sc).floor().max(0.0) as i32,
            (rect.right() * sc).ceil() as i32,
            ((rect.bottom() * sc).ceil() as i32).min(self.win.3),
            0,
        );
        if self.region != Some(px) {
            self.region = Some(px);
            unsafe {
                let rgn = CreateRectRgn(px.0, px.1, px.2, px.3);
                // Windows owns the region after this call
                SetWindowRgn(self.hwnd, Some(rgn), false);
            }
        }
    }

    fn paint(&self, p: &mut Painter) {
        let bar = self.bar_rect();
        let s = self.size();
        let radius = (bar.h * 0.5).min(30.0);
        let fg = self.fg;
        p.shadow(bar, radius, 0.5);
        p.fill_rounded(bar, radius, Color::rgba(0.07, 0.07, 0.08, 0.50));
        p.stroke_rounded(bar.inset(0.5), radius, 1.0, Color::white(0.14));

        let bottom = bar.y + PAD + s;
        let hovered = self.mouse.and_then(|(x, y)| self.item_at(x, y));
        let drag_idx = self.drag.as_ref().map(|d| d.idx);
        let mut order: Vec<usize> = (0..self.items.len()).filter(|i| Some(*i) != drag_idx).collect();
        if let Some(d) = drag_idx {
            order.push(d);
        }
        for &i in &order {
            let it = &self.items[i];
            let sc = it.s * (0.6 + 0.4 * it.appear);
            let size = s * sc;
            let rect = Rect::new(it.x - size * 0.5, bottom - size - (sc - 1.0) * 2.0, size, size);
            p.alpha = it.appear.clamp(0.0, 1.0);
            match &it.icon {
                Some(img) => p.image(img, rect, 0.0, 1.0),
                None => {
                    p.fill_rounded(rect.inset(2.0), size * 0.24, it.tint);
                    let letter: String = it.name.chars().take(1).collect::<String>().to_uppercase();
                    let st = TextStyle::new(size * 0.46, Color::white(0.95)).semibold().center();
                    p.text(&letter, rect, &st);
                }
            }
            // running indicator
            if !it.wins.is_empty() {
                let active = it.wins.iter().any(|w| w.h == fg);
                let (cy, cx) = (bar.bottom() - 8.0, it.x);
                if active {
                    p.fill_rounded(Rect::new(cx - 6.0, cy - 1.5, 12.0, 3.0), 1.5, Color::white(0.95));
                } else {
                    p.fill_circle(cx, cy, 2.0, Color::white(0.55));
                }
            }
            p.alpha = 1.0;
            let _ = hovered;
        }

        // label for the pointed-at icon (not while a panel or menu is open)
        if self.pop_key.is_none() && self.menu.is_none() && self.drag.is_none() {
            if let Some(i) = hovered {
                if let Some((hk, since)) = &self.hover {
                    let it = &self.items[i];
                    if &it.key == hk && since.elapsed() > Duration::from_millis(120) {
                        let text = if it.wins.len() == 1 && !it.wins[0].title.is_empty() {
                            it.wins[0].title.clone()
                        } else {
                            it.name.clone()
                        };
                        let text: String = text.chars().take(48).collect();
                        let st = TextStyle::new(12.0, Color::white(0.95)).medium().center();
                        let w = (text.chars().count() as f32 * 6.6 + 22.0).clamp(54.0, 380.0);
                        let rect = Rect::new(it.x - w * 0.5, bar.y - s * (0.5 * it.s) - 34.0, w, 26.0);
                        let rect = Rect::new(rect.x.clamp(6.0, (self.win_w() - w - 6.0).max(6.0)), rect.y, w, 26.0);
                        p.fill_rounded(rect, 9.0, Color::rgba(0.08, 0.08, 0.09, 0.88));
                        p.stroke_rounded(rect.inset(0.5), 9.0, 1.0, Color::white(0.12));
                        p.text(&text, rect, &st);
                    }
                }
            }
        }

        // window previews
        if self.pop_alpha > 0.01 {
            if let Some(key) = &self.pop_key {
                self.paint_previews(p, key);
            }
        }
        // right-click menu
        if let Some(m) = &self.menu {
            p.fill_rounded(m.rect, 14.0, Color::rgba(0.09, 0.09, 0.10, 0.94));
            p.stroke_rounded(m.rect.inset(0.5), 14.0, 1.0, Color::white(0.14));
            for (i, (label, act)) in m.rows.iter().enumerate() {
                let row = Rect::new(m.rect.x + 6.0, m.rect.y + 6.0 + i as f32 * MENU_ROW, m.rect.w - 12.0, MENU_ROW);
                let over = self.mouse.is_some_and(|(x, y)| row.contains(x, y));
                if over {
                    p.fill_rounded(row, 8.0, Color::white(0.10));
                }
                let danger = matches!(act, Act::Close | Act::CloseAll);
                let c = if danger { Color::rgba(1.0, 0.42, 0.40, 1.0) } else { Color::white(0.92) };
                p.text(label, Rect::new(row.x + 10.0, row.y, row.w - 20.0, row.h), &TextStyle::new(13.0, c).medium());
            }
        }
    }

    fn paint_previews(&self, p: &mut Painter, key: &str) {
        let Some((panel, cells)) = self.popup_geo(key) else { return };
        let Some(it) = self.items.iter().find(|i| i.key == key) else { return };
        let a = self.pop_alpha.clamp(0.0, 1.0);
        let lift = (1.0 - a) * 8.0;
        p.alpha = a;
        let panel = panel.translate(0.0, lift);
        p.shadow(panel, 16.0, 0.5);
        p.fill_rounded(panel, 16.0, Color::rgba(0.08, 0.08, 0.09, 0.90));
        p.stroke_rounded(panel.inset(0.5), 16.0, 1.0, Color::white(0.14));
        for (c, w) in cells.iter().zip(&it.wins) {
            let c = c.translate(0.0, lift);
            let thumb = Rect::new(c.x, c.y, c.w, THUMB_H);
            let over = self.mouse.is_some_and(|(x, y)| c.translate(0.0, -lift).contains(x, y));
            let active = w.h == self.fg;
            p.fill_rounded(thumb, 10.0, Color::white(if over { 0.12 } else { 0.07 }));
            match self.thumbs.get(&w.h).and_then(|t| t.img.as_ref()) {
                Some(img) => p.image_zoom(img, thumb, 10.0, 1.0, 0.0),
                None => {
                    if let Some(icon) = &it.icon {
                        let r = Rect::new(thumb.cx() - 24.0, thumb.cy() - 24.0, 48.0, 48.0);
                        p.image(icon, r, 0.0, 0.9);
                    }
                }
            }
            if active || over {
                p.stroke_rounded(
                    thumb.inset(0.5),
                    10.0,
                    1.5,
                    if active { Color::white(0.7) } else { Color::white(0.3) },
                );
            }
            let title: String = w.title.chars().take(30).collect();
            p.text(
                &title,
                Rect::new(c.x + 2.0, c.y + THUMB_H + 3.0, c.w - 4.0, 20.0),
                &TextStyle::new(11.5, Color::white(0.85)).center(),
            );
            if over {
                let close = Rect::new(thumb.right() - 28.0, thumb.y + 8.0, 20.0, 20.0);
                p.fill_circle(close.cx(), close.cy(), 10.0, Color::rgba(0.07, 0.07, 0.08, 0.9));
                p.stroke_circle(close.cx(), close.cy(), 9.5, 1.0, Color::white(0.3));
                p.line(close.cx() - 3.5, close.cy() - 3.5, close.cx() + 3.5, close.cy() + 3.5, 1.4, Color::white(0.9));
                p.line(close.cx() + 3.5, close.cy() - 3.5, close.cx() - 3.5, close.cy() + 3.5, 1.4, Color::white(0.9));
            }
        }
        p.alpha = 1.0;
    }
}
