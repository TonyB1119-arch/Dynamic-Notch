//! The notch shell: owns the window, renderer, modules and the state
//! machine (idle → peek → expanded), turns state into spring targets and
//! draws a frame whenever something moves.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::Graphics::Gdi::{
    CreateRectRgn, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, SetWindowRgn, HDC,
    HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::System::Power::RegisterPowerSettingNotification;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, NIN_SELECT, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
const WM_MOUSELEAVE: u32 = 0x02A3;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{lerp, smoothstep, Spring};
use crate::bus::{Bus, Msg};
use crate::config::{self, Config, LoadResult};
use crate::dock::{self, Dock};
use crate::gfx::{self, icons, Gfx, Icon, Painter, Res, Snapshot, TextStyle};
use crate::host::Host;
use crate::hotkeys::{self, Action};
use crate::modules::{
    self, Activity, CardSize, Cx, Effects, Indicator, Key, Module, ModuleId, Peek, Slot, SystemEvent, Trailing,
};
use crate::pacer::Pacer;
use crate::sys::dragdrop::{self, DropEvent};
use crate::tray::{self, MenuState, Tray};
use crate::ui::{gkey, ButtonStyle, Input, Ui};
use crate::util::{palette, Color, Rect, SendHwnd};
use crate::window::{WM_APP_BUS, WM_APP_DRAG, WM_APP_FOREGROUND, WM_APP_FRAME, WM_APP_TRAY};
use crate::{ipc, log, sys};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
    RID_INPUT, RIM_TYPEKEYBOARD,
};

const HEADER_H: f32 = 54.0;

/// Height of the header (and the tab strip under it in the Bloom look).
fn top_h() -> f32 {
    if crate::ui::look_alcove() {
        crate::alcove::TOP
    } else if crate::ui::look_bloom() {
        crate::bloom::TOP_H
    } else {
        HEADER_H
    }
}

/// Space left and right of the expanded content, and below it.
fn side_pad() -> f32 {
    if crate::ui::look_alcove() {
        crate::alcove::LEFT
    } else if crate::ui::look_bloom() {
        crate::bloom::SIDE
    } else {
        PAD
    }
}

/// Space right of the content (Alcove leaves room for its page dots).
fn right_pad() -> f32 {
    if crate::ui::look_alcove() {
        crate::alcove::RIGHT
    } else {
        side_pad()
    }
}

fn bottom_pad() -> f32 {
    if crate::ui::look_alcove() {
        crate::alcove::BOTTOM
    } else if crate::ui::look_bloom() {
        crate::bloom::BOTTOM
    } else {
        BOTTOM_PAD
    }
}
const PAD: f32 = 28.0;
const BOTTOM_PAD: f32 = 28.0;
const MARGIN: f32 = 48.0;
const WIN_H: f32 = 600.0;
const HOME: ModuleId = "home";
const TIMER_WAKE: usize = 1;
const HOME_HEIGHT: f32 = 176.0;
/// With four tiles in the right column the home page grows a little.
const HOME_HEIGHT_TALL: f32 = 200.0;
const HOME_TILES: usize = 4;

thread_local! {
    /// When the wheel last flipped a tab (a touchpad would otherwise race through all of them).
    static WHEEL_FLIP: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}
/// Height of the clock strip under the media player on the home page.
const CLOCK_STRIP_H: f32 = 52.0;
/// Quiet time before GPU caches are released.
/// How long the notch must be idle before GPU scratch memory is handed back.
/// Trimming right after every burst made the next open re-allocate everything.
const TRIM_DELAY: Duration = Duration::from_secs(10);
/// Minimum time between purely ambient frames (see `frame`).
const AMBIENT_GAP: Duration = Duration::from_millis(12);

static NOTCH_HWND: AtomicIsize = AtomicIsize::new(0);
/// A peek would be visible right now (not hidden, not expanded).
static PEEKS_SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether a peek pushed now would actually be seen. Changes are also
/// broadcast as `SystemEvent::PeeksShown`.
pub fn peeks_shown() -> bool {
    PEEKS_SHOWN.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Idle,
    Peek,
    Expanded,
}

struct ActivePeek {
    peek: Peek,
    until: Instant,
    /// The content that was showing before a quiet follow-up swapped it, and when that happened.
    swap: Option<(Instant, Box<Peek>)>,
}

/// A small round bubble that pops out beside the notch, stays a moment and is pulled back in.
#[derive(Clone, Copy)]
struct Bubble {
    at: Instant,
    icon: Icon,
    color: Color,
    /// Drawn with a slash through the icon (caps lock off, microphone muted).
    off: bool,
    /// Page opened when the bubble is clicked.
    page: Option<ModuleId>,
    in_played: bool,
    out_played: bool,
}

/// How many bubbles can sit side by side.
const MAX_BUBBLES: usize = 3;
/// Seconds a bubble takes to pop out, when it starts to leave, and when it is gone.
const BUBBLE_OUT: f32 = 0.3;
const BUBBLE_HOLD: f32 = 1.1;
const BUBBLE_END: f32 = 1.5;

/// Add a bubble starting at `at`; the oldest goes when there are too many.
fn push_bubble(list: &mut Vec<Bubble>, at: Instant, icon: Icon, color: Color, off: bool, page: Option<ModuleId>) {
    list.push(Bubble { at, icon, color, off, page, in_played: false, out_played: false });
    if list.len() > MAX_BUBBLES {
        list.remove(0);
    }
}

/// How far out (0..1+), how visible, how big and how much room a bubble `t` seconds old takes.
fn bubble_curve(t: f32) -> Option<(f32, f32, f32, f32)> {
    if t >= BUBBLE_END {
        return None;
    }
    Some(if t < BUBBLE_OUT {
        let u = t / BUBBLE_OUT;
        // ease-out with a touch of overshoot
        let c = 1.5;
        let e = 1.0 + (c + 1.0) * (u - 1.0).powi(3) + c * (u - 1.0).powi(2);
        (e, u.min(1.0), 0.6 + 0.4 * u, 1.0)
    } else if t < BUBBLE_HOLD {
        (1.0, 1.0, 1.0, 1.0)
    } else {
        let u = (t - BUBBLE_HOLD) / (BUBBLE_END - BUBBLE_HOLD);
        let e = u * u * u;
        (1.0 - e, 1.0 - u * u, 1.0 - 0.55 * e, 1.0 - e)
    })
}

/// Peeks that Alcove shows as one small bubble instead: caps lock and the microphone switch.
fn small_bubble(p: &Peek) -> bool {
    match p.key {
        Some("osd-mic") => true,
        Some("locks") => matches!(&p.trailing, Trailing::Level { text: Some(t), .. } if t.starts_with("Caps")),
        _ => false,
    }
}

/// Whether such a bubble shows the "off" state (a slash through the icon).
fn bubble_off(p: &Peek) -> bool {
    match p.key {
        Some("osd-mic") => p.subtitle == "Muted",
        Some("locks") => matches!(&p.trailing, Trailing::Level { frac, .. } if *frac < 0.5),
        _ => false,
    }
}

/// Alcove shows some notices as one big round dial. This is how much taller than the notch the
/// dial is for this notice, or `None` when it is shown the usual way. Volume and brightness can
/// be switched back to the line (Settings, General); battery and Bluetooth are always dials.
fn dial_height(p: &Peek, cfg: &Config) -> Option<f32> {
    if !crate::ui::look_alcove() {
        return None;
    }
    match p.key {
        Some("osd-volume" | "osd-brightness") if cfg.general.hud_ring => Some(172.0),
        // these two have a status line under the value
        Some("battery" | "bluetooth") => Some(196.0),
        _ => None,
    }
}

/// How wide each page wants the open notch, as a share of the widest setting (Appearance, width).
/// A short list needs far less room than the dashboard, so the notch only opens as wide as the
/// page it shows.
fn page_share(id: ModuleId) -> f32 {
    match id {
        "notifications" | "timer" => 0.72,
        "downloads" | "clipboard" | "mirror" | "notes" => 0.74,
        "control" | "shelf" => 0.76,
        "media" | "stats" | "weather" => 0.78,
        "calendar" | "ports" | "voice" | "ai" => 0.84,
        "gemini" => 0.86,
        _ => 1.0,
    }
}

/// How wide the open notch is for a page (the widest setting outside the Alcove look).
fn page_width_of(cfg: &Config, id: ModuleId) -> f32 {
    let max = cfg.appearance.expanded_width;
    if !crate::ui::look_alcove() {
        return max;
    }
    (max * page_share(id)).max(420.0_f32.min(max))
}

/// The height a page wants under the header, for the given content width.
fn page_content_h(cfg: &Config, modules: &[Box<dyn Module>], started: &[bool], id: ModuleId, content_w: f32) -> f32 {
    if id == HOME && crate::ui::look_alcove() {
        crate::alcove::HOME_H
    } else if id == HOME && crate::ui::look_bloom() {
        crate::bloom::HOME_H
    } else if id == HOME {
        if home_tiles(modules, started, cfg).len() >= HOME_TILES {
            HOME_HEIGHT_TALL
        } else {
            HOME_HEIGHT
        }
    } else {
        modules
            .iter()
            .position(|m| m.id() == id)
            .map(|i| modules[i].page_height(content_w))
            .unwrap_or(HOME_HEIGHT)
            .clamp(60.0, WIN_H - MARGIN - top_h() - bottom_pad())
    }
}

/// The size of the open notch showing a page: (width, height). Every page is laid out at its own
/// size, also while another page is sliding in or out, so nothing reflows to the wrong width.
fn page_size(cfg: &Config, modules: &[Box<dyn Module>], started: &[bool], id: ModuleId) -> (f32, f32) {
    let w = page_width_of(cfg, id);
    (w, top_h() + page_content_h(cfg, modules, started, id, w - side_pad() - right_pad()) + bottom_pad())
}

/// The area a page draws in, for an open notch of this size centred on `cx`.
fn page_content(cx: f32, y: f32, (w, h): (f32, f32)) -> Rect {
    let full = Rect::new(cx - w * 0.5, y, w, h);
    Rect::new(full.x + side_pad(), full.y + top_h(), full.w - side_pad() - right_pad(), full.h - top_h() - bottom_pad())
}

/// How long a quiet peek swap (next track) takes to play.
const SWAP_SECS: f32 = 0.55;

/// Spring response of the notch body when it grows (seconds); it shrinks a bit faster.
const BODY_RESPONSE: f32 = 0.42;
/// How long the body waits for the content to fade before it shrinks (seconds).
const CLOSE_HOLD: f32 = 0.09;

struct Anim {
    w: Spring,
    h: Spring,
    r: Spring,
    ear: Spring,
    glass: Spring,
    compact: Spring,
    peek: Spring,
    exp: Spring,
    shown: Spring,
    /// Secondary live-activity bubble (0 hidden .. 1 fully out).
    sec: Spring,
    /// Alcove media: cover and level bars sit in their own bubbles beside a blank notch (0..1).
    split: Spring,
    /// Page switch progress 0..1 (slow and soft on purpose).
    page: Spring,
}

impl Anim {
    fn new(w: f32, h: f32) -> Self {
        Self {
            w: Spring::new(w * 0.6, 0.45, 0.85),
            h: Spring::new(0.0, 0.45, 0.85),
            r: Spring::new(10.0, 0.45, 0.9),
            ear: Spring::new(6.0, 0.4, 1.0),
            glass: Spring::unit(0.0, 0.3, 1.0),
            compact: Spring::unit(0.0, 0.18, 1.0),
            peek: Spring::unit(0.0, 0.2, 1.0),
            exp: Spring::unit(0.0, 0.22, 1.0),
            shown: Spring::unit(0.0, 0.5, 0.8),
            sec: Spring::unit(0.0, 0.45, 0.8),
            split: Spring::unit(0.0, 0.5, 0.78),
            page: Spring::unit(1.0, 0.64, 1.0),
        }
        .with_h(h)
    }
    fn with_h(mut self, _h: f32) -> Self {
        self.h.snap(0.0);
        self
    }
    fn step(&mut self, dt: f32) -> bool {
        let mut m = false;
        for s in [
            &mut self.w,
            &mut self.h,
            &mut self.r,
            &mut self.ear,
            &mut self.glass,
            &mut self.compact,
            &mut self.peek,
            &mut self.exp,
            &mut self.shown,
            &mut self.sec,
            &mut self.split,
            &mut self.page,
        ] {
            m |= s.step(dt);
        }
        m
    }
}

/// The few look settings the drawing code reads every frame, copied out so no text is cloned.
#[derive(Clone, Copy)]
struct Look {
    shadow: bool,
    blur: bool,
    expanded_opacity: f32,
    collapsed_opacity: f32,
    notch_height: f32,
}

struct Targets {
    w: f32,
    h: f32,
    r: f32,
    ear: f32,
    glass: f32,
    compact: bool,
    peek: bool,
    exp: bool,
    shown: bool,
    sec: bool,
}

pub struct App {
    hwnd: HWND,
    cfg: Config,
    gfx: Gfx,
    res: Res,
    host: Host,
    /// The open notch drawn once while it fades in or out (see `prepare_snapshots`).
    snap_all: Option<Snapshot>,
    /// The old and the new page, drawn once while the page slide runs.
    snap_pages: Option<(Snapshot, Snapshot)>,
    /// The bottom strip of the open page, for the soft blur along the bottom edge (Alcove).
    snap_band: Option<Snapshot>,
    /// When the blur strip first appeared (it fades in from here).
    band_since: Option<Instant>,
    pacer: Pacer,
    bus: Bus,
    rx: Receiver<Msg>,
    modules: Vec<Box<dyn Module>>,
    started: Vec<bool>,
    /// Tick deadline each module asked for when the wake timer was last armed.
    /// `next_tick()` is relative to "now", so it must be cached to ever come due.
    tick_at: Vec<Option<Instant>>,

    monitor: HMONITOR,
    scale: f32,
    win_w: f32,
    win_h: f32,
    win_px: (i32, i32, i32, i32),

    mode: Mode,
    page: ModuleId,
    /// Page being faded out while the next one slides in, and the slide direction.
    page_from: Option<(ModuleId, f32)>,
    peek: Option<ActivePeek>,
    /// Alcove: a finished clipboard / notification / download peek lingers as a bubble, then is sucked in.
    bubbles: Vec<Bubble>,
    /// A bubble to show once the notch has closed again (files just parked on the shelf).
    pending_bubble: Option<(Icon, Color)>,
    /// When the notch last opened (drives the light sweep across it).
    opened_at: Option<Instant>,
    peek_queue: VecDeque<Peek>,

    hovering: bool,
    hover_since: Option<Instant>,
    leave_since: Option<Instant>,
    /// Opened by click/hotkey/CLI: auto-close if the mouse never comes by.
    auto_close: Option<Instant>,
    tracking: bool,
    input: Input,
    keyboard: bool,
    high_surrogate: Option<u16>,

    anim: Anim,
    last_frame: Instant,
    dirty: bool,
    /// When to release GPU caches (set when a frame burst ends).
    trim_at: Option<Instant>,
    /// No spring is moving: the backdrop blur may be shown.
    settled: bool,
    region: Option<(i32, i32, i32, i32)>,
    region_anim: Option<Rect>,
    fs_hidden: bool,
    dock: Option<Dock>,
    user_hidden: bool,
    shown_once: bool,

    tray: Option<Tray>,
    hotkeys: Vec<hotkeys::Registered>,
    msg_taskbar_created: u32,
    accent: Color,
    hooks: Vec<HWINEVENTHOOK>,
    loc_hook: Option<HWINEVENTHOOK>,
    loc_pid: u32,
    activity: Option<Activity>,
    secondary: Option<Activity>,
    indicators: Vec<Indicator>,
    /// Second shown on the home flip clock when it was last drawn.
    clock_min: u16,
    /// Virtual key of the dictation hotkey (its release ends push-to-talk).
    dictate_vk: u16,
    /// Watched keys currently held (raw input repeats are dropped).
    raw_down: Vec<u16>,
    /// Files are being dragged over the notch (keeps it open).
    drag_hover: bool,
    /// Files to drag out once the window proc is outside the app borrow.
    pending_drag: Option<Vec<String>>,
    /// Frame statistics for the DN_DEBUG log.
    /// Frames in the current burst: (start, spring, animate, dirty).
    stats: (Option<Instant>, u32, u32, u32),
    debug: bool,
}

