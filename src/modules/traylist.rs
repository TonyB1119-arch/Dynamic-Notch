//! Tray: the icons other apps keep in the Windows notification area, shown as a tab of the
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
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ,
};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowW, GetClassNameW, GetCursorPos,
    GetWindowThreadProcessId, PostMessageW, RegisterClassExW, RegisterWindowMessageW, SendMessageTimeoutW, SendNotifyMessageW,
    FindWindowExW, SetWindowPos, HWND_MESSAGE, HICON, HWND_BROADCAST, HWND_TOPMOST, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WM_COPYDATA,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use windows::Win32::System::Threading::{TerminateProcess, PROCESS_TERMINATE};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, TrackPopupMenuEx, MF_STRING, TPM_BOTTOMALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CLOSE, WM_NULL,
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
/// The same for the icons Windows lists that never announced themselves: (place, index in FALLBACK).
static FB_TILES: Mutex<Vec<(Rect, usize)>> = Mutex::new(Vec::new());

/// The pointer is over a tray icon: the right-click belongs to that icon's app.
pub fn over_tile(x: f32, y: f32) -> bool {
    TILES.lock().is_ok_and(|t| t.iter().any(|r| r.contains(x, y)))
}

pub fn clear_tiles() {
    if let Ok(mut t) = TILES.lock() {
        t.clear();
    }
    if let Ok(mut t) = FB_TILES.lock() {
        t.clear();
    }
}

/// The listed (not announced) icon under the pointer, as its index.
pub fn fb_at(x: f32, y: f32) -> Option<usize> {
    FB_TILES.lock().ok().and_then(|t| t.iter().find(|(r, _)| r.contains(x, y)).map(|(_, i)| *i))
}

/// Such an app gave Windows no way to ask for its own menu, so right-click offers what can be done
/// from outside: open it, close its windows, end it. Runs outside the app borrow (a menu waits).
pub fn fb_menu(owner: HWND, idx: usize) {
    let Some(f) = FALLBACK.lock().ok().and_then(|f| f.get(idx).cloned()) else { return };
    let name = if f.tip.trim().is_empty() { f.name.clone() } else { f.tip.trim().to_string() };
    let cmd = unsafe {
        let menu = CreatePopupMenu().unwrap_or_default();
        for (id, text) in [
            (1usize, format!("Open {}", name.replace('&', "&&"))),
            (2, "Close its windows".to_string()),
            (3, "End task".to_string()),
        ] {
            let t: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(t.as_ptr()));
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(owner);
        let flags = (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0;
        let c = TrackPopupMenuEx(menu, flags, pt.x, pt.y, owner, None);
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        c.0
    };
    match cmd {
        1 => open_fb(&f),
        2 => close_fb(&f),
        3 => end_fb(&f),
        _ => {}
    }
}

/// Ask the program's visible windows to close (like the X button).
fn close_fb(f: &Fb) {
    struct Find {
        pids: Vec<u32>,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let f = &*(lp.0 as *const Find);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if f.pids.contains(&pid) && IsWindowVisible(h).as_bool() && GetWindowTextLengthW(h) > 0 {
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        true.into()
    }
    let find = Find { pids: f.pids.clone() };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&find as *const Find as isize));
    }
}

