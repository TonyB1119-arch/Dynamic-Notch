//! A macOS-style dock.
//!
//! * Its own always-on-top, non-activating window, drawn with the same Direct2D
//!   stack as the notch (it borrows the notch's `Gfx`, `Res` and compositor, so it
//!   costs one small swap chain, not a second graphics device).
//! * Magnification: every icon grows with a smooth cosine falloff around the
//!   cursor; the cursor position is spring-smoothed so the wave glides.
//! * Icons come from the app itself (the shell's own icon), from a PNG named in
//!   `[[dock.items]] icon = "..."`, or from `%APPDATA%\DynamicNotch\dock-icons\NAME.png`.
//!   They are decoded on a worker thread.
//! * Adjustable blur: the area behind the glass is grabbed (the dock window is
//!   excluded from screen capture, so it never sees itself), shrunk 4× and
//!   box-blurred on the CPU three times (≈ gaussian). That is a few hundred
//!   microseconds, and it is refreshed only a little over once a second while the
//!   dock is on screen. A blur of 0 switches it off.
//! * Auto-hide: the dock slides below the screen edge and a thin strip at the
//!   bottom brings it back on hover.
//! * Frames are only produced while something moves (own vsync pacer); a resting
//!   dock costs nothing but the slow blur refresh.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateRectRgn, DeleteDC, DeleteObject, GdiFlush, GetDC, GetDIBits,
    GetMonitorInfoW, GetObjectW, ReleaseDC, SelectObject, SetStretchBltMode, SetWindowRgn, StretchBlt, BITMAP,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HALFTONE, HMONITOR, MONITORINFO, SRCCOPY,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, ShellExecuteW, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::anim::Spring;
use crate::config::{DockCfg, DockItem};
use crate::gfx::image::{decode, ImageData};
use crate::gfx::{Gfx, Painter, Res, TextStyle};
use crate::host::Host;
use crate::log;
use crate::pacer::Pacer;
use crate::util::{app_dir, Color, Rect, SendHwnd};
use crate::window::WM_APP_FRAME;

pub const WM_DOCK_ICONS: u32 = WM_APP + 20;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: PCWSTR = w!("DynamicNotch.Dock");
const T_HIDE: usize = 1;
const T_BLUR: usize = 2;
const HIDE_DELAY_MS: u32 = 400;
const BLUR_EVERY_MS: u32 = 1200;

/// Transparent room under the glass for the drop shadow (logical px).
const MARGIN: f32 = 10.0;
/// Room above the tallest icon for the name label.
const LABEL_ROOM: f32 = 30.0;
/// The capture is shrunk this much before blurring.
const BLUR_DOWN: i32 = 4;
const BOUNCE_SECS: f32 = 0.9;

struct Item {
    cfg: DockItem,
    img: Option<ImageData>,
    bounce: Option<Instant>,
    /// An icon lookup was already tried (the letter tile is the final answer).
    tried: bool,
    tile: Color,
    initial: String,
}

impl Item {
    fn new(cfg: &DockItem, img: Option<ImageData>) -> Self {
        let name = if cfg.name.trim().is_empty() { &cfg.path } else { &cfg.name };
        let initial = name.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
        let mut h: u32 = 2166136261;
        for b in name.bytes() {
            h = (h ^ b as u32).wrapping_mul(16777619);
        }
        let tile = hsv((h % 360) as f32, 0.55, 0.78);
        Self { cfg: cfg.clone(), img, bounce: None, tried: false, tile, initial }
    }

    fn label(&self) -> &str {
        if self.cfg.name.trim().is_empty() {
            &self.cfg.path
        } else {
            &self.cfg.name
        }
    }
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::rgba(r + m, g + m, b + m, 1.0)
}

/// Where everything is right now (logical px, window coordinates).
struct Layout {
    shelf: Rect,
    icons: Vec<Rect>,
    /// Rest height of the glass (without the slide offset).
    shelf_h: f32,
}