impl App {
    pub fn new(hwnd: HWND, cfg: Config) -> Result<Box<App>> {
        let gfx = Gfx::new()?;
        let res = Res::new(&gfx)?;
        let host = Host::new(hwnd, &gfx, 64, 64)?;
        let shwnd = SendHwnd::new(hwnd);
        let pacer = Pacer::new(shwnd);
        let (bus, rx) = Bus::new(shwnd);
        let modules = modules::all();
        let n = modules.len();
        NOTCH_HWND.store(hwnd.0 as isize, Ordering::Release);
        crate::ui::set_white_beams(cfg.appearance.white_beams);
        crate::ui::set_look(&cfg.appearance.look);
        let anim = Anim::new(cfg.appearance.notch_width, cfg.appearance.notch_height);
        Ok(Box::new(App {
            hwnd,
            gfx,
            res,
            host,
            snap_all: None,
            snap_pages: None,
            snap_band: None,
            band_since: None,
            pacer,
            bus,
            rx,
            modules,
            started: vec![false; n],
            tick_at: vec![None; n],
            monitor: HMONITOR::default(),
            scale: 1.0,
            win_w: 0.0,
            win_h: 0.0,
            win_px: (0, 0, 0, 0),
            mode: Mode::Idle,
            page: HOME,
            page_from: None,
            peek: None,
            bubbles: Vec::new(),
            pending_bubble: None,
            opened_at: None,
            peek_queue: VecDeque::new(),
            hovering: false,
            hover_since: None,
            leave_since: None,
            auto_close: None,
            tracking: false,
            input: Input::default(),
            keyboard: false,
            high_surrogate: None,
            anim,
            last_frame: Instant::now(),
            dirty: true,
            trim_at: None,
            settled: true,
            region: None,
            region_anim: None,
            fs_hidden: false,
            dock: None,
            user_hidden: false,
            shown_once: false,
            tray: None,
            hotkeys: Vec::new(),
            msg_taskbar_created: unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) },
            accent: Color::white(0.92),
            hooks: Vec::new(),
            loc_hook: None,
            loc_pid: 0,
            activity: None,
            secondary: None,
            indicators: Vec::new(),
            clock_min: 99,
            dictate_vk: 0,
            raw_down: Vec::new(),
            drag_hover: false,
            pending_drag: None,
            stats: (None, 0, 0, 0),
            debug: std::env::var_os("DN_DEBUG").is_some(),
            cfg,
        }))
    }

    /// Second-stage init once the app is reachable from the window proc.
    pub fn start(&mut self) {
        self.place();
        self.sync_dock();
        self.tray = Some(Tray::new(self.hwnd));
        self.register_hotkeys();
        unsafe {
            let _ = AddClipboardFormatListener(self.hwnd);
            // tells us when the PC is locked / unlocked (the greeting)
            let _ = WTSRegisterSessionNotification(self.hwnd, NOTIFY_FOR_THIS_SESSION);
            let h = HANDLE(self.hwnd.0);
            let _ = RegisterPowerSettingNotification(h, &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);
            let _ =
                RegisterPowerSettingNotification(h, &GUID_BATTERY_PERCENTAGE_REMAINING, DEVICE_NOTIFY_WINDOW_HANDLE);
            let fg = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event_cb),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if !fg.is_invalid() {
                self.hooks.push(fg);
            }
            // Keyboard raw input in the background: lock-key toggles and the
            // release of the dictation hotkey. Nothing is blocked or recorded.
            let rid =
                RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x06, dwFlags: RIDEV_INPUTSINK, hwndTarget: self.hwnd };
            if let Err(e) = RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
                log!("raw input unavailable: {e}");
            }
        }
        dragdrop::register_target(self.hwnd, |ev| crate::window::with_app(|a| a.on_drop_event(ev)).unwrap_or(false));
        sys::watch_config(self.bus.clone());
        if self.cfg.general.start_with_windows != sys::is_autostart() {
            sys::set_autostart(self.cfg.general.start_with_windows);
        }
        self.sync_modules();
        self.on_foreground_changed();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        self.request_frame();
        log!("started: scale={} monitor={:?}", self.scale, self.win_px);
    }

    /// Create, update or remove the dock to match the settings.
    fn sync_dock(&mut self) {
        let cfg = self.cfg.dock.clone();
        let scale = self.cfg.appearance.scale;
        if !cfg.enabled {
            self.dock = None;
            return;
        }
        match self.dock.as_mut() {
            Some(d) => d.set_config(&cfg, scale),
            None => {
                let created = crate::window::create_dock()
                    .ok()
                    .and_then(|h| Dock::new(h, &self.gfx, &self.host.compositor, &cfg, scale));
                if created.is_none() {
                    log!("could not start the dock");
                }
                self.dock = created;
            }
        }
    }

    /// Messages for the dock's window (see `window::wndproc_inner`).
    pub fn dock_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let d = self.dock.as_mut()?;
        if msg == WM_TIMER && wp.0 == dock::TIMER_FRAME {
            if !d.frame(&self.gfx, Instant::now()) {
                d.stop_timer();
            }
            return Some(LRESULT(0));
        }
        let (r, frame) = d.handle(msg, wp, lp);
        if frame {
            d.start_timer();
        }
        r
    }

    pub fn shutdown(&mut self) {
        for (i, m) in self.modules.iter_mut().enumerate() {
            if self.started[i] {
                m.stop();
                self.started[i] = false;
            }
        }
        for h in self.hooks.drain(..) {
            unsafe {
                let _ = UnhookWinEvent(h);
            }
        }
        if let Some(h) = self.loc_hook.take() {
            unsafe {
                let _ = UnhookWinEvent(h);
            }
        }
        hotkeys::unregister_all(self.hwnd);
        unsafe {
            let _ = WTSUnRegisterSessionNotification(self.hwnd);
        }
        dragdrop::revoke_target(self.hwnd);
        self.dock = None;
        self.tray = None;
        self.pacer.stop();
    }

    // -----------------------------------------------------------------------
    // Message handling
    // -----------------------------------------------------------------------

    pub fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_APP_FRAME => {
                self.pacer.frame_consumed();
                self.frame();
                Some(LRESULT(0))
            }
            WM_APP_BUS => {
                self.drain_bus();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_WAKE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_WAKE);
                }
                self.on_wake();
                Some(LRESULT(0))
            }
            // WM_WTSSESSION_CHANGE: wParam 8 = WTS_SESSION_UNLOCK
            0x02B1 => {
                if wp.0 == 8 {
                    self.broadcast(SystemEvent::Command { verb: "unlocked".into(), args: vec![] });
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (x, y) = self.lp_to_logical(lp);
                self.on_mouse_move(x, y);
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.set_hover(false);
                self.input.mouse = None;
                self.input.down = None;
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let p = self.lp_to_logical(lp);
                self.input.down = Some(p);
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let p = self.lp_to_logical(lp);
                self.on_click(p);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16;
                self.input.wheel += delta as f32 / 120.0;
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_MOUSEHWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16;
                self.input.hwheel += delta as f32 / 120.0;
                self.request_frame();
                Some(LRESULT(0))
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_HOTKEY => {
                let id = wp.0 as i32;
                if let Some(h) = self.hotkeys.iter().find(|h| h.id == id) {
                    let a = h.action;
                    self.on_hotkey(a);
                } else {
                    self.broadcast(SystemEvent::Hotkey(id));
                }
                Some(LRESULT(0))
            }
            WM_APP_TRAY => {
                let ev = (lp.0 as u32) & 0xFFFF;
                if ev == NIN_SELECT || ev == NIN_KEYSELECT || ev == WM_LBUTTONUP {
                    self.toggle();
                }
                Some(LRESULT(0))
            }
            WM_APP_FOREGROUND => {
                self.on_foreground_changed();
                Some(LRESULT(0))
            }
            WM_INPUT => {
                self.on_raw_input(lp);
                None // DefWindowProc must still see WM_INPUT
            }
            WM_CLIPBOARDUPDATE => {
                self.broadcast(SystemEvent::ClipboardChanged);
                Some(LRESULT(0))
            }
            WM_POWERBROADCAST => {
                self.broadcast(SystemEvent::PowerChanged);
                Some(LRESULT(1))
            }
            0x0219 => {
                // WM_DEVICECHANGE: a drive letter appeared or went away
                if let Some(args) = usb_event(wp.0 as u32, lp.0) {
                    self.broadcast(SystemEvent::Command { verb: "usb".into(), args });
                }
                Some(LRESULT(1))
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                self.place();
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE => {
                if wp.0 as u32 == SPI_SETWORKAREA.0 {
                    self.place();
                }
                if let Some(t) = self.tray.as_mut() {
                    t.refresh_icon();
                }
                None
            }
            WM_COPYDATA => {
                if let Some(args) = ipc::decode(lp) {
                    self.on_ipc(args);
                }
                Some(LRESULT(1))
            }
            WM_ACTIVATE => {
                let state = (wp.0 & 0xFFFF) as u32;
                if state == WA_INACTIVE && self.keyboard {
                    self.set_keyboard(false);
                    if self.mode == Mode::Expanded && !self.hovering {
                        self.collapse();
                    }
                }
                None
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if self.keyboard {
                    if let Some(k) = translate_key(wp.0 as u32) {
                        self.on_key(k);
                        return Some(LRESULT(0));
                    }
                }
                None
            }
            WM_CHAR => {
                if self.keyboard {
                    let u = wp.0 as u16;
                    let ch = if (0xD800..0xDC00).contains(&u) {
                        self.high_surrogate = Some(u);
                        None
                    } else if (0xDC00..0xE000).contains(&u) {
                        self.high_surrogate.take().and_then(|hi| char::decode_utf16([hi, u]).next()?.ok())
                    } else {
                        char::from_u32(u as u32)
                    };
                    if let Some(c) = ch {
                        if !c.is_control() {
                            self.on_key(Key::Char(c));
                        }
                    }
                    return Some(LRESULT(0));
                }
                None
            }
            WM_CLOSE => {
                self.quit();
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                self.shutdown();
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ if msg == self.msg_taskbar_created && msg != 0 => {
                if let Some(t) = self.tray.as_mut() {
                    t.readd();
                }
                if let Some(d) = self.dock.as_mut() {
                    d.explorer_restarted();
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn on_raw_input(&mut self, lp: LPARAM) {
        let mut ri = RAWINPUT::default();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let n = unsafe {
            GetRawInputData(
                HRAWINPUT(lp.0 as *mut _),
                RID_INPUT,
                Some(&mut ri as *mut _ as *mut _),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if n == u32::MAX || ri.header.dwType != RIM_TYPEKEYBOARD.0 {
            return;
        }
        let kb = unsafe { ri.data.keyboard };
        let vk = kb.VKey;
        let watched = [VK_CAPITAL.0, VK_NUMLOCK.0, VK_SCROLL.0].contains(&vk) || (vk != 0 && vk == self.dictate_vk);
        if !watched {
            return;
        }
        let down = kb.Flags as u32 & RI_KEY_BREAK == 0;
        if down {
            if self.raw_down.contains(&vk) {
                return; // auto-repeat
            }
            self.raw_down.push(vk);
        } else {
            // A registered hotkey's key-down never reaches raw input, its key-up does.
            self.raw_down.retain(|&k| k != vk);
        }
        self.broadcast(SystemEvent::RawKey { vk, down });
    }

    /// OLE drop target callbacks (files dragged onto the notch).
    pub fn on_drop_event(&mut self, ev: DropEvent) -> bool {
        if !self.module_index("shelf").is_some_and(|i| self.started[i]) {
            return false;
        }
        match ev {
            DropEvent::Enter => {
                self.drag_hover = true;
                self.user_hidden = false;
                if self.mode != Mode::Expanded || self.page != "shelf" {
                    self.expand(Some("shelf"));
                }
                self.broadcast(SystemEvent::DragHover(true));
            }
            DropEvent::Leave => {
                self.drag_hover = false;
                self.leave_since = Some(Instant::now());
                self.broadcast(SystemEvent::DragHover(false));
            }
            DropEvent::Drop(files) => {
                self.drag_hover = false;
                // the cursor is on the notch but sends no moves until it does
                self.leave_since = None;
                self.auto_close = Some(Instant::now() + Duration::from_secs(4));
                self.broadcast(SystemEvent::DragHover(false));
                self.pending_bubble = Some((Icon::Package, palette::YELLOW));
                self.broadcast(SystemEvent::Dropped(files));
            }
        }
        self.request_frame();
        self.schedule_wake();
        true
    }

    /// Files to drag out (see `WM_APP_DRAG` in window.rs). The button-up ends
    /// up in the drag loop, so the press is forgotten here.
    pub fn take_drag(&mut self) -> Option<Vec<String>> {
        self.input.down = None;
        self.pending_drag.take()
    }

    pub fn drag_finished(&mut self) {
        // The drag loop held the mouse capture, so WM_MOUSELEAVE may never
        // have arrived; resync hover from the real cursor position.
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut pt);
        }
        let (x, y) = (pt.x as f32 / self.scale, pt.y as f32 / self.scale);
        if !self.hover_zone().contains(x, y) {
            self.tracking = false;
            self.input.mouse = None;
            self.set_hover(false);
        }
        self.broadcast(SystemEvent::DragOutDone);
        self.request_frame();
    }

    fn lp_to_logical(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        (x / self.scale, y / self.scale)
    }

    fn drain_bus(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Module(id, payload) => {
                    if let Some(i) = self.modules.iter().position(|m| m.id() == id) {
                        if self.started[i] {
                            let mut fx = Effects::default();
                            {
                                let mut cx = Cx {
                                    bus: &self.bus,
                                    cfg: &self.cfg,
                                    fx: &mut fx,
                                    hwnd: SendHwnd::new(self.hwnd),
                                    now: Instant::now(),
                                };
                                self.modules[i].on_message(payload, &mut cx);
                            }
                            fx.redraw = true;
                            self.apply_effects(fx);
                        }
                    }
                }
                Msg::ConfigFileChanged => self.reload_config(),
            }
        }
        self.schedule_wake();
    }

    fn broadcast(&mut self, ev: SystemEvent) {
        let mut fx = Effects::default();
        {
            let now = Instant::now();
            let hwnd = SendHwnd::new(self.hwnd);
            for (i, m) in self.modules.iter_mut().enumerate() {
                if self.started[i] {
                    let mut cx = Cx { bus: &self.bus, cfg: &self.cfg, fx: &mut fx, hwnd, now };
                    m.on_system(&ev, &mut cx);
                }
            }
        }
        self.apply_effects(fx);
        self.schedule_wake();
    }

    fn apply_effects(&mut self, fx: Effects) {
        if fx.nudge && self.cfg.general.haptics && crate::ui::look_alcove() && self.mode == Mode::Expanded {
            // a tiny give, like a key travelling down and back up
            self.anim.w.velocity -= 30.0;
            self.anim.h.velocity -= 10.0;
            self.request_frame();
        }
        for (s, k, v) in &fx.save_config {
            config::set_value(s, k, v);
        }
        for p in fx.peeks {
            self.push_peek(p);
        }
        if let Some(v) = fx.set_volume {
            modules::osd::set_volume(&self.bus, v);
        }
        if let Some(b) = fx.set_brightness {
            modules::osd::set_brightness(b);
        }
        if fx.reload_config {
            self.reload_config();
        }
        if let Some(page) = fx.open_page {
            self.expand(Some(page));
        }
        if fx.expand && self.mode != Mode::Expanded {
            self.expand(None);
        }
        if fx.collapse {
            self.collapse();
        }
        if let Some(k) = fx.keyboard_focus {
            self.set_keyboard(k);
        }
        if let Some(files) = fx.drag_files {
            if !files.is_empty() && self.pending_drag.is_none() {
                self.pending_drag = Some(files);
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_APP_DRAG, WPARAM(0), LPARAM(0));
                }
            }
        }
        if fx.redraw || fx.animate || fx.layout_changed {
            self.request_frame();
        }
    }

    // -----------------------------------------------------------------------
    // Modules
    // -----------------------------------------------------------------------

    fn sync_modules(&mut self) {
        let hwnd = SendHwnd::new(self.hwnd);
        let mut fx = Effects::default();
        for (i, m) in self.modules.iter_mut().enumerate() {
            let want = self.cfg.modules.is_enabled(m.id());
            if want && !self.started[i] {
                let mut cx = Cx { bus: &self.bus, cfg: &self.cfg, fx: &mut fx, hwnd, now: Instant::now() };
                m.start(&mut cx);
                self.started[i] = true;
            } else if !want && self.started[i] {
                m.stop();
                self.started[i] = false;
            }
        }
        if !self.page_available(self.page) {
            self.page = HOME;
        }
        self.apply_effects(fx);
        self.request_frame();
    }

    fn page_available(&self, page: ModuleId) -> bool {
        page == HOME || self.modules.iter().enumerate().any(|(i, m)| self.started[i] && m.id() == page && m.has_page())
    }

    /// Change the open page with a soft slide and cross-fade.
    fn switch_page(&mut self, new: ModuleId) {
        if new == self.page {
            return;
        }
        // later tabs slide in from the right, earlier ones from the left
        // (by the order of the header tabs, which the user can rearrange; settings sits last)
        let order = &self.cfg.tabs.order;
        let pos = |id: ModuleId| -> i32 {
            if id == HOME {
                return -1;
            }
            if id == "settings" {
                return 10_000;
            }
            match order.iter().position(|o| o == id) {
                Some(i) => i as i32,
                None => 1_000 + self.module_index(id).map_or(0, |i| i as i32),
            }
        };
        let dir = if pos(new) >= pos(self.page) { 1.0 } else { -1.0 };
        self.page_from = Some((self.page, dir));
        self.page = new;
        self.anim.page.snap(0.0);
        self.anim.page.set_target(1.0);
        self.request_frame();
    }

    /// Whether the hello is written with an installed font (`[greeting] font`) instead of the pen drawing.
    fn hello_custom(&self) -> bool {
        self.gfx.font_installed(self.cfg.greeting.font_name())
    }

    /// How wide the open notch is for the page shown: the page's own share of the widest setting,
    /// or the full width on the home page.
    fn page_width(&self) -> f32 {
        page_width_of(&self.cfg, self.page)
    }

    fn module_index(&self, id: ModuleId) -> Option<usize> {
        self.modules.iter().position(|m| m.id() == id)
    }

    fn refresh_activity(&mut self) {
        let mut acts: Vec<Activity> = Vec::new();
        self.indicators.clear();
        for (i, m) in self.modules.iter().enumerate() {
            if !self.started[i] {
                continue;
            }
            if let Some(a) = m.activity() {
                acts.push(a);
            }
            m.indicators(&mut self.indicators);
        }
        // stable sort: equal priorities keep module order
        acts.sort_by_key(|a| std::cmp::Reverse(a.priority));
        let mut it = acts.into_iter();
        let before = self.activity.is_some() as u8 + self.secondary.is_some() as u8;
        self.activity = it.next();
        self.secondary = it.next();
        // Alcove: a small give in the notch when a live activity starts or ends
        if crate::ui::look_alcove()
            && self.mode == Mode::Idle
            && before != self.activity.is_some() as u8 + self.secondary.is_some() as u8
        {
            self.anim.w.velocity += 70.0;
            self.anim.h.velocity += 22.0;
        }
        // accent from album art if configured
        self.accent = match Color::parse(&self.cfg.appearance.accent) {
            Some(c) => c,
            None => self
                .module_index("media")
                .filter(|&i| self.started[i])
                .and_then(|i| self.modules[i].activity())
                .and_then(|a| match a.left {
                    Slot::Image(img) => img.accent,
                    _ => None,
                })
                .unwrap_or(if crate::ui::look_nothing() {
                    crate::ui::NOTHING_RED
                } else if crate::ui::look_bloom() {
                    crate::ui::BLOOM_BLUE
                } else {
                    Color::white(0.92)
                }),
        };
    }

    fn on_wake(&mut self) {
        let now = Instant::now();
        // module ticks
        let mut fx = Effects::default();
        {
            let hwnd = SendHwnd::new(self.hwnd);
            for (i, m) in self.modules.iter_mut().enumerate() {
                if self.started[i] && self.tick_at[i].is_some_and(|t| t <= now + Duration::from_millis(2)) {
                    let mut cx = Cx { bus: &self.bus, cfg: &self.cfg, fx: &mut fx, hwnd, now };
                    m.on_tick(&mut cx);
                    fx.redraw = true;
                }
            }
        }
        self.apply_effects(fx);
        self.process_timeouts(now);
        if self.trim_at.is_some_and(|t| t <= now + Duration::from_millis(2)) {
            self.trim_at = None;
            if self.snap_band.is_some() {
                log!("trim while the bottom blur is showing");
            }
            self.res.trim_bitmaps();
            self.host.reset_dc(&self.gfx);
            self.gfx.trim();
            // hand the freed pages back to Windows (Task Manager shows the real footprint)
            unsafe {
                let _ = windows::Win32::System::Memory::SetProcessWorkingSetSizeEx(
                    windows::Win32::System::Threading::GetCurrentProcess(),
                    usize::MAX,
                    usize::MAX,
                    windows::Win32::System::Memory::SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
                );
            }
        }
        if self.mode == Mode::Expanded && self.page == HOME {
            let st = unsafe { GetLocalTime() };
            if st.wSecond != self.clock_min {
                self.request_frame();
            }
        }
        self.schedule_wake();
    }

    /// Arm a single Win32 timer for the earliest pending deadline.
    fn schedule_wake(&mut self) {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        let mut consider = |t: Instant| {
            next = Some(next.map_or(t, |n: Instant| n.min(t)));
        };
        for (i, m) in self.modules.iter().enumerate() {
            self.tick_at[i] = if self.started[i] { m.next_tick() } else { None };
            if let Some(t) = self.tick_at[i] {
                consider(t);
            }
        }
        let g = &self.cfg.general;
        if self.mode != Mode::Expanded && self.hovering && g.expand_on_hover {
            if let Some(h) = self.hover_since {
                consider(h + Duration::from_millis(g.hover_delay_ms as u64));
            }
        }
        if self.mode == Mode::Expanded && !self.hovering && !self.keyboard && !self.drag_hover {
            if let Some(l) = self.leave_since {
                consider(l + Duration::from_millis(g.collapse_delay_ms as u64));
            }
            if let Some(t) = self.auto_close {
                consider(t);
            }
        }
        if let Some(p) = &self.peek {
            consider(p.until);
        }
        if let Some(t) = self.trim_at {
            consider(t);
        }
        if self.mode == Mode::Expanded && self.page == HOME {
            // the flip clock shows seconds: wake at the next full second
            let ms = 1000 - (unsafe { GetLocalTime() }.wMilliseconds as u64).min(999);
            consider(now + Duration::from_millis(ms + 2));
        }
        unsafe {
            match next {
                Some(t) => {
                    let ms = t.saturating_duration_since(now).as_millis().clamp(1, 0x7FFF_FFFF) as u32;
                    SetTimer(Some(self.hwnd), TIMER_WAKE, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_WAKE);
                }
            }
        }
    }

    fn process_timeouts(&mut self, now: Instant) {
        let (expand_on_hover, hover_delay_ms, collapse_delay_ms) = {
            let g = &self.cfg.general;
            (g.expand_on_hover, g.hover_delay_ms, g.collapse_delay_ms)
        };
        if self.mode != Mode::Expanded && self.hovering && expand_on_hover && !self.user_hidden {
            if let Some(h) = self.hover_since {
                if now >= h + Duration::from_millis(hover_delay_ms as u64) {
                    self.hover_since = None;
                    let page = self.peek.as_ref().and_then(|p| p.peek.page);
                    self.expand(page);
                }
            }
        }
        if self.mode == Mode::Expanded && !self.hovering && !self.keyboard && !self.drag_hover {
            let leave = self.leave_since.map(|l| l + Duration::from_millis(collapse_delay_ms as u64));
            if leave.into_iter().chain(self.auto_close).any(|t| now >= t) {
                self.collapse();
            }
        }
        if let Some(p) = &self.peek {
            if now >= p.until && !(self.hovering && self.mode == Mode::Peek) {
                if crate::ui::look_alcove()
                    && self.cfg.general.bubbles
                    && matches!(p.peek.key, Some("notify" | "clipboard" | "download"))
                    && !p.peek.is_hud()
                {
                    push_bubble(&mut self.bubbles, now, p.peek.icon, p.peek.accent, false, p.peek.page);
                }
                self.peek = None;
                self.next_peek();
            }
        }
    }

    // -----------------------------------------------------------------------
    // State transitions
    // -----------------------------------------------------------------------

    fn expand(&mut self, page: Option<ModuleId>) {
        let target = page.filter(|p| self.page_available(p)).unwrap_or_else(|| {
            if self.mode == Mode::Expanded {
                self.page
            } else {
                self.activity.as_ref().and_then(|a| a.page).filter(|p| self.page_available(p)).unwrap_or(HOME)
            }
        });
        let was = self.mode;
        if was == Mode::Expanded {
            self.switch_page(target);
        } else {
            self.page = target;
            self.page_from = None;
        }
        self.mode = Mode::Expanded;
        self.peek = None;
        self.leave_since = None;
        self.auto_close = if self.hovering { None } else { Some(Instant::now() + Duration::from_secs(8)) };
        if was != Mode::Expanded {
            self.opened_at = Some(Instant::now());
            self.broadcast(SystemEvent::Expanded(true));
        }
        self.request_frame();
        self.schedule_wake();
    }

    fn collapse(&mut self) {
        if self.mode != Mode::Expanded {
            return;
        }
        self.mode = Mode::Idle;
        self.leave_since = None;
        self.auto_close = None;
        self.hover_since = None;
        if self.keyboard {
            self.set_keyboard(false);
        }
        self.broadcast(SystemEvent::Expanded(false));
        // files were just parked on the shelf: a bubble once the notch has closed
        if let Some((icon, color)) = self.pending_bubble.take() {
            if crate::ui::look_alcove() && self.cfg.general.bubbles {
                push_bubble(
                    &mut self.bubbles,
                    Instant::now() + Duration::from_millis(350),
                    icon,
                    color,
                    false,
                    Some("shelf"),
                );
            }
        }
        self.next_peek();
        self.request_frame();
        self.schedule_wake();
    }

    fn toggle(&mut self) {
        if self.mode == Mode::Expanded {
            self.collapse();
        } else {
            if self.user_hidden {
                self.user_hidden = false;
            }
            self.expand(None);
        }
    }

    fn push_peek(&mut self, p: Peek) {
        if self.user_hidden || self.fs_hidden {
            return;
        }
        // Do not disturb (the moon in the control center): only volume and brightness still show
        if crate::bloom::dnd() && !p.is_hud() {
            return;
        }
        if self.mode == Mode::Expanded {
            return; // the user is already looking at the notch
        }
        // A new track only changes the closed notch (cover and bars); it never pops a peek
        if p.key == Some("media") {
            return;
        }
        // Alcove: a finished copy or download is just a bubble that pops out beside the notch and
        // is pulled back in, not a full notification
        if self.peek.is_none()
            && (small_bubble(&p) || (matches!(p.key, Some("clipboard" | "download")) && !p.is_hud()))
            && crate::ui::look_alcove()
            && self.cfg.general.bubbles
        {
            push_bubble(&mut self.bubbles, Instant::now(), p.icon, p.accent, bubble_off(&p), p.page);
            self.request_frame();
            self.schedule_wake();
            return;
        }
        if p.quiet {
            // a follow-up (the next track): swap the visible peek with an animation, never pop a new one
            if let (Some(k), Some(cur)) = (p.key, self.peek.as_mut()) {
                if cur.peek.key == Some(k) {
                    let old = std::mem::replace(&mut cur.peek, p);
                    cur.swap = Some((Instant::now(), Box::new(old)));
                    self.request_frame();
                    self.schedule_wake();
                }
            }
            return;
        }
        if let (Some(k), Some(cur)) = (p.key, self.peek.as_mut()) {
            if cur.peek.key == Some(k) {
                cur.swap = None;
                cur.until = Instant::now() + p.duration;
                cur.peek = p;
                self.request_frame();
                self.schedule_wake();
                return;
            }
        }
        if let Some(k) = p.key {
            self.peek_queue.retain(|q| q.key != Some(k));
        }
        if p.instant {
            if let Some(cur) = self.peek.take() {
                self.peek_queue.push_front(cur.peek);
                self.peek_queue.truncate(4);
            }
        }
        if self.peek.is_some() {
            if self.peek_queue.len() >= 4 {
                self.peek_queue.pop_front();
            }
            self.peek_queue.push_back(p);
        } else {
            self.peek = Some(ActivePeek { until: Instant::now() + p.duration, peek: p, swap: None });
            self.mode = Mode::Peek;
        }
        self.request_frame();
        self.schedule_wake();
    }

    fn next_peek(&mut self) {
        if self.mode == Mode::Expanded {
            return;
        }
        match self.peek_queue.pop_front() {
            Some(p) => {
                self.peek = Some(ActivePeek { until: Instant::now() + p.duration, peek: p, swap: None });
                self.mode = Mode::Peek;
                if crate::ui::look_alcove() {
                    // a small pulse as it arrives
                    self.anim.w.velocity += 90.0;
                    self.anim.h.velocity += 30.0;
                }
            }
            None => {
                self.peek = None;
                self.mode = Mode::Idle;
            }
        }
        self.request_frame();
    }

    fn set_hover(&mut self, on: bool) {
        if on == self.hovering {
            return;
        }
        self.hovering = on;
        let now = Instant::now();
        if on {
            if self.cfg.general.haptics && self.mode != Mode::Expanded && !self.user_hidden {
                sys::haptic(false);
            }
            self.hover_since = Some(now);
            self.leave_since = None;
            self.auto_close = None;
        } else {
            self.hover_since = None;
            self.leave_since = Some(now);
        }
        self.request_frame();
        self.schedule_wake();
    }

    fn set_keyboard(&mut self, on: bool) {
        if on == self.keyboard {
            return;
        }
        self.keyboard = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            if on {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE.0 as isize));
                if !SetForegroundWindow(self.hwnd).as_bool() || GetForegroundWindow() != self.hwnd {
                    // Unlock foreground rights with a harmless synthetic key press.
                    let inputs = [key_input(VK_MENU, false), key_input(VK_MENU, true)];
                    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
                    let _ = SetForegroundWindow(self.hwnd);
                }
                let _ = SetFocus(Some(self.hwnd));
            } else {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
            }
        }
        self.request_frame();
    }

    /// What the selected closed-notch widgets show right now.
    fn idle_glances(&self) -> Vec<Glance> {
        let mut out = Vec::new();
        if self.mode != Mode::Idle {
            return out;
        }
        for id in self.cfg.notch.widgets.iter().take(3) {
            match id.as_str() {
                "clock" => out.push((None, clock_text(), palette::TEXT, None)),
                "date" => out.push((None, date_text(), palette::TEXT, None)),
                "battery" => {
                    if let Some((level, charging)) = self.indicators.iter().find_map(|i| i.battery) {
                        let icon = if charging { Some(Icon::Bolt) } else { None };
                        out.push((icon, format!("{:.0}%", level * 100.0), battery_color(level, charging), Some(level)));
                    }
                }
                other => {
                    let module = if matches!(other, "wifi" | "bluetooth") { "control" } else { other };
                    if let Some(i) = self.modules.iter().position(|m| m.id() == module) {
                        if self.started[i] {
                            if let Some((icon, text)) = self.modules[i].glance_for(other) {
                                out.push((Some(icon), text, palette::TEXT, None));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    fn quit(&mut self) {
        self.shutdown();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            // WM_DESTROY arrives while the app is still borrowed (re-entrant),
            // so its handler never runs; end the message loop here.
            PostQuitMessage(0);
        }
    }

    // -----------------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------------

    fn body_rect(&self) -> Rect {
        let w = self.anim.w.value;
        let h = self.anim.h.value;
        let shift = (1.0 - self.anim.shown.value) * (h + 14.0);
        Rect::new(self.win_w * 0.5 - w * 0.5, -shift, w, h)
    }

    /// Alcove: a playing track shows its cover and level bars as two bubbles beside the notch.
    fn split_media(&self) -> bool {
        crate::ui::look_alcove()
            && self.cfg.general.bubbles
            && self.mode != Mode::Expanded
            && self.activity.as_ref().is_some_and(|a| matches!(a.left, Slot::Vinyl { .. }))
    }

    /// The two media bubbles (left, right); they slide out from under the notch edges.
    fn split_rects(&self) -> (Rect, Rect) {
        let s = self.cfg.appearance.notch_height;
        let b = self.body_rect();
        let k = self.anim.split.value;
        let y = b.y.min(0.0);
        (Rect::new(b.x - (s + 8.0) * k, y, s, s), Rect::new(b.right() - s + (s + 8.0) * k, y, s, s))
    }

    /// The lingering bubbles (oldest nearest the notch): rect, opacity, icon and colour. Each pops
    /// out beside the notch, stays about a second, then shrinks back under the edge as if the
    /// notch pulled it in; the ones further out slide in to fill the gap as it goes.
    fn bubble_states(&self, now: Instant) -> Vec<(Rect, f32, Icon, Color, bool, Option<ModuleId>)> {
        let mut out = Vec::new();
        // the common case (no bubble showing) is asked on every mouse move and every frame
        if self.bubbles.is_empty() || self.mode != Mode::Idle || self.peek.is_some() {
            return out;
        }
        let s = self.cfg.appearance.notch_height;
        // Anchored to the full-size notch, so a bubble looks the same whatever the idle style
        // (notch, pill or hidden); beside the music bubble when there is one
        let right = self.body_rect().right().max(self.win_w * 0.5 + self.cfg.appearance.notch_width * 0.5);
        let split = self.anim.split.value;
        let mut off = 0.0;
        for b in &self.bubbles {
            let Some((k, alpha, scale, room)) = bubble_curve(now.saturating_duration_since(b.at).as_secs_f32()) else {
                continue;
            };
            let x = right - s + (s + 8.0) * ((1.0 + off) * k + split);
            let r = Rect::new(x, 0.0, s, s);
            out.push((Rect::centered(r.cx(), r.cy(), r.w * scale, r.h * scale), alpha, b.icon, b.color, b.off, b.page));
            off += room;
        }
        out
    }

    /// The secondary activity bubble slides out from under the right edge.
    fn bubble_rect(&self) -> Rect {
        let s = self.cfg.appearance.notch_height;
        let b = self.body_rect();
        let k = self.anim.sec.value;
        Rect::new(b.right() - s + (s + 8.0) * k, b.y.min(0.0), s, s)
    }

    fn hover_zone(&self) -> Rect {
        let b = self.body_rect();
        let strip_w = self.cfg.appearance.notch_width;
        let strip = Rect::new(self.win_w * 0.5 - strip_w * 0.5, 0.0, strip_w, 5.0);
        let b = Rect::new(b.x, 0.0, b.w, b.bottom().max(0.0));
        let z = b.union(&strip);
        let z = if self.anim.split.value > 0.5 {
            let (l, r) = self.split_rects();
            z.union(&l).union(&r)
        } else {
            z
        };
        let mut z = if self.anim.sec.value > 0.5 { z.union(&self.bubble_rect()) } else { z };
        // a bubble that opens a page on click counts as part of the notch while it shows
        for b in self.bubble_states(Instant::now()) {
            if b.5.is_some() {
                z = z.union(&b.0);
            }
        }
        z
    }

    fn on_mouse_move(&mut self, x: f32, y: f32) {
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
        let inside = self.hover_zone().contains(x, y) && !self.fs_hidden;
        self.set_hover(inside);
        let before = self.input.mouse;
        self.input.mouse = if inside { Some((x, y)) } else { None };
        // Only the expanded/peek UI has hover states worth a frame.
        if self.mode != Mode::Idle && before != self.input.mouse {
            self.request_frame();
        }
    }

    fn on_click(&mut self, p: (f32, f32)) {
        let down = self.input.down.take();
        let inside = self.body_rect().contains(p.0, p.1);
        match self.mode {
            Mode::Expanded => {
                if !inside {
                    self.collapse();
                    return;
                }
                if let Some(d) = down {
                    self.input.click = Some((d, p));
                }
            }
            Mode::Peek => match self.peek.as_ref().and_then(|a| a.peek.focus) {
                Some(h) => {
                    sys::activate_window(h);
                    self.peek = None;
                    self.next_peek();
                }
                None => {
                    let page = self.peek.as_ref().and_then(|a| a.peek.page);
                    self.expand(page);
                }
            },
            Mode::Idle => {
                let hit = self
                    .bubble_states(Instant::now())
                    .into_iter()
                    .find(|b| b.5.is_some() && b.1 > 0.4 && b.0.expand(4.0).contains(p.0, p.1))
                    .and_then(|b| b.5);
                if let Some(page) = hit {
                    // a click on a lingering bubble opens what it is about
                    self.bubbles.clear();
                    self.user_hidden = false;
                    if self.cfg.general.haptics {
                        sys::haptic(true);
                    }
                    self.expand(Some(page));
                } else if self.anim.split.value > 0.5 && {
                    let (l, r) = self.split_rects();
                    l.contains(p.0, p.1) || r.contains(p.0, p.1)
                } {
                    let page = self.activity.as_ref().and_then(|a| a.page);
                    self.expand(page);
                } else if self.anim.sec.value > 0.5 && self.bubble_rect().contains(p.0, p.1) {
                    let page = self.secondary.as_ref().and_then(|a| a.page);
                    self.expand(page);
                } else if inside || self.hover_zone().contains(p.0, p.1) {
                    self.user_hidden = false;
                    if self.cfg.general.haptics {
                        sys::haptic(true);
                    }
                    // with the media split into two bubbles the notch between them is blank: a click
                    // there opens Home, while the bubbles themselves open the media page
                    let page = if self.anim.split.value > 0.5 { Some(HOME) } else { None };
                    self.expand(page);
                }
            }
        }
        self.request_frame();
    }

    fn on_key(&mut self, k: Key) {
        if k == Key::Escape {
            self.set_keyboard(false);
            self.collapse();
            return;
        }
        let Some(i) = self.module_index(self.page) else { return };
        if !self.started[i] || !self.modules[i].wants_keyboard() {
            return;
        }
        let mut fx = Effects::default();
        {
            let mut cx =
                Cx { bus: &self.bus, cfg: &self.cfg, fx: &mut fx, hwnd: SendHwnd::new(self.hwnd), now: Instant::now() };
            self.modules[i].on_key(k, &mut cx);
        }
        fx.redraw = true;
        self.apply_effects(fx);
        self.schedule_wake();
    }

    fn on_hotkey(&mut self, a: Action) {
        match a {
            Action::Toggle => self.toggle(),
            Action::Ai => {
                if self.mode == Mode::Expanded && self.page == "ai" {
                    self.collapse();
                } else {
                    self.user_hidden = false;
                    self.expand(Some("ai"));
                    if self.page == "ai" {
                        self.set_keyboard(true);
                    }
                }
            }
            Action::Timer => self.expand(Some("timer")),
            Action::Clipboard => self.expand(Some("clipboard")),
            Action::PlayPause => self.broadcast(SystemEvent::Command { verb: "playpause".into(), args: vec![] }),
            Action::Dictate => {
                self.broadcast(SystemEvent::Command { verb: "dictate".into(), args: vec!["hotkey".into()] })
            }
        }
    }

    pub fn on_ipc(&mut self, args: Vec<String>) {
        let Some(verb) = args.first().map(|s| s.to_ascii_lowercase()) else {
            self.expand(None);
            return;
        };
        let rest: Vec<String> = args[1..].to_vec();
        log!("ipc: {verb} {rest:?}");
        match verb.as_str() {
            "toggle" => self.toggle(),
            "expand" | "show" | "open" => {
                self.user_hidden = false;
                self.expand(rest.first().map(|p| leak_page(p)));
            }
            "collapse" | "close" => self.collapse(),
            "ai" => self.on_hotkey(Action::Ai),
            "reload" => self.reload_config(),
            "notify" => {
                // Scriptable banner: dynamic-notch notify "Title" "Body"
                let title = rest.first().cloned().unwrap_or_else(|| "Notification".into());
                let body = rest.get(1..).map(|a| a.join(" ")).unwrap_or_default();
                self.push_peek(Peek::new(Icon::Bell, palette::BLUE, title, body).duration_ms(4500));
            }
            "claude" => self.push_peek(claude_peek(&rest)),
            "quit" | "exit" => self.quit(),
            _ => self.broadcast(SystemEvent::Command { verb, args: rest }),
        }
    }

    // -----------------------------------------------------------------------
    // Tray menu
    // -----------------------------------------------------------------------

    pub fn menu_state(&self) -> MenuState {
        MenuState {
            expanded: self.mode == Mode::Expanded,
            user_hidden: self.user_hidden,
            modules: self
                .modules
                .iter()
                .map(|m| (m.title().to_string(), self.cfg.modules.is_enabled(m.id())))
                .collect(),
            monitor_count: monitors().len(),
            monitor: self.cfg.general.monitor.clone(),
            autostart: self.cfg.general.start_with_windows,
            blur: self.cfg.appearance.blur,
            hover: self.cfg.general.expand_on_hover,
            hide_fullscreen: self.cfg.general.hide_in_fullscreen,
        }
    }

    pub fn on_menu(&mut self, cmd: u32) {
        let b = |v: bool| if v { "true".to_string() } else { "false".to_string() };
        match cmd {
            tray::CMD_TOGGLE => self.toggle(),
            tray::CMD_AI => self.on_hotkey(Action::Ai),
            tray::CMD_TIMER => self.expand(Some("timer")),
            tray::CMD_HIDE => {
                self.user_hidden = !self.user_hidden;
                if self.user_hidden {
                    self.collapse();
                    self.peek = None;
                    self.peek_queue.clear();
                    self.mode = Mode::Idle;
                }
                self.request_frame();
            }
            tray::CMD_MON_PRIMARY => config::set_value("general", "monitor", "\"primary\""),
            tray::CMD_MON_ACTIVE => config::set_value("general", "monitor", "\"active\""),
            c if (tray::CMD_MON_BASE..tray::CMD_MON_BASE + 16).contains(&c) => {
                config::set_value("general", "monitor", &format!("\"{}\"", c - tray::CMD_MON_BASE + 1))
            }
            c if (tray::CMD_MODULE_BASE..tray::CMD_MODULE_BASE + 32).contains(&c) => {
                let i = (c - tray::CMD_MODULE_BASE) as usize;
                if let Some(m) = self.modules.get(i) {
                    let id = m.id();
                    let on = !self.cfg.modules.is_enabled(id);
                    config::set_value("modules", id, &b(on));
                }
            }
            tray::CMD_AUTOSTART => {
                config::set_value("general", "start_with_windows", &b(!self.cfg.general.start_with_windows))
            }
            tray::CMD_BLUR => config::set_value("appearance", "blur", &b(!self.cfg.appearance.blur)),
            tray::CMD_HOVER => config::set_value("general", "expand_on_hover", &b(!self.cfg.general.expand_on_hover)),
            tray::CMD_FULLSCREEN => {
                config::set_value("general", "hide_in_fullscreen", &b(!self.cfg.general.hide_in_fullscreen))
            }
            tray::CMD_EDIT_CONFIG => {
                let path = config::config_path();
                if !path.exists() {
                    let _ = std::fs::write(&path, config::DEFAULT_TEMPLATE);
                }
                sys::shell_open(&path.to_string_lossy());
            }
            tray::CMD_OPEN_FOLDER => sys::shell_open(&crate::util::app_dir().to_string_lossy()),
            tray::CMD_RELOAD => self.reload_config(),
            tray::CMD_QUIT => self.quit(),
            _ => {}
        }
    }

    fn reload_config(&mut self) {
        match config::load() {
            LoadResult::Ok(c) => {
                let autostart_changed = c.general.start_with_windows != self.cfg.general.start_with_windows;
                self.cfg = c;
                crate::ui::set_white_beams(self.cfg.appearance.white_beams);
                crate::ui::set_look(&self.cfg.appearance.look);
                if autostart_changed {
                    sys::set_autostart(self.cfg.general.start_with_windows);
                }
                self.register_hotkeys();
                self.sync_modules();
                self.place();
                self.sync_dock();
                self.broadcast(SystemEvent::ConfigReloaded);
                log!("config reloaded");
            }
            LoadResult::Invalid(e) => {
                log!("config error: {e}");
                let msg = crate::util::one_line(&e);
                self.push_peek(
                    Peek::new(Icon::Settings, palette::ORANGE, "Settings file has an error", msg)
                        .duration_ms(6000)
                        .key("config"),
                );
            }
        }
    }

    fn register_hotkeys(&mut self) {
        let (ok, errors) = hotkeys::register_all(self.hwnd, &self.cfg.hotkeys);
        self.hotkeys = ok;
        self.dictate_vk = hotkeys::parse(&self.cfg.hotkeys.dictate).map_or(0, |(_, vk)| vk as u16);
        if !errors.is_empty() {
            log!("hotkey problems: {errors:?}");
            self.push_peek(
                Peek::new(Icon::Settings, palette::ORANGE, "Hotkey unavailable", errors.join(", "))
                    .duration_ms(5000)
                    .key("hotkeys"),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Placement (multi-monitor + per-monitor DPI)
    // -----------------------------------------------------------------------

    fn place(&mut self) {
        let mon = self.pick_monitor();
        unsafe {
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let scale = dx as f32 / 96.0 * self.cfg.appearance.scale;
            let win_w = self.cfg.appearance.expanded_width + 2.0 * MARGIN;
            let win_h = WIN_H;
            let wpx = (win_w * scale).ceil() as i32;
            let hpx = (win_h * scale).ceil() as i32;
            let m = mi.rcMonitor;
            let top = if mi.rcWork.top > m.top { mi.rcWork.top } else { m.top };
            let x = (m.left + m.right) / 2 - wpx / 2;
            let geom = (x, top, wpx, hpx);
            self.monitor = mon;
            if geom != self.win_px || (scale - self.scale).abs() > 0.001 {
                self.scale = scale;
                self.win_w = win_w;
                self.win_h = win_h;
                self.win_px = geom;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, top, wpx, hpx, SWP_NOACTIVATE);
                if let Err(e) = self.host.resize(wpx, hpx) {
                    log!("surface resize failed: {e}");
                }
                self.region = None;
                self.region_anim = None;
            }
        }
        self.check_fullscreen();
        self.request_frame();
    }

    fn pick_monitor(&self) -> HMONITOR {
        let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        match self.cfg.general.monitor.trim() {
            "active" => unsafe {
                let fg = GetForegroundWindow();
                if fg.is_invalid() || fg == self.hwnd {
                    if self.monitor.is_invalid() {
                        primary
                    } else {
                        self.monitor
                    }
                } else {
                    MonitorFromWindow(fg, MONITOR_DEFAULTTOPRIMARY)
                }
            },
            s => match s.parse::<usize>() {
                Ok(n) if n >= 1 => monitors().get(n - 1).copied().unwrap_or(primary),
                _ => primary,
            },
        }
    }

    fn on_foreground_changed(&mut self) {
        unsafe {
            let fg = GetForegroundWindow();
            // Re-hook location changes for the foreground process only, so a
            // window becoming fullscreen (F11, videos) is noticed without
            // listening to every window in the system.
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid != self.loc_pid && pid != std::process::id() {
                if let Some(h) = self.loc_hook.take() {
                    let _ = UnhookWinEvent(h);
                }
                let h = SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    None,
                    Some(win_event_cb),
                    pid,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                if !h.is_invalid() {
                    self.loc_hook = Some(h);
                }
                self.loc_pid = pid;
            }
        }
        if self.cfg.general.monitor == "active" {
            let m = self.pick_monitor();
            if m != self.monitor {
                self.place();
                return;
            }
        }
        self.check_fullscreen();
        if !self.fs_hidden {
            unsafe {
                let _ =
                    SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn check_fullscreen(&mut self) {
        let hide = self.cfg.general.hide_in_fullscreen && unsafe { is_fullscreen_on(self.monitor, self.hwnd) };
        if hide != self.fs_hidden {
            self.fs_hidden = hide;
            if hide {
                self.collapse();
            }
            if let Some(d) = self.dock.as_mut() {
                d.set_hidden(hide);
            }
            log!("fullscreen hidden = {hide}");
            self.request_frame();
        }
    }

    // -----------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------

    fn request_frame(&mut self) {
        self.dirty = true;
        self.pacer.start();
    }

    fn targets(&self) -> Targets {
        let ap = &self.cfg.appearance;
        let base_w = ap.notch_width;
        let base_h = ap.notch_height;
        let mut t = Targets {
            w: base_w,
            h: base_h,
            r: (base_h * 0.56).min(18.0),
            ear: (base_h * 0.26).clamp(5.0, 9.0),
            glass: 0.0,
            compact: false,
            peek: false,
            exp: false,
            shown: !(self.user_hidden || self.fs_hidden),
            sec: false,
        };
        match self.mode {
            Mode::Expanded => {
                let page_w = self.page_width();
                let content_w = page_w - side_pad() - right_pad();
                let content_h = page_content_h(&self.cfg, &self.modules, &self.started, self.page, content_w);
                t.w = page_w;
                t.h = top_h() + content_h + bottom_pad();
                t.r = if crate::ui::look_alcove() { 46.0 } else { 34.0 };
                t.ear = 30.0;
                t.glass = 1.0;
                t.exp = true;
            }
            Mode::Peek => {
                let pk = self.peek.as_ref().map(|a| &a.peek);
                if pk.is_some_and(|p| p.scan) && crate::ui::look_alcove() {
                    // unlock: a round dial like the volume one, with "Hello" and the name under it
                    let name = hello_name(pk.map_or("", |p| p.title.as_str()));
                    t.w = crate::alcove::hello_stage(name).0.min(ap.expanded_width);
                    t.h = base_h + if name.is_empty() { 172.0 } else { 184.0 };
                    t.r = 44.0;
                    t.ear = 26.0;
                } else if pk.is_some_and(|p| p.scan) {
                    // unlock scan: the notch grows into a small square stage, then
                    // settles into a smaller one for the handwritten hello
                    let prog = self.peek.as_ref().map_or(0.0, |a| {
                        1.0 - a.until.saturating_duration_since(Instant::now()).as_secs_f32()
                            / a.peek.duration.as_secs_f32().max(0.1)
                    });
                    if prog > SCAN_HELLO_AT {
                        let title = pk.map_or("", |p| p.title.as_str());
                        let (w, h) = if crate::ui::look_alcove() {
                            crate::alcove::hello_stage(hello_name(title))
                        } else {
                            hello_stage(title, self.hello_custom())
                        };
                        t.w = w.min(ap.expanded_width);
                        t.h = h;
                    } else {
                        t.w = 230.0_f32.min(ap.expanded_width);
                        t.h = 168.0;
                    }
                    t.r = 34.0;
                    t.ear = 28.0;
                } else if let Some(dh) = pk.and_then(|p| dial_height(p, &self.cfg)) {
                    // volume / brightness / battery / Bluetooth as one big round dial
                    t.w = 190.0_f32.min(ap.expanded_width);
                    t.h = base_h + dh;
                    t.r = 44.0;
                    t.ear = 26.0;
                } else if pk.is_some_and(|p| p.is_hud()) {
                    // volume / brightness: one row, icon + beams + value
                    t.w = 300.0_f32.min(ap.expanded_width);
                    t.h = base_h + 34.0;
                    t.r = 26.0;
                    t.ear = 18.0;
                } else if pk.is_some_and(|p| p.is_compact()) {
                    // icon-only pill, like the unlock animation
                    t.w = (base_w + 70.0).max(250.0).min(ap.expanded_width);
                    t.h = base_h + 10.0;
                    t.r = 21.0;
                    t.ear = 10.0;
                } else if pk.is_some_and(crate::alcove::peek_notify) && crate::ui::look_alcove() {
                    // a message: avatar, name and two lines of text
                    t.w = (base_w + 190.0).max(430.0).min(ap.expanded_width);
                    t.h = base_h + 58.0;
                    t.r = 40.0;
                    t.ear = 24.0;
                } else if pk.is_some_and(crate::alcove::peek_conn) && crate::ui::look_alcove() {
                    // a device or network: small and left-weighted
                    t.w = (base_w + 70.0).max(280.0).min(ap.expanded_width);
                    t.h = base_h + 24.0;
                    t.r = 30.0;
                    t.ear = 18.0;
                } else if pk.is_some_and(crate::alcove::peek_tall) && crate::ui::look_alcove() {
                    // a notice with a message: the text sits in its own box under the title
                    t.w = (base_w + 190.0).max(430.0).min(ap.expanded_width);
                    t.h = base_h + 92.0;
                    t.r = 40.0;
                    t.ear = 24.0;
                } else if crate::ui::look_alcove() {
                    t.w = (base_w + 170.0).max(400.0).min(ap.expanded_width);
                    t.h = base_h + 46.0;
                    t.r = 32.0;
                    t.ear = 22.0;
                } else {
                    t.w = (base_w + 150.0).max(360.0).min(ap.expanded_width);
                    t.h = base_h + 44.0;
                    t.r = 24.0;
                    t.ear = 11.0;
                }
                t.glass = 0.55;
                t.peek = true;
            }
            Mode::Idle => {
                if let Some(a) = &self.activity {
                    t.w = if self.split_media() { base_w } else { base_w + 2.0 * a.wing * base_h };
                    t.compact = true;
                    t.sec = self.secondary.is_some() && t.shown;
                } else {
                    match self.cfg.general.idle_style.as_str() {
                        "pill" if !self.hovering => {
                            t.w = base_w * 0.55;
                            t.h = 7.0;
                            t.r = 3.5;
                            t.ear = 3.0;
                        }
                        "hidden" if !self.hovering => t.shown = false,
                        _ => {}
                    }
                }
                if self.activity.is_none() && !self.cfg.notch.widgets.is_empty() {
                    let shows = !matches!(self.cfg.general.idle_style.as_str(), "pill" | "hidden") || self.hovering;
                    if shows {
                        let n = self.cfg.notch.widgets.len().min(3) as f32;
                        t.w = t.w.max(26.0 + 66.0 * n);
                    }
                }
                if self.hovering && !self.user_hidden && crate::ui::look_alcove() && self.input.down.is_some() {
                    // pressed: the notch gives a little under the finger, then springs open on release
                    t.w -= 4.0;
                    t.h -= 2.0;
                } else if self.hovering && !self.user_hidden {
                    t.w += 10.0;
                    t.h += 3.0;
                    t.r += 1.0;
                }
            }
        }
        t
    }

    fn retarget(&mut self) {
        let t = self.targets();
        let split = self.split_media();
        let ap = &self.cfg.appearance;
        let speed = ap.animation_speed;
        // minimal look: no overshoot anywhere, and a touch more patience in every move
        let minimal = crate::ui::look_minimal();
        let nothing = crate::ui::look_nothing();
        let alcove = crate::ui::look_alcove();
        let bloom = crate::ui::look_bloom() && !alcove;
        // Bloom settles like a smooth ease-out: almost no overshoot
        let bounce = if minimal || nothing {
            0.0
        } else if bloom {
            ap.bounce * 0.35
        } else {
            ap.bounce
        };
        // minimal is a touch slower and softer; Nothing is crisp and mechanical
        let speed = if minimal {
            speed * 0.9
        } else if nothing {
            speed * 1.15
        } else if alcove {
            speed * 1.1
        } else if bloom {
            speed * 1.05
        } else {
            speed
        };
        let a = &mut self.anim;
        // page changes follow the animation speed setting too
        a.page.set_params(1.05 / speed, 1.0);
        let grow = t.w > a.w.value + 0.5 || t.h > a.h.value + 0.5;
        // One spring character for the whole body (size, corner radius and ears move as one
        // object): a little softer on the way out, snappier on the way back in.
        let (resp, damp) = if grow {
            // Alcove: a visible but gentle overshoot on the way out
            (BODY_RESPONSE / speed, if alcove { 1.0 - 0.8 * bounce } else { 1.0 - 0.45 * bounce })
        } else {
            (BODY_RESPONSE * 0.7 / speed, 1.0 - 0.1 * bounce)
        };
        // Shape morphing: height leads, width follows a beat later and the corners keep
        // catching up, so the notch flows from one form to the other instead of scaling.
        let (kh, kw, kr) = if alcove { (0.9, 1.08, 1.3) } else { (1.0, 1.0, 1.0) };
        if t.exp && a.exp.value > 0.9 {
            // moving between open pages: the body resizes with the very same slow, smooth motion
            // as the page sliding up or down, so the two read as one movement
            a.h.set_params(0.77 / speed, 1.0);
            a.w.set_params(0.77 / speed, 1.0);
        } else {
            a.h.set_params(resp * kh, damp);
            a.w.set_params(resp * kw, damp);
        }
        a.r.set_params(resp * kr, damp.max(0.9));
        a.ear.set_params(resp * kr, damp.max(0.9));
        // Staged like the Mac apps: when closing, the content fades first and the body only then
        // shrinks around the empty space; when opening, the body leads and the content follows.
        let closing = !grow && !t.exp && a.exp.value > 0.05;
        let hold = if closing { CLOSE_HOLD / speed } else { 0.0 };
        a.w.set_target_delayed(t.w, hold);
        a.h.set_target_delayed(t.h, hold);
        a.r.set_target_delayed(t.r, hold);
        a.ear.set_target_delayed(t.ear, hold);
        a.glass.set_target(t.glass);
        a.shown.set_params(0.5 / speed, 1.0 - 0.5 * bounce);
        a.shown.set_target(if t.shown { 1.0 } else { 0.0 });
        a.split.set_params(0.5 / speed, 1.0 - 0.7 * bounce);
        a.split.set_target(if split && t.shown { 1.0 } else { 0.0 });
        a.sec.set_params(0.45 / speed, 1.0 - 0.6 * bounce);
        a.sec.set_target(if t.sec { 1.0 } else { 0.0 });
        let fade = |s: &mut Spring, on: bool, delay: f32| {
            if on {
                s.set_params(0.24 / speed, 1.0);
                s.set_target_delayed(1.0, delay / speed);
            } else {
                s.set_params(0.12 / speed, 1.0);
                s.set_target(0.0);
            }
        };
        fade(&mut a.compact, t.compact, 0.03);
        fade(&mut a.peek, t.peek, 0.04);
        fade(&mut a.exp, t.exp, 0.09);
    }

    fn frame(&mut self) {
        let now = Instant::now();
        // Ambient animation only (a sweeping shimmer, the record turning): nothing is being moved by
        // a spring and nobody touched anything. Frames closer together than AMBIENT_GAP are skipped,
        // which only trims displays faster than 60 Hz; on a 60 Hz screen every frame is drawn, since
        // halving to 30 fps made sweeps and shimmers look choppy.
        if !self.dirty && self.settled && now.duration_since(self.last_frame) < AMBIENT_GAP {
            return;
        }
        let dt = (now - self.last_frame).as_secs_f32().min(1.0 / 20.0);
        self.last_frame = now;
        self.dirty = false;
        self.trim_at = None;
        let shown = !(self.user_hidden || self.fs_hidden) && self.mode != Mode::Expanded;
        if PEEKS_SHOWN.swap(shown, Ordering::Relaxed) != shown {
            self.broadcast(SystemEvent::PeeksShown(shown));
        }
        self.process_timeouts(now);
        self.refresh_activity();
        self.retarget();
        let moving = self.anim.step(dt);
        if self.page_from.is_some() && self.anim.page.value > 0.998 && self.anim.page.target >= 1.0 {
            self.page_from = None;
        }
        self.settled = !moving;
        let fx = self.render(now);
        let animate = fx.animate;
        self.apply_effects(fx);
        self.update_region(!moving);
        self.input.click = None;
        self.input.right_click = None;
        self.input.wheel = 0.0;
        self.input.hwheel = 0.0;
        if self.debug {
            let st = &mut self.stats;
            st.0.get_or_insert(now);
            if moving {
                st.1 += 1;
            } else if animate {
                st.2 += 1;
            } else if self.dirty {
                st.3 += 1;
            }
        }
        if !moving && !animate && !self.dirty {
            if let (true, Some(t0)) = (self.debug, self.stats.0) {
                let (_, a, b, c) = self.stats;
                log!(
                    "burst: {} frames in {} ms (spring {a}, animate {b}, dirty {c}) mode {:?}",
                    a + b + c + 1,
                    t0.elapsed().as_millis(),
                    self.mode
                );
                self.stats = (None, 0, 0, 0);
            }
            self.pacer.stop();
            // Release GPU scratch memory once nothing has been drawn for a
            // while. Doing it after every burst would rebuild the glyph and
            // image caches on each once-a-second update (stats, timer, media).
            self.trim_at = Some(now + TRIM_DELAY);
        }
        if !self.shown_once {
            self.shown_once = true;
        }
        self.schedule_wake();
    }

    fn update_region(&mut self, settled: bool) {
        let ear = self.anim.ear.target.max(self.anim.ear.value);
        let t = self.targets();
        let shown = t.shown;
        let bubble = if t.sec { self.cfg.appearance.notch_height + 8.0 + ear } else { 0.0 };
        let side = if self.split_media() { self.cfg.appearance.notch_height + 8.0 } else { 0.0 };
        let tb = Rect::new(self.win_w * 0.5 - t.w * 0.5 - ear - side, 0.0, t.w + 2.0 * (ear + side) + bubble, t.h);
        let cur = self.body_rect();
        let mut cb = Rect::new(cur.x - ear, 0.0, cur.w + 2.0 * ear, cur.bottom().max(0.0));
        if self.anim.sec.value > 0.01 {
            cb = cb.union(&self.bubble_rect().expand(ear));
        }
        for (r, ..) in self.bubble_states(Instant::now()) {
            cb = cb.union(&r.expand(ear + 6.0));
        }
        if self.anim.split.value > 0.01 {
            let (l, r) = self.split_rects();
            cb = cb.union(&l.expand(ear)).union(&r.expand(ear));
        }
        let strip_w = self.cfg.appearance.notch_width;
        let strip = Rect::new(self.win_w * 0.5 - strip_w * 0.5, 0.0, strip_w, 5.0);
        let want = if self.fs_hidden {
            None
        } else if settled {
            self.region_anim = None;
            let mut r = if shown { tb.union(&strip) } else { strip };
            // the lingering bubble sits outside the resting notch
            for (lr, ..) in self.bubble_states(Instant::now()) {
                r = r.union(&lr.expand(ear + 6.0));
            }
            Some(r)
        } else {
            let mut r = tb.expand(16.0).union(&cb).union(&strip);
            if let Some(prev) = self.region_anim {
                r = r.union(&prev);
            }
            self.region_anim = Some(r);
            Some(r)
        };
        let s = self.scale;
        let px = match want {
            Some(r) => (
                (r.x * s).floor().max(0.0) as i32,
                0,
                (r.right() * s).ceil() as i32,
                ((r.bottom() * s).ceil() as i32).min(self.win_px.3),
            ),
            None => (0, 0, 0, 0),
        };
        if self.region != Some(px) {
            self.region = Some(px);
            unsafe {
                let rgn = CreateRectRgn(px.0, px.1, px.2, px.3);
                SetWindowRgn(self.hwnd, Some(rgn), false);
            }
        }
    }

    /// The header tabs in the user's order (Home first).
    fn tab_list(&self) -> Vec<(ModuleId, Icon, String)> {
        let mut tabs: Vec<(ModuleId, Icon, String)> = std::iter::once((HOME, Icon::Home, String::new()))
            .chain(
                self.modules
                    .iter()
                    .enumerate()
                    .filter(|(i, m)| self.started[*i] && m.has_page() && m.id() != "settings")
                    .map(|(_, m)| (m.id(), m.icon(), String::new())),
            )
            .collect();
        // the user's tab order (Home stays first; unlisted tabs keep their place at the end)
        if !self.cfg.tabs.order.is_empty() && tabs.len() > 1 {
            let order = &self.cfg.tabs.order;
            tabs[1..].sort_by_key(|t| order.iter().position(|o| o == t.0).unwrap_or(usize::MAX));
        }
        tabs
    }

    /// Alcove: the bottom edge of the open notch (a page, or a bigger peek such as the volume dial
    /// or a message) softens into a light blur. Only the thin strip along the bottom is drawn a
    /// second time into a small picture (and only on frames that are drawn anyway), then laid over
    /// the live drawing in `bottom_fade`. It is there as soon as the notch is open, also while pages
    /// switch.
    fn prepare_band(&mut self, now: Instant) {
        let t = self.targets();
        let tw = self.page_width();
        let body = self.body_rect();
        let ap = &self.cfg.appearance;
        let on = ap.bottom_fade && ap.blur && crate::ui::look_alcove();
        let page_band = on
            && self.mode == Mode::Expanded
            && self.page != "settings"
            && self.anim.exp.value >= 0.88
            && (body.h - t.h).abs() < 40.0;
        let peek_band = on
            && self.mode == Mode::Peek
            && self.anim.peek.value >= 0.95
            && body.h >= 56.0
            && (body.h - t.h).abs() < 24.0
            && self.peek.as_ref().is_some_and(|a| a.swap.is_none());
        if !page_band && !peek_band {
            if self.band_since.is_some() {
                log!("bottom blur off: mode {:?} page {} exp {:.3} dh {:.1}", self.mode, self.page, self.anim.exp.value, body.h - t.h);
            }
            self.snap_band = None;
            self.band_since = None;
            return;
        }
        self.band_since.get_or_insert(now);
        let input = Input::default();
        let mut scratch = Effects::default();
        let (accent, page, scale) = (self.accent, self.page, self.scale);
        if page_band {
            let full = Rect::new(body.cx() - tw * 0.5, body.y, tw, t.h);
            let content =
                page_content(body.cx(), full.y, page_size(&self.cfg, &self.modules, &self.started, self.page));
            let h = BAND_H.min(full.h * 0.4);
            let region = Rect::new(full.x, full.bottom() - h, full.w, h);
            self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4D, |p| {
                let mut ui = Ui {
                    p,
                    input: &input,
                    fx: &mut scratch,
                    cfg: &self.cfg,
                    bus: &self.bus,
                    now,
                    accent,
                    hot: false,
                    interactive: false,
                };
                draw_body(&mut ui, content, &mut self.modules, &self.started, page);
            });
        } else if let Some(act) = &self.peek {
            let h = BAND_H.min(body.h * 0.4);
            let region = Rect::new(body.x, body.bottom() - h, body.w, h);
            let left = (act.until.saturating_duration_since(now).as_secs_f32()
                / act.peek.duration.as_secs_f32().max(0.1))
            .clamp(0.0, 1.0);
            self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4E, |p| {
                let mut ui = Ui {
                    p,
                    input: &input,
                    fx: &mut scratch,
                    cfg: &self.cfg,
                    bus: &self.bus,
                    now,
                    accent,
                    hot: false,
                    interactive: false,
                };
                draw_peek(&mut ui, body, &act.peek, left, None);
            });
        }
    }

    /// 1 while the soft bottom edge is showing, else 0.
    fn band_k(&self) -> f32 {
        if self.snap_band.is_some() {
            1.0
        } else {
            0.0
        }
    }

    /// While the notch opens or closes, and while a page slides in, the content itself does not
    /// change: it is only moved and faded. So it is drawn once into a picture and the frames of the
    /// animation just place that picture (one rectangle fill instead of laying out and drawing
    /// every text, icon and beam again). Drawn live again as soon as the motion has settled.
    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_band(now);
        let a_exp = self.anim.exp.value;
        let paging = self.page_from.is_some();
        let on = self.cfg.appearance.motion_cache;
        // the live view takes over a little before the end, so clicks work as soon as they used to
        let want_all = on && a_exp > 0.003 && a_exp < 0.88 && !paging;
        let want_pages = on && paging && a_exp >= 0.998;
        if !want_all {
            self.snap_all = None;
        }
        if !want_pages {
            self.snap_pages = None;
        }
        if !(want_all || want_pages) {
            return;
        }
        let scale = self.scale;
        let tw = self.page_width();
        let body = self.body_rect();
        let target_h = self.targets().h;
        let full = Rect::new(
            body.cx() - tw * 0.5,
            body.y,
            tw,
            // an open page is laid out at its own height even while the body is still catching up, so
            // nothing drifts or stretches mid-resize; while closing the page keeps the body's height
            if self.mode == Mode::Expanded { target_h } else { target_h.max(body.h) },
        );
        let page = self.page;
        let accent = self.accent;
        let input = Input::default();
        let mut scratch = Effects::default();
        // keep the pictures inside the window
        let clamp = |r: Rect| {
            let (x0, y0) = (r.x.max(0.0), r.y.max(0.0));
            let (x1, y1) = (r.right().min(self.win_w), r.bottom().min(self.win_h));
            Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
        };
        let hash = |parts: &[i32]| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(parts, &mut h);
            std::hash::Hasher::finish(&h)
        };
        let page_id = |id: ModuleId| id.bytes().fold(0i32, |a, b| a.wrapping_mul(31).wrapping_add(b as i32));
        let sc = (scale * 100.0) as i32;

        if want_all {
            // the height is left out of the key: it changes every frame while closing
            let key = hash(&[page_id(page), full.x.round() as i32, full.y.round() as i32, full.w.round() as i32, sc]);
            let fresh =
                self.snap_all.as_ref().is_some_and(|s| s.key == key && s.born.elapsed() < Duration::from_millis(900));
            if !fresh {
                let tabs = self.tab_list();
                let region = clamp(full);
                self.snap_all = self.res.render_snapshot(&self.gfx, scale, region, key, |p| {
                    let mut ui = Ui {
                        p,
                        input: &input,
                        fx: &mut scratch,
                        cfg: &self.cfg,
                        bus: &self.bus,
                        now,
                        accent,
                        hot: false,
                        interactive: false,
                    };
                    draw_expanded(
                        &mut ui,
                        full,
                        full,
                        &mut self.modules,
                        &self.started,
                        &tabs,
                        page,
                        None,
                        &self.indicators,
                        None,
                    );
                });
            }
        }

        if want_pages {
            let Some((from, _)) = self.page_from else { return };
            // each page is drawn at its own size, centred; the picture is as big as the larger one
            let (gf, gt) = (
                page_size(&self.cfg, &self.modules, &self.started, from),
                page_size(&self.cfg, &self.modules, &self.started, page),
            );
            let cx = body.cx();
            let (cf, ct) = (page_content(cx, full.y, gf), page_content(cx, full.y, gt));
            let region = clamp(Rect::new(cx - gf.0.max(gt.0) * 0.5, ct.y - 4.0, gf.0.max(gt.0), cf.h.max(ct.h) + 8.0));
            let key_of = |id: ModuleId, g: (f32, f32)| {
                hash(&[
                    page_id(id),
                    (cx - g.0 * 0.5).round() as i32,
                    full.y.round() as i32,
                    g.0.round() as i32,
                    g.1.round() as i32,
                    sc,
                ])
            };
            let (kf, kt) = (key_of(from, gf), key_of(page, gt));
            let fresh = self
                .snap_pages
                .as_ref()
                .is_some_and(|(a, b)| a.key == kf && b.key == kt && a.born.elapsed() < Duration::from_millis(1500));
            if !fresh {
                let mut shots: Vec<Option<Snapshot>> = Vec::with_capacity(2);
                for (id, key, content) in [(from, kf, cf), (page, kt, ct)] {
                    let snap = self.res.render_snapshot(&self.gfx, scale, region, key, |p| {
                        let mut ui = Ui {
                            p,
                            input: &input,
                            fx: &mut scratch,
                            cfg: &self.cfg,
                            bus: &self.bus,
                            now,
                            accent,
                            hot: false,
                            interactive: false,
                        };
                        draw_body(&mut ui, content, &mut self.modules, &self.started, id);
                    });
                    shots.push(snap);
                }
                let (b, a) = (shots.pop().flatten(), shots.pop().flatten());
                self.snap_pages = a.zip(b);
            }
        }
    }

    fn render(&mut self, now: Instant) -> Effects {
        let mut fx = Effects::default();
        self.prepare_snapshots(now);
        let (dc, off) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(e) => {
                log!("BeginDraw failed: {e} — recreating devices");
                self.recover_device();
                return fx;
            }
        };
        let body = self.body_rect();
        let scale = self.scale;
        let ap = {
            let a = &self.cfg.appearance;
            Look {
                shadow: a.shadow,
                blur: a.blur,
                expanded_opacity: a.expanded_opacity,
                collapsed_opacity: a.collapsed_opacity,
                notch_height: a.notch_height,
            }
        };
        let settled = self.settled;
        let see_through = ap.expanded_opacity < 0.97 || ap.collapsed_opacity < 0.97;
        let a_glass = self.anim.glass.value;
        let a_r = self.anim.r.value;
        let a_ear = self.anim.ear.value;
        let a_compact = self.anim.compact.value;
        let a_peek = self.anim.peek.value;
        let a_exp = self.anim.exp.value;
        crate::alcove::set_open(a_exp);
        let target_h = self.targets().h;
        // read before the painter borrows the window's resources
        let page_w = self.page_width();
        let band_k = self.band_k();
        let bubble = self.bubble_rect();
        if !self.bubbles.is_empty() {
            if self.mode != Mode::Idle || self.peek.is_some() {
                self.bubbles.clear();
            }
            self.bubbles.retain(|b| now.saturating_duration_since(b.at).as_secs_f32() < BUBBLE_END);
            // a quiet sound as a bubble pops out and again as it is pulled in
            let sounds = self.cfg.general.bubble_sounds;
            for b in &mut self.bubbles {
                if !b.in_played && now >= b.at {
                    b.in_played = true;
                    if sounds {
                        crate::sys::bubble_sound(false);
                    }
                }
                if !b.out_played && now.saturating_duration_since(b.at).as_secs_f32() >= BUBBLE_HOLD {
                    b.out_played = true;
                    if sounds {
                        crate::sys::bubble_sound(true);
                    }
                }
            }
        }
        let bubble_list = self.bubble_states(now);
        let split_v = self.anim.split.value;
        let split_on = split_v > 0.01 && self.mode != Mode::Expanded;
        let (lb, rb) = self.split_rects();
        let secondary = self.secondary.clone();
        let glances = self.idle_glances();
        let mut bars: Option<(Rect, Color, bool)> = None;
        let tabs: Vec<(ModuleId, Icon, String)> = if a_exp <= 0.003 || self.snap_all.is_some() {
            Vec::new() // nothing to draw (or one picture covers it): skip the per-frame allocations
        } else {
            self.tab_list()
        };

        {
            let mut p = Painter::new(dc.clone(), &self.gfx, &mut self.res, scale, off.x as f32, off.y as f32);
            p.clear();
            if body.bottom() > 0.25 || !bubble_list.is_empty() {
                // shadow
                if ap.shadow && a_glass > 0.01 {
                    // extend above the window so only the sides/bottom show
                    let sr = Rect::new(body.x, body.y - 60.0, body.w, body.h + 60.0);
                    p.shadow(sr, a_r, 0.5 * a_glass);
                }
                // secondary activity bubble (drawn first; the main body covers it while sliding out)
                let sec_v = self.anim.sec.value;
                if sec_v > 0.01 {
                    if let Some(sa) = secondary {
                        let br = bubble;
                        if let Some(g) = gfx::notch_path(&p, br, br.w * 0.5, a_ear.min(6.0)) {
                            p.fill_geometry(&g, Color::black(ap.collapsed_opacity));
                        }
                        let input = self.input.clone();
                        let mut ui = Ui {
                            p: &mut p,
                            input: &input,
                            fx: &mut fx,
                            cfg: &self.cfg,
                            bus: &self.bus,
                            now,
                            accent: self.accent,
                            hot: false,
                            interactive: false,
                        };
                        ui.p.alpha = sec_v.clamp(0.0, 1.0);
                        let slot = br.inset(5.0);
                        let mut no_bars = None;
                        draw_slot(&mut ui, &sa.left, slot, slot, true, &mut no_bars);
                        ui.p.alpha = 1.0;
                    }
                }
                for &(br, alpha, icon, accent, off, _) in &bubble_list {
                    if let Some(g) = gfx::notch_path(&p, br, br.w * 0.5, a_ear.min(6.0)) {
                        p.fill_geometry(&g, Color::black(ap.collapsed_opacity * alpha));
                    }
                    p.alpha = alpha;
                    let ic = br.center_square(br.w * 0.5);
                    let tint = accent.lerp(Color::white(1.0), 0.3);
                    p.icon(icon, ic, tint);
                    if off {
                        // a slash through the icon, with a dark edge so it reads on the icon
                        p.line(ic.x - 1.0, ic.y - 1.0, ic.right() + 1.0, ic.bottom() + 1.0, 4.6, Color::black(0.95));
                        p.line(ic.x - 1.0, ic.y - 1.0, ic.right() + 1.0, ic.bottom() + 1.0, 2.2, tint);
                    }
                    p.alpha = 1.0;
                    fx.animate = true;
                }
                if split_on {
                    if let Some(act) = self.activity.clone() {
                        let input = self.input.clone();
                        let mut ui = Ui {
                            p: &mut p,
                            input: &input,
                            fx: &mut fx,
                            cfg: &self.cfg,
                            bus: &self.bus,
                            now,
                            accent: self.accent,
                            hot: false,
                            interactive: false,
                        };
                        for (br, slot, left) in [(lb, &act.left, true), (rb, &act.right, false)] {
                            if let Some(g) = gfx::notch_path(ui.p, br, br.w * 0.5, a_ear.min(6.0)) {
                                ui.p.fill_geometry(&g, Color::black(ap.collapsed_opacity));
                            }
                            ui.p.alpha = split_v.clamp(0.0, 1.0);
                            let inner = br.inset(br.w * 0.2);
                            draw_slot(&mut ui, slot, inner, inner, left, &mut bars);
                        }
                        ui.p.alpha = 1.0;
                    }
                }
                let opaque = if ap.blur { ap.expanded_opacity } else { ap.expanded_opacity.max(0.95) };
                let bg_alpha = lerp(ap.collapsed_opacity, opaque, a_glass);
                // no blur while moving: stay a little more solid so it doesn't flash
                let bg_alpha = if ap.blur && see_through && !settled { bg_alpha.max(0.94) } else { bg_alpha };
                if let Some(g) = gfx::notch_path(&p, body, a_r, a_ear) {
                    let bh = self.snap_band.as_ref().map_or(BAND_H, |b| b.rect.h);
                    if band_k > 0.01 && body.h > bh * 2.0 {
                        // Alcove: the glass turns see-through toward the bottom edge
                        let from = 1.0 - bh / body.h;
                        let low = bg_alpha * (1.0 - 0.74 * band_k);
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[(0.0, Color::black(bg_alpha)), (from, Color::black(bg_alpha)), (1.0, Color::black(low))],
                        );
                    } else {
                        p.fill_geometry(&g, Color::black(bg_alpha));
                    }
                }

                p.push_clip(body);
                let input = self.input.clone();
                let accent = self.accent;
                let mut ui = Ui {
                    p: &mut p,
                    input: &input,
                    fx: &mut fx,
                    cfg: &self.cfg,
                    bus: &self.bus,
                    now,
                    accent,
                    hot: false,
                    interactive: false,
                };

                // compact live activity / idle indicators
                if a_compact > 0.003 || self.mode == Mode::Idle {
                    ui.p.alpha = if self.activity.is_some() { a_compact } else { 1.0 - a_peek.max(a_exp) };
                    let acts = if split_on { None } else { self.activity.as_ref() };
                    draw_compact(&mut ui, body, acts, &self.indicators, ap.notch_height, &mut bars);
                    if self.activity.is_none() && !glances.is_empty() {
                        draw_idle_widgets(&mut ui, body, &glances, ap.notch_height);
                    }
                }
                // peek banner
                if a_peek > 0.003 {
                    if let Some(pk) = &self.peek {
                        ui.p.alpha = a_peek;
                        let s = 0.92 + 0.08 * a_peek;
                        ui.p.set_local(s, body.cx(), 0.0, 0.0, 0.0);
                        ui.interactive = a_peek > 0.9;
                        let left = pk.until.saturating_duration_since(now).as_secs_f32()
                            / pk.peek.duration.as_secs_f32().max(0.1);
                        let swap = pk.swap.as_ref().and_then(|(at, old)| {
                            let t = now.saturating_duration_since(*at).as_secs_f32() / SWAP_SECS;
                            (t < 1.0).then_some((t, &**old))
                        });
                        let band = if band_k > 0.01 && self.mode == Mode::Peek && swap.is_none() {
                            self.snap_band.as_ref()
                        } else {
                            None
                        };
                        if let Some(b) = band {
                            ui.p.push_clip(Rect::new(0.0, -200.0, self.win_w, b.rect.y + 200.0));
                        }
                        draw_peek(&mut ui, body, &pk.peek, left.clamp(0.0, 1.0), swap);
                        if let Some(b) = band {
                            ui.p.pop_clip();
                            bottom_fade(&mut ui, b, body.bottom(), a_r, band_k, self.scale);
                        }
                        ui.p.reset_local();
                    }
                }
                // expanded
                if a_exp > 0.003 {
                    ui.p.alpha = a_exp;
                    let s = 0.94 + 0.06 * a_exp;
                    ui.p.set_local(s, body.cx(), 0.0, 0.0, 0.0);
                    ui.interactive = a_exp > 0.85 && self.mode == Mode::Expanded;
                    let tw = page_w;
                    let full = Rect::new(
                        body.cx() - tw * 0.5,
                        body.y,
                        tw,
                        // an open page is laid out at its own height even while the body is still catching up, so
                        // nothing drifts or stretches mid-resize; while closing the page keeps the body's height
                        if self.mode == Mode::Expanded { target_h } else { target_h.max(body.h) },
                    );
                    match &self.snap_all {
                        // opening or closing: the finished picture is only faded and scaled
                        Some(snap) => {
                            ui.interactive = false;
                            ui.p.snapshot(snap, 0.0, 0.0, 1.0);
                        }
                        None => {
                            let trans =
                                self.page_from.map(|(from, dir)| (from, dir, self.anim.page.value.clamp(0.0, 1.0)));
                            let pages = self.snap_pages.as_ref().map(|(a, b)| (a, b));
                            // the strip along the bottom is drawn from its own picture (sharp, then
                            // softer toward the rim), so the live page stops just above it
                            let band = if band_k > 0.01 && self.mode == Mode::Expanded { self.snap_band.as_ref() } else { None };
                            if let Some(b) = band {
                                ui.p.push_clip(Rect::new(0.0, -200.0, self.win_w, b.rect.y + 200.0));
                            }
                            draw_expanded(
                                &mut ui,
                                full,
                                body,
                                &mut self.modules,
                                &self.started,
                                &tabs,
                                self.page,
                                trans,
                                &self.indicators,
                                pages,
                            );
                            if let Some(b) = band {
                                ui.p.pop_clip();
                                bottom_fade(&mut ui, b, body.bottom(), a_r, band_k, self.scale);
                            }
                        }
                    }
                    ui.p.reset_local();
                }
                ui.p.alpha = 1.0;
                if crate::ui::look_alcove() {
                    if let Some(t0) = self.opened_at {
                        let t = now.saturating_duration_since(t0).as_secs_f32() / 0.6;
                        if t < 1.0 && self.mode == Mode::Expanded {
                            // a soft band of light crosses the top of the notch as it opens
                            let e = 1.0 - (1.0 - t).powi(3);
                            let cx = body.x - 90.0 + (body.w + 180.0) * e;
                            let band = [
                                (0.0, Color::white(0.0)),
                                (0.5, Color::white(0.13 * (1.0 - t))),
                                (1.0, Color::white(0.0)),
                            ];
                            let h = (body.h * 0.55).min(70.0);
                            let pts = [
                                (cx - 90.0, body.y),
                                (cx + 90.0, body.y),
                                (cx + 60.0, body.y + h),
                                (cx - 120.0, body.y + h),
                            ];
                            ui.p.fill_gradient_poly(&pts, &band, cx - 90.0, cx + 90.0);
                            ui.fx.animate = true;
                        }
                    }
                }
                p.pop_clip();
            }
        }
        if let Err(e) = self.host.end_draw() {
            log!("EndDraw failed: {e}");
            self.recover_device();
        }
        self.res.end_frame();

        // compositor-driven visualizer
        let show_bars = self.cfg.media.visualizer && self.mode == Mode::Idle;
        match bars {
            Some((r, c, playing)) if show_bars && a_compact > 0.01 => {
                let s = self.scale;
                let pr = Rect::new(r.x * s, r.y * s, r.w * s, r.h * s);
                self.host.bars.update(pr, c, a_compact * self.anim.shown.value, playing);
            }
            _ => self.host.bars.hide(),
        }

        // blur behind the notch body
        let s = self.scale;
        let bp = Rect::new(body.x * s, body.y * s, body.w * s, body.h.max(0.0) * s);
        // The live blur is the costliest thing the compositor does for us:
        // skip it while the panel is opaque anyway and while it is still moving.
        let blur = if ap.blur && ((see_through && settled) || band_k > 0.01) { a_glass } else { 0.0 };
        // The frosted layer is dark and strong; under the see-through bottom edge it is only
        // blended in lightly, so the page behind still shows through softly.
        let blur = if band_k > 0.01 && !see_through { blur * 0.4 } else { blur };
        self.host.set_backdrop(bp, a_r * s, blur * self.anim.shown.value);

        if let Some(page) = fx.open_page.take() {
            if self.mode == Mode::Expanded && self.page_available(page) {
                if self.page != page {
                    self.switch_page(page);
                    self.set_keyboard(page == "ai");
                }
            } else {
                fx.open_page = Some(page);
            }
        }
        if self.mode == Mode::Expanded && self.page == HOME {
            let st = unsafe { GetLocalTime() };
            self.clock_min = st.wSecond;
        }
        fx
    }

    fn recover_device(&mut self) {
        if self.gfx.recreate_devices().is_ok() {
            let _ = self.host.set_rendering_device(&self.gfx);
            self.snap_all = None;
            self.snap_pages = None;
            if let Ok(r) = Res::new(&self.gfx) {
                self.res = r;
            }
            if let Some(d) = self.dock.as_mut() {
                d.recover(&self.gfx);
            }
        }
        self.request_frame();
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers (free functions to keep borrows simple)
// ---------------------------------------------------------------------------

fn draw_compact(
    ui: &mut Ui,
    body: Rect,
    activity: Option<&Activity>,
    indicators: &[Indicator],
    base_h: f32,
    bars: &mut Option<(Rect, Color, bool)>,
) {
    let h = body.h.min(base_h + 6.0);
    let slot = (base_h - 10.0).max(10.0);
    let cy = body.y + h * 0.5;
    let mut right_edge = body.right() - 10.0;
    if let Some(a) = activity {
        let wing = a.wing * base_h;
        let lr = Rect::new(body.x + 9.0, cy - slot * 0.5, slot, slot);
        let rr = Rect::new(body.right() - 9.0 - slot, cy - slot * 0.5, slot, slot);
        let lw = Rect::new(body.x + 9.0, cy - slot * 0.5, wing - 6.0, slot);
        let rw = Rect::new(body.right() - 9.0 - (wing - 6.0), cy - slot * 0.5, wing - 6.0, slot);
        draw_slot(ui, &a.left, lr, lw, true, bars);
        draw_slot(ui, &a.right, rr, rw, false, bars);
        right_edge = body.right() - wing - 4.0;
    }
    // privacy dots (mic / camera)
    let mut x = right_edge;
    for ind in indicators.iter().filter(|i| i.dot).rev() {
        ui.p.fill_circle(x - 3.0, cy, 3.2, ind.color);
        x -= 10.0;
    }
}

/// Up to three small widgets inside the closed notch, side by side.
/// One closed-notch widget: icon, text, colour and an optional 0..1 tick gauge.
type Glance = (Option<Icon>, String, Color, Option<f32>);

fn draw_idle_widgets(ui: &mut Ui, body: Rect, items: &[Glance], base_h: f32) {
    // fades in as the pill grows into a notch
    let k = ((body.h - 8.0) / (base_h - 8.0).max(1.0)).clamp(0.0, 1.0);
    if k < 0.05 || items.is_empty() {
        return;
    }
    let alpha = ui.p.alpha;
    ui.p.alpha = alpha * k;
    let h = body.h.min(base_h + 6.0);
    let cy = body.y + h * 0.5;
    let n = items.len().min(3);
    let slot_w = ((body.w - 20.0) / n as f32).min(86.0);
    let x0 = body.cx() - slot_w * n as f32 * 0.5;
    let al = crate::ui::look_alcove();
    let base = if al {
        // Alcove: plain white text, a little larger, no dividers and no tick gauges
        TextStyle::new((base_h * 0.46).clamp(12.0, 15.0), Color::white(0.96)).semibold()
    } else {
        TextStyle::new((base_h * 0.42).clamp(11.0, 14.0), palette::TEXT).semibold().display()
    };
    for (i, (icon, text, color, gauge)) in items.iter().take(3).enumerate() {
        let st = if al { base } else { base.color(*color) };
        let slot = Rect::new(x0 + i as f32 * slot_w, body.y, slot_w, h);
        // a single faint tick between neighbours
        if i > 0 && !al {
            ui.p.fill_rounded(Rect::new(slot.x - 0.75, cy - 5.0, 1.5, 10.0), 0.75, Color::white(0.16));
        }
        let isz = 13.0;
        let (iw, gap) = if icon.is_some() { (isz, 4.0) } else { (0.0, 0.0) };
        let (tw, _) = ui.p.measure(text, &st);
        let gw = if gauge.is_some() {
            if al {
                28.0
            } else {
                30.0
            }
        } else {
            0.0
        };
        let gg = if gauge.is_some() { 6.0 } else { 0.0 };
        let tw = tw.min((slot_w - iw - gap - gw - gg - 6.0).max(0.0));
        let x = slot.cx() - (iw + gap + tw + gg + gw) * 0.5;
        if let Some(ic) = icon {
            let ic_color = if al { Color::white(0.85) } else { color.with_a(0.9) };
            ui.p.icon(*ic, Rect::new(x, cy - isz * 0.5, isz, isz), ic_color);
        }
        ui.p.text(text, Rect::new(x + iw + gap, body.y, tw + 2.0, h), &st);
        if let Some(g) = gauge {
            if al {
                crate::alcove::battery_glyph(ui, Rect::new(x + iw + gap + tw + gg, cy - 6.5, gw, 13.0), *g, false);
            } else {
                ui.beams(|ui| ui.ticks(Rect::new(x + iw + gap + tw + gg, cy - 4.0, gw, 8.0), *g, *color));
            }
        }
    }
    ui.p.alpha = alpha;
}

fn clock_text() -> String {
    let st = unsafe { GetLocalTime() };
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&st), PCWSTR::null(), Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

fn date_text() -> String {
    let st = unsafe { GetLocalTime() };
    let day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][(st.wDayOfWeek % 7) as usize];
    format!("{day} {}", st.wDay)
}

fn draw_slot(ui: &mut Ui, slot: &Slot, sq: Rect, wing: Rect, left: bool, bars: &mut Option<(Rect, Color, bool)>) {
    match slot {
        Slot::Empty => {}
        Slot::Image(img) => ui.p.image(img, sq, sq.w * 0.26, 1.0),
        Slot::Icon(ic, c) => ui.p.icon(*ic, sq.inset(sq.w * 0.06), *c),
        Slot::Text(s, c) => {
            let st = TextStyle::new((sq.h * 0.62).clamp(11.0, 15.0), *c).semibold().display();
            let st = if left { st } else { st.right() };
            ui.p.text(s, wing, &st);
        }
        Slot::Bars { color, playing } => {
            let r = Rect::new(sq.x + sq.w * 0.12, sq.y + sq.h * 0.18, sq.w * 0.76, sq.h * 0.64);
            *bars = Some((r, *color, *playing));
        }
        Slot::Ring(progress, c) => {
            let (cx, cy) = (sq.cx(), sq.cy());
            let d = sq.w * 0.8;
            let w = (sq.w * 0.12).max(2.0);
            match progress {
                Some(f) => ui.ring(cx, cy, d, w, *f, *c),
                None => ui.ring_spin(cx, cy, d, w, *c),
            }
        }
        Slot::Battery { level, charging } => {
            let c = battery_color(*level, *charging);
            icons::battery(ui.p, sq.inset_xy(0.0, sq.h * 0.18), *level, *charging, c);
        }
        Slot::Vinyl { color, angle, art, flip } if crate::ui::look_alcove() => {
            // Alcove: no record, the cover itself (or a note) in a rounded square
            let (mut img, mut k) = (art.as_ref(), 1.0f32);
            if let Some((p, old)) = flip {
                let th = p.clamp(0.0, 1.0) * std::f32::consts::PI;
                k = th.cos().abs().max(0.02);
                if *p < 0.5 {
                    img = old.as_ref();
                }
                ui.animate();
            }
            let s = sq.inset(sq.w * 0.06);
            let r = Rect::centered(s.cx(), s.cy(), s.w * k, s.h * (0.94 + 0.06 * k));
            match img {
                Some(i) => ui.p.image(i, r, s.w * 0.26 * k.max(0.4), 1.0),
                None => {
                    ui.p.fill_rounded(r, s.w * 0.26 * k.max(0.4), color.lerp(Color::black(1.0), 0.6));
                    ui.p.icon(Icon::Music, Rect::centered(r.cx(), r.cy(), r.w * 0.55, s.h * 0.55), Color::white(0.95));
                }
            }
        }
        Slot::Vinyl { color, angle, art, .. } => ui.vinyl(sq.inset(sq.w * 0.04), *angle, *color, art.as_ref()),
        Slot::Wave { levels, color } => {
            let r = Rect::new(wing.x, wing.y + wing.h * 0.12, wing.w, wing.h * 0.76);
            modules::voice::draw_wave(ui, r, levels, *color);
        }
    }
}

pub fn battery_color(level: f32, charging: bool) -> Color {
    if charging {
        palette::GREEN
    } else if level <= 0.1 {
        palette::RED
    } else if level <= 0.2 {
        palette::YELLOW
    } else {
        Color::white(0.92)
    }
}

/// Glyph inside a peek's status badge: a check for green, a "!" style info
/// mark for warnings, nothing (a plain dot) for other colours.
#[allow(dead_code)]
fn badge_glyph(c: Color) -> Option<Icon> {
    if c == palette::GREEN {
        Some(Icon::Check)
    } else if c == palette::RED || c == palette::ORANGE || c == palette::YELLOW {
        Some(Icon::Info)
    } else {
        None
    }
}

// Volume / brightness: the icon top left, the value top right and a bar of
// thin ticks underneath.
// Pointer dwell on the volume peek's speaker icon: (since, already switched).
thread_local! {
    static HUD_DWELL: std::cell::Cell<(Option<Instant>, bool)> = const { std::cell::Cell::new((None, false)) };
}
const HUD_DWELL_SECS: f32 = 0.7;

/// The sweep of a dial's ring for a level (0..1): clockwise or counter-clockwise by the setting.
fn dial_sweep(cfg: &Config, level: f32) -> f32 {
    let sweep = std::f32::consts::TAU * level;
    if cfg.general.hud_clockwise {
        sweep
    } else {
        -sweep
    }
}

/// A device connecting or leaving as one big round dial: the device's icon in a ring that shows
/// its battery (full when connected without a reading, empty when gone), the name and the state.
fn draw_device_dial(ui: &mut Ui, body: Rect, pk: &Peek) {
    let on = pk.subtitle.starts_with("Connected");
    let battery = match &pk.trailing {
        Trailing::Battery { level, .. } => Some(*level),
        _ => None,
    };
    let (cx, cy) = (body.cx(), body.y + ui.cfg.appearance.notch_height + 66.0);
    let target = battery.unwrap_or(if on { 1.0 } else { 0.0 });
    let level = ui.glide(gkey("alcove-device-dial", 0), target);
    let fill = if !on {
        Color::white(0.38)
    } else if battery.is_some_and(|l| l < 0.2) {
        palette::RED
    } else {
        Color::white(0.97)
    };
    ui.p.stroke_circle(cx, cy, 46.0, 12.0, Color::white(0.12));
    ui.p.arc(cx, cy, 46.0, 0.0, dial_sweep(ui.cfg, level), 12.0, fill);
    let ic = Rect::centered(cx, cy, 40.0, 40.0);
    let tint = if on { Color::white(0.97) } else { Color::white(0.5) };
    ui.p.icon(pk.icon, ic, tint);
    if !on {
        // a slash through the icon when the device has gone
        ui.p.line(ic.x + 4.0, ic.y + 4.0, ic.right() - 4.0, ic.bottom() - 4.0, 4.6, Color::black(0.95));
        ui.p.line(ic.x + 4.0, ic.y + 4.0, ic.right() - 4.0, ic.bottom() - 4.0, 2.2, tint);
    }
    ui.p.text(
        &pk.title,
        Rect::new(body.x + 10.0, cy + 58.0, body.w - 20.0, 28.0),
        &TextStyle::new(20.0, Color::white(0.95)).semibold().center(),
    );
    ui.p.text(
        &pk.subtitle,
        Rect::new(body.x + 10.0, cy + 88.0, body.w - 20.0, 20.0),
        &TextStyle::new(13.0, palette::TEXT_DIM).medium().center(),
    );
}

fn draw_hud(ui: &mut Ui, body: Rect, pk: &Peek, frac: f32, color: Color, text: Option<&str>) {
    let pad = 24.0;
    let row_y = body.bottom() - 36.0;
    // Alcove: one big round dial instead of the slider (setting: General, round dial)
    let dial = dial_height(pk, ui.cfg).is_some();
    let dial_cy = body.y + ui.cfg.appearance.notch_height + 66.0;
    let ir =
        if dial { Rect::centered(body.cx(), dial_cy, 40.0, 40.0) } else { Rect::new(body.x + pad, row_y, 24.0, 24.0) };
    // Volume only: rest the pointer on the speaker for a moment to move to the next output device
    let mut hint = false;
    if pk.key == Some("osd-volume") {
        let zone = ir.expand(8.0);
        let over = ui.hovered(zone);
        let go = HUD_DWELL.with(|d| {
            let (since, fired) = d.get();
            match (over, since) {
                (false, _) => {
                    d.set((None, false));
                    (0.0, false)
                }
                (true, None) => {
                    d.set((Some(ui.now), false));
                    (0.0, false)
                }
                (true, Some(t)) => {
                    let k = (ui.now.saturating_duration_since(t).as_secs_f32() / HUD_DWELL_SECS).min(1.0);
                    if k >= 1.0 && !fired {
                        d.set((Some(t), true));
                        (1.0, true)
                    } else {
                        (k, false)
                    }
                }
            }
        });
        if over {
            ui.hot = true;
            hint = true;
            if go.0 < 1.0 {
                ui.animate();
            }
        }
        if go.1 {
            modules::osd::request_next_output(ui.bus);
        }
    }
    if dial {
        let fill = if color.a < 0.9 {
            Color::white(0.38)
        } else if color == palette::GREEN || color == palette::RED || color == palette::ORANGE {
            color
        } else {
            Color::white(0.97)
        };
        let level = ui.glide(gkey("alcove-hud", 0), frac.clamp(0.0, 1.0));
        // a thick ring with round ends that fills from the top, in the direction chosen in Settings
        ui.p.stroke_circle(body.cx(), dial_cy, 46.0, 12.0, Color::white(0.12));
        ui.p.arc(body.cx(), dial_cy, 46.0, 0.0, dial_sweep(ui.cfg, level), 12.0, fill);
        let battery = pk.key == Some("battery");
        if battery {
            // a battery in the middle of the ring (with a bolt while charging)
            let charging = text.is_some_and(|t| t.starts_with("Charging") || t.starts_with("Fully"));
            crate::alcove::battery_glyph(ui, Rect::centered(body.cx(), dial_cy, 46.0, 24.0), frac, charging);
        } else {
            ui.p.icon(pk.icon, ir, fill.lerp(Color::white(1.0), 0.2));
        }
        let label = Rect::new(body.x + 8.0, dial_cy + 60.0, body.w - 16.0, 30.0);
        // the big line, and a small one under it ("Charging" over "45%")
        let pct = format!("{:.0}%", frac.clamp(0.0, 1.0) * 100.0);
        let (big, small) = match text {
            Some(t) if battery => match t.split_once(" · ") {
                Some((status, p)) => (p.to_string(), Some(status.to_string())),
                None => (pct, Some(t.to_string())),
            },
            Some(t) => (t.to_string(), None),
            None => (pct, None),
        };
        if hint {
            ui.p.text("Switch output", label, &TextStyle::new(13.0, palette::TEXT_DIM).medium().center());
        } else {
            ui.p.text(&big, label, &TextStyle::new(24.0, Color::white(0.95)).semibold().display().center());
            if let Some(sm) = small {
                let line = Rect::new(body.x + 8.0, dial_cy + 90.0, body.w - 16.0, 20.0);
                ui.p.text(&sm, line, &TextStyle::new(13.0, palette::TEXT_DIM).medium().center());
            }
        }
        return;
    }
    if crate::ui::look_alcove() {
        // a round, quiet pill: the icon, then one thick capsule that fills with the level
        let fill = if color.a < 0.9 {
            Color::white(0.38)
        } else if color == palette::GREEN || color == palette::RED || color == palette::ORANGE {
            color
        } else {
            Color::white(0.97)
        };
        ui.p.icon(pk.icon, ir, fill.lerp(Color::white(1.0), 0.2));
        let level = ui.glide(gkey("alcove-hud", 0), frac.clamp(0.0, 1.0));
        let bx = ir.right() + 16.0;
        if hint {
            ui.p.text("Switch output", Rect::new(bx, ir.y, 140.0, ir.h), &TextStyle::new(12.5, palette::TEXT_DIM));
        } else {
            let tw = match text {
                Some(t) => ui.p.measure(t, &TextStyle::new(13.0, Color::white(0.85)).medium()).0 + 14.0,
                None => 0.0,
            };
            let bar = Rect::new(bx, ir.cy() - 6.5, (body.right() - pad - tw - bx).max(30.0), 13.0);
            crate::alcove::meter(ui, bar, level, fill);
            if let Some(t) = text {
                ui.p.text(
                    t,
                    Rect::new(bar.right() + 10.0, ir.y, tw, ir.h),
                    &TextStyle::new(13.0, Color::white(0.85)).medium().right(),
                );
            }
        }
        return;
    }
    ui.p.icon(pk.icon, ir, color);
    let frac = frac.clamp(0.0, 1.0);
    let value = text.map(str::to_string).unwrap_or_else(|| format!("{:.0}", frac * 100.0));
    let vs = TextStyle::new(13.0, Color::white(0.80)).medium().display().right();
    let vw = 46.0;
    ui.p.text(&value, Rect::new(body.right() - pad - vw, ir.y, vw, ir.h), &vs);
    let bx = if hint { ir.right() + 14.0 } else { ir.right() + 16.0 };
    if hint {
        ui.p.text("Switch output", Rect::new(bx, ir.y, 110.0, ir.h), &TextStyle::new(11.5, palette::TEXT_DIM));
    } else {
        let bar = Rect::new(bx, ir.cy() - 7.0, (body.right() - pad - vw - 12.0 - bx).max(20.0), 14.0);
        if color.a >= 0.9 {
            ui.ticks_hud(bar, frac); // blue with white highlights
        } else {
            ui.beams(|ui| ui.ticks_ex(bar, frac, color, true)); // muted: stays quiet
        }
    }
}

thread_local! {
    /// The last fitted hello text: (font|title, size, width).
    static HELLO_FIT: std::cell::RefCell<Option<(String, f32, f32)>> = const { std::cell::RefCell::new(None) };
}

/// When (0..1 of the peek) the scan gives way to the handwritten hello.
const SCAN_HELLO_AT: f32 = 0.46;

/// The unlock greeting in the Alcove look: a big open padlock in the middle of the same thick ring
/// the volume dial uses. The ring fills, the lock opens, then "Hello" and the name come in below.
fn draw_scan_alcove(ui: &mut Ui, body: Rect, pk: &Peek, t: f32) {
    let alpha = ui.p.alpha;
    let cx = body.cx();
    let cy = body.y + ui.cfg.appearance.notch_height + 66.0;
    let pop = smoothstep(0.0, 0.12, t);
    let fade = 1.0 - smoothstep(0.90, 1.0, t);
    let fill = smoothstep(0.06, 0.42, t);
    let done = smoothstep(0.40, 0.48, t);

    // the ring: a faint track and a thick arc with round ends, in the direction chosen in Settings
    let r = 46.0 * (0.9 + 0.1 * pop);
    ui.p.alpha = alpha * pop * fade;
    ui.p.stroke_circle(cx, cy, r, 12.0, Color::white(0.12));
    ui.p.arc(cx, cy, r, 0.0, dial_sweep(ui.cfg, fill), 12.0, Color::white(0.97));

    // the closed lock gives way to the open one
    ui.p.alpha = alpha * pop * fade * (1.0 - done);
    ui.p.icon(Icon::Lock, Rect::centered(cx, cy, 40.0, 40.0), Color::white(0.97));
    let k = 0.75 + 0.25 * done;
    ui.p.alpha = alpha * fade * done;
    ui.p.icon(Icon::Unlock, Rect::centered(cx, cy, 44.0 * k, 44.0 * k), Color::white(1.0));

    // "Hello" first, the name a beat later
    let hello = smoothstep(0.50, 0.62, t);
    ui.p.alpha = alpha * hello * fade;
    ui.p.text(
        "Hello",
        Rect::new(body.x + 8.0, cy + 60.0 + 8.0 * (1.0 - hello), body.w - 16.0, 34.0),
        &TextStyle::new(28.0, Color::white(0.97)).semibold().display().center(),
    );
    let name = hello_name(&pk.title);
    if !name.is_empty() {
        let nm = smoothstep(0.62, 0.74, t);
        ui.p.alpha = alpha * nm * fade;
        let mut size = 20.0;
        let (nw, _) = ui.p.measure(name, &TextStyle::new(size, Color::white(1.0)).semibold());
        size *= ((body.w - 36.0) / nw.max(1.0)).min(1.0);
        ui.p.text(
            name,
            Rect::new(body.x + 8.0, cy + 96.0 + 8.0 * (1.0 - nm), body.w - 16.0, 28.0),
            &TextStyle::new(size, Color::white(0.62)).semibold().center(),
        );
    }
    ui.p.alpha = alpha;
    if t < 0.95 {
        ui.animate();
    }
}

/// The name after "Hello" in the peek title ("" for a plain hello).
fn hello_name(title: &str) -> &str {
    title.strip_prefix("Hello").map_or("", str::trim)
}

/// Size of the stage for the hello: (width, height). A name adds a line below.
fn hello_stage(title: &str, custom_font: bool) -> (f32, f32) {
    let name = hello_name(title);
    if custom_font {
        ((52.0 + title.chars().count() as f32 * 22.0).clamp(210.0, 440.0), 112.0)
    } else if name.is_empty() {
        (210.0, 112.0)
    } else {
        ((52.0 + name.chars().count() as f32 * 17.0).clamp(210.0, 420.0), 158.0)
    }
}

/// "hello" as one continuous pen stroke (monoline cursive), in a 240 x 110 box.
const HELLO_PATH: [(f32, f32); 63] = [
    (10.0, 96.0),
    (18.0, 72.0),
    (28.0, 40.0),
    (37.0, 17.0),
    (43.0, 9.0),
    (50.0, 11.0),
    (51.0, 22.0),
    (46.0, 44.0),
    (38.0, 68.0),
    (33.0, 86.0),
    (34.0, 80.0),
    (38.0, 68.0),
    (46.0, 62.0),
    (55.0, 60.0),
    (63.0, 65.0),
    (65.0, 76.0),
    (65.0, 86.0),
    (70.0, 90.0),
    (77.0, 84.0),
    (83.0, 72.0),
    (91.0, 64.0),
    (98.0, 66.0),
    (97.0, 75.0),
    (88.0, 79.0),
    (79.0, 80.0),
    (83.0, 88.0),
    (93.0, 91.0),
    (102.0, 85.0),
    (110.0, 66.0),
    (119.0, 37.0),
    (125.0, 15.0),
    (130.0, 8.0),
    (135.0, 13.0),
    (131.0, 40.0),
    (125.0, 68.0),
    (122.0, 84.0),
    (127.0, 91.0),
    (135.0, 86.0),
    (142.0, 66.0),
    (151.0, 37.0),
    (157.0, 15.0),
    (162.0, 8.0),
    (167.0, 13.0),
    (163.0, 40.0),
    (157.0, 68.0),
    (154.0, 84.0),
    (159.0, 91.0),
    (167.0, 86.0),
    (173.0, 75.0),
    (180.0, 64.0),
    (190.0, 60.0),
    (198.0, 66.0),
    (198.0, 78.0),
    (190.0, 89.0),
    (179.0, 90.0),
    (172.0, 82.0),
    (173.0, 72.0),
    (182.0, 64.0),
    (195.0, 59.0),
    (210.0, 56.0),
    (226.0, 51.0),
    (226.0, 51.0),
    (226.0, 51.0),
];

/// The pen path as evenly spaced points (smoothed with a Catmull-Rom spline), built once.
fn hello_points() -> &'static [(f32, f32)] {
    static PTS: std::sync::OnceLock<Vec<(f32, f32)>> = std::sync::OnceLock::new();
    PTS.get_or_init(|| {
        // the last points only pad the end of the spline
        let src = &HELLO_PATH[..HELLO_PATH.len() - 2];
        let n = src.len();
        let at = |i: isize| -> (f32, f32) {
            if i < 0 {
                (2.0 * src[0].0 - src[1].0, 2.0 * src[0].1 - src[1].1)
            } else if i as usize >= n {
                (2.0 * src[n - 1].0 - src[n - 2].0, 2.0 * src[n - 1].1 - src[n - 2].1)
            } else {
                src[i as usize]
            }
        };
        let mut dense = Vec::new();
        for i in 0..(n as isize - 1) {
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            for k in 0..16 {
                let t = k as f32 / 16.0;
                let f = |a: f32, b: f32, c: f32, d: f32| {
                    0.5 * (2.0 * b
                        + (-a + c) * t
                        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
                        + (-a + 3.0 * b - 3.0 * c + d) * t * t * t)
                };
                dense.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
            }
        }
        dense.push(src[n - 1]);
        // resample at a fixed spacing so the pen moves at an even speed
        let step = 2.2f32;
        let mut out = vec![dense[0]];
        let mut carry = 0.0f32;
        for w in dense.windows(2) {
            let (a, b) = (w[0], w[1]);
            let seg = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let mut d = step - carry;
            while d <= seg {
                let k = d / seg.max(0.0001);
                out.push((a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k));
                d += step;
            }
            carry = seg - (d - step);
        }
        out
    })
}

/// Face-ID style unlock animation, in the app's blue: four corner brackets
/// close in, a ring of beams fills while a smiley waits, the smiley turns into
/// a check, then the stage shrinks and "Hello" (or "Hello <name>") is written
/// out in handwriting with a beam at the pen's edge. `t` runs 0..1 over the peek.
fn draw_scan(ui: &mut Ui, body: Rect, pk: &Peek, t: f32) {
    if crate::ui::look_alcove() {
        return draw_scan_alcove(ui, body, pk, t);
    }
    let blue = palette::BLUE;
    let cx = body.cx();
    let cy = body.y + 66.0;
    let alpha = ui.p.alpha;
    // everything of the scan fades out as the stage shrinks
    let gone = smoothstep(0.40, 0.50, t);
    let scan_a = alpha * (1.0 - gone);

    if gone < 0.999 {
        // corner brackets: fly in from slightly outside
        let appear = smoothstep(0.0, 0.08, t);
        let half = 40.0 * (1.0 + 0.25 * (1.0 - appear)) * (1.0 - 0.1 * gone);
        ui.p.alpha = scan_a * appear;
        let (arm, th) = (17.0, 3.4);
        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let (x, y) = (cx + sx * half, cy + sy * half);
            let hx = if sx < 0.0 { x - th * 0.5 } else { x - arm + th * 0.5 };
            let vy = if sy < 0.0 { y - th * 0.5 } else { y - arm + th * 0.5 };
            ui.p.fill_rounded(Rect::new(hx, y - th * 0.5, arm, th), th * 0.5, blue);
            ui.p.fill_rounded(Rect::new(x - th * 0.5, vy, th, arm), th * 0.5, blue);
        }

        // ring of beams filling while it "scans"
        ui.p.alpha = scan_a * smoothstep(0.02, 0.1, t);
        ui.ring(cx, cy, 62.0, 4.0, smoothstep(0.06, 0.30, t), blue);

        // smiley → check
        let done = smoothstep(0.26, 0.34, t);
        ui.p.alpha = scan_a * (1.0 - done) * smoothstep(0.02, 0.1, t);
        let blink = 1.0 - 0.8 * (smoothstep(0.16, 0.19, t) - smoothstep(0.19, 0.22, t));
        draw_smiley(ui, cx, cy, blink, blue);
        ui.p.alpha = scan_a * done;
        let k = 0.6 + 0.4 * done;
        ui.p.icon(Icon::Check, Rect::centered(cx, cy, 30.0 * k, 30.0 * k), Color::white(1.0));

        // label and a bar of beams underneath
        ui.p.alpha = scan_a * smoothstep(0.32, 0.40, t);
        ui.p.text(
            "Unlocked",
            Rect::new(body.x + 12.0, cy + 52.0, body.w - 24.0, 20.0),
            &TextStyle::new(14.0, palette::TEXT).semibold().center(),
        );
        ui.p.alpha = scan_a * smoothstep(0.0, 0.1, t);
        ui.ticks(Rect::new(body.x + 40.0, body.bottom() - 22.0, body.w - 80.0, 8.0), smoothstep(0.04, 0.40, t), blue);
    }

    // the handwritten hello: "hello" is drawn by a pen along one flowing stroke,
    // a name (if there is one) is written below it in a script font
    if t > SCAN_HELLO_AT {
        let name = hello_name(&pk.title);
        let fade_in = smoothstep(SCAN_HELLO_AT, SCAN_HELLO_AT + 0.05, t);
        let ease = |x: f32| x * x * (3.0 - 2.0 * x);

        // an installed handwriting font was chosen: write the whole greeting with it
        let font = ui.cfg.greeting.font_name().to_string();
        if ui.p.gfx.font_installed(&font) {
            gfx::set_hello_font(&font);
            let stage_w = hello_stage(&pk.title, true).0;
            // measuring builds a text layout, so the fitted size is worked out once per greeting
            let key = format!("{font}|{}", pk.title);
            let cached = HELLO_FIT.with(|c| c.borrow().as_ref().filter(|f| f.0 == key).map(|f| (f.1, f.2)));
            let (size, tw) = match cached {
                Some(v) => v,
                None => {
                    let probe = TextStyle::new(48.0, blue).hello_font();
                    let (pw, _) = ui.p.measure(&pk.title, &probe);
                    let size = 48.0 * ((stage_w - 44.0) / pw.max(1.0)).min(1.0);
                    let (tw, _) = ui.p.measure(&pk.title, &TextStyle::new(size, blue).hello_font());
                    HELLO_FIT.with(|c| *c.borrow_mut() = Some((key, size, tw)));
                    (size, tw)
                }
            };
            let style = TextStyle::new(size, blue).hello_font().center();
            let box_h = size * 1.6;
            let ty = body.cy() - box_h * 0.5 - 8.0;
            let tx = cx - tw * 0.5 - 10.0;
            let wr = ease(smoothstep(SCAN_HELLO_AT + 0.04, 0.82, t));
            let reveal = (tw + 20.0) * wr;
            ui.p.alpha = alpha * fade_in;
            ui.p.push_clip(Rect::new(tx, ty - 6.0, reveal, box_h + 12.0));
            ui.p.text(&pk.title, Rect::new(tx, ty, tw + 20.0, box_h), &style);
            ui.p.pop_clip();
            let pen_a = alpha * fade_in * (1.0 - smoothstep(0.82, 0.90, t));
            if pen_a > 0.003 && wr > 0.0 && wr < 1.0 {
                let lead = crate::ui::beam_lead();
                let ex = tx + reveal;
                for (i, a) in [0.40f32, 0.20, 0.09].iter().enumerate() {
                    ui.p.alpha = pen_a * a;
                    ui.p.fill_rounded(
                        Rect::new(ex - 6.0 * (i as f32 + 1.0) - 1.2, ty + 4.0, 2.4, box_h - 8.0),
                        1.2,
                        blue,
                    );
                }
                ui.p.alpha = pen_a;
                ui.p.fill_rounded(Rect::new(ex - 1.7, ty, 3.4, box_h), 1.7, lead);
            }
            ui.p.alpha = alpha * smoothstep(SCAN_HELLO_AT + 0.02, SCAN_HELLO_AT + 0.08, t);
            ui.ticks(Rect::new(body.x + 40.0, body.bottom() - 20.0, body.w - 80.0, 8.0), wr, blue);
            ui.p.alpha = alpha;
            // nothing moves once the writing is done: no more frames until the peek ends
            if t < 0.92 {
                ui.animate();
            }
            return;
        }
        let path_end = if name.is_empty() { 0.76 } else { 0.70 };
        let wr = ease(smoothstep(SCAN_HELLO_AT + 0.04, path_end, t));
        let pts = hello_points();
        let s = 0.84f32;
        let (bw, bh) = (240.0 * s, 110.0 * s);
        let top = if name.is_empty() { body.cy() - bh * 0.5 - 8.0 } else { body.y + 16.0 };
        let (ox, oy) = (cx - bw * 0.5, top);
        let n_vis = ((pts.len() as f32) * wr) as usize;
        ui.p.alpha = alpha * fade_in;
        for &(px, py) in &pts[..n_vis.min(pts.len())] {
            ui.p.fill_circle(ox + px * s, oy + py * s, 1.6, blue);
        }
        // a beam rides at the pen with a short trail behind it
        let write_end = if name.is_empty() { path_end } else { 0.90 };
        let pen_a = alpha * fade_in * (1.0 - smoothstep(write_end, write_end + 0.08, t));
        let mut tip = (ox, oy);
        if pen_a > 0.003 && wr > 0.0 {
            let lead = crate::ui::beam_lead();
            let at_pen = |back: usize| {
                let i = n_vis.saturating_sub(back).min(pts.len() - 1);
                (ox + pts[i].0 * s, oy + pts[i].1 * s)
            };
            if wr < 1.0 {
                for (k, a) in [(30usize, 0.10f32), (20, 0.18), (10, 0.32)] {
                    let (x, y) = at_pen(k);
                    ui.p.alpha = pen_a * a;
                    ui.p.fill_rounded(Rect::new(x - 1.2, y - 11.0, 2.4, 22.0), 1.2, blue);
                }
                let (x, y) = at_pen(0);
                tip = (x, y);
                ui.p.alpha = pen_a;
                ui.p.fill_rounded(Rect::new(x - 1.7, y - 13.0, 3.4, 26.0), 1.7, lead);
            }
        }
        let _ = tip;

        let mut frac = wr;
        if !name.is_empty() {
            let nt = ease(smoothstep(0.72, 0.90, t));
            frac = 0.5 * wr + 0.5 * nt;
            let max_w = hello_stage(&pk.title, false).0 - 40.0;
            let probe = TextStyle::new(28.0, blue).script();
            let (pw, _) = ui.p.measure(name, &probe);
            let size = 28.0 * (max_w / pw.max(1.0)).min(1.0);
            let style = TextStyle::new(size, blue).script().center();
            let (tw, _) = ui.p.measure(name, &style);
            let box_h = size * 1.5;
            let ty = body.y + 16.0 + bh + 2.0;
            let tx = cx - tw * 0.5 - 8.0;
            let reveal = (tw + 16.0) * nt;
            ui.p.alpha = alpha * fade_in;
            ui.p.push_clip(Rect::new(tx, ty - 4.0, reveal, box_h + 8.0));
            ui.p.text(name, Rect::new(tx, ty, tw + 16.0, box_h), &style);
            ui.p.pop_clip();
            if nt > 0.0 && nt < 1.0 {
                let lead = crate::ui::beam_lead();
                let ex = tx + reveal;
                for (i, a) in [0.40f32, 0.20, 0.09].iter().enumerate() {
                    ui.p.alpha = alpha * a;
                    ui.p.fill_rounded(
                        Rect::new(ex - 6.0 * (i as f32 + 1.0) - 1.2, ty + 2.0, 2.4, box_h - 4.0),
                        1.2,
                        blue,
                    );
                }
                ui.p.alpha = alpha;
                ui.p.fill_rounded(Rect::new(ex - 1.7, ty, 3.4, box_h), 1.7, lead);
            }
        }

        // a rule of beams along the bottom fills as the pen moves
        ui.p.alpha = alpha * smoothstep(SCAN_HELLO_AT + 0.02, SCAN_HELLO_AT + 0.08, t);
        ui.ticks(Rect::new(body.x + 40.0, body.bottom() - 20.0, body.w - 80.0, 8.0), frac, blue);
    }
    ui.p.alpha = alpha;
    if t < 0.92 {
        ui.animate();
    }
}

/// The Face ID smiley: two eyes, a hooked nose and a smile, all round-capped strokes.
fn draw_smiley(ui: &mut Ui, cx: f32, cy: f32, eye_open: f32, color: Color) {
    let w = 3.6;
    // eyes (they squash towards their middle when blinking)
    let (top, bot) = (cy - 19.0, cy - 9.0);
    let mid = (top + bot) * 0.5;
    let half = (bot - top) * 0.5 * eye_open.clamp(0.1, 1.0);
    for x in [cx - 12.0, cx + 12.0] {
        ui.p.line(x, mid - half, x, mid + half, w, color);
    }
    // nose: a stroke that hooks to the left at the bottom
    let nx = cx + 3.0;
    ui.p.line(nx, cy - 13.0, nx, cy + 3.0, w, color);
    ui.p.arc(nx - 5.0, cy + 3.0, 5.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI * 0.62, w, color);
    // smile
    ui.p.arc(cx, cy + 2.0, 15.0, std::f32::consts::PI - 1.0, 2.0, w, color);
}

fn draw_peek(ui: &mut Ui, body: Rect, pk: &Peek, left: f32, swap: Option<(f32, &Peek)>) {
    if ui.hovered(body) {
        ui.hot = true;
    }
    // One look for every peek, taken from the unlock animation: a plain white
    // icon on the left, a small coloured status badge on the right.
    if pk.scan {
        draw_scan(ui, body, pk, 1.0 - left);
        return;
    }
    if let Trailing::Level { frac, color, text } = &pk.trailing {
        draw_hud(ui, body, pk, *frac, *color, text.as_deref());
        return;
    }
    if dial_height(pk, ui.cfg).is_some() {
        draw_device_dial(ui, body, pk);
        return;
    }
    // text peeks carry a rail of ticks along the bottom that drains while the peek is up
    let rail = !pk.is_compact() && !crate::ui::look_alcove();
    let inner = Rect::new(body.x + 22.0, body.y + 8.0, body.w - 44.0, body.h - 16.0 - if rail { 12.0 } else { 0.0 });
    if rail {
        let c = pk.accent.lerp(Color::white(1.0), 0.35);
        ui.ticks(Rect::new(body.x + 26.0, body.bottom() - 17.0, body.w - 52.0, 6.0), left, c);
        ui.animate();
    }
    match swap {
        // the next track: the old content drifts up and fades, the new one rises in
        Some((t, old)) => {
            let e = t.clamp(0.0, 1.0);
            let a0 = ui.p.alpha;
            ui.p.push_clip(body);
            let out = smoothstep(0.0, 0.5, e);
            let inn = smoothstep(0.35, 1.0, e);
            ui.p.alpha = a0 * (1.0 - out);
            draw_peek_content(ui, inner.translate(0.0, -10.0 * out), old);
            ui.p.alpha = a0 * inn;
            draw_peek_content(ui, inner.translate(0.0, 10.0 * (1.0 - inn)), pk);
            ui.p.alpha = a0;
            ui.p.pop_clip();
            ui.animate();
        }
        None => draw_peek_content(ui, inner, pk),
    }
}

/// Icon or cover, title, subtitle and trailing part of a text peek, inside `inner`.
fn draw_peek_content(ui: &mut Ui, inner: Rect, pk: &Peek) {
    if crate::ui::look_alcove() {
        return crate::alcove::peek_content(ui, inner, pk);
    }
    let is = if pk.is_compact() { 22.0 } else { 26.0 };
    let ir = Rect::new(inner.x, inner.cy() - is * 0.5, is, is);
    match &pk.image {
        Some(img) => ui.p.image(img, ir, 8.0, 1.0),
        None => {
            if crate::ui::look_alcove() {
                // a round tint of the accent with the icon inside
                ui.p.fill_circle(ir.cx(), ir.cy(), is * 0.62, pk.accent.with_a(0.24));
                ui.p.icon(pk.icon, ir.inset(is * 0.06), pk.accent.lerp(Color::white(1.0), 0.25));
            } else {
                ui.p.icon(pk.icon, ir.inset(is * 0.04), Color::white(0.96))
            }
        }
    }
    let badge = false;
    if pk.is_compact() {
        return;
    }
    let trailing_w = match &pk.trailing {
        Trailing::None | Trailing::Level { .. } => {
            if badge {
                30.0
            } else {
                0.0
            }
        }
        Trailing::Text(s, _) => ui.p.measure(s, &TextStyle::new(15.0, palette::TEXT).semibold()).0 + 8.0,
        Trailing::Ring(..) => 34.0,
        Trailing::Battery { .. } => 40.0,
    };
    let tx = ir.right() + 12.0;
    let tw = inner.right() - tx - trailing_w;
    let has_sub = !pk.subtitle.is_empty();
    let title_r =
        if has_sub { Rect::new(tx, inner.cy() - 19.0, tw, 20.0) } else { Rect::new(tx, inner.cy() - 10.0, tw, 20.0) };
    ui.p.text(&pk.title, title_r, &TextStyle::new(14.5, palette::TEXT).semibold());
    if has_sub {
        ui.p.text(&pk.subtitle, Rect::new(tx, inner.cy() + 1.0, tw, 18.0), &TextStyle::new(12.5, palette::TEXT_DIM));
    }
    let tr = Rect::new(inner.right() - trailing_w, inner.y, trailing_w, inner.h);
    match &pk.trailing {
        Trailing::None | Trailing::Level { .. } => {}
        Trailing::Text(s, c) => ui.p.text(s, tr, &TextStyle::new(15.0, *c).semibold().display().right()),
        Trailing::Ring(f, c) => {
            let (cx, cy) = (tr.right() - 14.0, tr.cy());
            ui.ring(cx, cy, 24.0, 3.0, *f, *c);
        }
        Trailing::Battery { level, charging } => {
            let r = Rect::new(tr.right() - 36.0, tr.cy() - 8.0, 36.0, 16.0);
            icons::battery(ui.p, r, *level, *charging, battery_color(*level, *charging));
        }
    }
}

/// How tall the soft-blur strip along the bottom of the open notch is.
const BAND_H: f32 = 64.0;

/// The soft blur along the bottom edge (Alcove): the content in the last strip of the open notch
/// goes from sharp to a light blur and fades into the glass toward the rim. `snap` is the strip's
/// own picture, `bottom` the notch's bottom edge and `radius` its corner radius; `k` scales the
/// effect. Drawn as a few thin slices, which follow the rounded corners and keep it cheap.
fn bottom_fade(ui: &mut Ui, snap: &Snapshot, bottom: f32, radius: f32, k: f32, scale: f32) {
    let band = snap.rect;
    let n = ((band.h * scale).round() as usize).clamp(8, 160);
    let rad = radius.max(6.0);
    let step = band.h / n as f32;
    // slice edges sit exactly on device pixels, so neighbouring slices meet without a seam
    let px = |v: f32| (v * scale).round() / scale;
    let ease = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for i in 0..n {
        let u = (i as f32 + 0.5) / n as f32;
        let y0 = px(band.y + step * i as f32);
        let y1 = if i + 1 == n { band.bottom() } else { px(band.y + step * (i + 1) as f32) };
        // keep inside the rounded bottom corners
        let dy = (y1 - (bottom - rad)).clamp(0.0, rad);
        let inset = rad - (rad * rad - dy * dy).max(0.0).sqrt();
        let (x0, x1) = (px(band.x + inset), px(band.right() - inset));
        let r = Rect::new(x0, y0, x1 - x0, y1 - y0);
        if r.w <= 1.0 || r.h <= 0.0 {
            continue;
        }
        let soft = ease(0.0, 0.8, u) * k;
        let fade = 1.0 - 0.72 * ease(0.25, 1.0, u) * k;
        ui.p.push_clip(r);
        ui.p.snapshot(snap, 0.0, 0.0, (1.0 - soft) * fade);
        ui.p.snapshot_soft(snap, 0.0, 0.0, soft * fade);
        ui.p.pop_clip();
    }
}

/// The open notch. In the Alcove look the page dots sit on top of it, and while their page list
/// is open the page underneath does not react to the mouse.
#[allow(clippy::too_many_arguments)]
fn draw_expanded(
    ui: &mut Ui,
    full: Rect,
    view: Rect,
    modules: &mut [Box<dyn Module>],
    started: &[bool],
    tabs: &[(ModuleId, Icon, String)],
    page: ModuleId,
    trans: Option<(ModuleId, f32, f32)>,
    indicators: &[Indicator],
    snaps: Option<(&Snapshot, &Snapshot)>,
) {
    if !crate::ui::look_alcove() {
        return draw_expanded_core(ui, full, modules, started, tabs, page, trans, indicators, snaps);
    }
    crate::ui::reset_wheel_taken();
    draw_expanded_core(ui, full, modules, started, tabs, page, trans, indicators, snaps);
    crate::alcove::rail(ui, view, tabs, page);

    // anywhere on the notch: a swipe sideways, or the wheel where nothing scrolls, turns the page
    if tabs.is_empty() || !ui.hovered(full) {
        return;
    }
    let (h, v) = (ui.input.hwheel, ui.input.wheel);
    let dir = if h != 0.0 {
        if h > 0.0 {
            1
        } else {
            -1
        }
    } else if v != 0.0 && !crate::ui::wheel_taken() {
        if v < 0.0 {
            1
        } else {
            -1
        }
    } else {
        0
    };
    if dir == 0 {
        return;
    }
    let ready = WHEEL_FLIP.with(|c| c.get().is_none_or(|t| t.elapsed() > Duration::from_millis(450)));
    if !ready {
        return;
    }
    let n = tabs.len();
    let next = match tabs.iter().position(|t| t.0 == page) {
        Some(i) if dir > 0 => (i + 1) % n,
        Some(i) => (i + n - 1) % n,
        None => 0,
    };
    ui.fx.open_page = Some(tabs[next].0);
    WHEEL_FLIP.with(|c| c.set(Some(Instant::now())));
}

#[allow(clippy::too_many_arguments)]
fn draw_expanded_core(
    ui: &mut Ui,
    full: Rect,
    modules: &mut [Box<dyn Module>],
    started: &[bool],
    tabs: &[(ModuleId, Icon, String)],
    page: ModuleId,
    trans: Option<(ModuleId, f32, f32)>,
    indicators: &[Indicator],
    snaps: Option<(&Snapshot, &Snapshot)>,
) {
    if crate::ui::look_alcove() {
        // no header: the page dots on the right edge do the navigating
    } else if crate::ui::look_bloom() {
        crate::bloom::header(ui, full, modules, tabs, page, indicators);
    } else {
        // header: tabs on the left, status and the settings gear on the right
        let ts = 32.0;
        let hy = full.y + (HEADER_H - ts) * 0.5 - 2.0;
        let mut x = full.x + PAD;
        // bare icons; the open tab is bright with a short lit beam under it
        let tab_style = |selected: bool| ButtonStyle {
            fg: if selected { Color::white(1.0) } else { Color::white(0.50) },
            bg: Color::white(0.0),
            hover_bg: Color::white(0.0),
            radius: 999.0,
            icon_scale: 0.54,
        };
        let avail = full.w - 2.0 * PAD - 200.0;
        let tab = ((avail + 4.0) / tabs.len().max(1) as f32 - 4.0).clamp(22.0, ts);
        for (id, icon, _title) in tabs {
            let sel = *id == page;
            let r = Rect::new(x, hy + (ts - tab) * 0.5, tab, tab);
            x += tab + 4.0;
            let hover = ui.hovered(r);
            if hover {
                ui.hot = true;
            }
            let lit = ui.glide(gkey(id, 11), if sel { 1.0 } else { 0.0 });
            let fg = Color::white(if hover { 0.85 } else { 0.50 }).lerp(Color::white(1.0), lit);
            ui.p.icon(*icon, r.center_square(ts * 0.52), fg);
            if lit > 0.02 {
                let bw = tab * 0.62 * lit;
                ui.underline(Rect::new(r.cx() - bw * 0.5, hy + ts + 1.0, bw, 3.0), Color::white(0.9 * lit));
            }
            if !sel && ui.clicked(r) {
                ui.fx.open_page = Some(id);
            }
        }
        // scroll the wheel over the header to flip through the tabs
        let wheel = ui.wheel(Rect::new(full.x, full.y, full.w, HEADER_H));
        if wheel != 0.0 && !tabs.is_empty() {
            let ready = WHEEL_FLIP.with(|c| c.get().is_none_or(|t| t.elapsed() > Duration::from_millis(450)));
            if ready {
                let cur = tabs.iter().position(|t| t.0 == page);
                let next = match cur {
                    Some(i) if wheel < 0.0 => (i + 1).min(tabs.len() - 1),
                    Some(i) => i.saturating_sub(1),
                    // e.g. on the settings page: jump back into the tabs
                    None => 0,
                };
                if cur != Some(next) {
                    ui.fx.open_page = Some(tabs[next].0);
                }
                WHEEL_FLIP.with(|c| c.set(Some(Instant::now())));
            }
        }
        // status (right): the settings gear sits at the far end, next to the battery
        let mut rx = full.right() - PAD;
        if modules.iter().enumerate().any(|(i, m)| started[i] && m.id() == "settings") {
            let g = Rect::new(rx - ts, hy, ts, ts);
            if ui.icon_button(g, Icon::Settings, tab_style(page == "settings")) {
                ui.fx.open_page = Some("settings");
            }
            if page == "settings" {
                ui.underline(Rect::new(g.cx() - ts * 0.31, hy + ts + 1.0, ts * 0.62, 3.0), Color::white(0.9));
            }
            rx = g.x - 12.0;
        }
        for ind in indicators.iter().rev() {
            if let Some((level, charging)) = ind.battery {
                let pct = format!("{:.0}%", level * 100.0);
                let st = TextStyle::new(12.5, palette::TEXT_DIM).medium().right();
                let (pw, _) = ui.p.measure(&pct, &st);
                let br = Rect::new(rx - 30.0, hy + ts * 0.5 - 7.0, 30.0, 14.0);
                icons::battery(ui.p, br, level, charging, battery_color(level, charging));
                let mut tx = br.x - 8.0;
                // time left (or an infinity sign while plugged in), always in blue
                if let Some(extra) = &ind.label {
                    let blue = palette::BLUE;
                    if extra == "\u{221E}" {
                        let (cx, cy, a) = (tx - 11.0, hy + ts * 0.5, 9.0);
                        let pt = |t: f32| {
                            let d = 1.0 + t.sin() * t.sin();
                            (cx + a * t.cos() / d, cy + a * t.sin() * t.cos() / d)
                        };
                        let n = 28;
                        for k in 0..n {
                            let t0 = k as f32 / n as f32 * std::f32::consts::TAU;
                            let t1 = (k + 1) as f32 / n as f32 * std::f32::consts::TAU;
                            let (x0, y0) = pt(t0);
                            let (x1, y1) = pt(t1);
                            ui.p.line(x0, y0, x1, y1, 1.9, blue);
                        }
                        tx -= 28.0;
                    } else {
                        let es = TextStyle::new(12.5, blue).medium().right();
                        let (ew, _) = ui.p.measure(extra, &es);
                        ui.p.text(extra, Rect::new(tx - ew - 2.0, hy, ew + 2.0, ts), &es);
                        tx -= ew + 12.0;
                    }
                }
                ui.p.text(&pct, Rect::new(tx - pw - 2.0, hy, pw + 2.0, ts), &st);
                rx = tx - pw - 14.0;
            } else if let Some(icon) = ind.icon {
                let r = Rect::new(rx - 22.0, hy + (ts - 22.0) * 0.5, 22.0, 22.0);
                ui.p.icon(icon, r.inset(3.0), ind.color);
                rx -= 28.0;
            }
        }
    }

    let content = Rect::new(
        full.x + side_pad(),
        full.y + top_h(),
        full.w - side_pad() - right_pad(),
        full.h - top_h() - bottom_pad(),
    );
    let Some((from, dir, t)) = trans else {
        draw_body(ui, content, modules, started, page);
        return;
    };
    // the page that is leaving keeps its own width and height
    let from_size = page_size(ui.cfg, modules, started, from);
    let from_content = page_content(full.cx(), full.y, from_size);
    // One page is drawn at a time: the old one slides a little and fades out, then the new one
    // slides in and fades up. (Drawing both at once doubled the work for no visible gain.)
    const SLIDE: f32 = 16.0;
    const SPLIT: f32 = 0.45;
    let base = ui.p.alpha;
    let was_interactive = ui.interactive;
    let wide = full.w.max(from_size.0);
    let clip = Rect::new(full.cx() - wide * 0.5, content.y - 4.0, wide, content.h.max(from_content.h) + 8.0);
    ui.p.push_clip(clip);
    // Alcove: the pages move the way the wheel does (up and down) and smear along that
    // direction while they move, like a quick scroll
    let al = crate::ui::look_alcove();
    let slide = if al { 28.0 } else { SLIDE };
    if t < SPLIT {
        let u = t / SPLIT;
        let e = smoothstep(0.0, 1.0, u);
        ui.p.alpha = base * (1.0 - e);
        ui.interactive = false;
        let (dx, dy) = if al { (0.0, (-dir * slide * e).round()) } else { ((-dir * slide * e).round(), 0.0) };
        match snaps {
            Some((old, _)) if al => snapshot_depth(ui, old, 1.0 - 0.04 * e, -dir * slide * 0.6 * e, 0.8 * e),
            Some((old, _)) => ui.p.snapshot(old, dx, dy, 1.0),
            None => draw_body(ui, from_content.translate(dx, dy), modules, started, from),
        }
    } else {
        let u = (t - SPLIT) / (1.0 - SPLIT);
        // Alcove: the new page is already clearly there while it is still being stretched
        ui.p.alpha = base * if al { smoothstep(0.0, 0.4, u) } else { smoothstep(0.0, 0.8, u) };
        ui.interactive = was_interactive && u > 0.6;
        let e = 1.0 - smoothstep(0.0, 1.0, u);
        let (dx, dy) = if al { (0.0, (dir * slide * e).round()) } else { ((dir * slide * e).round(), 0.0) };
        match snaps {
            // Alcove: the new page rises into place from a little larger and further down, goes a
            // touch past and settles, a soft blur clearing as it lands
            Some((_, new)) if al => {
                let c = 0.6_f32;
                let back = 1.0 + (c + 1.0) * (u - 1.0).powi(3) + c * (u - 1.0).powi(2);
                let k = 1.0 - back;
                snapshot_depth(ui, new, 1.0 + 0.04 * k, dir * slide * 1.2 * k, 0.8 * k.max(0.0));
            }
            Some((_, new)) => ui.p.snapshot(new, dx, dy, 1.0),
            None => draw_body(ui, content.translate(dx, dy), modules, started, page),
        }
    }
    ui.p.alpha = base;
    ui.interactive = was_interactive;
    ui.p.pop_clip();
}

/// A page picture drawn between sharp and soft: `blur` 0 is the crisp picture, 1 the blurred
/// copy. The two are cross-faded, so the page seems to lose or gain focus as it moves.
/// A page picture scaled about its centre and shifted, drawn between sharp and soft like
/// `snapshot_motion`: one solid piece, so nothing can look cut while it moves.
fn snapshot_depth(ui: &mut Ui, s: &Snapshot, scale: f32, dy: f32, blur: f32) {
    let dy = dy.round();
    if blur < 0.03 {
        ui.p.snapshot_scaled(s, scale, 0.0, dy, 1.0, false);
    } else {
        ui.p.snapshot_scaled(s, scale, 0.0, dy, 1.0 - blur, false);
        ui.p.snapshot_scaled(s, scale, 0.0, dy, blur, true);
    }
}

fn snapshot_motion(ui: &mut Ui, s: &Snapshot, dx: f32, dy: f32, blur: f32) {
    if blur < 0.03 {
        ui.p.snapshot(s, dx, dy, 1.0);
    } else {
        ui.p.snapshot(s, dx, dy, 1.0 - blur);
        ui.p.snapshot_soft(s, dx, dy, blur);
    }
}

fn draw_body(ui: &mut Ui, content: Rect, modules: &mut [Box<dyn Module>], started: &[bool], page: ModuleId) {
    if page == HOME && crate::ui::look_alcove() {
        crate::alcove::home(ui, content, modules);
    } else if page == HOME && crate::ui::look_bloom() {
        crate::bloom::control_center(ui, content, modules, started);
    } else if page == HOME {
        draw_home(ui, content, modules, started);
    } else if let Some(i) = modules.iter().position(|m| m.id() == page) {
        modules[i].draw_page(ui, content);
    }
}

/// The widgets to show in the right-hand column of the home page: the ones
/// chosen in `[home] widgets` (Settings → Home) that are available right now,
/// at most `HOME_TILES`. An empty list keeps the old behaviour: the first
/// modules with a small card. The clock is not a choice any more: it always
/// lives on the left.
fn home_tiles(modules: &[Box<dyn Module>], started: &[bool], cfg: &Config) -> Vec<usize> {
    let small = |i: usize| started[i] && modules[i].card() == Some(CardSize::Small);
    if cfg.home.widgets.is_empty() {
        return (0..modules.len()).filter(|&i| small(i)).take(HOME_TILES).collect();
    }
    let mut out = Vec::new();
    for id in &cfg.home.widgets {
        if out.len() >= HOME_TILES {
            break;
        }
        // Wi-Fi and Bluetooth are one module now; older configs still say "wifi" / "bluetooth"
        let id = if matches!(id.as_str(), "wifi" | "bluetooth") { "control" } else { id.as_str() };
        if let Some(i) = modules.iter().position(|m| m.id() == id) {
            if small(i) && !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out
}

/// Where the small round Mirror button sits on the home page's clock card.
fn draw_home(ui: &mut Ui, r: Rect, modules: &mut [Box<dyn Module>], started: &[bool]) {
    let large = (0..modules.len()).find(|&i| started[i] && modules[i].card() == Some(CardSize::Large));
    let tiles = home_tiles(modules, started, ui.cfg);
    let (left, right) = if tiles.is_empty() { (r, Rect::default()) } else { r.split_left((r.w * 0.56).round(), 16.0) };
    // flat columns on black, separated by hairlines
    let line = Color::white(0.09);
    match large {
        // something is playing: the player on top, the clock in its own strip below
        Some(i) => {
            let (media, clock) = left.split_top((left.h - CLOCK_STRIP_H - 10.0).max(60.0), 10.0);
            modules[i].draw_card(ui, media);
            ui.p.fill_rect(Rect::new(left.x + 6.0, clock.y - 5.0, left.w - 12.0, 1.0), line);
            draw_clock_strip(ui, clock);
        }
        None => draw_clock_card(ui, left),
    }
    if !tiles.is_empty() {
        ui.p.fill_rect(Rect::new(left.right() + 8.0, r.y + 6.0, 1.0, r.h - 12.0), line);
        let n = tiles.len() as f32;
        let gap = 10.0;
        let ch = (right.h - gap * (n - 1.0)) / n;
        for (k, &i) in tiles.iter().enumerate() {
            let cr = Rect::new(right.x, right.y + k as f32 * (ch + gap), right.w, ch);
            modules[i].draw_card(ui, cr);
            if k > 0 {
                ui.p.fill_rect(Rect::new(right.x + 6.0, cr.y - gap * 0.5, right.w - 12.0, 1.0), line.alpha(0.8));
            }
        }
    }
}

/// Local time pieces for the flip clock: (hour, minute, second, "AM"/"PM" or "", date text).
pub(crate) fn clock_parts() -> (u8, u8, u8, String, String) {
    let st = unsafe { GetLocalTime() };
    // the formatted time and the date text only change when the minute does: work them out once per minute
    thread_local! {
        static FMT: std::cell::RefCell<Option<((u16, u16, u16), String, String)>> = const { std::cell::RefCell::new(None) };
    }
    let key = (st.wDay, st.wHour, st.wMinute);
    let cached = FMT.with(|c| c.borrow().as_ref().filter(|e| e.0 == key).map(|e| (e.1.clone(), e.2.clone())));
    let (suffix, date) = match cached {
        Some(v) => v,
        None => {
            let mut tbuf = [0u16; 64];
            let tn =
                unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&st), PCWSTR::null(), Some(&mut tbuf)) };
            let time = String::from_utf16_lossy(&tbuf[..(tn.max(1) - 1) as usize]);
            // "Friday 26th": the day of the week and the date, without the month
            let day = st.wDay;
            let ord = match (day % 10, day % 100) {
                (_, 11..=13) => "th",
                (1, _) => "st",
                (2, _) => "nd",
                (3, _) => "rd",
                _ => "th",
            };
            let weekday = crate::util::WEEKDAY_NAMES[(st.wDayOfWeek as usize).min(6)];
            let date = format!("{weekday} {day}{ord}");
            // a letter in the formatted time means a 12 hour clock (AM / PM)
            let suffix: String = time.chars().filter(|c| c.is_alphabetic()).collect();
            FMT.with(|c| *c.borrow_mut() = Some((key, suffix.clone(), date.clone())));
            (suffix, date)
        }
    };
    let h = if suffix.is_empty() {
        st.wHour as u8
    } else {
        match st.wHour % 12 {
            0 => 12,
            h => h as u8,
        }
    };
    (h, st.wMinute as u8, st.wSecond as u8, suffix, date)
}

/// Flip clock under the media player: digits on the left, date on the right.
fn draw_clock_strip(ui: &mut Ui, r: Rect) {
    ui.card(r, false);
    let (h, m, s, suffix, date) = clock_parts();
    let inner = r.inset_xy(14.0, 0.0);
    let th = (r.h - 14.0).clamp(24.0, 40.0);
    let tw = th * 0.72;
    let used = ui.clock(inner.x, r.cy() - th * 0.5, tw, th, h, m, s, 100);
    let text = if suffix.is_empty() { date } else { format!("{suffix} · {date}") };
    ui.p.text(
        &text,
        Rect::new(inner.x + used + 12.0, r.y, (inner.w - used - 12.0).max(0.0), r.h),
        &TextStyle::new(12.0, palette::TEXT_DIM).right(),
    );
}

fn draw_clock_card(ui: &mut Ui, r: Rect) {
    ui.card(r, false);
    let (h, m, s, suffix, _date) = clock_parts();
    let inner = r.inset_xy(18.0, 14.0);
    // size the tiles so six digits and the gaps fill the card (but never get huge)
    let tw = ((inner.w - 24.0) / 6.6).clamp(26.0, 50.0);
    let th = (tw * 1.28).min(inner.h - 62.0).max(34.0);
    let used = if crate::ui::look_nothing() {
        draw_dot_clock(ui, Rect::new(inner.x, inner.y, inner.w, th), h, m)
    } else {
        ui.clock(inner.x, inner.y, tw, th, h, m, s, 0)
    };
    if !suffix.is_empty() {
        ui.p.text(
            &suffix,
            Rect::new(inner.x + used + 8.0, inner.y + th - 20.0, 40.0, 20.0),
            &TextStyle::new(12.0, palette::TEXT_DIM),
        );
    }
    // the calendar strip sits under the time
    let top = inner.y + th + 10.0;
    let strip = Rect::new(inner.x, top, inner.w, (inner.bottom() - top).clamp(30.0, 48.0));
    draw_calendar(ui, strip);
}

/// 5x7 dot-matrix digits, one row per byte (bit 4 = leftmost column).
const DOT_DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// The time as a dot-matrix display (the Nothing look): white dots, a red colon, faint unlit dots.
/// Returns the width used.
fn draw_dot_clock(ui: &mut Ui, area: Rect, h: u8, m: u8) -> f32 {
    // 4 digits of 5 columns, a 1-column gap between digits, and a 3-column colon block
    const COLS: f32 = 27.0;
    let pitch = (area.h / 7.0).min(area.w / COLS).max(3.0);
    let dot = pitch * 0.34;
    let top = area.y + (area.h - pitch * 7.0).max(0.0) * 0.5;
    let digits = [h / 10 % 10, h % 10, m / 10 % 10, m % 10];
    let mut col = 0.0;
    for (k, d) in digits.iter().enumerate() {
        if k == 2 {
            // the colon: two red dots
            let cx = area.x + (col + 1.5) * pitch;
            ui.p.fill_circle(cx, top + 2.5 * pitch, dot, crate::ui::NOTHING_RED);
            ui.p.fill_circle(cx, top + 4.5 * pitch, dot, crate::ui::NOTHING_RED);
            col += 3.0 + 1.0;
        }
        let rows = &DOT_DIGITS[*d as usize];
        for (ry, bits) in rows.iter().enumerate() {
            for cx in 0..5 {
                let on = bits & (0b10000 >> cx) != 0;
                let c = if on { Color::white(0.96) } else { Color::white(0.07) };
                ui.p.fill_circle(area.x + (col + cx as f32 + 0.5) * pitch, top + (ry as f32 + 0.5) * pitch, dot, c);
            }
        }
        col += 5.0 + 1.0;
    }
    (col - 1.0) * pitch
}

thread_local! {
    /// Days the calendar strip is moved from today (0 = today is selected).
    static CAL_OFF: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAY_LETTER: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];
const DAY_SHORT: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// Days since 1970-01-01 for a calendar date (proleptic Gregorian).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = (if m <= 2 { y - 1 } else { y }) as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The calendar date (year, month 1..=12, day) for a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

/// 0 = Sunday for a day count since 1970-01-01 (a Thursday).
fn weekday_of(z: i64) -> usize {
    (z + 4).rem_euclid(7) as usize
}

/// A week strip: the month on the left, then days that scroll with the mouse wheel (or a click).
/// The chosen day is blue with a lit beam under it that glides along a rail of beams.
fn draw_calendar(ui: &mut Ui, area: Rect) {
    let st = unsafe { GetLocalTime() };
    let today = days_from_civil(st.wYear as i32, st.wMonth as u32, st.wDay as u32);
    let mut off = CAL_OFF.with(|c| c.get());

    // wheel over the strip moves through the days (down = later)
    if ui.hovered(area) {
        let wheel = ui.input.wheel;
        if wheel.abs() > 0.01 {
            let step = (wheel.abs().round() as i32).max(1);
            off += if wheel < 0.0 { step } else { -step };
        }
    }

    let month_w = 54.0;
    let days = Rect::new(area.x + month_w, area.y, (area.w - month_w).max(60.0), area.h);
    let cw = days.w / 6.0;
    let blue = Color::hex(0x3D95FF);

    // the month of the chosen day; a click on it jumps back to today
    let (_, month, _) = civil_from_days(today + off as i64);
    let label = Rect::new(area.x, area.y, month_w, area.h);
    ui.p.text(
        MONTHS[(month as usize - 1).min(11)],
        Rect::new(area.x, area.y + 2.0, month_w, 28.0),
        &TextStyle::new(22.0, Color::white(0.98)).semibold().display(),
    );
    if off != 0 {
        ui.p.text("today", Rect::new(area.x, area.y + 28.0, month_w, 14.0), &TextStyle::new(10.5, palette::TEXT_FAINT));
        if ui.clicked(label) {
            off = 0;
        }
    }

    let scroll = ui.glide(gkey("cal-scroll", 0), off as f32);
    let base_alpha = ui.p.alpha;
    ui.p.push_clip(days);
    let first = scroll.floor() as i32 - 3;
    for k in 0..=10 {
        let d = first + k;
        let idx = 2.0 + (d as f32 - scroll);
        // fade out at both ends of the strip
        let fade = (idx + 1.0).clamp(0.0, 1.0) * (6.0 - idx).clamp(0.0, 1.0);
        if fade <= 0.01 {
            continue;
        }
        let cell = Rect::new(days.x + idx * cw, area.y, cw, area.h - 8.0);
        let z = today + d as i64;
        let (_, _, dd) = civil_from_days(z);
        let wd = weekday_of(z);
        // how close this day is to being the chosen one (smooth while scrolling)
        let near = (1.0 - (d as f32 - scroll).abs()).clamp(0.0, 1.0);
        let body = if d < 0 { Color::white(0.50) } else { Color::white(0.90) };
        let col = body.lerp(blue, near);
        ui.p.alpha = base_alpha * fade;
        let name = if near > 0.5 { DAY_SHORT[wd] } else { DAY_LETTER[wd] };
        ui.p.text(
            name,
            Rect::new(cell.x, cell.y + 2.0, cell.w, 12.0),
            &TextStyle::new(9.5, Color::white(0.50).lerp(blue, near)).medium().center(),
        );
        ui.p.text(
            &dd.to_string(),
            Rect::new(cell.x, cell.y + 14.0, cell.w, 22.0),
            &TextStyle::new(16.0, col).semibold().display().center(),
        );
        if ui.clicked(cell) {
            off = d;
        }
    }
    ui.p.alpha = base_alpha;
    ui.p.pop_clip();

    // a rail of beams under the days; the lit stretch follows the chosen day
    let rail = Rect::new(days.x, area.bottom() - 5.0, days.w, 4.0);
    let cx = days.x + (2.5 + (off as f32 - scroll)) * cw;
    let from = ((cx - cw * 0.34 - days.x) / days.w).clamp(0.0, 1.0);
    let to = ((cx + cw * 0.34 - days.x) / days.w).clamp(0.0, 1.0);
    ui.tick_range(rail, from, to, blue);

    CAL_OFF.with(|c| c.set(off.clamp(-3650, 3650)));
}

// ---------------------------------------------------------------------------
// Win32 helpers
// ---------------------------------------------------------------------------

unsafe extern "system" fn win_event_cb(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event == EVENT_OBJECT_LOCATIONCHANGE
        && (id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd != GetForegroundWindow())
    {
        return;
    }
    let h = NOTCH_HWND.load(Ordering::Acquire);
    if h != 0 {
        let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP_FOREGROUND, WPARAM(event as usize), LPARAM(0));
    }
}

unsafe fn is_fullscreen_on(mon: HMONITOR, own: HWND) -> bool {
    let fg = GetForegroundWindow();
    if fg.is_invalid() || fg == own {
        return false;
    }
    let mut cls = [0u16; 64];
    let n = GetClassNameW(fg, &mut cls);
    let class = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
    // Desktop, taskbar and shell overlays (Start, Alt+Tab, Task View, flyouts)
    // cover the screen but are not fullscreen apps.
    if matches!(
        class.as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "XamlExplorerHostIslandWindow"
            | "Windows.UI.Core.CoreWindow"
            | "MultitaskingViewFrame"
            | "TaskSwitcherWnd"
            | "ForegroundStaging"
            | "TopLevelWindowForOverflowXamlIsland"
    ) {
        return false;
    }
    if MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) != mon {
        return false;
    }
    // The shell's own verdict covers D3D exclusive fullscreen and presentations.
    if let Ok(state) = SHQueryUserNotificationState() {
        if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
            return true;
        }
    }
    // A maximized window is never "fullscreen" (matters with an auto-hide taskbar).
    if IsZoomed(fg).as_bool() {
        return false;
    }
    let mut wr = RECT::default();
    if GetWindowRect(fg, &mut wr).is_err() {
        return false;
    }
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoW(mon, &mut mi).as_bool() {
        return false;
    }
    let m = mi.rcMonitor;
    let style = GetWindowLongW(fg, GWL_STYLE) as u32;
    let has_caption = style & WS_CAPTION.0 == WS_CAPTION.0;
    let fs = wr.left <= m.left && wr.top <= m.top && wr.right >= m.right && wr.bottom >= m.bottom && !has_caption;
    if fs {
        crate::log!("fullscreen window: class={class} rect=({},{},{},{})", wr.left, wr.top, wr.right, wr.bottom);
    }
    fs
}

