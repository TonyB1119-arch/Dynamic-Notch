//! Gives the Windows taskbar back if an earlier version (whose dock could hide it) was closed
//! abnormally: it is shown again at the next start. Nothing else lives here any more.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, ShowWindow, SW_SHOWNA};

use crate::util::app_dir;

fn marker_path() -> std::path::PathBuf {
    app_dir().join("taskbar-hidden")
}

/// After a crash the Windows taskbar may still be hidden: give it back at the next start.
pub fn recover_taskbar() {
    let p = marker_path();
    if let Ok(text) = std::fs::read_to_string(&p) {
        let old = text.trim().parse::<usize>().unwrap_or(0);
        show_taskbar(old);
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

/// Show every Windows taskbar again and put its old auto-hide state back.
fn show_taskbar(old_state: usize) {
    unsafe {
        let bars = taskbar_windows();
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: HWND::default(),
            lParam: LPARAM(old_state as isize),
            ..Default::default()
        };
        SHAppBarMessage(10, &mut abd); // ABM_SETSTATE
        for b in &bars {
            let _ = ShowWindow(*b, SW_SHOWNA);
        }
    }
}
