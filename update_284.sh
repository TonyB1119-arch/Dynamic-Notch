#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
RUST = r'''//! The launcher: press the hotkey, type, press Enter. Two glass boxes as in Tuna: the thing you
//! found on the left, what will happen to it on the right. The list of matches stays hidden until
//! you press the Down arrow.
//!
//! It finds apps (Start menu and Store apps), files and folders (Desktop, Documents, Downloads,
//! Pictures, Videos, Music and the folders in `[launcher] folders`), answers sums like `12*(3+4)`,
//! opens web addresses and falls back to a web search.
//!
//! Look: dark glass. The picture behind the window is copied when it opens, blurred once and drawn
//! inside the card, so the blur works everywhere and moves with the card. Everything animates (the
//! window opening and closing, the list dropping down, the highlight sliding, the underline growing);
//! the animation timer runs only while something is moving.
//!
//! Three modes, switched with Tab while it is open: searching, the clipboard history (Win+C) and a
//! one-question chat with Gemini (Win+G): the question on the left, the answer on the right.
//!
//! Cost: nothing exists until the hotkey is pressed for the first time. The window, its drawing
//! surface and the lists are built then; 20 seconds after it closes the drawing memory is handed
//! back, and after five minutes everything is gone again. While it is closed it uses no CPU.

use std::collections::HashMap;
use std::mem::size_of;
use std::os::windows::fs::MetadataExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_USE_HOSTBACKDROPBRUSH,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::{
    CreateRectRgn, GetMonitorInfoW, MonitorFromPoint, SetWindowRgn, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::config::{GeminiCfg, LauncherCfg};
use crate::gfx::{Gfx, Icon, ImageData, Painter, Res, TextStyle};
use crate::host::Host;
use crate::modules::weather::Brief;
use crate::sys::{self, clip, dragdrop};
use crate::util::{app_dir, Color, Rect, SendHwnd};

const CLASS: PCWSTR = w!("DynamicNotch.Launcher");
const T_ANIM: usize = 1;
const T_ICON: usize = 2;
const T_TRIM: usize = 3;
const T_FREE: usize = 4;
const T_FLASH: usize = 5;
/// A background scan has finished.
const WM_INDEX: u32 = WM_APP + 31;
/// Gemini has sent more of the answer.
const WM_AI: u32 = WM_APP + 32;

// -- layout (dips) -------------------------------------------------------------------------------------
const PAD: f32 = 14.0;
const BOX: f32 = 172.0;
const GAP: f32 = 10.0;
const W: f32 = PAD * 2.0 + BOX * 2.0 + GAP;
const FRAME_H: f32 = BOX + PAD * 2.0;
const HEAD: f32 = 30.0;
const ROW: f32 = 46.0;
/// Rows of the list that show at once.
const VIS: usize = 6;
const FULL_H: f32 = FRAME_H + HEAD + VIS as f32 * ROW + 10.0;
const CARD_R: f32 = 34.0;
const BOX_R: f32 = 24.0;

/// Matches kept per search.
const KEEP: usize = 40;
const MAX_FILES: usize = 30_000;
const MAX_DEPTH: u8 = 5;
const SKIP_DIRS: [&str; 3] = ["node_modules", "__pycache__", "site-packages"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    App,
    Folder,
    File,
    Calc,
    Url,
    Web,
    Weather,
    Day,
    /// An item of the clipboard history (its `path` is `clip:<id>`).
    Clip,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::App => "Application",
            Kind::Folder => "Folder",
            Kind::File => "File",
            Kind::Calc => "Calculator",
            Kind::Url => "Link",
            Kind::Web => "Web search",
            Kind::Weather => "Weather",
            Kind::Day => "Forecast",
            Kind::Clip => "Clipboard",
        }
    }
    fn has_icon(self) -> bool {
        matches!(self, Kind::App | Kind::Folder | Kind::File)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Open,
    Admin,
    Reveal,
    CopyPath,
    CopyResult,
    OpenLink,
    Search,
    Forecast,
    Quit,
    Restart,
    ForceQuit,
    Paste,
    CopyClip,
    DeleteClip,
}

impl Act {
    fn label(self) -> &'static str {
        match self {
            Act::Open => "Open",
            Act::Admin => "Run as administrator",
            Act::Reveal => "Show in folder",
            Act::CopyPath => "Copy path",
            Act::CopyResult => "Copy result",
            Act::OpenLink => "Open link",
            Act::Search => "Search the web",
            Act::Forecast => "Open forecast",
            Act::Quit => "Quit",
            Act::Restart => "Restart",
            Act::ForceQuit => "Force quit",
            Act::Paste => "Paste",
            Act::CopyClip => "Copy",
            Act::DeleteClip => "Remove from history",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Act::Open => Icon::Send,
            Act::Admin => Icon::Bolt,
            Act::Reveal => Icon::Folder,
            Act::CopyPath | Act::CopyResult => Icon::Copy,
            Act::OpenLink => Icon::Link,
            Act::Search => Icon::Globe,
            Act::Forecast => Icon::Cloud,
            Act::Quit => Icon::Close,
            Act::Restart => Icon::Refresh,
            Act::ForceQuit => Icon::Stop,
            Act::Paste => Icon::Clipboard,
            Act::CopyClip => Icon::Copy,
            Act::DeleteClip => Icon::Trash,
        }
    }
}

/// What can be done with a match; the first one is what Enter does.
fn actions(h: &Hit) -> &'static [Act] {
    match h.kind {
        Kind::App if h.path.starts_with("shell:") => &[Act::Open],
        Kind::App => &[Act::Open, Act::Admin, Act::Reveal, Act::Quit, Act::Restart, Act::ForceQuit],
        Kind::Folder | Kind::File => &[Act::Open, Act::Reveal, Act::CopyPath],
        Kind::Calc => &[Act::CopyResult],
        Kind::Url => &[Act::OpenLink, Act::CopyPath],
        Kind::Web => &[Act::Search],
        Kind::Weather | Kind::Day => &[Act::Forecast],
        Kind::Clip => &[Act::Paste, Act::CopyClip, Act::DeleteClip],
    }
}

/// One thing that can be found. Files keep only their path (the name is its tail).
struct Entry {
    path: Box<str>,
    label: Option<Box<str>>,
    at: u32,
    kind: Kind,
}

impl Entry {
    fn name(&self) -> &str {
        match &self.label {
            Some(l) => &**l,
            None => &self.path[self.at as usize..],
        }
    }
}

/// A match as shown.
#[derive(Clone)]
struct Hit {
    name: String,
    /// The second line: a path, the sum, the address.
    detail: String,
    /// What gets opened (and the key of its icon).
    path: String,
    kind: Kind,
    /// Matched letters, as byte ranges of `name`.
    spans: Vec<(usize, usize)>,
    /// Which day of the forecast (Weather rows).
    idx: usize,
}

/// What the launcher is for: finding things, or going through what was copied.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Search,
    Clip,
    Ai,
}

impl Mode {
    /// The next mode (or the one before) when Tab (Shift+Tab) is pressed.
    fn cycle(self, back: bool) -> Mode {
        match (self, back) {
            (Mode::Search, false) | (Mode::Ai, true) => Mode::Clip,
            (Mode::Clip, false) | (Mode::Search, true) => Mode::Ai,
            (Mode::Ai, false) | (Mode::Clip, true) => Mode::Search,
        }
    }
}

/// How far the Gemini question has got.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AiState {
    Idle,
    Waiting,
    Done,
    Failed,
}

/// The answer as the background request fills it in.
struct AiShared {
    /// Which question this belongs to (an answer to an older one is ignored).
    seq: u64,
    text: String,
    done: bool,
    err: Option<String>,
}

/// The Gemini mode: one question, one answer.
struct Ai {
    seq: u64,
    state: AiState,
    text: String,
    /// How far the answer is scrolled, and how far it can be (set while drawing).
    scroll: f32,
    max_scroll: f32,
    /// The question's text size and top edge as drawn, where they are heading, and for which text.
    sz: f32,
    y: f32,
    goal: (f32, f32),
    fit_for: u64,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    shared: Arc<Mutex<AiShared>>,
}

impl Ai {
    fn new() -> Self {
        Self {
            seq: 0,
            state: AiState::Idle,
            text: String::new(),
            scroll: 0.0,
            max_scroll: 0.0,
            sz: Q_BIG,
            y: Q_Y0,
            goal: (Q_BIG, Q_Y0),
            fit_for: 0,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(AiShared { seq: 0, text: String::new(), done: false, err: None })),
        }
    }

    /// Stop the request that is running and forget the answer.
    fn clear(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.seq += 1;
        self.state = AiState::Idle;
        self.text = String::new();
        self.scroll = 0.0;
        self.max_scroll = 0.0;
    }

    /// The question's text eases toward its size and place.
    fn glide(&mut self, dt: f32, snap: bool) {
        if snap {
            (self.sz, self.y) = self.goal;
        } else {
            self.sz = logic::follow(self.sz, self.goal.0, dt, 14.0, 0.05);
            self.y = logic::follow(self.y, self.goal.1, dt, 14.0, 0.05);
        }
    }

    fn moving(&self) -> bool {
        (self.sz - self.goal.0).abs() > 0.05 || (self.y - self.goal.1).abs() > 0.05
    }
}

/// The question starts big in the middle of the box and gets smaller as it grows.
const Q_BIG: f32 = 34.0;
const Q_SIZES: [f32; 6] = [Q_BIG, 28.0, 23.0, 19.0, 16.0, 14.0];
/// Where the top of a one-line question starts (the box is 144 high inside).
const Q_Y0: f32 = 48.0;

/// The biggest text size at which the question still fits the box, and where its top edge then goes:
/// the middle while it fits, the top once it is too long (then the end stays in view).
fn ai_fit(p: &mut Painter, q: &str, w: f32, h: f32) -> (f32, f32) {
    if q.is_empty() {
        return (Q_BIG, Q_Y0);
    }
    let mut size = Q_SIZES[Q_SIZES.len() - 1];
    let mut used = 0.0;
    for &sz in &Q_SIZES {
        used = p.measure_wrapped(q, &TextStyle::new(sz, white(0.9)).semibold().center().wrap(), w);
        size = sz;
        if used <= h {
            break;
        }
    }
    (size, if used <= h { (h - used) * 0.5 } else { h - used })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Subject,
    Action,
}

type Slot = Arc<Mutex<Option<Vec<Entry>>>>;

pub struct Launcher {
    hwnd: HWND,
    host: Host,
    res: Res,
    cfg: LauncherCfg,
    /// Taking input (false while it fades out).
    open: bool,
    /// The window is on screen (also while it fades out).
    visible: bool,
    mode: Mode,
    /// Shown for a moment by a combo (it puts itself away).
    flashing: bool,
    /// What to show instead of search results (a combo's target).
    pinned: Option<Hit>,
    free_me: bool,
    scale: f32,
    win_px: (i32, i32, i32, i32),
    region_h: i32,
    // what can be found
    apps: Vec<Entry>,
    files: Vec<Entry>,
    apps_ready: bool,
    apps_slot: Slot,
    files_slot: Slot,
    files_started: bool,
    usage: HashMap<String, u32>,
    /// What was chosen for what was typed: (typed text, path) -> times. It makes the same typing find the same thing first.
    learn: HashMap<(String, String), u32>,
    icons: HashMap<String, Option<ImageData>>,
    // what the user is doing
    query: String,
    hits: Vec<Hit>,
    sel: usize,
    top: usize,
    act: usize,
    focus: Focus,
    list_open: bool,
    high: Option<u16>,
    mouse: (f32, f32),
    // animation: all of these move toward where they belong while the timer runs
    timer_on: bool,
    last_tick: Instant,
    /// Window opening (1) and closing (0).
    anim: f32,
    /// Height of the dropped-down list, in dips.
    lh: f32,
    /// Underline growing, the icon popping in (box 1 and box 2): 0 to 1.
    ul: f32,
    pop_s: f32,
    pop_a: f32,
    /// Which box has the focus (0 = left, 1 = right).
    focus_t: f32,
    /// The highlighted row, as a (moving) row number.
    sel_y: f32,
    /// The blurred picture of the screen behind the window.
    blur_img: Option<ImageData>,
    wx: Option<std::rc::Rc<crate::modules::weather::Brief>>,
    scratch: Vec<usize>,
    ai: Ai,
    /// The contents sliding in after Tab: 0 to 1, and the way they come from (1 or -1).
    sw: f32,
    sw_dir: f32,
    /// This time the glass shows the live screen behind it (media is playing) instead of one picture.
    live: bool,
}

impl Launcher {
    pub fn new(gfx: &Gfx, compositor: Compositor) -> windows::core::Result<Self> {
        let hwnd = create_window()?;
        let host = Host::new_shared(hwnd, gfx, 64, 64, compositor)?;
        let res = Res::new(gfx)?;
        let apps_slot: Slot = Arc::new(Mutex::new(None));
        spawn_apps(apps_slot.clone(), SendHwnd::new(hwnd));
        Ok(Self {
            hwnd,
            host,
            res,
            cfg: LauncherCfg::default(),
            open: false,
            visible: false,
            mode: Mode::Search,
            flashing: false,
            pinned: None,
            free_me: false,
            scale: 1.0,
            win_px: (0, 0, 0, 0),
            region_h: -1,
            apps: Vec::new(),
            files: Vec::new(),
            apps_ready: false,
            apps_slot,
            files_slot: Arc::new(Mutex::new(None)),
            files_started: false,
            usage: load_usage(),
            learn: load_learn(),
            icons: HashMap::new(),
            query: String::new(),
            hits: Vec::new(),
            sel: 0,
            top: 0,
            act: 0,
            focus: Focus::Subject,
            list_open: false,
            high: None,
            mouse: (-1.0, -1.0),
            timer_on: false,
            last_tick: Instant::now(),
            anim: 0.0,
            lh: 0.0,
            ul: 1.0,
            pop_s: 1.0,
            pop_a: 1.0,
            focus_t: 0.0,
            sel_y: 0.0,
            blur_img: None,
            wx: None,
            scratch: Vec::new(),
            ai: Ai::new(),
            sw: 1.0,
            sw_dir: 1.0,
            live: false,
        })
    }

    pub fn configure(&mut self, cfg: LauncherCfg) {
        self.cfg = cfg;
    }

    /// The idle timer ran out: the app should drop the launcher.
    pub fn wants_free(&self) -> bool {
        self.free_me
    }

    pub fn recover(&mut self, gfx: &Gfx) {
        let _ = self.host.set_rendering_device(gfx);
        if let Ok(r) = Res::new(gfx) {
            self.res = r;
        }
        self.draw(gfx);
    }

    pub fn toggle(&mut self, gfx: &Gfx, ui_scale: f32) {
        if self.open && self.mode == Mode::Search {
            self.hide();
        } else {
            self.mode = Mode::Search;
            self.show(gfx, ui_scale, true);
        }
    }

    /// The clipboard hotkey: the launcher as a history of what was copied.
    pub fn toggle_clip(&mut self, gfx: &Gfx, ui_scale: f32) {
        if self.open && self.mode == Mode::Clip {
            self.hide();
        } else {
            self.mode = Mode::Clip;
            self.show(gfx, ui_scale, true);
        }
    }

    /// The Gemini hotkey: ask one quick question.
    pub fn toggle_ai(&mut self, gfx: &Gfx, ui_scale: f32) {
        if self.open && self.mode == Mode::Ai {
            self.hide();
        } else {
            self.mode = Mode::Ai;
            self.show(gfx, ui_scale, true);
        }
    }

    /// A combo fired: show what is being opened for a moment (0.7 s) without taking the focus away
    /// from it, then put the launcher away again.
    pub fn flash(&mut self, gfx: &Gfx, ui_scale: f32, label: &str, target: &str) {
        let t = target.trim();
        let expanded = crate::dock::expand_env(t);
        let low = t.to_ascii_lowercase();
        let on_disk = Path::new(&expanded);
        let browser = low == "browser" || low == "default browser";
        let (kind, path) = if browser {
            (Kind::Url, self.cfg.search_url.clone())
        } else if low.starts_with("http://") || low.starts_with("https://") || low.starts_with("www.") {
            (Kind::Url, expanded.clone())
        } else if on_disk.is_dir() {
            (Kind::Folder, expanded.clone())
        } else if low.starts_with("shell:") || low.ends_with(".lnk") || low.ends_with(".exe") || !on_disk.exists() {
            (Kind::App, expanded.clone())
        } else {
            (Kind::File, expanded.clone())
        };
        let name = if !label.trim().is_empty() {
            label.trim().to_string()
        } else if browser {
            "Browser".to_string()
        } else {
            on_disk.file_stem().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or(expanded)
        };
        self.pinned = Some(Hit { name, detail: String::new(), path, kind, spans: Vec::new(), idx: 0 });
        self.mode = Mode::Search;
        self.show(gfx, ui_scale, false);
        self.flashing = true;
        unsafe {
            SetTimer(Some(self.hwnd), T_FLASH, 700, None);
        }
    }

    fn show(&mut self, gfx: &Gfx, ui_scale: f32, focus: bool) {
        // asked again while it was still fading out: the window is already there
        let again = self.visible;
        self.query.clear();
        self.hits.clear();
        self.sel = 0;
        self.top = 0;
        self.act = 0;
        self.focus = Focus::Subject;
        self.list_open = false;
        self.high = None;
        self.mouse = (-1.0, -1.0);
        self.ai.clear();
        // nothing typed yet: the apps used most are what the Down arrow lists (a combo shows its own thing)
        if let Some(h) = self.pinned.take() {
            self.hits.push(h);
        } else {
            self.search();
        }

        // the clipboard history is a list: it is open from the start
        self.list_open = self.mode == Mode::Clip && !self.hits.is_empty();
        if self.cfg.files && !self.files_started && self.mode == Mode::Search {
            self.files_started = true;
            spawn_files(self.files_slot.clone(), SendHwnd::new(self.hwnd), self.cfg.folders.clone());
        }
        if !again {
            if !self.place(ui_scale * self.cfg.size) {
                return;
            }
            // the screen behind the window, copied before the window is shown
            let (x, y, w, h) = self.win_px;
            if let Some(old) = self.blur_img.take() {
                self.res.forget(old.id);
            }
            // media is playing: the system's own blur keeps showing what moves behind the window;
            // otherwise the screen is copied once and blurred
            self.live = self.cfg.glass && self.cfg.live && self.cfg.media_playing;
            self.blur_img = if self.live || !self.cfg.glass { None } else { capture_blur(x, y, w, h, self.cfg.blur) };
            self.anim = 0.0;
            self.lh = 0.0;
            self.focus_t = 0.0;
            self.sel_y = 0.0;
        }
        self.open = true;
        self.visible = true;
        self.ul = 0.0;
        self.pop_s = 0.0;
        self.pop_a = 0.0;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_TRIM);
            let _ = KillTimer(Some(self.hwnd), T_FREE);
            let _ = KillTimer(Some(self.hwnd), T_FLASH);
            self.flashing = false;
            if !again {
                // a combo's flash must not take the focus from the thing that is opening
                let _ = ShowWindow(self.hwnd, if focus { SW_SHOW } else { SW_SHOWNOACTIVATE });
            }
            if focus {
                if !SetForegroundWindow(self.hwnd).as_bool() || GetForegroundWindow() != self.hwnd {
                    // Windows only lets the window in front take the focus: a harmless key tap unlocks it
                    let inputs = [key_input(VK_MENU, false), key_input(VK_MENU, true)];
                    SendInput(&inputs, size_of::<INPUT>() as i32);
                    let _ = SetForegroundWindow(self.hwnd);
                }
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        self.draw(gfx);
    }

    /// Start closing: it fades out, then `finish_hide` puts the window away.
    pub fn hide(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.flashing = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_ICON);
            let _ = KillTimer(Some(self.hwnd), T_FLASH);
        }
        self.kick();
    }

    fn finish_hide(&mut self) {
        self.visible = false;
        self.sw = 1.0;
        self.query.clear();
        self.hits.clear();
        self.ai.clear();
        self.list_open = false;
        self.lh = 0.0;
        self.wx = None;
        if let Some(old) = self.blur_img.take() {
            self.res.forget(old.id);
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            SetTimer(Some(self.hwnd), T_TRIM, 20_000, None);
        }
    }

    // -- animation ---------------------------------------------------------------------------------------

    /// Ask for animation frames: the timer runs while something is still moving and stops by itself.
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

    /// How tall the list wants to be.
    fn list_target(&self) -> f32 {
        let rows = self.rows_len();
        if self.list_open && rows > 0 {
            HEAD + rows.min(VIS) as f32 * ROW + 10.0
        } else {
            0.0
        }
    }

    fn sel_target(&self) -> f32 {
        self.cursor().saturating_sub(self.view_top()) as f32
    }

    fn needs_anim(&self) -> bool {
        let open_to = if self.open { 1.0 } else { 0.0 };
        let focus_to = if self.focus == Focus::Action { 1.0 } else { 0.0 };
        self.anim != open_to
            || (self.lh - self.list_target()).abs() > 0.3
            || self.ul < 1.0
            || self.pop_s < 1.0
            || self.pop_a < 1.0
            || self.sw < 1.0
            || (self.mode == Mode::Ai && self.ai.moving())
            || (self.focus_t - focus_to).abs() > 0.01
            || (self.sel_y - self.sel_target()).abs() > 0.01
    }

    /// Move everything a little closer to where it belongs. True while something is still moving.
    fn tick(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        let snap = !self.cfg.animate;
        let open_to = if self.open { 1.0 } else { 0.0 };
        let dur = if self.open { 0.20 } else { 0.13 };
        self.anim = if snap { open_to } else { logic::toward(self.anim, open_to, dt / dur) };
        let lt = self.list_target();
        self.lh = if snap { lt } else { logic::follow(self.lh, lt, dt, 15.0, 0.3) };
        self.ul = if snap { 1.0 } else { logic::toward(self.ul, 1.0, dt / 0.32) };
        self.pop_s = if snap { 1.0 } else { logic::toward(self.pop_s, 1.0, dt / 0.30) };
        self.pop_a = if snap { 1.0 } else { logic::toward(self.pop_a, 1.0, dt / 0.30) };
        self.sw = if snap { 1.0 } else { logic::toward(self.sw, 1.0, dt / 0.28) };
        self.ai.glide(dt, snap);
        let ft = if self.focus == Focus::Action { 1.0 } else { 0.0 };
        self.focus_t = if snap { ft } else { logic::follow(self.focus_t, ft, dt, 18.0, 0.01) };
        let st = self.sel_target();
        // a list that is just opening, or a jump over many rows, does not slide
        self.sel_y = if snap || self.lh < 1.0 || (st - self.sel_y).abs() > 7.0 {
            st
        } else {
            logic::follow(self.sel_y, st, dt, 24.0, 0.01)
        };
        self.needs_anim()
    }

    /// The window goes in the upper part of the screen the pointer is on.
    fn place(&mut self, ui_scale: f32) -> bool {
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
            let scale = dx as f32 / 96.0 * ui_scale;
            let work = mi.rcWork;
            let (wpx, hpx) = ((W * scale).ceil() as i32, (FULL_H * scale).ceil() as i32);
            let x = (work.left + work.right) / 2 - wpx / 2;
            // the two boxes sit in the middle of the screen (a bit above the exact middle); the list drops below them
            let mid = work.top + ((work.bottom - work.top) as f32 * 0.40) as i32;
            let mut y = mid - (FRAME_H * scale * 0.5) as i32;
            if y + hpx > work.bottom - 8 {
                y = (work.bottom - 8 - hpx).max(work.top + 8);
            }
            let geom = (x, y, wpx, hpx);
            if geom != self.win_px || (scale - self.scale).abs() > 0.001 {
                self.scale = scale;
                self.win_px = geom;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, wpx, hpx, SWP_NOACTIVATE);
                let _ = self.host.resize(wpx, hpx);
                self.region_h = -1;
            }
        }
        true
    }

    /// Only the glass card takes clicks; the rest of the window is empty.
    fn set_region(&mut self, card_h: f32) {
        let (_, _, wpx, hpx) = self.win_px;
        let h = ((card_h * self.scale).ceil() as i32).min(hpx);
        if h == self.region_h || wpx == 0 {
            return;
        }
        self.region_h = h;
        unsafe {
            let rgn = CreateRectRgn(0, 0, wpx, h);
            SetWindowRgn(self.hwnd, Some(rgn), false);
        }
    }

    // -- search -----------------------------------------------------------------------------------------

    fn search(&mut self) {
        if self.mode == Mode::Ai {
            self.hits.clear();
            self.wx = None;
            return;
        }
        if self.mode == Mode::Clip {
            self.clip_search();
            return;
        }
        self.hits.clear();
        self.wx = None;
        let q = self.query.trim().to_string();
        if q.is_empty() {
            self.recent();
            return;
        }
        if logic::weather_query(&q) {
            self.wx = crate::modules::weather::brief();
            let search = self.cfg.search_url.clone();
            match self.wx.clone() {
                Some(b) => {
                    let url = format!("{search}{}", logic::enc(&format!("weather {}", b.place)));
                    self.hits.push(Hit {
                        name: format!("{:.0}{}", b.temp, b.unit()),
                        detail: format!("{} · {}", b.label, b.place),
                        path: url.clone(),
                        kind: Kind::Weather,
                        spans: Vec::new(),
                        idx: 0,
                    });
                    for (i, d) in b.days.iter().take(5).enumerate() {
                        let rain = d.rain.map(|r| format!("  ·  rain {r:.0}%")).unwrap_or_default();
                        self.hits.push(Hit {
                            name: d.label.clone(),
                            detail: format!("{:.0}° / {:.0}°{rain}", d.hi, d.lo),
                            path: url.clone(),
                            kind: Kind::Day,
                            spans: Vec::new(),
                            idx: i,
                        });
                    }
                }
                None => self.hits.push(Hit {
                    name: "--".into(),
                    detail: "No weather yet: set your city in Settings".into(),
                    path: format!("{search}{}", logic::enc("weather")),
                    kind: Kind::Weather,
                    spans: Vec::new(),
                    idx: 0,
                }),
            }
        }
        if let Some(r) = logic::calc(&q) {
            self.hits.push(Hit {
                name: r,
                detail: q.clone(),
                path: String::new(),
                kind: Kind::Calc,
                spans: Vec::new(),
                idx: 0,
            });
        } else if let Some(u) = logic::as_url(&q) {
            self.hits.push(Hit {
                name: u.clone(),
                detail: "Open in the browser".into(),
                path: u,
                kind: Kind::Url,
                spans: Vec::new(),
                idx: 0,
            });
        }
        let ql = q.to_ascii_lowercase();
        let qb = &ql.as_bytes()[..ql.len().min(40)];
        let na = self.apps.len();
        let mut pos = std::mem::take(&mut self.scratch);
        let mut best: Vec<(i32, u32)> = Vec::with_capacity(KEEP + 1);
        // what was chosen before for this typing (or for a longer typing that starts like it) comes first
        let mut learned: HashMap<&str, i32> = HashMap::new();
        if ql.len() >= 2 {
            for ((typed, path), &n) in &self.learn {
                if typed.starts_with(&ql) {
                    let n = n.min(4) as i32;
                    let b = if typed.len() == ql.len() { 4_000 + 1_000 * n } else { 2_000 + 500 * n };
                    let e = learned.entry(path.as_str()).or_insert(0);
                    *e = (*e).max(b);
                }
            }
        }
        for (k, e) in self.apps.iter().chain(self.files.iter()).enumerate() {
            let name = e.name();
            let Some(s) = logic::matched(name.as_bytes(), qb, k < na || qb.len() >= 3, &mut pos) else { continue };
            let mut s = s - name.len().min(80) as i32;
            if !learned.is_empty() {
                s += learned.get(&*e.path).copied().unwrap_or(0);
            }
            s += match e.kind {
                Kind::App => 800,
                Kind::Folder => 100,
                _ => 0,
            };
            if let Some(&n) = self.usage.get(&*e.path) {
                s += n.min(20) as i32 * 12;
            }
            if best.len() < KEEP || s > best[best.len() - 1].0 {
                let at = best.partition_point(|b| b.0 >= s);
                best.insert(at, (s, k as u32));
                best.truncate(KEEP);
            }
        }
        for &(_, k) in &best {
            let k = k as usize;
            let e = if k < na { &self.apps[k] } else { &self.files[k - na] };
            let _ = logic::matched(e.name().as_bytes(), qb, k < na || qb.len() >= 3, &mut pos);
            let detail = match e.kind {
                Kind::App if e.path.starts_with("shell:") => "Store app".to_string(),
                Kind::App => "Application".to_string(),
                _ => e.path.to_string(),
            };
            self.hits.push(Hit {
                name: e.name().to_string(),
                detail,
                path: e.path.to_string(),
                kind: e.kind,
                spans: logic::spans(&pos),
                idx: 0,
            });
        }
        self.scratch = pos;
        let url = format!("{}{}", self.cfg.search_url, logic::enc(&q));
        self.hits.push(Hit {
            name: format!("Search the web for “{}”", logic::mid(&q, 40)),
            detail: "Web search".into(),
            path: url,
            kind: Kind::Web,
            spans: Vec::new(),
            idx: 0,
        });
    }

    /// Clipboard mode: what was copied (the letters typed narrow it down).
    fn clip_search(&mut self) {
        self.hits.clear();
        self.wx = None;
        for r in crate::cliphist::rows(&self.query) {
            let path = format!("clip:{}", r.id);
            if let Some(img) = r.thumb {
                self.icons.insert(path.clone(), Some(img));
            }
            let kind = ["Text", "Link", "Image", "Files"][usize::from(r.code).min(3)];
            self.hits.push(Hit {
                name: r.summary,
                detail: format!("{kind} · {}", logic::ago(r.age_secs)),
                path,
                kind: Kind::Clip,
                spans: Vec::new(),
                idx: usize::from(r.code),
            });
        }
    }

    /// Nothing typed yet: the apps used most.
    fn recent(&mut self) {
        let mut used: Vec<(&String, &u32)> = self.usage.iter().collect();
        used.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (path, _) in used.into_iter().take(8) {
            if let Some(e) = self.apps.iter().find(|e| &*e.path == path.as_str()) {
                let detail = if e.path.starts_with("shell:") { "Store app" } else { "Application" };
                self.hits.push(Hit {
                    name: e.name().to_string(),
                    detail: detail.into(),
                    path: e.path.to_string(),
                    kind: Kind::App,
                    spans: Vec::new(),
                    idx: 0,
                });
            }
        }
    }

    /// The text changed: start again from the best match.
    fn changed(&mut self) {
        let before = self.cur().map(|h| (h.path.clone(), h.kind));
        self.focus = Focus::Subject;
        self.sel = 0;
        self.top = 0;
        self.act = 0;
        self.search();
        if self.cur().map(|h| (h.path.clone(), h.kind)) != before {
            // a different best match: everything about it comes in again
            self.ul = 0.0;
            self.pop_s = 0.0;
            self.pop_a = 0.0;
        } else {
            // the same one with more letters underlined: only the underline grows
            self.ul = self.ul.min(0.5);
        }
        if self.hits.is_empty() {
            self.list_open = false;
        }
    }

    fn shows_hit(&self) -> bool {
        !self.hits.is_empty() && (!self.query.trim().is_empty() || self.list_open)
    }

    fn cur(&self) -> Option<&Hit> {
        if self.shows_hit() {
            self.hits.get(self.sel)
        } else {
            None
        }
    }

    fn rows_len(&self) -> usize {
        match self.focus {
            Focus::Subject => self.hits.len(),
            Focus::Action => self.hits.get(self.sel).map_or(0, |h| actions(h).len()),
        }
    }

    fn cursor(&self) -> usize {
        match self.focus {
            Focus::Subject => self.sel,
            Focus::Action => self.act,
        }
    }

    fn set_cursor(&mut self, i: usize) {
        match self.focus {
            Focus::Subject => {
                if i != self.sel {
                    self.sel = i;
                    self.act = 0;
                    self.ul = 0.0;
                    self.pop_s = 0.0;
                    self.pop_a = 0.0;
                }
                if i < self.top {
                    self.top = i;
                } else if i >= self.top + VIS {
                    self.top = i + 1 - VIS;
                }
            }
            Focus::Action => {
                if i != self.act {
                    self.act = i;
                    self.pop_a = 0.0;
                }
            }
        }
    }

    fn step(&mut self, d: i32) {
        let len = self.rows_len();
        if len == 0 {
            return;
        }
        let n = (self.cursor() as i32 + d).clamp(0, len as i32 - 1) as usize;
        self.set_cursor(n);
    }

    fn view_top(&self) -> usize {
        if self.focus == Focus::Subject {
            self.top
        } else {
            0
        }
    }

    // -- doing it ----------------------------------------------------------------------------------------

    /// Send the question to Gemini (the key is the one from Settings → Gemini).
    fn ask(&mut self) {
        let question = self.query.trim().to_string();
        if question.is_empty() {
            return;
        }
        self.ai.clear();
        let key = crate::sys::dotenv::get_saved_first(crate::modules::gemini::KEY_VAR)
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        let Some(key) = key else {
            self.ai.state = AiState::Failed;
            self.ai.text = "There is no Gemini key yet. Paste one in Settings \u{2192} Gemini and ask again.".into();
            return;
        };
        self.ai.state = AiState::Waiting;
        self.ai.cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Ok(mut sh) = self.ai.shared.lock() {
            *sh = AiShared { seq: self.ai.seq, text: String::new(), done: false, err: None };
        }
        spawn_ai(
            self.ai.shared.clone(),
            SendHwnd::new(self.hwnd),
            self.ai.seq,
            self.ai.cancel.clone(),
            self.cfg.gemini.clone(),
            key,
            question,
        );
    }

    /// Take what the request has written so far. True when the picture needs to change.
    fn ai_poll(&mut self) -> bool {
        let (text, done, err) = match self.ai.shared.lock() {
            Ok(sh) if sh.seq == self.ai.seq && self.ai.state == AiState::Waiting => {
                (sh.text.clone(), sh.done, sh.err.clone())
            }
            _ => return false,
        };
        let before = (self.ai.text.len(), self.ai.state);
        if let Some(e) = err {
            self.ai.state = AiState::Failed;
            self.ai.text = if text.is_empty() { e } else { format!("{text}\n\n{e}") };
        } else {
            self.ai.text = text;
            if done {
                self.ai.state = AiState::Done;
            }
        }
        before != (self.ai.text.len(), self.ai.state)
    }

    /// Keys that mean something else in Gemini mode. True when the key was used.
    fn ai_key(&mut self, vk: u16, ctrl: bool) -> bool {
        let step = |me: &mut Self, d: f32| me.ai.scroll = (me.ai.scroll + d).clamp(0.0, me.ai.max_scroll);
        match VIRTUAL_KEY(vk) {
            VK_RETURN => self.ask(),
            VK_DOWN => step(self, 24.0),
            VK_UP => step(self, -24.0),
            VK_NEXT => step(self, 120.0),
            VK_PRIOR => step(self, -120.0),
            VK_HOME => step(self, f32::MIN / 2.0),
            VK_END => step(self, f32::MAX / 2.0),
            // Ctrl+C: take the answer along
            VK_C if ctrl && self.ai.state == AiState::Done => {
                clip::set_text(self.hwnd, &self.ai.text);
            }
            _ => return false,
        }
        true
    }

    fn query_limit(&self) -> usize {
        if self.mode == Mode::Ai {
            400
        } else {
            200
        }
    }

    fn run(&mut self) {
        let Some(h) = self.cur().cloned() else { return };
        let acts = actions(&h);
        let a = acts[self.act.min(acts.len() - 1)];
        let hwnd = self.hwnd;
        let typed = self.query.trim().to_ascii_lowercase();
        if a == Act::DeleteClip {
            // stays open: the item just leaves the list
            if let Some(id) = clip_id(&h.path) {
                crate::cliphist::remove(id);
            }
            let keep = self.sel;
            self.focus = Focus::Subject;
            self.act = 0;
            self.search();
            self.sel = keep.min(self.hits.len().saturating_sub(1));
            self.kick();
            return;
        }
        self.hide();
        match a {
            Act::Paste | Act::CopyClip => {
                if let Some(id) = clip_id(&h.path) {
                    if crate::cliphist::copy_back(hwnd, id) && a == Act::Paste {
                        paste_soon();
                    }
                }
            }
            Act::DeleteClip => {}
            Act::Open | Act::OpenLink | Act::Search | Act::Forecast => sys::shell_open(&h.path),
            Act::Admin => run_as_admin(&h.path),
            Act::Reveal => sys::shell_reveal(&h.path),
            Act::Quit | Act::ForceQuit => {
                if let Some(exe) = exe_name(&h.path) {
                    taskkill(&exe, a == Act::ForceQuit);
                }
            }
            Act::Restart => {
                if let Some(exe) = exe_name(&h.path) {
                    restart_app(exe, h.path.clone());
                }
            }
            Act::CopyPath => {
                clip::set_text(hwnd, &h.path);
            }
            Act::CopyResult => {
                clip::set_text(hwnd, &h.name);
            }
        }
        if h.kind.has_icon() && matches!(a, Act::Open | Act::Admin) {
            self.bump(&h.path);
            self.teach(&typed, &h.path);
        }
    }

    /// Remember what was chosen for what was typed: the same typing finds it first next time. Other
    /// things chosen for the same typing before count a little less after this.
    fn teach(&mut self, typed: &str, path: &str) {
        if typed.len() < 2 || typed.len() > 40 {
            return;
        }
        let typed = typed.to_string();
        self.learn.retain(|(t, p), n| {
            if *t == typed && p != path {
                *n = n.saturating_sub(1);
            }
            *n > 0
        });
        *self.learn.entry((typed, path.to_string())).or_insert(0) += 1;
        if self.learn.len() > 400 {
            if let Some(low) = self.learn.iter().min_by_key(|(_, n)| **n).map(|(k, _)| k.clone()) {
                self.learn.remove(&low);
            }
        }
        save_learn(&self.learn);
    }

    /// Remember what was opened: it ranks higher next time.
    fn bump(&mut self, path: &str) {
        *self.usage.entry(path.to_string()).or_insert(0) += 1;
        if self.usage.len() > 400 {
            if let Some(low) = self.usage.iter().min_by_key(|(_, n)| **n).map(|(k, _)| k.clone()) {
                self.usage.remove(&low);
            }
        }
        save_usage(&self.usage);
    }

    // -- input -------------------------------------------------------------------------------------------

    fn key(&mut self, gfx: &Gfx, vk: u16) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let has_rows = !self.hits.is_empty();
        if VIRTUAL_KEY(vk) == VK_TAB {
            // Search -> clipboard -> Gemini (Shift+Tab goes back); Left and Right move between the two boxes
            self.mode = self.mode.cycle(shift);
            self.sw = if self.cfg.animate { 0.0 } else { 1.0 };
            self.sw_dir = if shift { -1.0 } else { 1.0 };
            self.show(gfx, 1.0, true);
            return;
        }
        if self.mode == Mode::Ai && self.ai_key(vk, ctrl) {
            self.draw(gfx);
            return;
        }
        match VIRTUAL_KEY(vk) {
            VK_ESCAPE => {
                if self.list_open && self.mode == Mode::Search {
                    self.list_open = false;
                } else {
                    self.hide();
                    return;
                }
            }
            VK_RETURN => {
                self.run();
                return;
            }
            // Ctrl+K: the menu of things to do with the match (quit, restart, force quit, ...)
            VIRTUAL_KEY(0x4B) if ctrl => {
                if self.focus == Focus::Action && self.list_open {
                    self.focus = Focus::Subject;
                    self.list_open = false;
                } else if self.cur().is_some_and(|h| actions(h).len() > 1) {
                    self.focus = Focus::Action;
                    self.list_open = true;
                    self.act = 0;
                    self.pop_a = 0.0;
                    let c = self.cursor();
                    self.set_cursor(c);
                }
            }
            VK_RIGHT => {
                if self.cur().is_some() {
                    self.focus = Focus::Action;
                }
            }
            VK_LEFT => self.focus = Focus::Subject,
            VK_DOWN => {
                if !self.list_open {
                    if has_rows && self.rows_len() > 0 {
                        self.list_open = true;
                        let c = self.cursor();
                        self.set_cursor(c);
                    }
                } else {
                    self.step(1);
                }
            }
            VK_UP => {
                if self.list_open {
                    if self.cursor() == 0 && self.mode == Mode::Search {
                        self.list_open = false;
                    } else {
                        self.step(-1);
                    }
                }
            }
            VK_NEXT => {
                if self.list_open {
                    self.step(VIS as i32);
                }
            }
            VK_PRIOR => {
                if self.list_open {
                    self.step(-(VIS as i32));
                }
            }
            VK_HOME => {
                if self.list_open {
                    self.step(i32::MIN / 2);
                }
            }
            VK_END => {
                if self.list_open {
                    self.step(i32::MAX / 2);
                }
            }
            VK_BACK => {
                if ctrl {
                    while self.query.ends_with(' ') {
                        self.query.pop();
                    }
                    while self.query.chars().last().is_some_and(|c| c != ' ') {
                        self.query.pop();
                    }
                } else {
                    self.query.pop();
                }
                self.changed();
            }
            // clipboard mode: Delete takes the item out of the history
            VK_DELETE if self.mode == Mode::Clip && has_rows => {
                if let Some(id) = self.cur().and_then(|h| clip_id(&h.path)) {
                    crate::cliphist::remove(id);
                }
                let keep = self.sel;
                self.search();
                self.sel = keep.min(self.hits.len().saturating_sub(1));
            }
            VK_DELETE => {
                self.query.clear();
                self.changed();
            }
            VK_V if ctrl => {
                if let Some(t) = clip::get_text(self.hwnd) {
                    let room = self.query_limit().saturating_sub(self.query.chars().count());
                    let one: String = if self.mode == Mode::Ai {
                        // a question may run over several lines: they are joined
                        t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(room).collect()
                    } else {
                        t.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(room).collect()
                    };
                    self.query.push_str(&one);
                    self.changed();
                }
            }
            _ => return,
        }
        self.draw(gfx);
    }

    fn typed(&mut self, gfx: &Gfx, c: char) {
        if c.is_control() || (c == ' ' && self.query.is_empty()) || self.query.chars().count() >= self.query_limit() {
            return;
        }
        self.query.push(c);
        self.changed();
        self.draw(gfx);
    }

    fn pt(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        (x / self.scale, y / self.scale)
    }

    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let list = self.list_open && self.rows_len() > 0;
        let y0 = FRAME_H + HEAD;
        if !list || x < 10.0 || x > W - 10.0 || y < y0 {
            return None;
        }
        let k = ((y - y0) / ROW) as usize;
        (k < self.rows_len().min(VIS)).then(|| self.view_top() + k)
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
                self.key(gfx, wp.0 as u16);
                Some(LRESULT(0))
            }
            WM_CHAR | WM_SYSCHAR if self.open => {
                let u = wp.0 as u16;
                let ch = if (0xD800..0xDC00).contains(&u) {
                    self.high = Some(u);
                    None
                } else if (0xDC00..0xE000).contains(&u) {
                    self.high.take().and_then(|hi| char::decode_utf16([hi, u]).next()?.ok())
                } else {
                    char::from_u32(u as u32)
                };
                if let Some(c) = ch {
                    self.typed(gfx, c);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE if self.open => {
                let (x, y) = self.pt(lp);
                if self.mouse.0 < 0.0 {
                    // the first report only says where the pointer already was
                    self.mouse = (x, y);
                    return Some(LRESULT(0));
                }
                if (x - self.mouse.0).abs() + (y - self.mouse.1).abs() < 1.0 {
                    return Some(LRESULT(0));
                }
                self.mouse = (x, y);
                if let Some(i) = self.row_at(x, y) {
                    if i != self.cursor() {
                        self.set_cursor(i);
                        self.draw(gfx);
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP if self.open => {
                let (x, y) = self.pt(lp);
                if let Some(i) = self.row_at(x, y) {
                    self.set_cursor(i);
                    self.run();
                    return Some(LRESULT(0));
                }
                let (sub, act) = (Rect::new(PAD, PAD, BOX, BOX), Rect::new(PAD + BOX + GAP, PAD, BOX, BOX));
                let want = if sub.contains(x, y) {
                    Some(Focus::Subject)
                } else if act.contains(x, y) {
                    Some(Focus::Action)
                } else {
                    None
                };
                if let Some(f) = want {
                    if f == Focus::Action && self.cur().is_none() {
                        return Some(LRESULT(0));
                    }
                    let same = self.focus == f;
                    self.focus = f;
                    // a second click on the same box opens or closes its list
                    self.list_open = if same {
                        !self.list_open && self.rows_len() > 0
                    } else {
                        self.list_open && self.rows_len() > 0
                    };
                    self.draw(gfx);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL if self.open => {
                if self.mode == Mode::Ai {
                    let d = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32 / 120;
                    self.ai.scroll = (self.ai.scroll - d as f32 * 36.0).clamp(0.0, self.ai.max_scroll);
                    self.draw(gfx);
                } else if self.list_open && self.focus == Focus::Subject {
                    let d = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32 / 120;
                    let max = self.hits.len().saturating_sub(VIS) as i32;
                    self.top = (self.top as i32 - d).clamp(0, max) as usize;
                    self.draw(gfx);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_ANIM => {
                let more = self.tick();
                if !more {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_ANIM);
                    }
                    self.timer_on = false;
                }
                if !self.open && self.anim <= 0.0 {
                    self.finish_hide();
                } else {
                    self.draw(gfx);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_FLASH => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_FLASH);
                }
                if self.flashing {
                    self.hide();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_ICON => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_ICON);
                }
                self.draw(gfx);
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_TRIM => {
                // quiet for a while: hand the drawing memory back, then forget everything later
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_TRIM);
                    SetTimer(Some(self.hwnd), T_FREE, 280_000, None);
                }
                self.icons.clear();
                self.host.reset_dc(gfx);
                self.res.trim_bitmaps();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_FREE => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_FREE);
                }
                if !self.open && !self.visible {
                    self.free_me = true;
                }
                Some(LRESULT(0))
            }
            WM_AI => {
                if self.ai_poll() && self.open && self.mode == Mode::Ai {
                    self.draw(gfx);
                }
                Some(LRESULT(0))
            }
            WM_INDEX => {
                if let Some(v) = self.apps_slot.lock().ok().and_then(|mut g| g.take()) {
                    self.apps = v;
                    self.apps_ready = true;
                }
                if let Some(v) = self.files_slot.lock().ok().and_then(|mut g| g.take()) {
                    self.files = v;
                }
                if self.open {
                    let (sel, top) = (self.sel, self.top);
                    self.search();
                    self.sel = sel.min(self.hits.len().saturating_sub(1));
                    self.top = top.min(self.hits.len().saturating_sub(VIS));
                    self.draw(gfx);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    // -- drawing -----------------------------------------------------------------------------------------

    /// Shell icons of what is on screen, a few per frame so typing never stalls. True while more are due.
    fn load_icons(&mut self) -> bool {
        let mut want: Vec<String> = Vec::new();
        if let Some(h) = self.cur() {
            if h.kind.has_icon() {
                want.push(h.path.clone());
            }
        }
        if self.list_open && self.focus == Focus::Subject {
            for h in self.hits.iter().skip(self.top).take(VIS) {
                if h.kind.has_icon() {
                    want.push(h.path.clone());
                }
            }
        }
        want.retain(|k| !self.icons.contains_key(k));
        if want.is_empty() {
            return false;
        }
        if self.icons.len() > 96 {
            self.icons.clear();
        }
        let mut loaded = 0;
        let mut missing = false;
        for k in want {
            if self.icons.contains_key(&k) {
                continue;
            }
            if loaded >= 3 {
                missing = true;
                break;
            }
            loaded += 1;
            let img = dragdrop::thumbnail(&k, 96);
            self.icons.insert(k, img);
        }
        missing
    }

    fn draw(&mut self, gfx: &Gfx) {
        if !self.visible || self.win_px.2 == 0 {
            return;
        }
        if self.needs_anim() {
            self.kick();
        }
        let missing = self.load_icons();
        if missing {
            unsafe {
                SetTimer(Some(self.hwnd), T_ICON, 6, None);
            }
        }
        LIGHT.store(self.cfg.light, std::sync::atomic::Ordering::Relaxed);
        let light = self.cfg.light;
        let sc = self.scale;
        let e = logic::ease_out(self.anim);
        let shown = self.rows_len().min(VIS);
        let card_h = FRAME_H + self.lh.max(0.0);
        // clicks land on the card only: the region is as tall as the card is (or is about to be)
        self.set_region(FRAME_H + self.lh.max(self.list_target()));
        let cur = self.cur().cloned();
        let vtop = self.view_top();
        let wx = self.wx.clone();
        let mut ai_max = self.ai.max_scroll;
        // live glass: the system blur behind the card, following the card as it grows
        let s0 = 0.95 + 0.05 * e;
        let live_op = if self.live && self.cfg.glass { e } else { 0.0 };
        self.host.set_backdrop_rounded(
            Rect::new(ANCHOR.0 * sc * (1.0 - s0), ANCHOR.1 * sc * (1.0 - s0), W * s0 * sc, card_h * s0 * sc),
            CARD_R * s0 * sc,
            live_op,
        );
        let mut ai_goal = None;
        let (dc, _) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(err) => {
                crate::log!("launcher: begin_draw failed: {err}");
                return;
            }
        };
        {
            let mut p = Painter::new(dc, gfx, &mut self.res, sc, 0.0, 0.0);
            p.clear();
            p.alpha = e;
            // opening: appears where it is, fading in while growing from 95% around its middle; closing runs it backwards
            let tr = Tr { s0: 0.95 + 0.05 * e, top: 0.0, dx: 0.0 };
            // changing the mode: the contents slide in from the side they came from
            let sw = logic::ease_out(self.sw);
            let ct = Tr { dx: self.sw_dir * (1.0 - sw) * 30.0, ..tr };
            card_xf(&mut p, &tr);
            let card = Rect::new(0.0, 0.0, W, card_h);
            let dark = self.cfg.dark;
            let glass = self.cfg.glass;
            match &self.blur_img {
                Some(img) if glass => {
                    let whole = Rect::new(0.0, 0.0, self.win_px.2 as f32 / sc, self.win_px.3 as f32 / sc);
                    // liquid glass: toward the edge the picture looks magnified, like thick glass, in a few soft steps
                    let (cx, cy) = (card.cx(), card.cy());
                    for (k, inset) in [(1.07, 0.0), (1.045, 5.0), (1.022, 9.0), (1.0, 13.0)] {
                        let dest =
                            Rect::new(cx + (whole.x - cx) * k, cy + (whole.y - cy) * k, whole.w * k, whole.h * k);
                        p.image_window(img, dest, card.inset(inset), (CARD_R - inset).max(8.0), 1.0);
                    }
                    let tint = if light {
                        Color::rgba(0.95, 0.97, 1.0, 0.35 + 0.5 * dark)
                    } else {
                        Color::rgba(0.03, 0.03, 0.05, dark)
                    };
                    p.fill_rounded(card, CARD_R, tint);
                }
                // media is playing: the system's blur is behind the card, only the tint is drawn
                None if glass && self.live => {
                    let tint = if light {
                        Color::rgba(0.95, 0.97, 1.0, 0.25 + 0.4 * dark)
                    } else {
                        Color::rgba(0.03, 0.03, 0.05, dark * 0.75)
                    };
                    p.fill_rounded(card, CARD_R, tint);
                }
                // glass off, or no picture of the screen: a solid card
                _ if light => p.fill_rounded(card, CARD_R, Color::rgba(0.93, 0.95, 0.98, 0.96)),
                _ => p.fill_rounded(card, CARD_R, Color::rgba(0.05, 0.05, 0.07, dark.max(0.9))),
            }
            if glass {
                // the rim is the dock's (its thickness and brightness come from the dock settings)
                let (rw, rb) = (self.cfg.rim_width, if light { 0.75 } else { self.cfg.rim_bright });
                if rw > 0.05 && rb > 0.005 {
                    p.stroke_rounded(card.inset(rw * 0.5), (CARD_R - rw * 0.5).max(0.0), rw, Color::white(rb));
                }
                // light catching the inside of the edge, and along the top
                let inner = rw + 1.5;
                p.stroke_rounded(card.inset(inner), (CARD_R - inner).max(0.0), 1.0, Color::white(0.09));
                for (a, x0, x1) in [(0.28, 0.30, 0.70), (0.18, 0.18, 0.82), (0.12, 0.08, 0.92)] {
                    p.line(W * x0, rw + 0.6, W * x1, rw + 0.6, 1.2, Color::white(a));
                }
            } else {
                let rim = if light { Color::white(0.6) } else { Color::white(0.12) };
                p.stroke_rounded(card.inset(0.5), CARD_R - 0.5, 1.0, rim);
            }
            let sub = Rect::new(PAD, PAD, BOX, BOX);
            let act = Rect::new(PAD + BOX + GAP, PAD, BOX, BOX);
            // the focus glides from one box to the other
            let ft = self.focus_t;
            for (r, on) in [(sub, 1.0 - ft), (act, ft)] {
                let w = 1.0 + 0.6 * on;
                if light {
                    // frosted white glass; the box with the focus turns soft blue
                    let fill = Color::white(0.40).lerp(Color::rgba(0.62, 0.78, 0.97, 0.85), on);
                    p.fill_rounded(r, BOX_R, fill);
                    p.stroke_rounded(r.inset(w * 0.5), BOX_R - w * 0.5, w, Color::white(0.55 + 0.30 * on));
                } else {
                    p.fill_rounded(r, BOX_R, Color::white(0.09 + 0.08 * on));
                    p.stroke_rounded(r.inset(w * 0.5), BOX_R - w * 0.5, w, Color::white(0.15 + 0.40 * on));
                }
            }
            let icon = cur.as_ref().and_then(|h| self.icons.get(&h.path)).and_then(|o| o.as_ref());
            let subject = Look {
                cur: cur.as_ref(),
                icon,
                wx: wx.as_deref(),
                pop: self.pop_s,
                ul: self.ul,
                tr: &ct,
                ready: self.apps_ready,
                clip: self.mode == Mode::Clip,
            };
            // the contents of both boxes slide and fade in when the mode changes
            let keep = p.alpha;
            p.alpha = keep * (0.25 + 0.75 * sw);
            card_xf(&mut p, &ct);
            if self.mode == Mode::Ai {
                let hash = {
                    use std::hash::{Hash, Hasher};
                    let mut hs = std::collections::hash_map::DefaultHasher::new();
                    self.query.hash(&mut hs);
                    hs.finish() | 1
                };
                if hash != self.ai.fit_for || self.query.is_empty() {
                    ai_goal = Some((hash, ai_fit(&mut p, &self.query, BOX - 28.0, BOX - 28.0)));
                }
                let view = AiView {
                    query: &self.query,
                    state: self.ai.state,
                    text: &self.ai.text,
                    scroll: self.ai.scroll,
                    pop_s: self.pop_s,
                    pop_a: self.pop_a,
                    qsz: self.ai.sz,
                    qy: self.ai.y,
                };
                ai_max = draw_ai(&mut p, sub, act, &view);
            } else {
                draw_subject(&mut p, sub, &subject);
                draw_action(&mut p, act, cur.as_ref(), self.act, ft > 0.5, self.pop_a, &ct);
            }
            p.alpha = keep;
            card_xf(&mut p, &tr);
            if self.lh > 1.0 {
                p.push_clip(card);
                let view = ListView {
                    hits: &self.hits,
                    icons: &self.icons,
                    query: &self.query,
                    focus: self.focus,
                    sel: self.sel,
                    top: vtop,
                    act: self.act,
                    shown,
                    lh: self.lh,
                    sel_y: self.sel_y,
                    ul: self.ul,
                    wx: wx.as_deref(),
                    clip: self.mode == Mode::Clip,
                };
                draw_list(&mut p, &view);
                p.pop_clip();
            }
        }
        if let Err(err) = self.host.end_draw() {
            crate::log!("launcher: end_draw failed: {err}");
        }
        self.res.end_frame();
        self.ai.max_scroll = ai_max;
        if let Some((hash, goal)) = ai_goal {
            self.ai.fit_for = hash;
            self.ai.goal = goal;
            if self.ai.moving() {
                self.kick();
            }
        }
        self.ai.scroll = self.ai.scroll.min(ai_max);
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.ai.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        unsafe {
            for t in [T_ANIM, T_ICON, T_TRIM, T_FREE] {
                let _ = KillTimer(Some(self.hwnd), t);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// -- pieces of the picture -----------------------------------------------------------------------------

/// Light glass look (dark writing) or dark glass look (white writing); set at the start of every frame.
static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The colour of writing, icons and soft highlights: white on dark glass, dark ink on light glass.
fn white(a: f32) -> Color {
    if LIGHT.load(std::sync::atomic::Ordering::Relaxed) {
        Color::rgba(0.10, 0.12, 0.18, a)
    } else {
        Color::white(a)
    }
}

/// The icon of an empty search box: a letter and a text cursor.
fn type_glyph(p: &mut Painter, cx: f32, cy: f32, c: Color) {
    p.text("A", Rect::centered(cx - 10.0, cy, 44.0, 56.0), &TextStyle::new(46.0, c).center());
    let x = cx + 17.0;
    p.fill_rect(Rect::new(x - 0.75, cy - 18.0, 1.5, 36.0), c);
    p.fill_rect(Rect::new(x - 5.0, cy - 18.0, 10.0, 1.5), c);
    p.fill_rect(Rect::new(x - 5.0, cy + 16.5, 10.0, 1.5), c);
}

/// How the card sits at this moment (it fades and grows a little while opening and closing).
struct Tr {
    s0: f32,
    top: f32,
    /// Sideways shift (the contents sliding in when the mode changes).
    dx: f32,
}

/// The point the card grows from.
const ANCHOR: (f32, f32) = (W * 0.5, FRAME_H * 0.5);

fn card_xf(p: &mut Painter, t: &Tr) {
    p.set_local(t.s0, ANCHOR.0, ANCHOR.1, t.dx, t.top);
}

/// Scale one thing by `k` around (cx, cy), on top of the card's own transform.
fn pop_xf(p: &mut Painter, t: &Tr, k: f32, cx: f32, cy: f32) {
    let dx = ANCHOR.0 + t.s0 * (cx - k * cx - ANCHOR.0) + t.dx;
    let dy = ANCHOR.1 + t.s0 * (cy - k * cy - ANCHOR.1) + t.top;
    p.set_local(t.s0 * k, 0.0, 0.0, dx, dy);
}

/// A thin line under the letters that matched what was typed. It grows from left to right.
fn underline(p: &mut Painter, text: &str, spans: &[(usize, usize)], r: Rect, st: &TextStyle, center: bool, grow: f32) {
    if spans.is_empty() || grow <= 0.01 {
        return;
    }
    let (tw, th) = p.measure(text, st);
    if tw > r.w {
        // the end is cut off: the positions would be wrong
        return;
    }
    let x0 = if center { r.x + (r.w - tw) * 0.5 } else { r.x };
    let y = r.cy() + th * 0.38;
    let mut parts: Vec<(f32, f32)> = Vec::with_capacity(spans.len());
    for &(a, b) in spans {
        let (a, b) = logic::snap(text, a, b);
        let xa = if a == 0 { 0.0 } else { p.measure(&text[..a], st).0 };
        parts.push((xa, p.measure(&text[..b], st).0));
    }
    let total: f32 = parts.iter().map(|(a, b)| b - a).sum();
    let mut left = total * logic::ease_out(grow);
    for (xa, xb) in parts {
        let w = (xb - xa).max(1.0).min(left.max(0.0));
        left -= xb - xa;
        if w > 0.5 {
            p.fill_rect(Rect::new(x0 + xa, y, w, 1.3), white(0.95));
        }
    }
}

/// The picture of a match: its icon, or a stand-in while the icon loads.
fn draw_thumb(p: &mut Painter, r: Rect, h: &Hit, img: Option<&ImageData>, wx: Option<&Brief>) {
    match h.kind {
        Kind::Calc => {
            p.fill_rounded(r, r.w * 0.26, white(0.14));
            p.text("=", r, &TextStyle::new(r.w * 0.55, white(0.92)).semibold().center());
        }
        Kind::Url | Kind::Web => p.icon(Icon::Globe, r.inset(r.w * 0.1), white(0.9)),
        Kind::Clip => match img {
            Some(i) => p.image(i, r, 5.0, 1.0),
            None => {
                p.fill_rounded(r, r.w * 0.26, white(0.12));
                let glyph = match h.idx {
                    1 => Icon::Link,
                    2 => Icon::Image,
                    3 => Icon::File,
                    _ => Icon::Clipboard,
                };
                p.icon(glyph, r.inset(r.w * 0.22), white(0.85));
            }
        },
        Kind::Weather => match wx {
            Some(b) => b.draw_icon(p, r),
            None => p.icon(Icon::Cloud, r.inset(r.w * 0.1), white(0.9)),
        },
        Kind::Day => match wx {
            Some(b) => b.day_icon(h.idx, p, r),
            None => p.icon(Icon::Cloud, r.inset(r.w * 0.1), white(0.9)),
        },
        _ => match img {
            Some(i) => p.image(i, r, 0.0, 1.0),
            None => {
                // no icon yet: a glass tile with the same glyph the rest of the app uses
                p.fill_rounded(r, r.w * 0.26, white(0.12));
                let glyph = match h.kind {
                    Kind::Folder => Icon::Folder,
                    Kind::File => Icon::File,
                    _ => Icon::Grid,
                };
                p.icon(glyph, r.inset(r.w * 0.22), white(0.85));
            }
        },
    }
}

/// What the left box shows right now.
struct Look<'a> {
    cur: Option<&'a Hit>,
    icon: Option<&'a ImageData>,
    wx: Option<&'a Brief>,
    pop: f32,
    ul: f32,
    tr: &'a Tr,
    ready: bool,
    clip: bool,
}

fn draw_subject(p: &mut Painter, r: Rect, v: &Look) {
    let cx = r.cx();
    let Some(h) = v.cur else {
        if v.clip {
            p.icon(Icon::Clipboard, Rect::centered(cx, r.y + 68.0, 52.0, 52.0), white(0.55));
            p.text(
                "Nothing copied yet",
                Rect::new(r.x + 10.0, r.y + 118.0, r.w - 20.0, 20.0),
                &TextStyle::new(11.5, white(0.80)).semibold().center(),
            );
            return;
        }
        type_glyph(p, cx, r.y + 68.0, white(0.62));
        let msg = if v.ready { "Type…" } else { "Getting ready…" };
        p.text(
            msg,
            Rect::new(r.x + 10.0, r.y + 118.0, r.w - 20.0, 20.0),
            &TextStyle::new(11.5, white(0.80)).semibold().center(),
        );
        return;
    };
    if h.kind == Kind::Clip {
        draw_clip_preview(p, r, h, v);
        return;
    }
    let a0 = p.alpha;
    // a new match pops in: the picture swells from 80% with a little overshoot, the words fade up
    let fade = logic::ease_out((v.pop * 1.6).min(1.0));
    let k = 0.80 + 0.20 * logic::ease_back(v.pop);
    let slide = (1.0 - fade) * 5.0;
    if h.kind == Kind::Weather {
        let icon = Rect::centered(cx, r.y + 44.0, 60.0, 60.0);
        pop_xf(p, v.tr, k, cx, icon.cy());
        p.alpha = a0 * (0.3 + 0.7 * fade);
        draw_thumb(p, icon, h, None, v.wx);
        card_xf(p, v.tr);
        p.alpha = a0 * fade;
        p.text(
            &h.name,
            Rect::new(r.x + 8.0, r.y + 72.0 + slide, r.w - 16.0, 44.0),
            &TextStyle::new(36.0, white(0.96)).semibold().center(),
        );
        p.text(
            &h.detail,
            Rect::new(r.x + 10.0, r.y + 116.0 + slide, r.w - 20.0, 20.0),
            &TextStyle::new(12.0, white(0.78)).semibold().center(),
        );
        p.alpha = a0;
        return;
    }
    let shown = if h.kind == Kind::Calc { h.detail.as_str() } else { h.name.as_str() };
    if h.kind == Kind::Calc {
        p.alpha = a0 * fade;
        p.text(
            &h.name,
            Rect::new(r.x + 8.0, r.y + 28.0, r.w - 16.0, 66.0),
            &TextStyle::new(36.0, white(0.96)).semibold().center(),
        );
    } else {
        let ir = Rect::centered(cx, r.y + 68.0, 72.0, 72.0);
        pop_xf(p, v.tr, k, cx, ir.cy());
        p.alpha = a0 * (0.3 + 0.7 * fade);
        draw_thumb(p, ir, h, v.icon, v.wx);
        card_xf(p, v.tr);
    }
    p.alpha = a0 * (0.35 + 0.65 * fade);
    let name_r = Rect::new(r.x + 10.0, r.y + 116.0 + slide, r.w - 20.0, 22.0);
    let st = TextStyle::new(11.5, white(0.90)).semibold().center();
    p.text(shown, name_r, &st);
    if h.kind.has_icon() {
        underline(p, shown, &h.spans, name_r, &st, true, v.ul);
    }
    if h.kind == Kind::Day {
        p.alpha = a0 * fade;
        p.text(
            &h.detail,
            Rect::new(r.x + 10.0, r.y + 138.0, r.w - 20.0, 18.0),
            &TextStyle::new(10.5, white(0.55)).center(),
        );
    }
    p.alpha = a0;
}

/// The left box in clipboard mode: the picture, or the first lines of the words.
fn draw_clip_preview(p: &mut Painter, r: Rect, h: &Hit, v: &Look) {
    let a0 = p.alpha;
    let fade = logic::ease_out((v.pop * 1.6).min(1.0));
    p.alpha = a0 * (0.35 + 0.65 * fade);
    match (h.idx, v.icon) {
        (2, Some(img)) => {
            p.image_zoom(img, Rect::new(r.x + 14.0, r.y + 14.0, r.w - 28.0, r.h - 50.0), 10.0, 1.0, 0.0);
        }
        (2, None) => p.icon(Icon::Image, Rect::centered(r.cx(), r.y + 66.0, 52.0, 52.0), white(0.55)),
        _ => {
            let text = clip_id(&h.path).and_then(crate::cliphist::preview).unwrap_or_else(|| h.name.clone());
            let st = TextStyle::new(12.0, white(0.88));
            for (i, line) in text.lines().map(str::trim).filter(|l| !l.is_empty()).take(6).enumerate() {
                p.text(
                    &logic::mid(line, 24),
                    Rect::new(r.x + 14.0, r.y + 14.0 + i as f32 * 20.0, r.w - 28.0, 20.0),
                    &st,
                );
            }
        }
    }
    p.alpha = a0 * fade;
    p.text(
        &h.detail,
        Rect::new(r.x + 10.0, r.bottom() - 32.0, r.w - 20.0, 18.0),
        &TextStyle::new(10.5, white(0.55)).center(),
    );
    p.alpha = a0;
}

/// What the two boxes of the Gemini mode show.
struct AiView<'a> {
    query: &'a str,
    state: AiState,
    text: &'a str,
    scroll: f32,
    pop_s: f32,
    pop_a: f32,
    /// The question's text size and top edge (they glide).
    qsz: f32,
    qy: f32,
}

/// Gemini mode: the question on the left, the answer on the right. Returns how far the answer can scroll.
fn draw_ai(p: &mut Painter, sub: Rect, act: Rect, v: &AiView) -> f32 {
    let a0 = p.alpha;
    let centered = TextStyle::new(11.5, white(0.80)).semibold().center();
    // left: what was typed, the end of it kept in view
    p.alpha = a0 * (0.35 + 0.65 * logic::ease_out((v.pop_s * 1.6).min(1.0)));
    if v.query.is_empty() {
        p.icon(Icon::Sparkle, Rect::centered(sub.cx(), sub.y + 68.0, 52.0, 52.0), white(0.62));
        p.text("Ask Gemini\u{2026}", Rect::new(sub.x + 10.0, sub.y + 118.0, sub.w - 20.0, 20.0), &centered);
    } else {
        let view = Rect::new(sub.x + 14.0, sub.y + 14.0, sub.w - 28.0, sub.h - 28.0);
        let st = TextStyle::new(v.qsz, white(0.94)).semibold().center().top().wrap();
        let h = p.measure_wrapped(v.query, &st, view.w);
        p.push_clip(view);
        p.text(v.query, Rect::new(view.x, view.y + v.qy, view.w, h), &st);
        p.pop_clip();
    }
    // right: the answer
    p.alpha = a0 * (0.35 + 0.65 * logic::ease_out((v.pop_a * 1.6).min(1.0)));
    let mut max = 0.0;
    if v.text.is_empty() {
        let (icon, msg) = match v.state {
            AiState::Waiting => (Icon::Sparkle, "Thinking\u{2026}"),
            _ => (Icon::Chat, "Press Enter to ask"),
        };
        p.icon(icon, Rect::centered(act.cx(), act.y + 68.0, 52.0, 52.0), white(0.62));
        p.text(msg, Rect::new(act.x + 10.0, act.y + 118.0, act.w - 20.0, 20.0), &centered);
    } else {
        let view = Rect::new(act.x + 14.0, act.y + 14.0, act.w - 28.0, act.h - 28.0);
        let st = TextStyle::new(12.5, white(0.92)).top().wrap();
        let text_w = view.w - 8.0;
        let h = p.measure_wrapped(v.text, &st, text_w);
        max = (h - view.h).max(0.0);
        let off = v.scroll.clamp(0.0, max);
        p.push_clip(view);
        p.text(v.text, Rect::new(view.x, view.y - off, text_w, h), &st);
        p.pop_clip();
        if max > 0.0 {
            // a thin bar shows there is more
            let track = view.h - 8.0;
            let bar = (track * view.h / h).max(18.0);
            let y = view.y + 4.0 + (track - bar) * off / max;
            p.fill_rounded(Rect::new(act.right() - 10.0, y, 3.0, bar), 1.5, white(0.35));
        }
    }
    p.alpha = a0;
    max
}

fn draw_action(p: &mut Painter, r: Rect, cur: Option<&Hit>, act: usize, _focused: bool, pop: f32, tr: &Tr) {
    let cx = r.cx();
    let Some(h) = cur else { return };
    let acts = actions(h);
    let a = acts[act.min(acts.len() - 1)];
    let a0 = p.alpha;
    let fade = logic::ease_out((pop * 1.6).min(1.0));
    let k = 0.80 + 0.20 * logic::ease_back(pop);
    let ir = Rect::centered(cx, r.y + 68.0, 56.0, 56.0);
    pop_xf(p, tr, k, cx, ir.cy());
    p.alpha = a0 * (0.3 + 0.7 * fade);
    p.icon(a.icon(), ir, white(0.92));
    card_xf(p, tr);
    p.alpha = a0 * (0.35 + 0.65 * fade);
    let slide = (1.0 - fade) * 5.0;
    p.text(
        a.label(),
        Rect::new(r.x + 10.0, r.y + 116.0 + slide, r.w - 20.0, 22.0),
        &TextStyle::new(11.5, white(0.90)).semibold().center(),
    );
    p.alpha = a0;
}

/// Everything the dropped-down list needs.
struct ListView<'a> {
    hits: &'a [Hit],
    icons: &'a HashMap<String, Option<ImageData>>,
    query: &'a str,
    focus: Focus,
    sel: usize,
    top: usize,
    act: usize,
    shown: usize,
    /// How far the list has dropped (dips).
    lh: f32,
    /// The highlighted row (moves between rows).
    sel_y: f32,
    ul: f32,
    wx: Option<&'a Brief>,
    clip: bool,
}

/// The list drops down: the card grows, each row fades and slides in as the edge passes it, and the
/// highlight glides from row to row.
fn draw_list(p: &mut Painter, v: &ListView) {
    let base = p.alpha;
    let dim = TextStyle::new(11.5, white(0.55));
    let head = Rect::new(PAD + 8.0, FRAME_H, W - 2.0 * (PAD + 8.0), HEAD);
    let y0 = FRAME_H + HEAD;
    let name_st = TextStyle::new(13.5, white(0.95)).semibold();
    let sub_st = TextStyle::new(10.5, white(0.5));
    let title = match v.focus {
        Focus::Action => match v.hits.get(v.sel) {
            Some(h) => format!("Actions for “{}”", logic::mid(&h.name, 30)),
            None => return,
        },
        Focus::Subject if v.clip => {
            let n = v.hits.len();
            let count = if n == 1 { "1 item".to_string() } else { format!("{n} items") };
            if v.query.trim().is_empty() {
                format!("Clipboard  ·  {count}")
            } else {
                format!("{count}  ·  “{}”", logic::mid(v.query.trim(), 24))
            }
        }
        Focus::Subject if v.query.trim().is_empty() => "Used most".to_string(),
        Focus::Subject => {
            let n = v.hits.iter().filter(|h| h.kind.has_icon()).count();
            let count = if n >= KEEP {
                format!("{KEEP}+ results")
            } else if n == 1 {
                "1 result".to_string()
            } else {
                format!("{n} results")
            };
            format!("{count}  ·  “{}”", logic::mid(v.query.trim(), 24))
        }
    };
    p.alpha = base * (v.lh / HEAD).clamp(0.0, 1.0);
    p.text(&title, head, &dim);
    // the highlight
    let ha = ((v.lh - HEAD) / ROW).clamp(0.0, 1.0);
    if ha > 0.0 {
        p.alpha = base * ha;
        p.fill_rounded(Rect::new(10.0, y0 + v.sel_y * ROW, W - 20.0, ROW - 2.0), 14.0, white(0.16));
    }
    let reveal = |i: usize| ((v.lh - HEAD - i as f32 * ROW) / ROW).clamp(0.0, 1.0);
    match v.focus {
        Focus::Action => {
            let Some(h) = v.hits.get(v.sel) else { return };
            for (i, a) in actions(h).iter().enumerate().take(v.shown) {
                let ra = reveal(i);
                if ra <= 0.01 {
                    continue;
                }
                p.alpha = base * ra;
                let row = Rect::new(10.0, y0 + i as f32 * ROW + (1.0 - ra) * 8.0, W - 20.0, ROW - 2.0);
                p.icon(a.icon(), Rect::centered(row.x + 26.0, row.cy(), 22.0, 22.0), white(0.92));
                p.text(a.label(), Rect::new(row.x + 52.0, row.y, row.w - 62.0, row.h), &name_st);
            }
        }
        Focus::Subject => {
            for (i, h) in v.hits.iter().enumerate().skip(v.top).take(v.shown) {
                let ra = reveal(i - v.top);
                if ra <= 0.01 {
                    continue;
                }
                p.alpha = base * ra;
                let row = Rect::new(10.0, y0 + (i - v.top) as f32 * ROW + (1.0 - ra) * 8.0, W - 20.0, ROW - 2.0);
                let ir = Rect::new(row.x + 10.0, row.y + (row.h - 28.0) * 0.5, 28.0, 28.0);
                draw_thumb(p, ir, h, v.icons.get(&h.path).and_then(|o| o.as_ref()), v.wx);
                let name_r = Rect::new(row.x + 50.0, row.y + 4.0, row.w - 60.0, 20.0);
                p.text(&h.name, name_r, &name_st);
                if h.kind.has_icon() && i == v.sel {
                    underline(p, &h.name, &h.spans, name_r, &name_st, false, v.ul);
                } else if h.kind.has_icon() {
                    underline(p, &h.name, &h.spans, name_r, &name_st, false, 1.0);
                }
                let detail = logic::mid(&h.detail, 54);
                p.text(&detail, Rect::new(row.x + 50.0, row.y + 24.0, row.w - 60.0, 16.0), &sub_st);
            }
        }
    }
    p.alpha = base;
}

// -- window and background work --------------------------------------------------------------------------

fn create_window() -> windows::core::Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(launcher_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // like the dock, but it may take the keyboard focus (no WS_EX_NOACTIVATE)
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS,
            w!("Dynamic Notch Launcher"),
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

extern "system" fn launcher_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| crate::window::with_app(|a| a.launcher_message(msg, wp, lp)).flatten());
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

/// Start something with administrator rights. Windows asks for confirmation in its own window; this
/// goes through a helper process so the notch is not held up while that question is open.
/// The screen behind the window (`w` x `h` pixels at `x`, `y`), blurred as much as `blur` (0 - 1) says.
fn capture_blur(x: i32, y: i32, w: i32, h: i32, blur: f32) -> Option<ImageData> {
    let shot = crate::refract::grab(x, y, w, h)?;
    // shrunk 4 times first: the blur works on a sixteenth of the pixels and still looks the same
    let (k, r) = if blur < 0.02 { (1, 0) } else { (4, (1.0 + blur * 7.0).round() as usize) };
    let (px, w2, h2) = logic::shrink_blur(&shot.px, shot.w, shot.h, k, r);
    Some(ImageData::from_pbgra(w2 as u32, h2 as u32, px))
}

/// The program behind an app match (its shortcut's target), when it is safe to stop it.
fn exe_name(path: &str) -> Option<String> {
    let target =
        if path.to_ascii_lowercase().ends_with(".exe") { path.to_string() } else { crate::dock::lnk_target(path)? };
    let name = target.rsplit(['\\', '/']).next()?.to_string();
    let l = name.to_ascii_lowercase();
    const KEEP_ALIVE: [&str; 9] = [
        "explorer.exe",
        "svchost.exe",
        "winlogon.exe",
        "csrss.exe",
        "wininit.exe",
        "services.exe",
        "lsass.exe",
        "dwm.exe",
        "taskkill.exe",
    ];
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    if !l.ends_with(".exe")
        || name.len() > 100
        || name.starts_with('-')
        || KEEP_ALIVE.contains(&l.as_str())
        || me.as_deref() == Some(l.as_str())
    {
        return None;
    }
    Some(name)
}

/// Ask Windows to close every window of the program (or end it at once when `force`).
fn taskkill(exe: &str, force: bool) -> bool {
    let mut c = std::process::Command::new("taskkill");
    if force {
        c.args(["/F", "/T"]);
    }
    c.args(["/IM", exe]).creation_flags(0x0800_0000);
    c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.status().is_ok_and(|s| s.success())
}

fn is_running(exe: &str) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe}"), "/NH", "/FO", "CSV"])
        .creation_flags(0x0800_0000)
        .output();
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains(&exe.to_ascii_lowercase()))
}

/// Close the app politely, end it if it does not leave, then start it again. Runs in the background.
fn restart_app(exe: String, path: String) {
    std::thread::spawn(move || {
        low_priority();
        taskkill(&exe, false);
        let mut forced = false;
        for i in 0..32 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if !is_running(&exe) {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let _ = std::process::Command::new("explorer.exe").arg(&path).spawn();
                return;
            }
            if i == 12 && !forced {
                forced = true;
                taskkill(&exe, true);
            }
        }
    });
}

/// The id of a clipboard history item (`clip:<id>`).
fn clip_id(path: &str) -> Option<u64> {
    path.strip_prefix("clip:")?.parse().ok()
}

/// Press Ctrl+V a moment from now, when the window that had the focus before has it back.
fn paste_soon() {
    let _ = std::thread::Builder::new().name("launcher-paste".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(260));
        let v = VIRTUAL_KEY(0x56);
        let keys = [key_input(VK_CONTROL, false), key_input(v, false), key_input(v, true), key_input(VK_CONTROL, true)];
        unsafe {
            SendInput(&keys, size_of::<INPUT>() as i32);
        }
    });
}

fn run_as_admin(path: &str) {
    let _ = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "Start-Process -FilePath $env:NOTCH_RUN -Verb RunAs",
        ])
        .env("NOTCH_RUN", path)
        .creation_flags(0x0800_0000)
        .spawn();
}

fn low_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
    }
}

fn finished(slot: &Slot, list: Vec<Entry>, hwnd: SendHwnd) {
    if let Ok(mut g) = slot.lock() {
        *g = Some(list);
    }
    unsafe {
        let _ = PostMessageW(Some(hwnd.hwnd()), WM_INDEX, WPARAM(0), LPARAM(0));
    }
}

/// Ask Gemini in the background. The answer is written into `shared` as it streams in, and the
/// launcher window is told each time.
fn spawn_ai(
    shared: Arc<Mutex<AiShared>>,
    hwnd: SendHwnd,
    seq: u64,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    cfg: GeminiCfg,
    key: String,
    question: String,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let _ = std::thread::Builder::new().name("launcher-ai".into()).spawn(move || {
        low_priority();
        let body = serde_json::json!({
            "systemInstruction": { "parts": [{ "text": cfg.system_prompt }] },
            "contents": [{ "role": "user", "parts": [{ "text": question }] }],
            "generationConfig": { "maxOutputTokens": cfg.max_tokens },
        });
        let url =
            format!("{}/models/{}:streamGenerateContent?alt=sse", cfg.endpoint.trim_end_matches('/'), cfg.model.trim());
        let headers = [
            ("x-goog-api-key", key),
            ("Content-Type", "application/json".to_string()),
            ("Accept", "text/event-stream".to_string()),
        ];
        let payload = body.to_string().into_bytes();
        let write = |f: &dyn Fn(&mut AiShared)| {
            if let Ok(mut sh) = shared.lock() {
                if sh.seq == seq {
                    f(&mut sh);
                }
            }
            unsafe {
                let _ = PostMessageW(Some(hwnd.hwnd()), WM_AI, WPARAM(0), LPARAM(0));
            }
        };
        let mut parser = crate::sys::http::SseParser::default();
        let mut raw = Vec::new();
        let mut stream_error: Option<String> = None;
        let res = crate::sys::http::post_stream(&url, &headers, &payload, &cancel, |chunk| {
            if raw.len() < 16 * 1024 {
                raw.extend_from_slice(chunk);
            }
            parser.push(chunk, |data| {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { return };
                if let Some(e) = v.get("error") {
                    stream_error =
                        Some(e.get("message").and_then(|m| m.as_str()).unwrap_or("stream error").to_string());
                    return;
                }
                if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(|p| p.as_array()) {
                    let text: String = parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect();
                    if !text.is_empty() {
                        write(&|sh: &mut AiShared| sh.text.push_str(&text));
                    }
                }
                if let Some(reason) = v.pointer("/promptFeedback/blockReason").and_then(|r| r.as_str()) {
                    stream_error = Some(format!("Gemini would not answer this ({reason})."));
                }
            });
            !cancel.load(Relaxed)
        });
        if cancel.load(Relaxed) {
            return;
        }
        match res {
            Ok(200) if stream_error.is_none() => write(&|sh: &mut AiShared| sh.done = true),
            Ok(status) => {
                let msg = stream_error.unwrap_or_else(|| {
                    serde_json::from_slice::<serde_json::Value>(&raw)
                        .ok()
                        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string))
                        .unwrap_or_else(|| format!("HTTP {status}"))
                });
                let hint = match status {
                    400 | 401 | 403 => " (check your key in Settings \u{2192} Gemini)",
                    404 => " (check the model name in Settings \u{2192} Gemini)",
                    429 => " (too many questions right now, try again shortly)",
                    _ => "",
                };
                let msg = format!("{msg}{hint}");
                write(&|sh: &mut AiShared| sh.err = Some(msg.clone()));
            }
            Err(e) => write(&|sh: &mut AiShared| sh.err = Some(e.clone())),
        }
    });
}

/// The installed apps, found in the background.
fn spawn_apps(slot: Slot, hwnd: SendHwnd) {
    let _ = std::thread::Builder::new().name("launcher-apps".into()).spawn(move || {
        low_priority();
        let list: Vec<Entry> = crate::dock::installed_apps()
            .into_iter()
            .map(|(name, path)| Entry {
                path: path.into_boxed_str(),
                label: Some(name.into_boxed_str()),
                at: 0,
                kind: Kind::App,
            })
            .collect();
        crate::log!("launcher: {} apps", list.len());
        finished(&slot, list, hwnd);
    });
}

/// The files and folders in the usual places, found in the background.
fn spawn_files(slot: Slot, hwnd: SendHwnd, extra: Vec<String>) {
    let _ = std::thread::Builder::new().name("launcher-files".into()).spawn(move || {
        low_priority();
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(home) = std::env::var_os("USERPROFILE") {
            for d in ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"] {
                roots.push(Path::new(&home).join(d));
            }
        }
        if let Some(od) = std::env::var_os("OneDrive") {
            for d in ["Desktop", "Documents", "Pictures"] {
                roots.push(Path::new(&od).join(d));
            }
        }
        roots.extend(extra.iter().map(PathBuf::from));
        // the same folder reached by two routes is searched once
        let mut roots: Vec<(PathBuf, PathBuf)> =
            roots.into_iter().filter_map(|r| std::fs::canonicalize(&r).ok().map(|c| (c, r))).collect();
        roots.sort_by(|a, b| a.0.cmp(&b.0));
        roots.dedup_by(|a, b| a.0 == b.0);
        let mut out: Vec<Entry> = Vec::new();
        for (_, r) in &roots {
            walk(r, &mut out);
        }
        out.shrink_to_fit();
        crate::log!("launcher: {} files and folders", out.len());
        finished(&slot, out, hwnd);
    });
}

fn walk(root: &Path, out: &mut Vec<Entry>) {
    let mut stack: Vec<(PathBuf, u8)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let fname = e.file_name();
            let Some(name) = fname.to_str() else { continue };
            if name.starts_with('.') || name.starts_with('$') || name.starts_with("~$") {
                continue;
            }
            let Ok(md) = e.metadata() else { continue };
            let attrs = md.file_attributes();
            // hidden and system files stay out; so do junctions and links (they can loop)
            if attrs & 0x6 != 0 {
                continue;
            }
            let is_dir = md.is_dir();
            if is_dir && attrs & 0x400 != 0 {
                continue;
            }
            let path = e.path();
            let Some(ps) = path.to_str() else { continue };
            if !ps.ends_with(name) {
                continue;
            }
            out.push(Entry {
                path: ps.into(),
                label: None,
                at: (ps.len() - name.len()) as u32,
                kind: if is_dir { Kind::Folder } else { Kind::File },
            });
            if is_dir && depth < MAX_DEPTH && !SKIP_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
                stack.push((path, depth + 1));
            }
        }
    }
}

// -- what was opened before ----------------------------------------------------------------------------------

fn usage_path() -> PathBuf {
    app_dir().join("launcher-usage.txt")
}

fn load_usage() -> HashMap<String, u32> {
    let mut m = HashMap::new();
    if let Ok(s) = std::fs::read_to_string(usage_path()) {
        for line in s.lines() {
            if let Some((n, path)) = line.split_once('\t') {
                if let Ok(n) = n.trim().parse::<u32>() {
                    m.insert(path.to_string(), n);
                }
            }
        }
    }
    m
}

fn learn_path() -> PathBuf {
    app_dir().join("launcher-learn.txt")
}

fn load_learn() -> HashMap<(String, String), u32> {
    let mut m = HashMap::new();
    if let Ok(s) = std::fs::read_to_string(learn_path()) {
        for line in s.lines() {
            let mut it = line.splitn(3, '\t');
            if let (Some(n), Some(typed), Some(path)) = (it.next(), it.next(), it.next()) {
                if let Ok(n) = n.trim().parse::<u32>() {
                    m.insert((typed.to_string(), path.to_string()), n);
                }
            }
        }
    }
    m
}

fn save_learn(m: &HashMap<(String, String), u32>) {
    let mut v: Vec<(&(String, String), &u32)> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let text: String = v.into_iter().take(400).map(|((t, p), n)| format!("{n}\t{t}\t{p}\n")).collect();
    let _ = std::fs::write(learn_path(), text);
}

fn save_usage(m: &HashMap<String, u32>) {
    let mut v: Vec<(&String, &u32)> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let text: String = v.into_iter().take(300).map(|(p, n)| format!("{n}\t{p}\n")).collect();
    let _ = std::fs::write(usage_path(), text);
}

// -- pure logic: matching, calculator, links (no Windows calls, unit-tested) ---------------------------------

mod logic {
    /// A new word starts here: the first letter, a letter after a separator, or a capital after a small letter.
    pub fn is_word_start(n: &[u8], i: usize) -> bool {
        if i == 0 {
            return true;
        }
        let (p, c) = (n[i - 1], n[i]);
        (p < 0x80 && !p.is_ascii_alphanumeric()) || (p.is_ascii_lowercase() && c.is_ascii_uppercase())
    }

    /// Like `fuzzy`, and when that finds nothing, a name with a slip in it ("chorme" for Chrome).
    pub fn matched(name: &[u8], q: &[u8], loose: bool, pos: &mut Vec<usize>) -> Option<i32> {
        fuzzy(name, q, loose, pos).or_else(|| if q.len() >= 4 { typo(name, q, pos) } else { None })
    }

    /// Letters wrong, missing, extra or swapped: the fewest changes to turn `a` into `b` (swapping two
    /// neighbours counts once). Only for short words.
    pub fn distance(a: &[u8], b: &[u8]) -> usize {
        let (n, m) = (a.len(), b.len());
        let mut d = vec![vec![0usize; m + 1]; n + 1];
        for (i, row) in d.iter_mut().enumerate() {
            row[0] = i;
        }
        for j in 0..=m {
            d[0][j] = j;
        }
        for i in 1..=n {
            for j in 1..=m {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    v = v.min(d[i - 2][j - 2] + 1);
                }
                d[i][j] = v;
            }
        }
        d[n][m]
    }

    /// The query is close to the start of one of the words of `name`: one slip for 4 - 5 letters,
    /// two for longer ones. The word is left in `pos`.
    pub fn typo(name: &[u8], q: &[u8], pos: &mut Vec<usize>) -> Option<i32> {
        pos.clear();
        let (n, m) = (name.len(), q.len());
        if !(4..=40).contains(&m) || n < m - 1 {
            return None;
        }
        let allowed = if m <= 5 { 1 } else { 2 };
        let low = |i: usize| name[i].to_ascii_lowercase();
        let mut best: Option<(usize, usize, usize)> = None; // (distance, start, length)
        for st in 0..n {
            if !is_word_start(name, st) || !name[st].is_ascii_alphanumeric() {
                continue;
            }
            // a cheap look at the first letters, so most words are skipped
            let swapped = st + 1 < n && m >= 2 && low(st) == q[1] && low(st + 1) == q[0];
            if low(st) != q[0] && !swapped {
                continue;
            }
            let mut end = st;
            while end < n && name[end].is_ascii_alphanumeric() && (end == st || !is_word_start(name, end)) {
                end += 1;
            }
            let word: Vec<u8> = name[st..end].iter().map(|c| c.to_ascii_lowercase()).collect();
            if word.len() + allowed < m {
                continue;
            }
            // the start of the word, about as long as the query
            for len in m.saturating_sub(1)..=(m + 1) {
                let len = len.min(word.len());
                let d = distance(&word[..len], q);
                if d <= allowed && best.map_or(true, |b| d < b.0) {
                    best = Some((d, st, len));
                }
            }
        }
        let (d, st, len) = best?;
        pos.extend(st..st + len);
        Some(3_400 - 500 * d as i32 - st as i32)
    }

    /// How well `name` fits the lowercase query `q` (None = not at all). The matched byte positions
    /// are left in `pos`. `loose` also accepts letters spread over the name ("vsc" for Visual Studio Code).
    pub fn fuzzy(name: &[u8], q: &[u8], loose: bool, pos: &mut Vec<usize>) -> Option<i32> {
        pos.clear();
        let (n, m) = (name.len(), q.len());
        if m == 0 || m > n {
            return None;
        }
        let eq = |a: u8, b: u8| a.to_ascii_lowercase() == b;
        // the query as one piece, preferably at the start of a word
        let mut first = None;
        let mut at_word = None;
        for i in 0..=n - m {
            if (0..m).all(|j| eq(name[i + j], q[j])) {
                if first.is_none() {
                    first = Some(i);
                }
                if is_word_start(name, i) {
                    at_word = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = at_word.or(first) {
            pos.extend(i..i + m);
            return Some(if i == 0 {
                if m == n {
                    10_000
                } else {
                    8_000 - (n - m) as i32
                }
            } else if is_word_start(name, i) {
                6_000 - i as i32
            } else {
                4_000 - i as i32
            });
        }
        // the first letters of the words ("vsc")
        let mut k = 0;
        for i in 0..n {
            if is_word_start(name, i) {
                if eq(name[i], q[k]) {
                    pos.push(i);
                    k += 1;
                    if k == m {
                        return Some(5_000 - n as i32);
                    }
                } else {
                    break;
                }
            }
        }
        pos.clear();
        if !loose {
            return None;
        }
        // letters in order, not too far apart
        let mut k = 0;
        let mut score = 1_000i32;
        let mut last: Option<usize> = None;
        for i in 0..n {
            if eq(name[i], q[k]) {
                pos.push(i);
                if last.is_some_and(|l| l + 1 == i) {
                    score += 40;
                }
                if is_word_start(name, i) {
                    score += 60;
                }
                last = Some(i);
                k += 1;
                if k == m {
                    break;
                }
            }
        }
        if k < m {
            pos.clear();
            return None;
        }
        let span = pos[m - 1] - pos[0] + 1;
        if span > m * 3 + 4 {
            pos.clear();
            return None;
        }
        Some((score - (span - m) as i32 * 10 - pos[0] as i32).min(3_900))
    }

    /// Byte positions as ranges.
    pub fn spans(pos: &[usize]) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        for &p in pos {
            match out.last_mut() {
                Some(l) if l.1 == p => l.1 = p + 1,
                _ => out.push((p, p + 1)),
            }
        }
        out
    }

    /// A range moved onto character boundaries.
    pub fn snap(s: &str, a: usize, b: usize) -> (usize, usize) {
        let (mut a, mut b) = (a.min(s.len()), b.min(s.len()));
        while a > 0 && !s.is_char_boundary(a) {
            a -= 1;
        }
        while b < s.len() && !s.is_char_boundary(b) {
            b += 1;
        }
        (a, b)
    }

    /// Long text cut in the middle: `C:\Users\me\Doc…\notes\todo.txt`.
    pub fn mid(s: &str, max: usize) -> String {
        let n = s.chars().count();
        if n <= max || max < 8 {
            return s.to_string();
        }
        let head = (max - 1) * 2 / 5;
        let tail = max - 1 - head;
        let a: String = s.chars().take(head).collect();
        let b: String = s.chars().skip(n - tail).collect();
        format!("{a}…{b}")
    }

    // -- calculator ------------------------------------------------------------------------------------

    struct P {
        s: Vec<char>,
        i: usize,
        ops: u32,
    }

    impl P {
        fn ws(&mut self) {
            while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<char> {
            self.ws();
            self.s.get(self.i).copied()
        }
        fn expr(&mut self) -> Option<f64> {
            let mut v = self.term()?;
            while let Some(c) = self.peek() {
                if c != '+' && c != '-' {
                    break;
                }
                self.i += 1;
                self.ops += 1;
                let r = self.term()?;
                v = if c == '+' { v + r } else { v - r };
            }
            Some(v)
        }
        fn term(&mut self) -> Option<f64> {
            let mut v = self.factor()?;
            while let Some(c) = self.peek() {
                if !matches!(c, '*' | '/' | '%' | '×' | '÷') {
                    break;
                }
                self.i += 1;
                self.ops += 1;
                let r = self.factor()?;
                v = match c {
                    '*' | '×' => v * r,
                    '/' | '÷' => v / r,
                    _ => v % r,
                };
            }
            Some(v)
        }
        fn factor(&mut self) -> Option<f64> {
            match self.peek()? {
                '-' => {
                    self.i += 1;
                    Some(-self.factor()?)
                }
                '+' => {
                    self.i += 1;
                    self.factor()
                }
                _ => {
                    let b = self.atom()?;
                    if self.peek() == Some('^') {
                        self.i += 1;
                        self.ops += 1;
                        let e = self.factor()?;
                        Some(b.powf(e))
                    } else {
                        Some(b)
                    }
                }
            }
        }
        fn atom(&mut self) -> Option<f64> {
            let c = self.peek()?;
            if c == '(' {
                self.i += 1;
                self.ops += 1;
                let v = self.expr()?;
                if self.peek()? != ')' {
                    return None;
                }
                self.i += 1;
                return Some(v);
            }
            if c.is_ascii_digit() || c == '.' {
                let st = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                    self.i += 1;
                }
                let t: String = self.s[st..self.i].iter().collect();
                return t.parse::<f64>().ok();
            }
            if c.is_ascii_alphabetic() {
                let st = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphabetic()) {
                    self.i += 1;
                }
                let name: String = self.s[st..self.i].iter().collect::<String>().to_ascii_lowercase();
                if self.peek() == Some('(') {
                    self.i += 1;
                    self.ops += 1;
                    let v = self.expr()?;
                    if self.peek()? != ')' {
                        return None;
                    }
                    self.i += 1;
                    return match name.as_str() {
                        "sqrt" => Some(v.sqrt()),
                        "sin" => Some(v.sin()),
                        "cos" => Some(v.cos()),
                        "tan" => Some(v.tan()),
                        "ln" => Some(v.ln()),
                        "log" => Some(v.log10()),
                        "abs" => Some(v.abs()),
                        "round" => Some(v.round()),
                        "floor" => Some(v.floor()),
                        "ceil" => Some(v.ceil()),
                        _ => None,
                    };
                }
                return match name.as_str() {
                    "pi" => Some(std::f64::consts::PI),
                    "e" => Some(std::f64::consts::E),
                    _ => None,
                };
            }
            None
        }
    }

    /// The answer when `src` is a sum like `12*(3+4)` or `sqrt(2)`; a lone number is not one.
    pub fn calc(src: &str) -> Option<String> {
        let times = times(src.trim());
        let s = times.as_str();
        if s.len() > 120 || !s.bytes().any(|b| b.is_ascii_digit() || b == b'p' || b == b'P') {
            return None;
        }
        let mut p = P { s: s.chars().collect(), i: 0, ops: 0 };
        let v = p.expr()?;
        p.ws();
        if p.i != p.s.len() || p.ops == 0 || !v.is_finite() {
            return None;
        }
        Some(num(v))
    }

    /// `6x7`, `6 x 7` and `6×7` mean 6*7 (and `÷` means `/`); the x of a word or of `0x1F` stays.
    pub fn times(s: &str) -> String {
        let c: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        for (i, &ch) in c.iter().enumerate() {
            match ch {
                '×' => out.push('*'),
                '÷' => out.push('/'),
                'x' | 'X' => {
                    let prev = c[..i].iter().rev().find(|k| !k.is_whitespace());
                    let next = c[i + 1..].iter().find(|k| !k.is_whitespace());
                    let ok = matches!(prev, Some(k) if k.is_ascii_digit() || *k == ')' || *k == '.')
                        && matches!(next, Some(k) if k.is_ascii_digit() || *k == '(' || *k == '.');
                    let hex = i >= 1
                        && c[i - 1] == '0'
                        && (i < 2 || !(c[i - 2].is_ascii_digit() || c[i - 2] == '.'))
                        && matches!(next, Some(k) if k.is_ascii_hexdigit());
                    out.push(if ok && !hex { '*' } else { ch });
                }
                _ => out.push(ch),
            }
        }
        out
    }

    /// "just now", "5m ago", "3h ago", "2d ago".
    pub fn ago(secs: u64) -> String {
        match secs {
            0..=9 => "just now".to_string(),
            10..=59 => format!("{secs}s ago"),
            60..=3599 => format!("{}m ago", secs / 60),
            3600..=86399 => format!("{}h ago", secs / 3600),
            _ => format!("{}d ago", secs / 86400),
        }
    }

    pub fn num(v: f64) -> String {
        if v == v.trunc() && v.abs() < 1e15 {
            return format!("{}", v as i64);
        }
        if v.abs() >= 1e15 || v.abs() < 1e-6 {
            return format!("{v:e}");
        }
        let t = format!("{v:.10}");
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    }

    // -- links -----------------------------------------------------------------------------------------

    /// `https://…`, `www.…` or a bare `name.com` becomes an address to open.
    pub fn as_url(q: &str) -> Option<String> {
        let q = q.trim();
        if q.is_empty() || q.contains(char::is_whitespace) || q.len() > 300 {
            return None;
        }
        let l = q.to_ascii_lowercase();
        if l.starts_with("http://") || l.starts_with("https://") {
            return (q.len() > 8).then(|| q.to_string());
        }
        if l.starts_with("www.") {
            return (q.len() > 6).then(|| format!("https://{q}"));
        }
        let host = l.split(['/', '?', '#']).next().unwrap_or("");
        let (name, tld) = host.rsplit_once('.')?;
        const TLDS: [&str; 24] = [
            "com", "org", "net", "io", "dev", "app", "ai", "co", "edu", "gov", "uk", "de", "fr", "me", "tv", "info",
            "us", "ca", "au", "nl", "es", "it", "xyz", "gg",
        ];
        let name_ok = !name.is_empty()
            && name.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        (name_ok && TLDS.contains(&tld)).then(|| format!("https://{q}"))
    }

    /// Text made safe for a web address.
    pub fn enc(q: &str) -> String {
        let mut out = String::with_capacity(q.len() * 3);
        for b in q.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    // -- weather words ---------------------------------------------------------------------------------

    /// "wea", "weather", "cast", "forecast", "temp", "temperature" ...: the user wants the weather.
    pub fn weather_query(q: &str) -> bool {
        let l = q.trim().to_ascii_lowercase();
        let n = l.len();
        (n >= 3 && "weather".starts_with(l.as_str()))
            || (n >= 4 && "forecast".contains(l.as_str()))
            || (n >= 4 && "temperature".starts_with(l.as_str()))
    }

    // -- easing ----------------------------------------------------------------------------------------

    /// Fast start, soft landing.
    pub fn ease_out(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
    }

    /// Like `ease_out`, but swings a little past 1 before settling.
    pub fn ease_back(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0) - 1.0;
        1.0 + 2.70158 * t * t * t + 1.70158 * t * t
    }

    /// `v` a step closer to `to`, without passing it.
    pub fn toward(v: f32, to: f32, step: f32) -> f32 {
        if (to - v).abs() <= step {
            to
        } else if to > v {
            v + step
        } else {
            v - step
        }
    }

    /// `v` moves a fraction of the way to `to` that grows with the time passed (smooth, never overshoots).
    pub fn follow(v: f32, to: f32, dt: f32, rate: f32, eps: f32) -> f32 {
        let d = to - v;
        if d.abs() <= eps {
            to
        } else {
            v + d * (1.0 - (-rate * dt).exp())
        }
    }

    // -- the blurred picture behind the window ----------------------------------------------------------

    /// One pass of a box blur along a row or a column of an interleaved BGRA picture (the first three
    /// channels; the edge pixels repeat).
    fn blur_line(src: &[u8], dst: &mut [u8], start: usize, step: usize, len: usize, r: usize) {
        let n = (2 * r + 1) as u32;
        let last = len as isize - 1;
        for c in 0..3 {
            let at = |i: isize| -> u32 { src[start + i.clamp(0, last) as usize * step + c] as u32 };
            let mut sum = 0u32;
            for i in -(r as isize)..=(r as isize) {
                sum += at(i);
            }
            for i in 0..len {
                dst[start + i * step + c] = ((sum + n / 2) / n) as u8;
                sum = sum + at(i as isize + r as isize + 1) - at(i as isize - r as isize);
            }
        }
    }

    /// A screen copy (`w` x `h`, BGRA, no alpha) shrunk `k` times by averaging and blurred with three box
    /// passes of radius `r`. Returns premultiplied BGRA bytes (opaque) and the new size.
    pub fn shrink_blur(src: &[u32], w: usize, h: usize, k: usize, r: usize) -> (Vec<u8>, usize, usize) {
        let k = k.max(1);
        let (w2, h2) = (w.div_ceil(k).max(1), h.div_ceil(k).max(1));
        let mut a = vec![255u8; w2 * h2 * 4];
        if src.len() >= w * h {
            for y in 0..h2 {
                for x in 0..w2 {
                    let (mut b, mut g, mut rr, mut cnt) = (0u32, 0u32, 0u32, 0u32);
                    for yy in (y * k)..((y + 1) * k).min(h) {
                        for xx in (x * k)..((x + 1) * k).min(w) {
                            let p = src[yy * w + xx];
                            b += p & 255;
                            g += (p >> 8) & 255;
                            rr += (p >> 16) & 255;
                            cnt += 1;
                        }
                    }
                    let cnt = cnt.max(1);
                    let o = (y * w2 + x) * 4;
                    a[o] = (b / cnt) as u8;
                    a[o + 1] = (g / cnt) as u8;
                    a[o + 2] = (rr / cnt) as u8;
                }
            }
        }
        if r > 0 {
            let mut b = a.clone();
            for _ in 0..3 {
                for y in 0..h2 {
                    blur_line(&a, &mut b, y * w2 * 4, 4, w2, r);
                }
                for x in 0..w2 {
                    blur_line(&b, &mut a, x * 4, w2 * 4, h2, r);
                }
            }
        }
        (a, w2, h2)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn f(name: &str, q: &str, loose: bool) -> Option<(i32, Vec<(usize, usize)>)> {
            let mut pos = Vec::new();
            let q = q.to_ascii_lowercase();
            fuzzy(name.as_bytes(), q.as_bytes(), loose, &mut pos).map(|s| (s, spans(&pos)))
        }

        #[test]
        fn ranks_sensibly() {
            let exact = f("Chrome", "chrome", true).unwrap().0;
            let prefix = f("Chrome Remote Desktop", "chrome", true).unwrap().0;
            let word = f("Google Chrome", "chrome", true).unwrap().0;
            let inner = f("Archrome", "chrome", true).unwrap().0;
            assert!(exact > prefix && prefix > word && word > inner, "{exact} {prefix} {word} {inner}");
            assert_eq!(f("bees.gif", "be", false).unwrap().1, vec![(0, 2)]);
            assert_eq!(f("Visual Studio Code", "vsc", false).unwrap().1, vec![(0, 1), (7, 8), (14, 15)]);
            assert!(f("Visual Studio Code", "vcode", false).is_some() == false);
            assert!(f("Visual Studio Code", "vstc", true).is_some());
            assert!(f("notes.txt", "xyz", true).is_none());
            assert!(f("a", "ab", true).is_none());
            assert_eq!(f("MyCoolApp", "cool", false).unwrap().1, vec![(2, 6)]);
        }

        #[test]
        fn slips_are_forgiven() {
            let mut pos = Vec::new();
            assert!(typo(b"Google Chrome", b"chorme", &mut pos).is_some());
            assert_eq!(pos, (7..13).collect::<Vec<_>>());
            assert!(typo(b"Notepad", b"nodepad", &mut pos).is_some());
            assert!(typo(b"Visual Studio Code", b"vsiual", &mut pos).is_some());
            assert!(typo(b"Firefox", b"chrome", &mut pos).is_none());
            assert!(typo(b"Calculator", b"chrome", &mut pos).is_none());
            assert!(typo(b"Chrome", b"chr", &mut pos).is_none());
            assert!(matched(b"Google Chrome", b"chorme", true, &mut pos).is_some());
            // a real match still wins over a slip
            let real = fuzzy(b"Chrome", b"chrome", true, &mut pos).unwrap();
            let slip = typo(b"Chrome", b"chorme", &mut pos).unwrap();
            assert!(real > slip);
            assert_eq!(distance(b"chrome", b"chorme"), 1);
            assert_eq!(distance(b"abc", b"abcd"), 1);
        }

        #[test]
        fn ages() {
            assert_eq!(ago(3), "just now");
            assert_eq!(ago(45), "45s ago");
            assert_eq!(ago(300), "5m ago");
            assert_eq!(ago(7300), "2h ago");
            assert_eq!(ago(200_000), "2d ago");
        }

        #[test]
        fn times_sign() {
            assert_eq!(calc("6x7").as_deref(), Some("42"));
            assert_eq!(calc("6 X 7").as_deref(), Some("42"));
            assert_eq!(calc("6×7").as_deref(), Some("42"));
            assert_eq!(calc("2x(3+4)").as_deref(), Some("14"));
            assert_eq!(calc("(1+1)x3").as_deref(), Some("6"));
            assert_eq!(calc("12÷4").as_deref(), Some("3"));
            assert_eq!(calc("6*7").as_deref(), Some("42"));
            assert_eq!(times("max(2,3)"), "max(2,3)");
            assert_eq!(times("0x1F"), "0x1F");
            assert_eq!(calc("xbox"), None);
        }

        #[test]
        fn loose_is_not_too_loose() {
            assert!(f("a very long file name with many letters inside it.txt", "avz", true).is_none());
            assert!(f("report-final-v2.docx", "rfv", true).is_some());
        }

        #[test]
        fn calculates() {
            assert_eq!(calc("2+2").as_deref(), Some("4"));
            assert_eq!(calc("12*(3+4)").as_deref(), Some("84"));
            assert_eq!(calc("2^10").as_deref(), Some("1024"));
            assert_eq!(calc("-2^2").as_deref(), Some("-4"));
            assert_eq!(calc("2^-1").as_deref(), Some("0.5"));
            assert_eq!(calc("10/4").as_deref(), Some("2.5"));
            assert_eq!(calc("1/3").as_deref(), Some("0.3333333333"));
            assert_eq!(calc("sqrt(16)+1").as_deref(), Some("5"));
            assert_eq!(calc("2*pi").as_deref(), Some("6.2831853072"));
            assert_eq!(calc("7 % 4").as_deref(), Some("3"));
            assert_eq!(calc("3 × 4").as_deref(), Some("12"));
            assert_eq!(calc("42"), None);
            assert_eq!(calc("1/0"), None);
            assert_eq!(calc("2+"), None);
            assert_eq!(calc("(2+3"), None);
            assert_eq!(calc("chrome"), None);
            assert_eq!(calc("notes2+x"), None);
            assert_eq!(calc("1e5"), None);
            assert_eq!(calc("0.1+0.2").as_deref(), Some("0.3"));
        }

        #[test]
        fn links() {
            assert_eq!(as_url("https://a.b/c").as_deref(), Some("https://a.b/c"));
            assert_eq!(as_url("www.rust-lang.org").as_deref(), Some("https://www.rust-lang.org"));
            assert_eq!(as_url("github.com/foo/bar").as_deref(), Some("https://github.com/foo/bar"));
            assert_eq!(as_url("notes.txt"), None);
            assert_eq!(as_url("hello world.com"), None);
            assert_eq!(as_url("chrome"), None);
            assert_eq!(enc("a b&c/é"), "a+b%26c%2F%C3%A9");
        }

        #[test]
        fn text_helpers() {
            assert_eq!(snap("héllo", 2, 2), (1, 3));
            assert_eq!(mid("short", 20), "short");
            let m = mid(r"C:\Users\someone\Documents\Projects\notes\todo.txt", 30);
            assert_eq!(m.chars().count(), 30);
            assert!(m.starts_with(r"C:\Users\s") && m.ends_with("todo.txt"));
        }
        #[test]
        fn weather_words() {
            for q in ["cast", "wea", "weather", "Forecast", "temp", "temperature", "fore"] {
                assert!(weather_query(q), "{q}");
            }
            for q in ["ca", "we", "weathers", "fort", "tem", "chrome", ""] {
                assert!(!weather_query(q), "{q}");
            }
        }

        #[test]
        fn easing_and_moving() {
            assert_eq!(ease_out(0.0), 0.0);
            assert_eq!(ease_out(1.0), 1.0);
            assert!(ease_out(0.5) > 0.8);
            assert!((ease_back(1.0) - 1.0).abs() < 1e-6);
            assert!(ease_back(0.8) > 1.0, "swings past 1");
            assert_eq!(toward(0.0, 1.0, 0.3), 0.3);
            assert_eq!(toward(0.9, 1.0, 0.3), 1.0);
            assert_eq!(toward(1.0, 0.0, 0.25), 0.75);
            assert_eq!(follow(0.0, 10.0, 1.0, 1e9, 0.1), 10.0);
            assert_eq!(follow(9.95, 10.0, 0.01, 5.0, 0.1), 10.0);
            let v = follow(0.0, 10.0, 0.016, 16.0, 0.01);
            assert!(v > 0.0 && v < 10.0);
        }

        #[test]
        fn blur_keeps_flat_pictures_and_spreads_points() {
            let flat = vec![0x00_40_80_C0u32; 20 * 12];
            let (px, w, h) = shrink_blur(&flat, 20, 12, 4, 2);
            assert_eq!((w, h), (5, 3));
            assert!(px.chunks(4).all(|c| c == [0xC0, 0x80, 0x40, 255]), "a flat picture stays flat");
            let mut one = vec![0u32; 16 * 16];
            one[8 * 16 + 8] = 0x00_FF_FF_FF;
            let (sharp, ..) = shrink_blur(&one, 16, 16, 1, 0);
            let (soft, ..) = shrink_blur(&one, 16, 16, 1, 2);
            assert_eq!(sharp[(8 * 16 + 8) * 4], 255);
            assert!(soft[(8 * 16 + 8) * 4] < 60 && soft[(8 * 16 + 9) * 4] > 0, "the point spreads out");
            assert!(soft.chunks(4).all(|c| c[3] == 255));
            let (odd, w, h) = shrink_blur(&flat[..7 * 5], 7, 5, 4, 1);
            assert_eq!((w, h, odd.len()), (2, 2, 16));
        }
    }
}
'''