/// Stop the program for good.
fn end_fb(f: &Fb) {
    for &pid in &f.pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Icons Windows 11 lists for the taskbar: found from its own list and the programs that are running.
/// Apps that do not announce their icon the old way still show up this way (without their own menu).
static FALLBACK: Mutex<Vec<Fb>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Fb {
    /// File name without .exe, lower case.
    name: String,
    tip: String,
    path: String,
    pids: Vec<u32>,
    img: Option<ImageData>,
}

enum Tile<'a> {
    Live(&'a Entry),
    Reg(&'a Fb),
}

/// The programs Windows 11 keeps a tray icon for (HKCU\Control Panel\NotifyIconSettings) that are running now.
fn registry_icons() -> Vec<Fb> {
    // (file name in lower case, tooltip)
    let mut want: Vec<(String, String)> = Vec::new();
    unsafe {
        let key: Vec<u16> = "Control Panel\\NotifyIconSettings".encode_utf16().chain(Some(0)).collect();
        let mut h = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), Some(0), KEY_READ, &mut h) != WIN32_ERROR(0) {
            return Vec::new();
        }
        let mut i = 0u32;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(h, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None)
                != WIN32_ERROR(0)
            {
                break;
            }
            i += 1;
            let sub: Vec<u16> = name[..len as usize].iter().copied().chain(Some(0)).collect();
            let read = |value: PCWSTR| -> String {
                let mut buf = [0u16; 520];
                let mut sz = (buf.len() * 2) as u32;
                let r = RegGetValueW(
                    h,
                    PCWSTR(sub.as_ptr()),
                    value,
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr() as *mut _),
                    Some(&mut sz),
                );
                if r == WIN32_ERROR(0) {
                    crate::util::from_wide(&buf)
                } else {
                    String::new()
                }
            };
            let exe = read(w!("ExecutablePath"));
            let file = exe.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
            if file.ends_with(".exe") && !want.iter().any(|(n, _)| *n == file) {
                want.push((file, read(w!("InitialTooltip"))));
            }
        }
        let _ = RegCloseKey(h);
    }
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    want.retain(|(n, _)| n != "explorer.exe" && me.as_deref() != Some(n.as_str()));
    // which of them are running
    let mut running: Vec<(String, Vec<u32>)> = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return Vec::new() };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = crate::util::from_wide(&e.szExeFile).to_ascii_lowercase();
                if want.iter().any(|(w, _)| *w == n) {
                    match running.iter_mut().find(|(r, _)| *r == n) {
                        Some((_, pids)) => pids.push(e.th32ProcessID),
                        None => running.push((n, vec![e.th32ProcessID])),
                    }
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    let mut out = Vec::new();
    for (n, pids) in running {
        let tip = want.iter().find(|(w, _)| *w == n).map(|(_, t)| t.clone()).unwrap_or_default();
        let path = pids.iter().find_map(|&pid| unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(h);
            ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
        });
        let Some(path) = path else { continue };
        let img = crate::sys::dragdrop::thumbnail(&path, 64);
        let stem = n.strip_suffix(".exe").unwrap_or(&n).to_string();
        out.push(Fb { name: stem, tip, path, pids, img });
    }
    out
}

