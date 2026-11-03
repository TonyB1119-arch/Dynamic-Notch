
#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
notes=[]
def get(p): return open(p).read()
def sub(s,a,b,tag,all_=False):
    if b in s: return s
    n=s.count(a)
    if n==0 or (n!=1 and not all_):
        notes.append(tag); return s
    return s.replace(a,b)

RUST = r'''//! Tray: the icons other apps keep in the Windows notification area, shown as a tab of the
//! Wi-Fi & Bluetooth page. Click an icon to open the app, right-click for the menu it would show
//! from the taskbar.
//!
//! Windows has no call that lists tray icons, so this does what a tray host does: for a moment it
//! stands in for the taskbar window ("Shell_TrayWnd"), asks every running app to announce its icon
//! again (the same "TaskbarCreated" message Explorer sends after a restart) and reads the
//! announcements. Each one is passed on to the real taskbar, so nothing changes for it. The stand-in
//! goes away again a moment later. Clicks are sent to the app's own callback window, exactly as the
//! taskbar does, so the app draws its own menu.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, Interface, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICBitmapSource, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowW, GetClassNameW, GetCursorPos,
    GetWindowThreadProcessId, PostMessageW, RegisterClassExW, RegisterWindowMessageW, SendMessageTimeoutW, SendNotifyMessageW,
    SetWindowPos, HICON, HWND_BROADCAST, HWND_TOPMOST, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WM_COPYDATA,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use super::widgets::scroll;
use crate::gfx::{Icon, ImageData, TextStyle};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, truncate_chars, Color, Rect};

const HEAD_H: f32 = 44.0;
const HEAD_GAP: f32 = 10.0;
const AREA_H: f32 = 116.0;
const TILE: f32 = 46.0;
const GAP: f32 = 8.0;
/// How long the stand-in window listens after a scan starts.
const LISTEN: Duration = Duration::from_millis(1800);
/// A scan is repeated when the tab is opened and the last one is older than this.
const STALE: Duration = Duration::from_secs(8);

const SIG: u32 = 0x3475_3423;
const NIN_SELECT: u32 = 0x400;
const WM_CONTEXTMENU: u32 = 0x7B;

/// The stand-in window while a scan is running (0 = none).
static SPY: AtomicIsize = AtomicIsize::new(0);
/// Every icon heard so far.
static ICONS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
/// Where the icons are on screen right now (window coordinates), so a right-click can be matched to one.
static TILES: Mutex<Vec<Rect>> = Mutex::new(Vec::new());

/// The pointer is over a tray icon: the right-click belongs to that icon's app.
pub fn over_tile(x: f32, y: f32) -> bool {
    TILES.lock().is_ok_and(|t| t.iter().any(|r| r.contains(x, y)))
}

pub fn clear_tiles() {
    if let Ok(mut t) = TILES.lock() {
        t.clear();
    }
}

#[derive(Clone)]
struct Entry {
    hwnd: isize,
    uid: u32,
    callback: u32,
    version: u32,
    hidden: bool,
    tip: String,
    img: Option<ImageData>,
}

// -- reading what apps announce --------------------------------------------------------------------

fn le32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn le64(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map_or(0, |s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

fn utf16(b: &[u8], o: usize, chars: usize) -> String {
    let Some(s) = b.get(o..o + chars * 2) else { return String::new() };
    let u: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&u)
}

/// One Shell_NotifyIcon call, as Windows passes it to the taskbar: a signature, the action, then
/// the NOTIFYICONDATA (handles are 32 bits wide in either layout).
fn handle_notify(data: &[u8]) {
    if data.len() < 320 || le32(data, 0) != SIG {
        return;
    }
    let action = le32(data, 4);
    if action > 4 {
        return;
    }
    // 64-bit callers send the 64-bit layout; 32-bit callers a shorter one
    let wide = data.len() >= 984;
    let (o_hwnd, o_uid, o_flags, o_cb, o_icon, o_tip, o_state, o_ver) =
        if wide { (16, 24, 28, 32, 40, 48, 304, 824) } else { (12, 16, 20, 24, 28, 32, 288, 808) };
    let h32 = |o: usize| -> isize {
        if wide {
            le64(data, o) as u32 as i32 as isize
        } else {
            le32(data, o) as i32 as isize
        }
    };
    let hwnd = h32(o_hwnd);
    let uid = le32(data, o_uid);
    let flags = le32(data, o_flags);
    let hicon = h32(o_icon);
    let state = le32(data, o_state);
    let mask = le32(data, o_state + 4);
    let img = if matches!(action, 0 | 1) && flags & 2 != 0 && hicon != 0 { icon_image(hicon) } else { None };
    let Ok(mut all) = ICONS.lock() else { return };
    let at = all.iter().position(|e| e.hwnd == hwnd && e.uid == uid);
    match action {
        0 | 1 => {
            let i = at.unwrap_or_else(|| {
                all.push(Entry { hwnd, uid, callback: 0, version: 0, hidden: false, tip: String::new(), img: None });
                all.len() - 1
            });
            let e = &mut all[i];
            if flags & 1 != 0 {
                e.callback = le32(data, o_cb);
            }
            if flags & 4 != 0 {
                e.tip = utf16(data, o_tip, 128);
            }
            if flags & 8 != 0 && mask & 1 != 0 {
                e.hidden = state & 1 != 0;
            }
            if img.is_some() {
                e.img = img;
            }
        }
        2 => {
            if let Some(i) = at {
                all.remove(i);
            }
        }
        _ => {
            if let Some(i) = at {
                all[i].version = le32(data, o_ver).min(4);
            }
        }
    }
}

/// The picture of an icon handle that another process shared.
fn icon_image(h: isize) -> Option<ImageData> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let bmp = wic.CreateBitmapFromHICON(HICON(h as *mut _)).ok()?;
        let src: IWICBitmapSource = bmp.cast().ok()?;
        crate::gfx::image::from_source(&wic, &src, 64).ok()
    }
}

fn exe_name(hwnd: isize) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let file = path.rsplit('\\').next().unwrap_or("");
        file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file).to_string()
    }
}

// -- the stand-in window ---------------------------------------------------------------------------

unsafe extern "system" fn spy_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| copy_data(hwnd, wp, lp)));
        return r.unwrap_or(LRESULT(1));
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe fn copy_data(me: HWND, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let cds = lp.0 as *const COPYDATASTRUCT;
    if !cds.is_null() {
        let c = &*cds;
        let n = c.cbData as usize;
        if !c.lpData.is_null() && (8..65536).contains(&n) {
            handle_notify(std::slice::from_raw_parts(c.lpData as *const u8, n));
        }
    }
    // pass every message on: the real taskbar still needs its icons, appbars and position requests
    match real_taskbar(me) {
        Some(real) => {
            let mut res = 0usize;
            let _ = SendMessageTimeoutW(real, WM_COPYDATA, wp, lp, SMTO_ABORTIFHUNG, 1000, Some(&mut res));
            LRESULT(res as isize)
        }
        None => LRESULT(1),
    }
}

/// Explorer's own taskbar window (not ours).
unsafe fn real_taskbar(not: HWND) -> Option<HWND> {
    struct Find {
        skip: isize,
        found: isize,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let f = &mut *(lp.0 as *mut Find);
        let mut buf = [0u16; 40];
        let n = GetClassNameW(h, &mut buf);
        if n > 0 && h.0 as isize != f.skip && String::from_utf16_lossy(&buf[..n as usize]) == "Shell_TrayWnd" {
            f.found = h.0 as isize;
            return false.into();
        }
        true.into()
    }
    let mut f = Find { skip: not.0 as isize, found: 0 };
    let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut Find as isize));
    (f.found != 0).then(|| HWND(f.found as *mut _))
}

fn stand_in() -> Option<HWND> {
    let cur = SPY.load(Ordering::Relaxed);
    if cur != 0 {
        return Some(HWND(cur as *mut _));
    }
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(spy_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("Shell_TrayWnd"),
            ..Default::default()
        };
        RegisterClassExW(&wc); // already there after the first scan: fine
        let h = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            w!("Shell_TrayWnd"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        SPY.store(h.0 as isize, Ordering::Relaxed);
        Some(h)
    }
}

/// Stand in for the taskbar and ask every app to announce its icon again.
fn start_listening() {
    let Some(spy) = stand_in() else {
        crate::log!("tray: could not create the stand-in window");
        return;
    };
    unsafe {
        // Windows hands tray messages to the first "Shell_TrayWnd" it finds, topmost first
        let _ = SetWindowPos(spy, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let first = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok();
        if first != Some(spy) {
            crate::log!("tray: the real taskbar is found first, icons may not show");
        }
        let again = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, again, WPARAM(0), LPARAM(0));
    }
}

fn stop_listening() {
    let cur = SPY.swap(0, Ordering::Relaxed);
    if cur != 0 {
        unsafe {
            let _ = DestroyWindow(HWND(cur as *mut _));
        }
    }
}

// -- clicks ----------------------------------------------------------------------------------------

/// Tell the app its icon was clicked, the way the taskbar does; the app answers (opens, or shows its menu).
fn click(e: &Entry, right: bool) {
    if e.callback == 0 {
        return;
    }
    unsafe {
        let h = HWND(e.hwnd as *mut _);
        let mut pid = 0u32;
        if GetWindowThreadProcessId(h, Some(&mut pid)) == 0 {
            return;
        }
        let _ = AllowSetForegroundWindow(pid);
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        let post = |ev: u32| {
            let (wp, lp) = if e.version >= 4 {
                let xy = (((at.y & 0xFFFF) as u32) << 16) | (at.x & 0xFFFF) as u32;
                (WPARAM(xy as usize), LPARAM(((((e.uid & 0xFFFF) << 16) | (ev & 0xFFFF)) as i32) as isize))
            } else {
                (WPARAM(e.uid as usize), LPARAM(ev as isize))
            };
            let _ = PostMessageW(Some(h), e.callback, wp, lp);
        };
        if right {
            post(WM_RBUTTONDOWN);
            post(WM_RBUTTONUP);
            if e.version >= 4 {
                post(WM_CONTEXTMENU);
            }
        } else {
            post(WM_LBUTTONDOWN);
            post(WM_LBUTTONUP);
            if e.version >= 4 {
                post(NIN_SELECT);
            }
        }
    }
}

// -- the tab ---------------------------------------------------------------------------------------

pub struct TrayList {
    until: Option<Instant>,
    last_scan: Option<Instant>,
    last_prune: Instant,
    off: f32,
}

impl TrayList {
    pub fn new() -> Self {
        Self { until: None, last_scan: None, last_prune: Instant::now(), off: 0.0 }
    }

    pub fn page_height(&self, _w: f32) -> f32 {
        HEAD_H + HEAD_GAP + AREA_H
    }

    /// When the stand-in window should go away again.
    pub fn deadline(&self) -> Option<Instant> {
        self.until
    }

    pub fn rescan(&mut self) {
        let now = Instant::now();
        start_listening();
        self.until = Some(now + LISTEN);
        self.last_scan = Some(now);
    }

    /// The tab was opened: look again unless it was only just done.
    pub fn enter(&mut self) {
        if self.until.is_none() && self.last_scan.is_none_or(|t| t.elapsed() > STALE) {
            self.rescan();
        }
    }

    /// Remove the stand-in window once the listening time is over. True when that just happened.
    pub fn finish(&mut self, now: Instant) -> bool {
        if self.until.is_some_and(|t| now >= t) {
            stop_listening();
            self.until = None;
            let n = ICONS.lock().map_or(0, |a| a.len());
            crate::log!("tray: {n} icons");
            return true;
        }
        false
    }

    pub fn stop(&mut self) {
        stop_listening();
        self.until = None;
    }

    pub fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        self.finish(ui.now);
        let scanning = self.until.is_some();
        if scanning {
            ui.animate();
        }
        // forget icons whose app has gone
        if self.last_prune.elapsed() > Duration::from_secs(2) {
            self.last_prune = Instant::now();
            if let Ok(mut all) = ICONS.lock() {
                all.retain(|e| unsafe { GetWindowThreadProcessId(HWND(e.hwnd as *mut _), None) != 0 });
            }
        }
        let me = std::process::id();
        let list: Vec<Entry> = ICONS
            .lock()
            .map(|a| {
                a.iter()
                    .filter(|e| !e.hidden && e.img.is_some() && {
                        let mut pid = 0u32;
                        unsafe { GetWindowThreadProcessId(HWND(e.hwnd as *mut _), Some(&mut pid)) };
                        pid != 0 && pid != me
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let head = Rect::new(r.x, r.y, r.w, HEAD_H);
        let rb = Rect::new(head.right() - 30.0, head.cy() - 12.0, 24.0, 24.0);
        let area = Rect::new(r.x, head.bottom() + HEAD_GAP, r.w, AREA_H);

        let cols = (((area.w + GAP) / (TILE + GAP)).floor() as usize).max(1);
        let rows = list.len().div_ceil(cols);
        let content_h = if rows == 0 { 0.0 } else { rows as f32 * (TILE + GAP) - GAP };
        scroll(ui, area, &mut self.off, content_h);

        let mut note: Option<String> = None;
        let mut hits: Vec<Rect> = Vec::new();
        if list.is_empty() {
            ui.empty_state(area, Icon::Grid, if scanning { "Looking for tray icons..." } else { "No tray icons found" });
        } else {
            let used = list.len().min(cols);
            let x0 = area.x + (area.w - (used as f32 * (TILE + GAP) - GAP)) * 0.5;
            ui.p.push_clip(area);
            for (k, e) in list.iter().enumerate() {
                let tile = Rect::new(x0 + (k % cols) as f32 * (TILE + GAP), area.y + (k / cols) as f32 * (TILE + GAP) - self.off, TILE, TILE);
                if tile.bottom() < area.y || tile.y > area.bottom() {
                    continue;
                }
                hits.push(tile);
                let (hover, press) = ui.touch(tile, "tray");
                if hover > 0.01 {
                    ui.hot = true;
                    ui.p.fill_rounded(tile, 13.0, Color::white(0.09 * hover));
                    let name = if e.tip.trim().is_empty() { exe_name(e.hwnd) } else { e.tip.trim().to_string() };
                    note = Some(name);
                }
                if let Some(img) = &e.img {
                    let pad = 9.0 + press * 2.0;
                    ui.p.image(img, tile.inset(pad), 0.0, 1.0);
                }
                if ui.clicked(tile) {
                    click(e, false);
                }
                if ui.right_clicked(tile) {
                    click(e, true);
                }
            }
            ui.p.pop_clip();
        }

        if let Ok(mut t) = TILES.lock() {
            *t = hits;
        }

        // header: what the pointer is on, else a count
        ui.p.icon(Icon::Grid, Rect::new(head.x + 2.0, head.cy() - 9.0, 18.0, 18.0), palette::TEXT_DIM);
        let text = match (&note, scanning) {
            (Some(n), _) => truncate_chars(n, 44),
            (None, true) => "Looking for tray icons...".to_string(),
            (None, false) if list.is_empty() => "Tray".to_string(),
            (None, false) => format!("{} tray icons: click to open, right-click for the menu", list.len()),
        };
        ui.p.text(&text, Rect::new(head.x + 28.0, head.y, (rb.x - head.x - 36.0).max(0.0), head.h), &TextStyle::new(13.0, palette::TEXT).medium());
        if ui.icon_button(rb, Icon::Refresh, ButtonStyle::default().scale(0.6).fg(palette::TEXT_DIM)) {
            self.rescan();
        }
    }
}
'''