notes = []
def get(p): return open(p).read()
def swap(s, a, b, tag):
    if s.count(a) != 1:
        notes.append(tag); return s
    return s.replace(a, b)

if "fn toggle_ai" not in get('src/launcher.rs'):
    print("NOTHING CHANGED - install updates 271 to 283 first"); raise SystemExit

# ---- config.rs -------------------------------------------------------------------------------------------------
cf = get('src/config.rs')
if "pub live: bool" not in cf:
    cf = swap(cf, "    /// Handed over by the app (the Gemini mode asks with these settings); not stored here.\n    #[serde(skip)]\n    pub gemini: GeminiCfg,\n}\n",
'''    /// Liquid glass: the edge bends what is behind it and catches the light (off = a plain solid card).
    pub glass: bool,
    /// While media is playing the blur behind the launcher is live, so it shows what is playing.
    pub live: bool,
    /// Handed over by the app, not stored here: the Gemini settings, whether media is playing, and
    /// the dock's rim (the launcher's rim follows it).
    #[serde(skip)]
    pub gemini: GeminiCfg,
    #[serde(skip)]
    pub media_playing: bool,
    #[serde(skip)]
    pub rim_width: f32,
    #[serde(skip)]
    pub rim_bright: f32,
}
''', "config: fields")
    cf = swap(cf, "            light: true,\n            size: 0.8,\n", "            light: false,\n            size: 0.8,\n            glass: true,\n            live: true,\n", "config: defaults")
    cf = swap(cf, "            gemini: GeminiCfg::default(),\n        }\n", "            gemini: GeminiCfg::default(),\n            media_playing: false,\n            rim_width: 1.2,\n            rim_bright: 0.35,\n        }\n", "config: default runtime")
    cf = swap(cf, "# Light frosted glass with dark writing (false = dark glass with white writing)\nlight = true\n",
              "# Light frosted glass with dark writing (false = dark glass with white writing)\nlight = false\n# Liquid glass: the edge bends what is behind it and catches the light (false = a plain solid card)\nglass = true\n# While media is playing, the blur behind the launcher is live and shows what is playing\nlive = true\n", "config: template")