/// Bring the program's window forward (or start it, which most programs answer by showing themselves).
fn open_fb(f: &Fb) {
    struct Find {
        pids: Vec<u32>,
        found: isize,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let f = &mut *(lp.0 as *mut Find);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if f.pids.contains(&pid) && IsWindowVisible(h).as_bool() && GetWindowTextLengthW(h) > 0 {
            f.found = h.0 as isize;
            return false.into();
        }
        true.into()
    }
    unsafe {
        let mut find = Find { pids: f.pids.clone(), found: 0 };
        let _ = EnumWindows(Some(each), LPARAM(&mut find as *mut Find as isize));
        if find.found != 0 {
            let h = HWND(find.found as *mut _);
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
            return;
        }
    }
    let _ = std::process::Command::new(&f.path).spawn();
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
        // the broadcast only reaches normal windows: many apps keep their tray window hidden as a
        // message-only window, so those are asked one by one
        let mut prev = HWND::default();
        for _ in 0..800 {
            let Ok(next) = FindWindowExW(Some(HWND_MESSAGE), Some(prev), PCWSTR::null(), PCWSTR::null()) else { break };
            let _ = SendNotifyMessageW(next, again, WPARAM(0), LPARAM(0));
            prev = next;
        }
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
        if let Ok(mut f) = FALLBACK.lock() {
            *f = registry_icons();
        }
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
            let hooked: Vec<String> =
                ICONS.lock().map(|a| a.iter().map(|e| exe_name(e.hwnd).to_ascii_lowercase()).collect()).unwrap_or_default();
            if let Ok(mut f) = FALLBACK.lock() {
                f.retain(|x| !hooked.contains(&x.name));
                crate::log!("tray: {} icons announced, {} more from the list of Windows", hooked.len(), f.len());
            }
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
            if let Ok(mut f) = FALLBACK.lock() {
                f.retain(|x| x.pids.iter().any(|&pid| unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map(|h| CloseHandle(h)).is_ok() }));
            }
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

        let fbs: Vec<Fb> = FALLBACK.lock().map(|f| f.clone()).unwrap_or_default();
        let all: Vec<Tile> = list.iter().map(Tile::Live).chain(fbs.iter().map(Tile::Reg)).collect();

        let head = Rect::new(r.x, r.y, r.w, HEAD_H);
        let rb = Rect::new(head.right() - 30.0, head.cy() - 12.0, 24.0, 24.0);
        let area = Rect::new(r.x, head.bottom() + HEAD_GAP, r.w, AREA_H);

        let cols = (((area.w + GAP) / (TILE + GAP)).floor() as usize).max(1);
        let rows = all.len().div_ceil(cols);
        let content_h = if rows == 0 { 0.0 } else { rows as f32 * (TILE + GAP) - GAP };
        scroll(ui, area, &mut self.off, content_h);

        let mut note: Option<String> = None;
        let mut hits: Vec<Rect> = Vec::new();
        let mut fb_hits: Vec<(Rect, usize)> = Vec::new();
        if all.is_empty() {
            ui.empty_state(area, Icon::Grid, if scanning { "Looking for tray icons..." } else { "No tray icons found" });
        } else {
            let used = all.len().min(cols);
            let x0 = area.x + (area.w - (used as f32 * (TILE + GAP) - GAP)) * 0.5;
            ui.p.push_clip(area);
            for (k, t) in all.iter().enumerate() {
                let tile = Rect::new(x0 + (k % cols) as f32 * (TILE + GAP), area.y + (k / cols) as f32 * (TILE + GAP) - self.off, TILE, TILE);
                if tile.bottom() < area.y || tile.y > area.bottom() {
                    continue;
                }
                hits.push(tile);
                if matches!(t, Tile::Reg(_)) {
                    fb_hits.push((tile, k - list.len()));
                }
                let (hover, press) = ui.touch(tile, "tray");
                if hover > 0.01 {
                    ui.hot = true;
                    ui.p.fill_rounded(tile, 13.0, Color::white(0.09 * hover));
                    let name = match t {
                        Tile::Live(e) if e.tip.trim().is_empty() => exe_name(e.hwnd),
                        Tile::Live(e) => e.tip.trim().to_string(),
                        Tile::Reg(f) if f.tip.trim().is_empty() => f.name.clone(),
                        Tile::Reg(f) => f.tip.trim().to_string(),
                    };
                    note = Some(name);
                }
                let img_of = match t {
                    Tile::Live(e) => e.img.as_ref(),
                    Tile::Reg(f) => f.img.as_ref(),
                };
                if let Some(img) = img_of {
                    let pad = 9.0 + press * 2.0;
                    ui.p.image(img, tile.inset(pad), 0.0, 1.0);
                }
                if ui.clicked(tile) {
                    match t {
                        Tile::Live(e) => click(e, false),
                        Tile::Reg(f) => open_fb(f),
                    }
                }
                if ui.right_clicked(tile) {
                    // (the listed icons get their own menu from the window procedure)
                    if let Tile::Live(e) = t {
                        click(e, true);
                    }
                }
            }
            ui.p.pop_clip();
        }

        if let Ok(mut t) = TILES.lock() {
            *t = hits;
        }
        if let Ok(mut t) = FB_TILES.lock() {
            *t = fb_hits;
        }

        // header: what the pointer is on, else a count
        ui.p.icon(Icon::Grid, Rect::new(head.x + 2.0, head.cy() - 9.0, 18.0, 18.0), palette::TEXT_DIM);
        let text = match (&note, scanning) {
            (Some(n), _) => truncate_chars(n, 44),
            (None, true) => "Looking for tray icons...".to_string(),
            (None, false) if all.is_empty() => "Tray".to_string(),
            (None, false) => format!("{} tray icons: click to open, right-click for the menu", all.len()),
        };
        ui.p.text(&text, Rect::new(head.x + 28.0, head.y, (rb.x - head.x - 36.0).max(0.0), head.h), &TextStyle::new(13.0, palette::TEXT).medium());
        if ui.icon_button(rb, Icon::Refresh, ButtonStyle::default().scale(0.6).fg(palette::TEXT_DIM)) {
            self.rescan();
        }
    }
}