# ---- modules/mod.rs --------------------------------------------------------------------------
m=get('src/modules/mod.rs')
m=sub(m,"pub mod control;\n","pub mod control;\npub mod traylist;\n","mod.rs")

# ---- modules/control.rs: a third tab ---------------------------------------------------------
c=get('src/modules/control.rs')
c=sub(c,"use super::bluetooth::Bluetooth;\n","use super::bluetooth::Bluetooth;\nuse super::traylist::TrayList;\n","import")
c=sub(c,"    bt: Bluetooth,\n    /// 0 = Wi-Fi, 1 = Bluetooth.\n    tab: usize,\n","    bt: Bluetooth,\n    tray: TrayList,\n    /// 0 = Wi-Fi, 1 = Bluetooth, 2 = Tray.\n    tab: usize,\n    /// The tab drawn last time (to notice when Tray is opened).\n    shown: usize,\n","struct")
c=sub(c,"Self { wifi: Wifi::new(), bt: Bluetooth::new(), tab: 0, pos: 0.0, last: Instant::now() }","Self { wifi: Wifi::new(), bt: Bluetooth::new(), tray: TrayList::new(), tab: 0, shown: 0, pos: 0.0, last: Instant::now() }","new")
c=sub(c,"        self.bt.stop();\n    }\n","        self.bt.stop();\n        self.tray.stop();\n    }\n","stop")
c=sub(c,"    fn glance(&self) -> Option<(Icon, String)> {","    fn next_tick(&self) -> Option<Instant> {\n        self.tray.deadline()\n    }\n\n    fn on_tick(&mut self, cx: &mut Cx) {\n        if self.tray.finish(Instant::now()) {\n            cx.fx.redraw = true;\n        }\n    }\n\n    fn glance(&self) -> Option<(Icon, String)> {","tick")
c=sub(c,"        let a = self.wifi.page_height(w);\n        let b = self.bt.page_height(w);\n        let t = self.pos.clamp(0.0, 1.0);\n        BAR_H + BAR_GAP + a + (b - a) * t\n","        let h = [self.wifi.page_height(w), self.bt.page_height(w), self.tray.page_height(w)];\n        let t = self.pos.clamp(0.0, 2.0);\n        let i = (t.floor() as usize).min(1);\n        BAR_H + BAR_GAP + h[i] + (h[i + 1] - h[i]) * (t - i as f32)\n","height")
c=sub(c,"        if want == 0 || want == 1 {","        if (0..=2).contains(&want) {","want")
c=sub(c,"[(Icon::Wifi, \"Wi-Fi\"), (Icon::Bluetooth, \"Bluetooth\")]","[(Icon::Wifi, \"Wi-Fi\"), (Icon::Bluetooth, \"Bluetooth\"), (Icon::Grid, \"Tray\")]","chips",True)
c=sub(c,"            let half = bar.w * 0.5;\n","            let half = bar.w / 3.0;\n","third")
c=sub(c,"        if wheel < 0.0 {\n            self.tab = 1;\n        } else if wheel > 0.0 {\n            self.tab = 0;\n        }\n","        if wheel < 0.0 {\n            self.tab = (self.tab + 1).min(2);\n        } else if wheel > 0.0 {\n            self.tab = self.tab.saturating_sub(1);\n        }\n        if self.tab == 2 && self.shown != 2 {\n            self.tray.enter();\n        }\n        self.shown = self.tab;\n","wheel")
c=sub(c,"        for i in 0..2usize {","        for i in 0..3usize {","loop")
c=sub(c,"            if i == 0 {\n                self.wifi.draw_page(ui, sub);\n            } else {\n                self.bt.draw_page(ui, sub);\n            }\n","            match i {\n                0 => self.wifi.draw_page(ui, sub),\n                1 => self.bt.draw_page(ui, sub),\n                _ => self.tray.draw_page(ui, sub),\n            }\n","dispatch")