fn monitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> windows::core::BOOL {
        let v = &mut *(lp.0 as *mut Vec<HMONITOR>);
        v.push(m);
        true.into()
    }
    let mut v: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
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

fn translate_key(vk: u32) -> Option<Key> {
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    let vk = VIRTUAL_KEY(vk as u16);
    Some(match vk {
        VK_RETURN => Key::Enter { shift },
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_ESCAPE => Key::Escape,
        VK_TAB => Key::Tab,
        _ if ctrl && vk == VIRTUAL_KEY(b'A' as u16) => Key::SelectAll,
        _ if ctrl && vk == VIRTUAL_KEY(b'C' as u16) => Key::Copy,
        _ if ctrl && vk == VIRTUAL_KEY(b'V' as u16) => Key::Paste,
        _ if ctrl && vk == VIRTUAL_KEY(b'X' as u16) => Key::Cut,
        _ => return None,
    })
}

/// Banner for `dynamic-notch claude <event> <project> <message> <hwnd>` (see claude.rs).
fn claude_peek(args: &[String]) -> Peek {
    const CLAUDE: Color = Color::hex(0xD97757);
    let arg = |i: usize| args.get(i).map(|s| s.trim()).unwrap_or("");
    let (event, project, message) = (arg(0).to_ascii_lowercase(), arg(1), arg(2));
    let hwnd = arg(3).parse::<isize>().unwrap_or(0);
    let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
        (false, false) => format!("{a} · {b}"),
        _ => format!("{a}{b}"),
    };
    let peek = if matches!(event.as_str(), "notification" | "attention" | "permission") {
        let msg = if message.is_empty() { "Waiting for your input" } else { message };
        Peek::new(Icon::Bell, CLAUDE, "Claude needs you", join(project, msg)).duration_ms(7000)
    } else {
        let sub = if project.is_empty() { "Ready for your next message".to_string() } else { project.to_string() };
        Peek::new(Icon::Sparkle, CLAUDE, "Claude finished", sub).duration_ms(4500)
    };
    peek.key("claude").focus(hwnd)
}

