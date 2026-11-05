//! The window switcher (Alt+Tab): the open windows, topmost first, and a small picture of each.

use std::collections::HashMap;
use std::ffi::c_void;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
};

use crate::gfx::ImageData;

/// Most windows the switcher lists.
const MAX_WINDOWS: usize = 30;

#[link(name = "user32")]
unsafe extern "system" {
    fn PrintWindow(hwnd: *mut c_void, hdc: *mut c_void, flags: u32) -> i32;
}

/// One window of the list.
#[derive(Clone, Debug)]
pub struct Win {
    pub hwnd: isize,
    pub title: String,
    /// The program's file (its path), empty when it cannot be told.
    pub exe: String,
    pub minimized: bool,
}

impl Win {
    /// The program's name for the second line of a card ("chrome", not "C:\...\chrome.exe").
    pub fn app(&self) -> String {
        let file = self.exe.rsplit('\\').next().unwrap_or("");
        file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file).to_string()
    }
}

/// The windows that would be in a taskbar, in the order they are stacked (the one in front first).
pub fn list() -> Vec<Win> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<(isize, u32)>);
        if crate::dock::is_app_window(h) {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            out.push((h.0 as isize, pid));
        }
        true.into()
    }
    let mut found: Vec<(isize, u32)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut _ as isize));
    }
    let me = std::process::id();
    let mut exes: HashMap<u32, String> = HashMap::new();
    let mut out = Vec::new();
    for (h, pid) in found {
        if pid == me {
            continue;
        }
        let exe = exes.entry(pid).or_insert_with(|| crate::dock::exe_of(pid)).clone();
        let hwnd = HWND(h as *mut c_void);
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) }.max(0) as usize;
        let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        let minimized = unsafe { IsIconic(hwnd) }.as_bool();
        out.push(Win { hwnd: h, title, exe, minimized });
        if out.len() >= MAX_WINDOWS {
            break;
        }
    }
    out
}

/// A small picture of a window (about `max_w` pixels wide), or None when it is minimised, gone or
/// shows nothing (some windows draw black when copied).
pub fn capture(hwnd: isize, max_w: i32) -> Option<ImageData> {
    unsafe {
        let h = HWND(hwnd as *mut c_void);
        if !IsWindow(Some(h)).as_bool() || IsIconic(h).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        let (w, ht) = (r.right - r.left, r.bottom - r.top);
        if !(16..=8192).contains(&w) || !(16..=8192).contains(&ht) {
            return None;
        }
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let dc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -ht,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let out = match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) => {
                let mut img = None;
                if !bits.is_null() {
                    let old = SelectObject(dc, HGDIOBJ(bmp.0));
                    // 2 = PW_RENDERFULLCONTENT: windows drawn by the graphics card show up too
                    if PrintWindow(h.0, dc.0, 2) != 0 {
                        let src = std::slice::from_raw_parts(bits as *const u8, w as usize * ht as usize * 4);
                        img = shrink(src, w as usize, ht as usize, (w / max_w.max(1)).max(1) as usize);
                    }
                    SelectObject(dc, old);
                }
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                img
            }
            Err(_) => None,
        };
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        out
    }
}

/// The picture made `k` times smaller (each new pixel is the average of a `k` x `k` block), opaque.
fn shrink(src: &[u8], w: usize, h: usize, k: usize) -> Option<ImageData> {
    let (ow, oh) = (w / k, h / k);
    if ow == 0 || oh == 0 {
        return None;
    }
    let mut out = vec![0u8; ow * oh * 4];
    let mut lit = false;
    let div = (k * k) as u32;
    for y in 0..oh {
        for x in 0..ow {
            let mut sum = [0u32; 3];
            for dy in 0..k {
                let row = ((y * k + dy) * w + x * k) * 4;
                for dx in 0..k {
                    let i = row + dx * 4;
                    sum[0] += src[i] as u32;
                    sum[1] += src[i + 1] as u32;
                    sum[2] += src[i + 2] as u32;
                }
            }
            let o = (y * ow + x) * 4;
            for c in 0..3 {
                out[o + c] = (sum[c] / div) as u8;
            }
            out[o + 3] = 255;
            lit |= sum[0] + sum[1] + sum[2] > div * 12;
        }
    }
    lit.then(|| ImageData::from_pbgra(ow as u32, oh as u32, out))
}
