//! The radial menu: press Win+M and a round menu of your apps opens around the pointer. Two rings of
//! eight slots (the inner ring first, both clockwise from the top). Moving the pointer slides a white
//! highlight from slice to slice on springs, a click opens the app, and the menu pops open and shrinks
//! away again.
//!
//! Cost: nothing exists until the hotkey is pressed for the first time. While it is closed there is no
//! timer and no drawing; while it is open the 15 ms timer runs only as long as something moves.

use std::mem::size_of;
use std::time::Instant;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::{
    CreateEllipticRgn, GetMonitorInfoW, MonitorFromPoint, SetWindowRgn, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::config::RadialCfg;
use crate::gfx::{Gfx, Icon, ImageData, Painter, Res, TextStyle};
use crate::host::Host;
use crate::util::{Color, Rect};

use geom::*;

const CLASS: PCWSTR = w!("DynamicNotch.Radial");
const T_ANIM: usize = 1;
/// The pointer left the window (the windows crate has no name for it).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Room around the disc for its shadow (dips).
const MARGIN: f32 = 28.0;
const SIDE: f32 = (OUT1 + MARGIN) * 2.0;
/// Spring of the highlight: stiffness and damping (a little overshoot, settled in about 0.25 s).
const SPRING_K: f32 = 900.0;
const SPRING_C: f32 = 40.0;
/// Keep the pointer in one direction this long (seconds) and that app opens, no click needed.
const DWELL: f32 = 0.8;

/// Pure geometry and springs (no Windows in here, so it can be tested anywhere).
mod geom {
    use std::f32::consts::{PI, TAU};

    /// The empty circle in the middle.
    pub const R_C: f32 = 58.0;
    pub const IN0: f32 = 62.0;
    pub const IN1: f32 = 120.0;
    pub const OUT0: f32 = 124.0;
    pub const OUT1: f32 = 188.0;
    /// With only the inner ring filled the menu is one bigger ring (to this radius).
    pub const SINGLE1: f32 = 150.0;
    pub const SLOTS: usize = 16;
    /// Half the angle a slice spans (a thin gap is left between neighbours).
    pub const HW: f32 = PI / 8.0 - 0.022;

    /// Where slot `i` points (radians, 0 = 12 o'clock, clockwise).
    pub fn slot_angle(i: usize) -> f32 {
        (i % 8) as f32 * PI / 4.0
    }

    /// The inner and outer radius of slot `i`.
    pub fn slot_radii(i: usize) -> (f32, f32) {
        if i < 8 {
            (IN0, IN1)
        } else {
            (OUT0, OUT1)
        }
    }

    /// `slot_radii`, or the one big ring when the outer ring is empty.
    pub fn radii(i: usize, single: bool) -> (f32, f32) {
        if single {
            (IN0, SINGLE1)
        } else {
            slot_radii(i)
        }
    }

    /// `hit`, or the one big ring when the outer ring is empty.
    pub fn hit_in(dx: f32, dy: f32, single: bool) -> Option<usize> {
        if !single {
            return hit(dx, dy);
        }
        let d = (dx * dx + dy * dy).sqrt();
        if d < R_C || d > SINGLE1 {
            return None;
        }
        hit(dx * (IN0 + 1.0) / d, dy * (IN0 + 1.0) / d)
    }

    pub fn polar(cx: f32, cy: f32, r: f32, a: f32) -> (f32, f32) {
        (cx + r * a.sin(), cy - r * a.cos())
    }

    /// The slot under a point `dx`, `dy` away from the middle (in dips).
    pub fn hit(dx: f32, dy: f32) -> Option<usize> {
        let d = (dx * dx + dy * dy).sqrt();
        if d < R_C || d > OUT1 {
            return None;
        }
        let mut a = dx.atan2(-dy);
        if a < 0.0 {
            a += TAU;
        }
        let k = (((a + PI / 8.0) / (PI / 4.0)) as usize) % 8;
        Some(if d < (IN1 + OUT0) * 0.5 { k } else { 8 + k })
    }

    /// `target` plus whole turns, the one closest to `cur` (so the highlight takes the short way round).
    pub fn nearest_turn(cur: f32, target: f32) -> f32 {
        target + TAU * ((cur - target) / TAU).round()
    }

    /// Move `x` towards `t` by at most `step`.
    pub fn toward(x: f32, t: f32, step: f32) -> f32 {
        if (t - x).abs() <= step {
            t
        } else {
            x + step * (t - x).signum()
        }
    }

    /// Opening: a little overshoot at the end (0 to 1).
    pub fn ease_back(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0) - 1.0;
        1.0 + 2.15 * t * t * t + 1.15 * t * t
    }

    pub fn ease_out(t: f32) -> f32 {
        let t = 1.0 - t.clamp(0.0, 1.0);
        1.0 - t * t * t
    }

    /// A value on a spring.
    #[derive(Clone, Copy, Default)]
    pub struct Sp {
        pub x: f32,
        pub v: f32,
    }

    impl Sp {
        pub fn at(x: f32) -> Self {
            Self { x, v: 0.0 }
        }

        pub fn step(&mut self, target: f32, dt: f32, k: f32, c: f32) {
            let n = ((dt / 0.008).ceil() as usize).clamp(1, 12);
            let h = dt / n as f32;
            for _ in 0..n {
                self.v += (k * (target - self.x) - c * self.v) * h;
                self.x += self.v * h;
            }
        }

        pub fn settled(&self, target: f32, eps: f32) -> bool {
            (self.x - target).abs() < eps && self.v.abs() < eps * 8.0
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hits() {
            assert_eq!(hit(0.0, -90.0), Some(0));
            assert_eq!(hit(0.0, -150.0), Some(8));
            assert_eq!(hit(100.0, 0.0), Some(2));
            assert_eq!(hit(0.0, 90.0), Some(4));
            assert_eq!(hit(-100.0, 0.0), Some(6));
            assert_eq!(hit(110.0, 110.0), Some(8 + 3));
            assert_eq!(hit(10.0, 10.0), None);
            assert_eq!(hit(0.0, -200.0), None);
        }

        #[test]
        fn short_way() {
            let t = nearest_turn(6.0, 0.0);
            assert!((t - TAU).abs() < 1e-4);
            let t = nearest_turn(0.1, 7.0 * PI / 4.0);
            assert!((t + PI / 4.0).abs() < 1e-3);
        }

        #[test]
        fn spring_settles() {
            let mut s = Sp::at(0.0);
            let mut over = 0.0f32;
            for _ in 0..60 {
                s.step(1.0, 1.0 / 60.0, 900.0, 40.0);
                over = over.max(s.x);
            }
            assert!(s.settled(1.0, 0.01), "{}", s.x);
            assert!(over > 1.0 && over < 1.2, "{over}");
        }

        #[test]
        fn eases() {
            assert!((ease_back(1.0) - 1.0).abs() < 1e-6);
            assert!(ease_back(0.0).abs() < 1e-6);
            assert!(ease_back(0.8) > 1.0);
            assert!((ease_out(1.0) - 1.0).abs() < 1e-6);
        }
    }
}

/// The colours of one look (every one is see-through: the blur shows behind it).
struct Pal {
    /// The glass disc.
    tint: Color,
    /// The slice under the pointer.
    hl: Color,
    ink: Color,
    pill: Color,
}

/// Classic: light frosted glass. Tuna: Tuna's dark mode, smoked glass with a dark blue selection.
/// `glass`: 0 = a solid tint, 1 = clear glass.
fn pal(tuna: bool, glass: f32) -> Pal {
    if tuna {
        Pal {
            tint: Color::hex(0x0E1013).alpha(0.85 * (1.0 - glass)),
            hl: Color::hex(0x2A6A9A).alpha(0.55),
            ink: Color::white(0.92),
            pill: Color::hex(0x000000).alpha(0.30),
        }
    } else {
        Pal {
            tint: Color::white(0.30 * (1.0 - glass)),
            hl: Color::white(0.22),
            ink: Color::white(0.92),
            pill: Color::hex(0x000000).alpha(0.22),
        }
    }
}

/// One app of the menu, worked out once when the list changes.
struct Slot {
    name: String,
    target: String,
}

fn make_slot(item: &str) -> Option<Slot> {
    let item = item.trim();
    let (name, target) = match item.split_once('|') {
        Some((n, t)) => (n.trim(), t.trim()),
        None => ("", item),
    };
    if target.is_empty() {
        return None;
    }
    let name = if name.is_empty() {
        std::path::Path::new(target)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| target.to_string())
    } else {
        name.to_string()
    };
    Some(Slot { name, target: target.to_string() })
}