/// Map an IPC page name onto a static module id.
fn leak_page(p: &str) -> ModuleId {
    match p {
        "media" => "media",
        "timer" => "timer",
        "clipboard" => "clipboard",
        "downloads" => "downloads",
        "ai" => "ai",
        "voice" => "voice",
        "shelf" => "shelf",
        "ports" => "ports",
        "mirror" => "mirror",
        _ => HOME,
    }
}

/// Drive letter and direction ("in" / "out") of a volume arrival or removal.
fn usb_event(wp: u32, lp: isize) -> Option<Vec<String>> {
    const ARRIVAL: u32 = 0x8000;
    const REMOVED: u32 = 0x8004;
    if (wp != ARRIVAL && wp != REMOVED) || lp == 0 {
        return None;
    }
    // DEV_BROADCAST_VOLUME { size, device type (2 = volume), reserved, unit mask, flags }
    let hdr = lp as *const u32;
    let kind = unsafe { *hdr.add(1) };
    if kind != 2 {
        return None;
    }
    let mask = unsafe { *hdr.add(3) };
    if mask == 0 {
        return None;
    }
    let letter = (b'A' + mask.trailing_zeros() as u8) as char;
    let dir = if wp == ARRIVAL { "in" } else { "out" };
    Some(vec![format!("{letter}:"), dir.to_string()])
}

#[cfg(test)]
mod calendar_tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for (y, m, d) in [(2000, 2, 29), (2024, 12, 31), (2026, 10, 26), (1999, 3, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        // 1970-01-01 was a Thursday, 2026-10-26 is a Monday
        assert_eq!(weekday_of(0), 4);
        assert_eq!(weekday_of(days_from_civil(2026, 10, 26)), 1);
    }
}