# ---- dock.rs: liquid glass on or off ----------------------------------------------------------------------------
dk = get('src/dock.rs')
if "!self.cfg.glass" not in dk:
    dk = swap(dk, "            if self.hide_t > 0.98 { 0.0 } else { op },\n", "            if self.hide_t > 0.98 || !self.cfg.glass { 0.0 } else { op },\n", "dock: backdrop")
    dk = swap(dk, "            p.set_local(1.0, 0.0, 0.0, 0.0, dy);\n            // plain blur:", "            p.set_local(1.0, 0.0, 0.0, 0.0, dy);\n            if !self.cfg.glass {\n                // liquid glass off: a plain solid bar\n                p.fill_rounded(bar, radius, Color::rgba(0.06, 0.06, 0.08, 0.94));\n            }\n            // plain blur:", "dock: solid")
    dk = swap(dk, "            if rw > 0.05 && self.cfg.rim_bright > 0.005 {", "            if self.cfg.glass && rw > 0.05 && self.cfg.rim_bright > 0.005 {", "dock: rim")

# ---- app.rs -------------------------------------------------------------------------------------------------------
ap = get('src/app.rs')
if "media_playing" not in ap:
    ap = swap(ap, '''    fn launcher_cfg(&self) -> crate::config::LauncherCfg {
        let mut cfg = self.cfg.launcher.clone();
        cfg.gemini = self.cfg.gemini.clone();
        cfg
    }''', '''    fn launcher_cfg(&mut self) -> crate::config::LauncherCfg {
        let mut cfg = self.cfg.launcher.clone();
        cfg.gemini = self.cfg.gemini.clone();
        cfg.media_playing = self.module_index("media").is_some_and(|i| self.started[i] && self.modules[i].is_playing());
        cfg.rim_width = self.cfg.dock.rim_width;
        cfg.rim_bright = self.cfg.dock.rim_bright;
        cfg
    }''', "app: cfg")

# ---- settings ------------------------------------------------------------------------------------------------------
st = get('src/modules/settings.rs')
if '"live"' not in st:
    st = swap(st, '            "Glass edge (bends what is behind)",', '            "Liquid glass: notch edge (bends what is behind)",', "settings: notch label")
    st = swap(st, '''            "launcher",
            "size",
        );
    }
''', '''            "launcher",
            "size",
        );
        toggle_row(ui, row_rect(body, 16), "Liquid glass: dock", d.glass, "dock", "glass");
        toggle_row(ui, row_rect(body, 17), "Liquid glass: launcher", l.glass, "launcher", "glass");
        toggle_row(ui, row_rect(body, 18), "Launcher: live blur while media plays", l.live, "launcher", "live");
    }
''', "settings: rows")

if notes:
    print("NOTHING CHANGED - these spots look different in your code:")
    for n in notes: print("  -", n)
else:
    open('src/launcher.rs', 'w').write(RUST)
    open('src/config.rs', 'w').write(cf)
    open('src/dock.rs', 'w').write(dk)
    open('src/app.rs', 'w').write(ap)
    open('src/modules/settings.rs', 'w').write(st)
    print("ok")
PYEOF
echo "update_284 done"