/// The white slice: where it is and where it is heading.
struct Hl {
    a: Sp,
    r0: Sp,
    r1: Sp,
    ta: f32,
    tr0: f32,
    tr1: f32,
    alpha: f32,
}

impl Hl {
    fn moving(&self) -> bool {
        !self.a.settled(self.ta, 0.001) || !self.r0.settled(self.tr0, 0.05) || !self.r1.settled(self.tr1, 0.05)
    }
}

pub struct Radial {
    hwnd: HWND,
    host: Host,
    res: Res,
    cfg: RadialCfg,
    search_url: String,
    ui_scale: f32,
    open: bool,
    visible: bool,
    scale: f32,
    win_px: (i32, i32, i32, i32),
    /// 0 = gone, 1 = fully open.
    anim: f32,
    hover: Option<usize>,
    hl: Hl,
    timer_on: bool,
    tracking: bool,
    last_tick: Instant,
    /// Since when the pointer has been over the current slice.
    dwell_since: Instant,
    /// The picture of each slot's app (where it has one).
    icons: Vec<Option<ImageData>>,
    icons_loaded: bool,
    /// The blurred screen behind the menu.
    blur_img: Option<ImageData>,
    /// Only the inner ring has apps: the menu is one big ring.
    single: bool,
    /// The apps, by slot.
    slots: Vec<Option<Slot>>,
}

