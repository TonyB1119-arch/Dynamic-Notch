//! The launcher: press the hotkey, type, press Enter. Two glass boxes as in Tuna: the thing you
//! found on the left, what will happen to it on the right. The list of matches stays hidden until
//! you press the Down arrow.
//!
//! It finds apps (Start menu and Store apps), files and folders (Desktop, Documents, Downloads,
//! Pictures, Videos, Music and the folders in `[launcher] folders`), answers sums like `12*(3+4)`,
//! opens web addresses and falls back to a web search.
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

use crate::config::LauncherCfg;
use crate::gfx::{Gfx, Icon, ImageData, Painter, Res, TextStyle};
use crate::host::Host;
use crate::sys::{self, clip, dragdrop};
use crate::util::{app_dir, Color, Rect, SendHwnd};

const CLASS: PCWSTR = w!("DynamicNotch.Launcher");
const T_ANIM: usize = 1;
const T_ICON: usize = 2;
const T_TRIM: usize = 3;
const T_FREE: usize = 4;
/// A background scan has finished.
const WM_INDEX: u32 = WM_APP + 31;

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
        }
    }
}

/// What can be done with a match; the first one is what Enter does.
fn actions(h: &Hit) -> &'static [Act] {
    match h.kind {
        Kind::App if h.path.starts_with("shell:") => &[Act::Open],
        Kind::App => &[Act::Open, Act::Admin, Act::Reveal],
        Kind::Folder | Kind::File => &[Act::Open, Act::Reveal, Act::CopyPath],
        Kind::Calc => &[Act::CopyResult],
        Kind::Url => &[Act::OpenLink, Act::CopyPath],
        Kind::Web => &[Act::Search],
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
    open: bool,
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
    anim: f32,
    anim_at: Instant,
    scratch: Vec<usize>,
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
            anim: 1.0,
            anim_at: Instant::now(),
            scratch: Vec::new(),
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
        if self.open {
            self.hide();
        } else {
            self.show(gfx, ui_scale);
        }
    }

    fn show(&mut self, gfx: &Gfx, ui_scale: f32) {
        self.query.clear();
        self.hits.clear();
        self.sel = 0;
        self.top = 0;
        self.act = 0;
        self.focus = Focus::Subject;
        self.list_open = false;
        self.high = None;
        self.mouse = (-1.0, -1.0);
        // nothing typed yet: the apps used most are what the Down arrow lists
        self.search();
        if self.cfg.files && !self.files_started {
            self.files_started = true;
            spawn_files(self.files_slot.clone(), SendHwnd::new(self.hwnd), self.cfg.folders.clone());
        }
        if !self.place(ui_scale) {
            return;
        }
        self.open = true;
        self.anim = 0.0;
        self.anim_at = Instant::now();
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_TRIM);
            let _ = KillTimer(Some(self.hwnd), T_FREE);
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            if !SetForegroundWindow(self.hwnd).as_bool() || GetForegroundWindow() != self.hwnd {
                // Windows only lets the window in front take the focus: a harmless key tap unlocks it
                let inputs = [key_input(VK_MENU, false), key_input(VK_MENU, true)];
                SendInput(&inputs, size_of::<INPUT>() as i32);
                let _ = SetForegroundWindow(self.hwnd);
            }
            let _ = SetFocus(Some(self.hwnd));
            SetTimer(Some(self.hwnd), T_ANIM, 15, None);
        }
        self.draw(gfx);
    }

    pub fn hide(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.query.clear();
        self.hits.clear();
        self.list_open = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_ANIM);
            let _ = KillTimer(Some(self.hwnd), T_ICON);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            SetTimer(Some(self.hwnd), T_TRIM, 20_000, None);
        }
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
            let mut y = work.top + ((work.bottom - work.top) as f32 * 0.18) as i32;
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
        self.hits.clear();
        let q = self.query.trim().to_string();
        if q.is_empty() {
            self.recent();
            return;
        }
        if let Some(r) = logic::calc(&q) {
            self.hits.push(Hit {
                name: r,
                detail: q.clone(),
                path: String::new(),
                kind: Kind::Calc,
                spans: Vec::new(),
            });
        } else if let Some(u) = logic::as_url(&q) {
            self.hits.push(Hit {
                name: u.clone(),
                detail: "Open in the browser".into(),
                path: u,
                kind: Kind::Url,
                spans: Vec::new(),
            });
        }
        let ql = q.to_ascii_lowercase();
        let qb = &ql.as_bytes()[..ql.len().min(40)];
        let na = self.apps.len();
        let mut pos = std::mem::take(&mut self.scratch);
        let mut best: Vec<(i32, u32)> = Vec::with_capacity(KEEP + 1);
        for (k, e) in self.apps.iter().chain(self.files.iter()).enumerate() {
            let name = e.name();
            let Some(s) = logic::fuzzy(name.as_bytes(), qb, k < na, &mut pos) else { continue };
            let mut s = s - name.len().min(80) as i32;
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
            let _ = logic::fuzzy(e.name().as_bytes(), qb, k < na, &mut pos);
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
        });
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
                });
            }
        }
    }

    /// The text changed: start again from the best match.
    fn changed(&mut self) {
        self.focus = Focus::Subject;
        self.sel = 0;
        self.top = 0;
        self.act = 0;
        self.search();
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
                }
                if i < self.top {
                    self.top = i;
                } else if i >= self.top + VIS {
                    self.top = i + 1 - VIS;
                }
            }
            Focus::Action => self.act = i,
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

    fn run(&mut self) {
        let Some(h) = self.cur().cloned() else { return };
        let acts = actions(&h);
        let a = acts[self.act.min(acts.len() - 1)];
        let hwnd = self.hwnd;
        self.hide();
        match a {
            Act::Open | Act::OpenLink | Act::Search => sys::shell_open(&h.path),
            Act::Admin => run_as_admin(&h.path),
            Act::Reveal => sys::shell_reveal(&h.path),
            Act::CopyPath => {
                clip::set_text(hwnd, &h.path);
            }
            Act::CopyResult => {
                clip::set_text(hwnd, &h.name);
            }
        }
        if h.kind.has_icon() {
            self.bump(&h.path);
        }
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
        match VIRTUAL_KEY(vk) {
            VK_ESCAPE => {
                if self.list_open {
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
            VK_TAB => {
                if self.cur().is_some() {
                    self.focus = if shift || self.focus == Focus::Action { Focus::Subject } else { Focus::Action };
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
                    if self.cursor() == 0 {
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
            VK_DELETE => {
                self.query.clear();
                self.changed();
            }
            VK_V if ctrl => {
                if let Some(t) = clip::get_text(self.hwnd) {
                    let one: String =
                        t.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(200).collect();
                    self.query.push_str(&one);
                    self.changed();
                }
            }
            _ => return,
        }
        self.draw(gfx);
    }

    fn typed(&mut self, gfx: &Gfx, c: char) {
        if c.is_control() || (c == ' ' && self.query.is_empty()) || self.query.chars().count() >= 200 {
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
                if self.list_open && self.focus == Focus::Subject {
                    let d = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32 / 120;
                    let max = self.hits.len().saturating_sub(VIS) as i32;
                    self.top = (self.top as i32 - d).clamp(0, max) as usize;
                    self.draw(gfx);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == T_ANIM => {
                self.anim = (self.anim_at.elapsed().as_secs_f32() / 0.12).min(1.0);
                if self.anim >= 1.0 {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), T_ANIM);
                    }
                }
                self.draw(gfx);
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
                if !self.open {
                    self.free_me = true;
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
        if !self.open || self.win_px.2 == 0 {
            return;
        }
        let missing = self.load_icons();
        if missing {
            unsafe {
                SetTimer(Some(self.hwnd), T_ICON, 6, None);
            }
        }
        let sc = self.scale;
        let a = self.anim.clamp(0.0, 1.0);
        let t = 1.0 - (1.0 - a) * (1.0 - a) * (1.0 - a);
        let rows = self.rows_len();
        let list = self.list_open && rows > 0;
        let shown = rows.min(VIS);
        let card_h = if list { FRAME_H + HEAD + shown as f32 * ROW + 10.0 } else { FRAME_H };
        self.set_region(card_h);
        self.host.set_backdrop_rounded(Rect::new(0.0, 0.0, W * sc, card_h * sc), CARD_R * sc, self.cfg.opacity * t);
        let cur = self.cur().cloned();
        let vtop = self.view_top();
        let (dc, _) = match self.host.begin_draw() {
            Ok(v) => v,
            Err(e) => {
                crate::log!("launcher: begin_draw failed: {e}");
                return;
            }
        };
        {
            let mut p = Painter::new(dc, gfx, &mut self.res, sc, 0.0, 0.0);
            p.clear();
            p.alpha = t;
            let card = Rect::new(0.0, 0.0, W, card_h);
            // a little dark tint keeps the text readable on any picture behind the glass
            p.fill_rounded(card, CARD_R, Color::rgba(0.05, 0.05, 0.07, 0.32));
            p.stroke_rounded(card.inset(0.5), CARD_R - 0.5, 1.0, Color::white(0.22));
            let sub = Rect::new(PAD, PAD, BOX, BOX);
            let act = Rect::new(PAD + BOX + GAP, PAD, BOX, BOX);
            for (r, on) in [(sub, self.focus == Focus::Subject), (act, self.focus == Focus::Action)] {
                p.fill_rounded(r, BOX_R, Color::white(if on { 0.17 } else { 0.09 }));
                let w = if on { 1.6 } else { 1.0 };
                p.stroke_rounded(r.inset(w * 0.5), BOX_R - w * 0.5, w, Color::white(if on { 0.55 } else { 0.15 }));
            }
            let icon = cur.as_ref().and_then(|h| self.icons.get(&h.path)).and_then(|o| o.as_ref());
            draw_subject(&mut p, sub, cur.as_ref(), icon, self.apps_ready);
            draw_action(&mut p, act, cur.as_ref(), self.act, self.focus == Focus::Action);
            if list {
                draw_list(&mut p, &self.hits, &self.icons, &self.query, self.focus, self.sel, vtop, self.act, shown);
            }
        }
        if let Err(e) = self.host.end_draw() {
            crate::log!("launcher: end_draw failed: {e}");
        }
        self.res.end_frame();
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        unsafe {
            for t in [T_ANIM, T_ICON, T_TRIM, T_FREE] {
                let _ = KillTimer(Some(self.hwnd), t);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// -- pieces of the picture -----------------------------------------------------------------------------

fn white(a: f32) -> Color {
    Color::white(a)
}

fn magnifier(p: &Painter, cx: f32, cy: f32, c: Color) {
    p.stroke_circle(cx - 5.0, cy - 5.0, 17.0, 4.0, c);
    p.line(cx + 7.5, cy + 7.5, cx + 20.0, cy + 20.0, 4.5, c);
}

/// A thin line under the letters that matched what was typed.
fn underline(p: &mut Painter, text: &str, spans: &[(usize, usize)], r: Rect, st: &TextStyle, center: bool) {
    if spans.is_empty() {
        return;
    }
    let (tw, th) = p.measure(text, st);
    if tw > r.w {
        // the end is cut off: the positions would be wrong
        return;
    }
    let x0 = if center { r.x + (r.w - tw) * 0.5 } else { r.x };
    let y = r.cy() + th * 0.38;
    for &(a, b) in spans {
        let (a, b) = logic::snap(text, a, b);
        let xa = if a == 0 { 0.0 } else { p.measure(&text[..a], st).0 };
        let xb = p.measure(&text[..b], st).0;
        p.fill_rect(Rect::new(x0 + xa, y, (xb - xa).max(1.0), 1.3), white(0.95));
    }
}

/// The picture of a match: its icon, or a stand-in while the icon loads.
fn draw_thumb(p: &mut Painter, r: Rect, h: &Hit, img: Option<&ImageData>) {
    match h.kind {
        Kind::Calc => {
            p.fill_rounded(r, r.w * 0.26, white(0.14));
            p.text("=", r, &TextStyle::new(r.w * 0.55, white(0.92)).semibold().center());
        }
        Kind::Url | Kind::Web => p.icon(Icon::Globe, r.inset(r.w * 0.1), white(0.9)),
        _ => match img {
            Some(i) => p.image(i, r, 0.0, 1.0),
            None => {
                p.fill_rounded(r, r.w * 0.24, white(0.14));
                let initial: String = h.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                p.text(&initial, r, &TextStyle::new(r.w * 0.46, white(0.9)).semibold().center());
            }
        },
    }
}

fn draw_subject(p: &mut Painter, r: Rect, cur: Option<&Hit>, icon: Option<&ImageData>, ready: bool) {
    let cx = r.cx();
    let Some(h) = cur else {
        magnifier(p, cx, r.y + 64.0, white(0.78));
        let msg = if ready { "Type to search" } else { "Getting ready…" };
        p.text(
            msg,
            Rect::new(r.x + 10.0, r.y + 106.0, r.w - 20.0, 22.0),
            &TextStyle::new(13.5, white(0.7)).semibold().center(),
        );
        p.text(
            "apps · files · sums · links",
            Rect::new(r.x + 10.0, r.y + 130.0, r.w - 20.0, 18.0),
            &TextStyle::new(10.5, white(0.42)).center(),
        );
        return;
    };
    let shown = if h.kind == Kind::Calc { h.detail.as_str() } else { h.name.as_str() };
    if h.kind == Kind::Calc {
        p.text(
            &h.name,
            Rect::new(r.x + 8.0, r.y + 28.0, r.w - 16.0, 66.0),
            &TextStyle::new(36.0, white(0.96)).semibold().center(),
        );
    } else {
        draw_thumb(p, Rect::centered(cx, r.y + 62.0, 72.0, 72.0), h, icon);
    }
    let name_r = Rect::new(r.x + 10.0, r.y + 104.0, r.w - 20.0, 24.0);
    let st = TextStyle::new(14.0, white(0.96)).semibold().center();
    p.text(shown, name_r, &st);
    if h.kind.has_icon() {
        underline(p, shown, &h.spans, name_r, &st, true);
    }
    p.text(
        h.kind.label(),
        Rect::new(r.x + 10.0, r.y + 130.0, r.w - 20.0, 18.0),
        &TextStyle::new(10.5, white(0.5)).center(),
    );
}

fn draw_action(p: &mut Painter, r: Rect, cur: Option<&Hit>, act: usize, focused: bool) {
    let cx = r.cx();
    let Some(h) = cur else {
        p.text(
            "Action",
            Rect::new(r.x + 10.0, r.y + 106.0, r.w - 20.0, 22.0),
            &TextStyle::new(13.5, white(0.4)).semibold().center(),
        );
        return;
    };
    let acts = actions(h);
    let a = acts[act.min(acts.len() - 1)];
    p.icon(a.icon(), Rect::centered(cx, r.y + 62.0, 46.0, 46.0), white(0.92));
    p.text(
        a.label(),
        Rect::new(r.x + 10.0, r.y + 104.0, r.w - 20.0, 24.0),
        &TextStyle::new(14.0, white(0.96)).semibold().center(),
    );
    let hint = if acts.len() < 2 {
        ""
    } else if focused {
        "↓ more actions"
    } else {
        "Tab to change"
    };
    p.text(hint, Rect::new(r.x + 10.0, r.y + 130.0, r.w - 20.0, 18.0), &TextStyle::new(10.5, white(0.5)).center());
}

#[allow(clippy::too_many_arguments)]
fn draw_list(
    p: &mut Painter,
    hits: &[Hit],
    icons: &HashMap<String, Option<ImageData>>,
    query: &str,
    focus: Focus,
    sel: usize,
    top: usize,
    act: usize,
    shown: usize,
) {
    let dim = TextStyle::new(11.5, white(0.55));
    let head = Rect::new(PAD + 8.0, FRAME_H, W - 2.0 * (PAD + 8.0), HEAD);
    let y0 = FRAME_H + HEAD;
    let name_st = TextStyle::new(13.5, white(0.95)).semibold();
    let sub_st = TextStyle::new(10.5, white(0.5));
    match focus {
        Focus::Action => {
            let Some(h) = hits.get(sel) else { return };
            p.text(&format!("Actions for “{}”", logic::mid(&h.name, 30)), head, &dim);
            for (i, a) in actions(h).iter().enumerate().take(shown) {
                let row = Rect::new(10.0, y0 + i as f32 * ROW, W - 20.0, ROW - 2.0);
                if i == act {
                    p.fill_rounded(row, 14.0, white(0.16));
                }
                p.icon(a.icon(), Rect::centered(row.x + 26.0, row.cy(), 22.0, 22.0), white(0.92));
                p.text(a.label(), Rect::new(row.x + 52.0, row.y, row.w - 62.0, row.h), &name_st);
            }
        }
        Focus::Subject => {
            let title = if query.trim().is_empty() {
                "Used most".to_string()
            } else {
                let n = hits.iter().filter(|h| h.kind.has_icon()).count();
                let count = if n >= KEEP {
                    format!("{KEEP}+ results")
                } else if n == 1 {
                    "1 result".to_string()
                } else {
                    format!("{n} results")
                };
                format!("{count}  ·  “{}”", logic::mid(query.trim(), 24))
            };
            p.text(&title, head, &dim);
            for (i, h) in hits.iter().enumerate().skip(top).take(shown) {
                let row = Rect::new(10.0, y0 + (i - top) as f32 * ROW, W - 20.0, ROW - 2.0);
                if i == sel {
                    p.fill_rounded(row, 14.0, white(0.16));
                }
                let ir = Rect::new(row.x + 10.0, row.y + (row.h - 28.0) * 0.5, 28.0, 28.0);
                draw_thumb(p, ir, h, icons.get(&h.path).and_then(|o| o.as_ref()));
                let name_r = Rect::new(row.x + 50.0, row.y + 4.0, row.w - 60.0, 20.0);
                p.text(&h.name, name_r, &name_st);
                if h.kind.has_icon() {
                    underline(p, &h.name, &h.spans, name_r, &name_st, false);
                }
                let detail = logic::mid(&h.detail, 54);
                p.text(&detail, Rect::new(row.x + 50.0, row.y + 24.0, row.w - 60.0, 16.0), &sub_st);
            }
        }
    }
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
        let s = src.trim();
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
    }
}