pub struct Dock {
    hwnd: HWND,
    host: Host,
    pacer: Pacer,
    cfg: DockCfg,
    items: Vec<Item>,
    icons_in: Arc<Mutex<Option<(u32, Vec<(usize, Option<ImageData>)>)>>>,
    icon_gen: u32,
    monitor: HMONITOR,
    scale: f32,
    /// Window rectangle in screen pixels.
    win: (i32, i32, i32, i32),
    win_w: f32,
    win_h: f32,
    // interaction
    hover: bool,
    tracking: bool,
    hovered: Option<usize>,
    label_idx: usize,
    pressed: Option<usize>,
    mx: Spring,
    amp: Spring,
    show: Spring,
    label: Spring,
    last_frame: Instant,
    suppressed: bool,
    window_visible: bool,
    region: Option<(i32, i32, i32, i32)>,
    // glass
    can_capture: bool,
    blur_img: Option<ImageData>,
    blur_dest: Rect,
    blur_sig: u64,
    stale: Vec<u64>,
}

impl Dock {
    pub fn new(gfx: &Gfx, compositor: Compositor, cfg: &DockCfg, monitor: HMONITOR) -> Result<Self> {
        let hwnd = create_window()?;
        // Keep the dock itself out of screen captures: that is how the blur can
        // sample "what is behind the dock" without ever seeing the dock.
        let can_capture = unsafe { SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(0x11)).is_ok() };
        if !can_capture {
            log!("dock: window capture exclusion unavailable, using flat glass");
        }
        let host = Host::new_shared(hwnd, gfx, 64, 64, compositor)?;
        let pacer = Pacer::new(SendHwnd::new(hwnd));
        let mut d = Self {
            hwnd,
            host,
            pacer,
            cfg: cfg.clone(),
            items: Vec::new(),
            icons_in: Arc::new(Mutex::new(None)),
            icon_gen: 0,
            monitor,
            scale: 1.0,
            win: (0, 0, 0, 0),
            win_w: 0.0,
            win_h: 0.0,
            hover: false,
            tracking: false,
            hovered: None,
            label_idx: 0,
            pressed: None,
            mx: Spring::new(0.0, 0.09, 1.0),
            amp: Spring::unit(0.0, 0.22, 1.0),
            show: Spring::unit(if cfg.autohide { 0.0 } else { 1.0 }, 0.42, 0.88),
            label: Spring::unit(0.0, 0.16, 1.0),
            last_frame: Instant::now(),
            suppressed: false,
            window_visible: false,
            region: None,
            can_capture,
            blur_img: None,
            blur_dest: Rect::new(0.0, 0.0, 0.0, 0.0),
            blur_sig: 0,
            stale: Vec::new(),
        };
        d.rebuild_items(&[]);
        d.place(monitor);
        unsafe {
            SetTimer(Some(hwnd), T_BLUR, BLUR_EVERY_MS, None);
        }
        Ok(d)
    }

    // -- configuration ------------------------------------------------------------

    /// Apply a changed configuration (hot reload).
    pub fn apply(&mut self, cfg: &DockCfg) {
        let old_items: Vec<(DockItem, Option<ImageData>)> = self.items.drain(..).map(|i| (i.cfg, i.img)).collect();
        let size_changed =
            (cfg.size - self.cfg.size).abs() > 0.5 || (cfg.magnify_scale - self.cfg.magnify_scale).abs() > 0.01;
        self.cfg = cfg.clone();
        let reuse: Vec<(DockItem, Option<ImageData>)> = if size_changed { Vec::new() } else { old_items };
        self.rebuild_items(&reuse);
        self.show.set_target(if self.cfg.autohide && !self.hover { 0.0 } else { 1.0 });
        let m = self.monitor;
        self.win = (0, 0, 0, 0); // force a re-layout
        self.place(m);
    }

    fn rebuild_items(&mut self, reuse: &[(DockItem, Option<ImageData>)]) {
        self.items = self
            .cfg
            .items
            .iter()
            .map(|c| {
                let img = reuse.iter().find(|(o, i)| o == c && i.is_some()).and_then(|(_, i)| i.clone());
                Item::new(c, img)
            })
            .collect();
    }

    /// Decode / fetch icons for the items that have none yet, off the UI thread.
    fn spawn_loader(&mut self) {
        let jobs: Vec<(usize, DockItem)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.img.is_none() && !i.tried)
            .map(|(n, i)| (n, i.cfg.clone()))
            .collect();
        if jobs.is_empty() {
            return;
        }
        for (n, _) in &jobs {
            self.items[*n].tried = true;
        }
        self.icon_gen += 1;
        let gen = self.icon_gen;
        let out = self.icons_in.clone();
        let hwnd = SendHwnd::new(self.hwnd);
        let px =
            (self.cfg.size * self.cfg.magnify_scale.max(1.0) * self.scale.max(1.0)).ceil().clamp(64.0, 256.0) as i32;
        let _ = std::thread::Builder::new().name("dock-icons".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let done: Vec<(usize, Option<ImageData>)> = jobs.iter().map(|(n, j)| (*n, load_icon(j, px))).collect();
            if let Ok(mut g) = out.lock() {
                *g = Some((gen, done));
            }
            unsafe {
                let _ = PostMessageW(Some(hwnd.hwnd()), WM_DOCK_ICONS, WPARAM(0), LPARAM(0));
                CoUninitialize();
            }
        });
    }

    fn take_icons(&mut self) {
        let Some((gen, done)) = self.icons_in.lock().ok().and_then(|mut g| g.take()) else { return };
        if gen != self.icon_gen {
            return;
        }
        for (n, img) in done {
            if let (Some(item), Some(img)) = (self.items.get_mut(n), img) {
                item.img = Some(img);
            }
        }
        self.wake();
    }

    // -- placement ----------------------------------------------------------------

    fn magnify(&self) -> f32 {
        if self.cfg.magnification {
            self.cfg.magnify_scale.max(1.0)
        } else {
            1.0
        }
    }

    fn shelf_h(&self) -> f32 {
        self.cfg.size * 1.34
    }

    fn pad_y(&self) -> f32 {
        self.cfg.size * 0.17
    }

    fn gap(&self) -> f32 {
        self.cfg.size * 0.14
    }

    fn base_width(&self) -> f32 {
        let n = self.items.len() as f32;
        if n < 1.0 {
            return 0.0;
        }
        n * self.cfg.size + (n - 1.0) * self.gap()
    }

    pub fn place(&mut self, monitor: HMONITOR) {
        self.monitor = monitor;
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
                return;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            self.scale = dx as f32 / 96.0;
        }
        let s = self.cfg.size;
        let m = self.magnify();
        let pitch = s + self.gap();
        let extra = (m - 1.0) * s * 2.4 * 1.15 + pitch * 0.5;
        let want_w = self.base_width() + 2.0 * s * 0.2 + extra + 24.0;
        let work_w = (mi.rcWork.right - mi.rcWork.left) as f32 / self.scale;
        self.win_w = want_w.min(work_w).max(40.0);
        self.win_h = MARGIN + self.shelf_h() + s * (m - 1.0) + LABEL_ROOM + s * 0.5;
        let wpx = (self.win_w * self.scale).ceil() as i32;
        let hpx = (self.win_h * self.scale).ceil() as i32;
        let x = mi.rcWork.left + ((mi.rcWork.right - mi.rcWork.left) - wpx) / 2;
        let y = mi.rcWork.bottom - hpx;
        let geom = (x, y, wpx, hpx);
        if geom == self.win {
            return; // nothing moved (the notch asks on every window switch)
        }
        {
            self.win = geom;
            unsafe {
                let flags = if self.suppressed { SWP_NOACTIVATE } else { SWP_NOACTIVATE | SWP_SHOWWINDOW };
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, wpx, hpx, flags);
            }
            self.window_visible = !self.suppressed;
            if let Err(e) = self.host.resize(wpx, hpx) {
                log!("dock: resize failed: {e}");
            }
            self.region = None;
        }
        self.blur_sig = 0;
        self.capture_blur();
        self.spawn_loader();
        self.wake();
    }

    /// Hide while a fullscreen app is up.
    pub fn set_hidden(&mut self, hide: bool) {
        self.suppressed = hide;
        if hide == !self.window_visible {
            return;
        }
        unsafe {
            if hide {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            } else {
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
        self.window_visible = !hide;
        if !hide {
            self.capture_blur();
            self.wake();
        }
    }

    /// Something else took the top of the z-order: take it back.
    pub fn raise(&self) {
        if self.window_visible {
            unsafe {
                let _ =
                    SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    /// After a graphics device loss.
    pub fn set_rendering_device(&mut self, gfx: &Gfx) {
        let _ = self.host.set_rendering_device(gfx);
        self.wake();
    }

    fn wake(&self) {
        self.pacer.start();
    }

    // -- layout -------------------------------------------------------------------

    fn layout(&self) -> Layout {
        let s = self.cfg.size;
        let n = self.items.len();
        let gap = self.gap();
        let pad = s * 0.2;
        let shelf_h = self.shelf_h();
        let m = self.magnify();
        let amp = self.amp.value.clamp(0.0, 1.0);
        let cx = self.win_w * 0.5;
        let pitch = s + gap;
        let reach = pitch * 2.4;
        let base_w = self.base_width();
        let base_x0 = cx - base_w * 0.5;
        let mx = self.mx.value;
        let mut widths = Vec::with_capacity(n);
        for i in 0..n {
            let bx = base_x0 + s * 0.5 + i as f32 * pitch;
            let d = (mx - bx).abs();
            let bump = if d >= reach { 0.0 } else { 0.5 * (1.0 + (std::f32::consts::PI * d / reach).cos()) };
            widths.push(s * (1.0 + (m - 1.0) * amp * bump));
        }
        let total: f32 = widths.iter().sum::<f32>() + gap * (n.max(1) as f32 - 1.0);
        let slide = (1.0 - self.show.value).max(0.0) * (shelf_h + MARGIN + 12.0);
        let bottom = self.win_h - MARGIN + slide;
        let shelf = Rect::new(cx - total * 0.5 - pad, bottom - shelf_h, total + 2.0 * pad, shelf_h);
        let icon_bottom = shelf.bottom() - self.pad_y();
        let mut x = cx - total * 0.5;
        let mut icons = Vec::with_capacity(n);
        for w in widths {
            icons.push(Rect::new(x, icon_bottom - w, w, w));
            x += w + gap;
        }
        Layout { shelf, icons, shelf_h }
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let l = self.layout();
        let gap = self.gap();
        if y < l.shelf.y - self.cfg.size * (self.magnify() - 1.0) - 4.0 || y > l.shelf.bottom() {
            return None;
        }
        l.icons.iter().position(|r| x >= r.x - gap * 0.5 && x < r.right() + gap * 0.5)
    }

    fn update_region(&mut self) {
        let s = self.scale;
        let (w, h) = (self.win.2, self.win.3);
        let m = self.magnify();
        let size = self.cfg.size;
        let base_pad = size * 0.2;
        let cx = self.win_w * 0.5;
        let bw = self.base_width();
        let hidden = self.cfg.autohide && self.show.value < 0.02 && self.show.target < 0.5;
        let big = self.hover || self.amp.value > 0.002 || (self.show.value > 0.02 && self.show.target > 0.5);
        let rect = if self.items.is_empty() {
            (0, 0, 0, 0)
        } else if hidden {
            // a thin hot strip on the bottom edge
            let x0 = ((cx - bw * 0.5 - base_pad - 16.0) * s).floor().max(0.0) as i32;
            let x1 = ((cx + bw * 0.5 + base_pad + 16.0) * s).ceil().min(w as f32) as i32;
            (x0, h - ((MARGIN * s) as i32).max(3), x1, h)
        } else if big {
            let top = self.win_h - MARGIN - self.shelf_h() - size * (m - 1.0) - 6.0;
            (0, (top * s).floor().max(0.0) as i32, w, h)
        } else {
            let top = self.win_h - MARGIN - self.shelf_h() - 2.0;
            let x0 = ((cx - bw * 0.5 - base_pad - 2.0) * s).floor().max(0.0) as i32;
            let x1 = ((cx + bw * 0.5 + base_pad + 2.0) * s).ceil().min(w as f32) as i32;
            (x0, (top * s).floor().max(0.0) as i32, x1, h)
        };
        if self.region != Some(rect) {
            self.region = Some(rect);
            unsafe {
                let rgn = CreateRectRgn(rect.0, rect.1, rect.2, rect.3);
                SetWindowRgn(self.hwnd, Some(rgn), false);
            }
        }
    }

    // -- messages -----------------------------------------------------------------

    pub fn handle(&mut self, gfx: &Gfx, res: &mut Res, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_APP_FRAME => {
                self.pacer.frame_consumed();
                self.frame(gfx, res);
                Some(LRESULT(0))
            }
            WM_DOCK_ICONS => {
                self.take_icons();
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (x, y) = self.lp_to_logical(lp);
                self.on_move(x, y);
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.on_leave();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.lp_to_logical(lp);
                self.pressed = self.hit(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = self.lp_to_logical(lp);
                let up = self.hit(x, y);
                if let (Some(d), Some(u)) = (self.pressed.take(), up) {
                    if d == u {
                        self.launch(u);
                    }
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => Some(LRESULT(0)),
            WM_MOUSEACTIVATE => Some(LRESULT(3)), // MA_NOACTIVATE
            WM_SETCURSOR => {
                if self.hovered.is_some() && (lp.0 & 0xFFFF) as u32 == 1 {
                    unsafe {
                        if let Ok(c) = LoadCursorW(None, IDC_HAND) {
                            SetCursor(Some(c));
                        }
                    }
                    Some(LRESULT(1))
                } else {
                    None
                }
            }
            WM_TIMER => {
                match wp.0 {
                    T_HIDE => {
                        unsafe {
                            let _ = KillTimer(Some(self.hwnd), T_HIDE);
                        }
                        if !self.hover && self.cfg.autohide {
                            self.show.set_target(0.0);
                            self.wake();
                        }
                    }
                    T_BLUR => {
                        let visible = self.window_visible && self.show.value > 0.5;
                        if visible && self.cfg.blur >= 0.5 && self.capture_blur() {
                            self.wake();
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn lp_to_logical(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        (x / self.scale, y / self.scale)
    }

    fn on_move(&mut self, x: f32, y: f32) {
        if !self.tracking {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            self.tracking = true;
        }
        if !self.hover {
            self.hover = true;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_HIDE);
            }
            if self.amp.value < 0.02 {
                self.mx.snap(x);
            }
            if self.cfg.autohide && self.show.value < 0.99 {
                // coming back: refresh the glass in the background of the slide
                self.blur_sig = 0;
                self.capture_blur();
            }
        }
        self.mx.set_target(x);
        self.amp.set_target(1.0);
        self.show.set_target(1.0);
        let hi = self.hit(x, y);
        if hi != self.hovered {
            self.hovered = hi;
            if let Some(i) = hi {
                self.label_idx = i;
            }
            self.label.set_target(if hi.is_some() { 1.0 } else { 0.0 });
        }
        self.wake();
    }

    fn on_leave(&mut self) {
        self.tracking = false;
        self.hover = false;
        self.hovered = None;
        self.pressed = None;
        self.label.set_target(0.0);
        self.amp.set_target(0.0);
        if self.cfg.autohide {
            unsafe {
                SetTimer(Some(self.hwnd), T_HIDE, HIDE_DELAY_MS, None);
            }
        }
        self.wake();
    }

    fn launch(&mut self, i: usize) {
        let Some(item) = self.items.get_mut(i) else { return };
        item.bounce = Some(Instant::now());
        let path = item.cfg.path.trim().trim_matches('"').to_string();
        let args = item.cfg.args.trim().to_string();
        if path.is_empty() {
            return;
        }
        if args.is_empty() {
            crate::sys::shell_open(&path);
        } else {
            let p = crate::util::wide(&path);
            let a = crate::util::wide(&args);
            unsafe {
                ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR(a.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
            }
        }
        self.wake();
    }

    // -- frame --------------------------------------------------------------------

    fn frame(&mut self, gfx: &Gfx, res: &mut Res) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(1.0 / 20.0);
        self.last_frame = now;
        let mut moving = false;
        moving |= self.show.step(dt);
        moving |= self.amp.step(dt);
        moving |= self.mx.step(dt);
        moving |= self.label.step(dt);
        let mut bouncing = false;
        for it in &mut self.items {
            if let Some(t) = it.bounce {
                if now.saturating_duration_since(t).as_secs_f32() >= BOUNCE_SECS {
                    it.bounce = None;
                } else {
                    bouncing = true;
                }
            }
        }
        self.update_region();
        if self.window_visible {
            self.render(gfx, res, now);
        }
        if !moving && !bouncing {
            self.pacer.stop();
        }
    }

    fn render(&mut self, gfx: &Gfx, res: &mut Res, now: Instant) {
        let (dc, off) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(e) => {
                log!("dock: BeginDraw failed: {e}");
                return;
            }
        };
        let l = self.layout();
        let s = self.cfg.size;
        {
            let mut p = Painter::new(dc, gfx, res, self.scale, off.x as f32, off.y as f32);
            p.clear();
            for id in self.stale.drain(..) {
                p.forget_image(id);
            }
            if !self.items.is_empty() && self.show.value > 0.003 {
                let radius = (l.shelf_h * 0.32).min(24.0);
                let slide = l.shelf.bottom() - (self.win_h - MARGIN);
                // shadow, then glass
                p.shadow(l.shelf.expand(1.0), radius, 0.38);
                match (&self.blur_img, self.cfg.blur >= 0.5 && self.can_capture) {
                    (Some(img), true) => {
                        let dest = self.blur_dest.translate(0.0, slide);
                        p.image_clipped(img, dest, l.shelf, radius, 1.0);
                    }
                    _ => p.fill_rounded(l.shelf, radius, Color::rgba(0.10, 0.10, 0.12, 0.55)),
                }
                p.fill_rounded(l.shelf, radius, Color::white(self.cfg.tint));
                // soft light from the top edge + hairline border
                p.fill_rounded(Rect::new(l.shelf.x, l.shelf.y, l.shelf.w, l.shelf.h * 0.5), radius, Color::white(0.03));
                p.stroke_rounded(l.shelf.inset(0.5), radius, 1.0, Color::white(0.20));

                for (i, (item, r)) in self.items.iter().zip(l.icons.iter()).enumerate() {
                    let mut r = *r;
                    if let Some(t) = item.bounce {
                        let u = (now.saturating_duration_since(t).as_secs_f32() / BOUNCE_SECS).clamp(0.0, 1.0);
                        let up = (u * std::f32::consts::PI * 3.0).sin().abs() * (1.0 - u) * s * 0.5;
                        r = r.translate(0.0, -up);
                    }
                    draw_icon(&mut p, item, r, i == self.hovered.unwrap_or(usize::MAX));
                }

                // name label above the hovered icon
                let la = self.label.value.clamp(0.0, 1.0);
                if self.cfg.labels && la > 0.02 {
                    if let (Some(item), Some(r)) = (self.items.get(self.label_idx), l.icons.get(self.label_idx)) {
                        let style = TextStyle::new(12.5, Color::white(0.96)).medium().center();
                        let (tw, _) = p.measure(item.label(), &style);
                        let pw = tw + 20.0;
                        let ph = 26.0;
                        let px = (r.cx() - pw * 0.5).clamp(2.0, (self.win_w - pw - 2.0).max(2.0));
                        let py = (r.y - ph - 8.0).max(1.0) + (1.0 - la) * 4.0;
                        let pill = Rect::new(px, py, pw, ph);
                        p.alpha = la;
                        p.fill_rounded(pill, ph * 0.5, Color::rgba(0.12, 0.12, 0.14, 0.82));
                        p.stroke_rounded(pill.inset(0.5), ph * 0.5, 1.0, Color::white(0.14));
                        p.text(item.label(), pill, &style);
                        p.alpha = 1.0;
                    }
                }
            }
        }
        if let Err(e) = self.host.end_draw() {
            log!("dock: EndDraw failed: {e}");
        }
    }

    // -- glass --------------------------------------------------------------------

    /// Grab and blur what is behind the dock. Returns true when the picture changed.
    fn capture_blur(&mut self) -> bool {
        if self.cfg.blur < 0.5 || !self.can_capture || self.win.2 <= 0 || self.items.is_empty() {
            if self.blur_img.is_some() {
                if let Some(old) = self.blur_img.take() {
                    self.stale.push(old.id);
                }
                return true;
            }
            return false;
        }
        let sc = self.scale;
        let pad = self.cfg.blur * 1.5 + 4.0;
        let shelf_top = self.win_h - MARGIN - self.shelf_h();
        let band = Rect::new(0.0, shelf_top - pad, self.win_w, self.shelf_h() + 2.0 * pad);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            if !GetMonitorInfoW(self.monitor, &mut mi).as_bool() {
                return false;
            }
        }
        let (wx, wy) = (self.win.0, self.win.1);
        let mut x0 = wx + (band.x * sc).floor() as i32;
        let mut y0 = wy + (band.y * sc).floor() as i32;
        let mut x1 = wx + (band.right() * sc).ceil() as i32;
        let mut y1 = wy + (band.bottom() * sc).ceil() as i32;
        x0 = x0.max(mi.rcMonitor.left);
        y0 = y0.max(mi.rcMonitor.top);
        x1 = x1.min(mi.rcMonitor.right);
        y1 = y1.min(mi.rcMonitor.bottom);
        let (sw, sh) = (x1 - x0, y1 - y0);
        if sw < 8 || sh < 8 {
            return false;
        }
        let dw = (sw / BLUR_DOWN).max(2);
        let dh = (sh / BLUR_DOWN).max(2);
        let Some(mut px) = grab_scaled(x0, y0, sw, sh, dw, dh) else { return false };
        let sigma = self.cfg.blur * sc / BLUR_DOWN as f32;
        let half = sigma.round().max(1.0) as usize;
        blur3(&mut px, dw as usize, dh as usize, half);

        let mut sig: u64 = 1469598103934665603;
        for b in px.iter().step_by(11) {
            sig = (sig ^ *b as u64).wrapping_mul(1099511628211);
        }
        let dest = Rect::new((x0 - wx) as f32 / sc, (y0 - wy) as f32 / sc, sw as f32 / sc, sh as f32 / sc);
        if sig == self.blur_sig && self.blur_img.is_some() && dest == self.blur_dest {
            return false;
        }
        self.blur_sig = sig;
        self.blur_dest = dest;
        if let Some(old) = self.blur_img.take() {
            self.stale.push(old.id);
        }
        self.blur_img = Some(ImageData::from_pbgra(dw as u32, dh as u32, px));
        true
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_BLUR);
            let _ = KillTimer(Some(self.hwnd), T_HIDE);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw_icon(p: &mut Painter, item: &Item, r: Rect, hovered: bool) {
    match &item.img {
        Some(img) => {
            // fit (not crop) so wide or tall PNGs are never cut off
            let (iw, ih) = (img.width as f32, img.height as f32);
            let k = (r.w / iw).min(r.h / ih);
            let (w, h) = (iw * k, ih * k);
            let fit = Rect::new(r.cx() - w * 0.5, r.bottom() - h, w, h);
            p.image(img, fit, 0.0, if hovered { 1.0 } else { 0.97 });
        }
        None => {
            let rad = r.w * 0.23;
            p.fill_rounded(r, rad, item.tile);
            p.fill_rounded(Rect::new(r.x, r.y, r.w, r.h * 0.5), rad, Color::white(0.08));
            p.stroke_rounded(r.inset(0.5), rad, 1.0, Color::white(0.18));
            p.text(&item.initial, r, &TextStyle::new(r.w * 0.46, Color::white(0.96)).semibold().center());
        }
    }
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

fn create_window() -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
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
        let on = windows::core::BOOL(1);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_EXCLUDED_FROM_PEEK, &on as *const _ as _, 4);
        let none: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        Ok(hwnd)
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| crate::window::with_app(|app| app.dock_msg(msg, wp, lp)).flatten());
    match r {
        Ok(Some(r)) => r,
        Ok(None) => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        Err(_) => {
            log!("dock: recovered from panic while handling message {msg:#x}");
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
    }
}

// ---------------------------------------------------------------------------
// Screen capture + blur
// ---------------------------------------------------------------------------

/// Copy a screen rectangle, shrunk to `dw`×`dh`, as opaque BGRA.
fn grab_scaled(x: i32, y: i32, sw: i32, sh: i32, dw: i32, dh: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: dw,
                biHeight: -dh, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        let mut out = None;
        if let Ok(dib) = dib {
            if !bits.is_null() && !mem.is_invalid() {
                let old = SelectObject(mem, dib.into());
                SetStretchBltMode(mem, HALFTONE);
                let ok = StretchBlt(mem, 0, 0, dw, dh, Some(screen), x, y, sw, sh, SRCCOPY | CAPTUREBLT);
                let _ = GdiFlush();
                if ok.as_bool() {
                    let n = (dw * dh * 4) as usize;
                    out = Some(std::slice::from_raw_parts(bits as *const u8, n).to_vec());
                }
                SelectObject(mem, old);
            }
            let _ = DeleteObject(dib.into());
        }
        if !mem.is_invalid() {
            let _ = DeleteDC(mem);
        }
        ReleaseDC(None, screen);
        if let Some(px) = out.as_mut() {
            for a in px.chunks_exact_mut(4) {
                a[3] = 255;
            }
        }
        out
    }
}

/// Three box blurs in each direction ≈ a gaussian. `r` is the half-width of one box.
fn blur3(px: &mut [u8], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0u8; px.len()];
    for _ in 0..3 {
        // horizontal: px → tmp
        for y in 0..h {
            for c in 0..3 {
                box_line(px, &mut tmp, y * w * 4 + c, 4, w, r);
            }
        }
        // vertical: tmp → px
        for x in 0..w {
            for c in 0..3 {
                box_line(&tmp, px, x * 4 + c, w * 4, h, r);
            }
        }
    }
}

/// One running-sum box filter along a line of `n` samples (edge pixels repeat).
fn box_line(src: &[u8], dst: &mut [u8], base: usize, stride: usize, n: usize, r: usize) {
    if n == 0 {
        return;
    }
    let at = |i: isize| -> usize { base + (i.clamp(0, n as isize - 1) as usize) * stride };
    let ri = r as isize;
    let win = (2 * r + 1) as u32;
    let mut sum: u32 = 0;
    for k in -ri..=ri {
        sum += src[at(k)] as u32;
    }
    for i in 0..n as isize {
        dst[at(i)] = (sum / win) as u8;
        sum += src[at(i + ri + 1)] as u32;
        sum -= src[at(i - ri)] as u32;
    }
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

fn load_icon(item: &DockItem, px: i32) -> Option<ImageData> {
    // 1) a PNG (or other image) chosen for this item
    let custom = item.icon.trim().trim_matches('"');
    if !custom.is_empty() {
        if let Some(img) = std::fs::read(custom).ok().and_then(|b| decode(&b, 256)) {
            return Some(img);
        }
    }
    // 2) %APPDATA%\DynamicNotch\dock-icons\<name>.png
    let dir = app_dir().join("dock-icons");
    for name in [item.name.trim().to_string(), item.name.trim().to_lowercase()] {
        if name.is_empty() {
            continue;
        }
        if let Some(img) = std::fs::read(dir.join(format!("{name}.png"))).ok().and_then(|b| decode(&b, 256)) {
            return Some(img);
        }
    }
    // 3) the app's own icon
    unsafe { shell_icon(&item.path, px) }
}

/// Find the real file behind "notepad.exe" / a quoted path (None for links like "ms-settings:").
fn resolve(target: &str) -> Option<String> {
    let t = target.trim().trim_matches('"');
    if t.is_empty() {
        return None;
    }
    if std::path::Path::new(t).exists() {
        return Some(t.to_string());
    }
    if t.contains(':') && t.len() > 2 && !t.as_bytes().get(1).is_some_and(|c| *c == b':') {
        return None; // URI scheme
    }
    let mut names = vec![t.to_string()];
    if std::path::Path::new(t).extension().is_none() {
        names.push(format!("{t}.exe"));
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for n in &names {
            let c = dir.join(n);
            if c.exists() {
                return Some(c.to_string_lossy().into_owned());
            }
        }
    }
    None
}

unsafe fn shell_icon(target: &str, px: i32) -> Option<ImageData> {
    let full = resolve(target)?;
    let wide = crate::util::wide(&full);
    let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
    let hbmp = factory.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;

    let mut bm = BITMAP::default();
    if GetObjectW(hbmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        let _ = DeleteObject(hbmp.into());
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    if w <= 0 || h <= 0 {
        let _ = DeleteObject(hbmp.into());
        return None;
    }
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0 as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let dc = GetDC(None);
    let lines = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    let _ = DeleteObject(hbmp.into());
    if lines == 0 {
        return None;
    }
    // A bitmap without an alpha channel comes back with alpha 0 everywhere.
    if buf.chunks_exact(4).all(|p| p[3] == 0) {
        for p in buf.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    // Direct2D wants premultiplied colour: never let a channel exceed alpha.
    for p in buf.chunks_exact_mut(4) {
        let a = p[3];
        p[0] = p[0].min(a);
        p[1] = p[1].min(a);
        p[2] = p[2].min(a);
    }
    Some(ImageData::from_pbgra(w as u32, h as u32, buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_blur_keeps_flat_colour() {
        let mut px = vec![200u8; 8 * 6 * 4];
        blur3(&mut px, 8, 6, 3);
        assert!(px.iter().all(|v| *v == 200 || *v == 199));
    }

    #[test]
    fn box_blur_spreads_a_spike() {
        let (w, h) = (9usize, 1usize);
        let mut px = vec![0u8; w * h * 4];
        px[4 * 4] = 255;
        blur3(&mut px, w, h, 1);
        assert!(px[3 * 4] > 0 && px[5 * 4] > 0 && px[4 * 4] < 255);
    }
}