c=sub(c,"        ui.p.pop_clip();\n    }\n}\n","        ui.p.pop_clip();\n        if self.tab != 2 {\n            super::traylist::clear_tiles();\n        }\n    }\n}\n","clear tiles")

# ---- window.rs / app.rs: a right-click on a tray icon goes to that icon's app ------------------
wn=get('src/window.rs')
wn=sub(wn,"fn wndproc_inner(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {\n","fn wndproc_inner(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {\n    // A right-click on a tray icon (Tray tab) goes to that icon's app, not to the notch's own menu.\n    if msg == WM_RBUTTONUP && with_app(|a| a.tray_right_click(lp)) == Some(true) {\n        return LRESULT(0);\n    }\n","window.rs")
ap=get('src/app.rs')
ap=sub(ap,"    fn on_mouse_move(&mut self, x: f32, y: f32) {\n","    /// A right-click on a tray icon of the Wi-Fi & Bluetooth page: true when it was taken.\n    pub fn tray_right_click(&mut self, lp: LPARAM) -> bool {\n        if self.mode != Mode::Expanded || self.page != \"control\" {\n            return false;\n        }\n        let (x, y) = self.lp_to_logical(lp);\n        if !crate::modules::traylist::over_tile(x, y) {\n            return false;\n        }\n        self.input.right_click = Some((x, y));\n        self.request_frame();\n        true\n    }\n\n    fn on_mouse_move(&mut self, x: f32, y: f32) {\n","app.rs")

if notes:
    print("NOTHING CHANGED - these spots in your code look different:")
    for n in notes: print("  -",n)
else:
    open('src/modules/traylist.rs','w').write(RUST)
    open('src/modules/mod.rs','w').write(m)
    open('src/modules/control.rs','w').write(c)
    open('src/window.rs','w').write(wn)
    open('src/app.rs','w').write(ap)
    print("ok")
PYEOF
echo "update_269 done"