impl Radial {
    pub fn new(gfx: &Gfx, compositor: Compositor) -> windows::core::Result<Self> {
        let hwnd = create_window()?;
        let host = Host::new_shared(hwnd, gfx, 64, 64, compositor)?;
        let res = Res::new(gfx)?;
        Ok(Self {
            hwnd,
            host,
            res,
            cfg: RadialCfg::default(),
            search_url: String::new(),
            ui_scale: 1.0,
            open: false,
            visible: false,
            scale: 1.0,
            win_px: (0, 0, 0, 0),
            anim: 0.0,
            hover: None,
            hl: Hl {
                a: Sp::at(0.0),
                r0: Sp::at(IN0),
                r1: Sp::at(IN1),
                ta: 0.0,
                tr0: IN0,
                tr1: IN1,
                alpha: 0.0,
            },
            timer_on: false,
            tracking: false,
            last_tick: Instant::now(),
            dwell_since: Instant::now(),
            icons: Vec::new(),
            icons_loaded: false,
            blur_img: None,
            single: false,
            slots: Vec::new(),
        })
    }

    pub fn configure(&mut self, cfg: RadialCfg, search_url: String, ui_scale: f32) {
        if self.slots.len() != SLOTS || cfg.apps != self.cfg.apps {
            self.slots = (0..SLOTS).map(|i| cfg.apps.get(i).and_then(|s| make_slot(s))).collect();
            self.icons_loaded = false;
        }
        self.single = self.slots.iter().skip(8).all(|s| s.is_none());
        self.cfg = cfg;
        self.search_url = search_url;
        self.ui_scale = ui_scale;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn recover(&mut self, gfx: &Gfx) {
        let _ = self.host.set_rendering_device(gfx);
        if let Ok(r) = Res::new(gfx) {
            self.res = r;
        }
        // the pictures are looked up again (they belong to the old drawing memory)
        self.icons_loaded = false;
        self.draw(gfx);
    }

    pub fn toggle(&mut self, gfx: &Gfx) {
        if self.open {
            self.hide();
        } else {
            self.show(gfx);
        }
    }

    fn filled(&self, i: usize) -> bool {
        self.slots.get(i).is_some_and(|s| s.is_some())
    }

    /// The pictures of the apps: the dock's own pictures (its dock-icons folder first), looked up again only
    /// when the list of apps has changed.
    fn load_icons(&mut self) {
        if self.icons_loaded {
            return;
        }
        for old in self.icons.drain(..).flatten() {
            self.res.forget(old.id);
        }
        for s in &self.slots {
            self.icons.push(s.as_ref().and_then(|s| icon_of(&s.name, &s.target)));
        }
        self.icons_loaded = true;
    }

    fn show(&mut self, gfx: &Gfx) {
        self.load_icons();
        if !self.place() {
            return;
        }
        let again = self.visible;
        self.hover = None;
        self.hl.alpha = 0.0;
        self.open = true;
        self.visible = true;
        if !again {
            self.anim = 0.0;
            // the screen behind the disc, copied before the window shows (so it does not photograph itself)
            if let Some(old) = self.blur_img.take() {
                self.res.forget(old.id);
            }
            let disc = if self.single { SINGLE1 + 2.0 } else { OUT1 };
            let d = (disc * 2.0 * self.scale).ceil() as i32;
            let (x, y, side, _) = self.win_px;
            self.blur_img = crate::launcher::capture_blur(x + (side - d) / 2, y + (side - d) / 2, d, d, self.cfg.blur);
        }
        unsafe {
            if !again {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            if !SetForegroundWindow(self.hwnd).as_bool() || GetForegroundWindow() != self.hwnd {
                // Windows only lets the window in front take the focus: a harmless key tap unlocks it
                let inputs = [key_input(VK_MENU, false), key_input(VK_MENU, true)];
                SendInput(&inputs, size_of::<INPUT>() as i32);
                let _ = SetForegroundWindow(self.hwnd);
            }
            let _ = SetFocus(Some(self.hwnd));
        }
        self.draw(gfx);
        self.kick();
        // the pointer may already be over a slice
        self.hover_at_cursor();
    }

    /// Start closing: it shrinks away, then `finish_hide` puts the window away.
    pub fn hide(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.kick();
    }

    fn finish_hide(&mut self) {
        self.visible = false;
        if let Some(old) = self.blur_img.take() {
            self.res.forget(old.id);
        }
        self.hover = None;
        self.hl.alpha = 0.0;
        self.tracking = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_ANIM);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.timer_on = false;
    }

    fn kick(&mut self) {
        if self.timer_on {
            return;
        }
        self.timer_on = true;
        self.last_tick = Instant::now();
        unsafe {
            SetTimer(Some(self.hwnd), T_ANIM, 15, None);
        }
    }

    fn needs_anim(&self) -> bool {
        let open_to = if self.open { 1.0 } else { 0.0 };
        let alpha_to = if self.hover.is_some() { 1.0 } else { 0.0 };
        self.anim != open_to || self.hl.moving() || self.hl.alpha != alpha_to || self.prog() < 1.0 && self.hover.is_some() && self.open
    }

    /// How far the hold on the slice under the pointer has come (0 to 1; at 1 the app opens).
    fn prog(&self) -> f32 {
        if self.open && self.hover.is_some() {
            (self.dwell_since.elapsed().as_secs_f32() / DWELL).min(1.0)
        } else {
            0.0
        }
    }

    fn tick(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        let open_to = if self.open { 1.0 } else { 0.0 };
        let dur = if self.open { 0.28 } else { 0.14 };
        self.anim = toward(self.anim, open_to, dt / dur);
        let h = &mut self.hl;
        h.a.step(h.ta, dt, SPRING_K, SPRING_C);
        h.r0.step(h.tr0, dt, SPRING_K, SPRING_C);
        h.r1.step(h.tr1, dt, SPRING_K, SPRING_C);
        let alpha_to = if self.hover.is_some() { 1.0 } else { 0.0 };
        h.alpha = toward(h.alpha, alpha_to, dt / 0.12);
        // held long enough on one slice: open it
        if let (true, Some(i)) = (self.open && self.prog() >= 1.0, self.hover) {
            if let Some(Some(s)) = self.slots.get(i) {
                let target = s.target.clone();
                self.hide();
                crate::hotkeys::open_target(&target, &self.search_url);
            }
        }
        self.needs_anim()
    }

    /// The window goes around the pointer (moved in if it would stick out of the screen).
    fn place(&mut self) -> bool {
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return false;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let scale = dx as f32 / 96.0 * self.ui_scale;
            let side = (SIDE * scale).ceil() as i32;
            let r = mi.rcMonitor;
            let x = (pt.x - side / 2).clamp(r.left, (r.right - side).max(r.left));
            let y = (pt.y - side / 2).clamp(r.top, (r.bottom - side).max(r.top));
            let resized = (side, side) != (self.win_px.2, self.win_px.3) || (scale - self.scale).abs() > 0.001;
            self.scale = scale;
            self.win_px = (x, y, side, side);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, side, side, SWP_NOACTIVATE);
            if resized {
                let _ = self.host.resize(side, side);
            }
            // round, so the corners of the window never catch a click
            let rgn = CreateEllipticRgn(0, 0, side + 1, side + 1);
            SetWindowRgn(self.hwnd, Some(rgn), false);
        }
        true
    }

    /// Dips from the middle of the menu for a client position in pixels.
    fn rel(&self, x: i32, y: i32) -> (f32, f32) {
        let s = self.scale.max(0.1);
        (x as f32 / s - SIDE * 0.5, y as f32 / s - SIDE * 0.5)
    }

    fn hover_at_cursor(&mut self) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let (x, y, _, _) = self.win_px;
        let (dx, dy) = self.rel(pt.x - x, pt.y - y);
        self.set_hover(hit_in(dx, dy, self.single).filter(|&i| self.filled(i)));
    }

    fn set_hover(&mut self, h: Option<usize>) {
        if h == self.hover {
            return;
        }
        self.hover = h;
        self.dwell_since = Instant::now();
        if let Some(i) = h {
            let (r0, r1) = radii(i, self.single);
            let a = nearest_turn(self.hl.a.x, slot_angle(i));
            let hl = &mut self.hl;
            hl.ta = a;
            hl.tr0 = r0;
            hl.tr1 = r1;
            if hl.alpha < 0.02 {
                // nothing is showing yet: it appears in place instead of flying in from where it was
                hl.a = Sp::at(a);
                hl.r0 = Sp::at(r0);
                hl.r1 = Sp::at(r1);
            }
        }
        self.kick();
    }

    pub fn handle(&mut self, gfx: &Gfx, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_ACTIVATE => {
                // it is a pop-up: clicking anywhere else puts it away
                if (wp.0 & 0xFFFF) == 0 && self.open {
                    self.hide();
                }
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_KEYDOWN | WM_SYSKEYDOWN if self.open => {
                if wp.0 as u16 == VK_ESCAPE.0 {
                    self.hide();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE if self.open => {
                if !self.tracking {
                    self.tracking = true;
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                let (x, y) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
                let (dx, dy) = self.rel(x, y);
                self.set_hover(hit_in(dx, dy, self.single).filter(|&i| self.filled(i)));
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.set_hover(None);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP if self.open => {
                let (x, y) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
                let (dx, dy) = self.rel(x, y);
                match hit_in(dx, dy, self.single) {
                    Some(i) => {
                        if let Some(Some(s)) = self.slots.get(i) {
                            let target = s.target.clone();
                            self.hide();
                            crate::hotkeys::open_target(&target, &self.search_url);
                        }
                    }
                    // the empty middle or the edge: put it away
                    None => self.hide(),
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP if self.open => {
                self.hide();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_ANIM => {
                let more = self.tick();
                if !more && !self.open {
                    self.finish_hide();
                    return Some(LRESULT(0));
                }
                self.draw(gfx);
                if !more {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_ANIM);
                    }
                    self.timer_on = false;
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    // -- drawing -----------------------------------------------------------------------------------------

    fn draw(&mut self, gfx: &Gfx) {
        if !self.visible || self.win_px.2 == 0 {
            return;
        }
        let (dc, _) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(err) => {
                crate::log!("radial: begin_draw failed: {err}");
                return;
            }
        };
        let c = SIDE * 0.5;
        let t = self.anim.clamp(0.0, 1.0);
        // opening: grows from 40% with a little overshoot; closing: shrinks to 75% while it fades
        let pop = if self.open { 0.4 + 0.6 * ease_back(t) } else { 0.75 + 0.25 * ease_out(t) };
        let fade = ease_out((t * 1.8).min(1.0));
        let disc = if self.single { SINGLE1 + 2.0 } else { OUT1 };
        // the slices and pictures appear a moment after the disc
        let content = ease_out(((t - 0.3) / 0.5).clamp(0.0, 1.0));
        let pl = pal(self.cfg.tuna, self.cfg.glass);
        let prog = self.prog();
        let single = self.single;
        self.load_icons();
        let hover_label = self.hover.and_then(|i| self.slots.get(i)).and_then(|s| s.as_ref()).map(|s| s.name.clone());
        {
            let scale = self.scale;
            let Self { res, slots, icons, hl, blur_img, .. } = &mut *self;
            let mut p = Painter::new(dc, gfx, res, scale, 0.0, 0.0);
            p.clear();
            p.set_local(pop, c, c, 0.0, 0.0);
            p.alpha = fade;

            // one disc of glass with a thin rim, and the round hollow in the middle
            // the frosted glass: the screen behind, copied and blurred when the menu opened
            if let Some(img) = blur_img.as_ref() {
                p.image_disc(img, c, c, disc, 0.0);
            }
            p.fill_circle(c, c, disc, pl.tint.alpha(content.max(0.5)));

            // the slice that follows the pointer
            if hl.alpha > 0.01 {
                let r0 = hl.r0.x.max(R_C + 1.0);
                let r1 = hl.r1.x.max(r0 + 6.0);
                let k = (r1 - r0) * 0.35;
                let sw = (HW * r0 * 0.9).min(k);
                if let Some(g) = p.path(|b| sector(b, c, c, r0, r1, hl.a.x, HW, sw.min(10.0))) {
                    // it gets stronger while the pointer is held there
                    p.fill_geometry(&g, pl.hl.with_a((pl.hl.a * (1.0 + 1.2 * prog)).min(1.0) * hl.alpha));
                }
            }

            // pictures and names: the picture and its name sit together in the middle of the slice
            p.alpha = fade * content;
            for (i, slot) in slots.iter().enumerate() {
                let Some(slot) = slot else { continue };
                let (r0, r1) = radii(i, single);
                let (mx, my) = polar(c, c, (r0 + r1) * 0.5, slot_angle(i));
                let ink = pl.ink;
                let size = if single {
                    66.0
                } else if i < 8 {
                    50.0
                } else {
                    58.0
                };
                let top = my - size * 0.5;
                let ir = Rect::new(mx - size * 0.5, top, size, size);
                match icons.get(i).and_then(|o| o.as_ref()) {
                    Some(img) => p.image(img, ir, 11.0, 1.0),
                    None if slot.target.to_ascii_lowercase().starts_with("ms-settings") => p.icon(Icon::Settings, ir, ink),
                    None => {
                        let ch = slot.name.chars().next().unwrap_or('?').to_uppercase().next().unwrap_or('?');
                        p.glyph(ch, ir, size * 0.8, ink);
                    }
                }
            }

            // the name of the slice under the pointer, in the middle
            if let Some(name) = hover_label {
                p.alpha = fade * content * hl.alpha;
                let label = crate::util::truncate_chars(&name, 16);
                let st = TextStyle::new(12.0, Color::white(0.92)).medium().center();
                let (tw, _) = p.measure(&label, &st);
                let pill = Rect::centered(c, c, (tw + 26.0).min(R_C * 2.0 - 6.0), 26.0);
                p.fill_rounded(pill, 13.0, pl.pill);
                p.text(&label, pill, &st);
            }
            p.reset_local();
        }
        if let Err(err) = self.host.end_draw() {
            crate::log!("radial: end_draw failed: {err}");
        }
        self.res.end_frame();
        if self.needs_anim() {
            self.kick();
        }
    }
}

impl Drop for Radial {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_ANIM);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// One slice of a ring, with rounded corners: from radius `r0` to `r1`, centred on angle `a`, half
/// of its angle `hw`, corners rounded by `c`.
#[allow(clippy::too_many_arguments)]
fn sector(b: &crate::gfx::PathBuilder, cx: f32, cy: f32, r0: f32, r1: f32, a: f32, hw: f32, c: f32) {
    let c = c.min((r1 - r0) * 0.4).min(r0 * hw * 0.9).max(0.5);
    let (a0, a1) = (a - hw, a + hw);
    let (d1, d0) = (c / r1, c / r0);
    let pt = |r: f32, an: f32| polar(cx, cy, r, an);
    let start = pt(r1, a0 + d1);
    b.move_to(start.0, start.1);
    let p = pt(r1, a1 - d1);
    b.arc_to(p.0, p.1, r1, true);
    let (k, e) = (pt(r1, a1), pt(r1 - c, a1));
    b.quad_to(k.0, k.1, e.0, e.1);
    let p = pt(r0 + c, a1);
    b.line_to(p.0, p.1);
    let (k, e) = (pt(r0, a1), pt(r0, a1 - d0));
    b.quad_to(k.0, k.1, e.0, e.1);
    let p = pt(r0, a0 + d0);
    b.arc_to(p.0, p.1, r0, false);
    let (k, e) = (pt(r0, a0), pt(r0 + c, a0));
    b.quad_to(k.0, k.1, e.0, e.1);
    let p = pt(r1 - c, a0);
    b.line_to(p.0, p.1);
    b.quad_to(pt(r1, a0).0, pt(r1, a0).1, start.0, start.1);
    b.close();
}

/// The picture of an app, the way the dock finds it: a .png in the dock-icons folder (named after the app or
/// its file) first, else the picture Windows has for the exe, shortcut or Store app.
fn icon_of(name: &str, target: &str) -> Option<ImageData> {
    let t = crate::dock::expand_env(target);
    let low = t.to_ascii_lowercase();
    let web = low.starts_with("http") || low == "browser" || low == "default browser";
    let full = if web || t.contains(['\\', '/', ':']) { t } else { find_exe(&t).unwrap_or(t) };
    crate::dock::icon_for(name, &full)
}

/// `notepad` or `calc`: the file in the system folders or on the PATH.
fn find_exe(name: &str) -> Option<String> {
    let file = if name.to_ascii_lowercase().ends_with(".exe") { name.to_string() } else { format!("{name}.exe") };
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(root) = std::env::var("SystemRoot") {
        dirs.push(std::path::Path::new(&root).join("System32"));
        dirs.push(std::path::PathBuf::from(&root));
        dirs.push(std::path::Path::new(&root).join(r"System32\WindowsPowerShell\v1.0"));
    }
    if let Ok(path) = std::env::var("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

fn create_window() -> windows::core::Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(radial_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS,
            w!("Dynamic Notch Radial"),
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
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_EXCLUDED_FROM_PEEK, &on as *const _ as _, 4);
        // lets the window blur what is behind it (the frosted glass)
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as _, 4);
        let none: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        Ok(hwnd)
    }
}

extern "system" fn radial_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| crate::window::with_app(|a| a.radial_message(msg, wp, lp)).flatten());
    match r {
        Ok(Some(v)) => v,
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
