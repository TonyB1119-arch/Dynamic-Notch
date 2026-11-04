//! The running programs, for the launcher's Kill mode: what each one uses, looked at in the background.
//!
//! Programs that run several processes (a browser, say) are counted as one: their memory and processor
//! use are added up. The ones Windows needs to stay alive, and this app itself, are left out.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    GetSystemTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::util::{from_wide, SendHwnd};

/// Mirrors PROCESS_MEMORY_COUNTERS.
#[repr(C)]
struct Counters {
    cb: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota_peak_paged: usize,
    quota_paged: usize,
    quota_peak_non_paged: usize,
    quota_non_paged: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(process: HANDLE, counters: *mut Counters, cb: u32) -> i32;
    fn GetProcessTimes(process: HANDLE, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
}

/// One program (all its processes together).
#[derive(Clone, Debug, Default)]
pub struct Proc {
    /// The file name, like `chrome.exe`.
    pub name: String,
    /// Where it lives (empty when Windows does not say).
    pub path: String,
    /// Memory in use (bytes).
    pub mem: u64,
    /// Processor use, in percent of the whole computer.
    pub cpu: f32,
    /// How many processes it runs.
    pub count: u32,
    /// A system program (or this app): listed, but it cannot be ended from here.
    pub locked: bool,
}

/// What the computer is doing right now.
#[derive(Clone, Debug, Default)]
pub struct Snap {
    pub procs: Vec<Proc>,
    /// Processor use of the whole computer (0 to 1).
    pub cpu: f32,
    pub ram_used: u64,
    pub ram_total: u64,
}

pub type Slot = Arc<Mutex<Option<Snap>>>;

/// Looks at the programs now and then until it is dropped.
pub struct Sampler {
    stop: Arc<AtomicBool>,
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What the previous look saw (the processor use needs two looks).
#[derive(Default)]
struct Prev {
    idle: u64,
    total: u64,
    ticks: HashMap<u32, u64>,
    paths: HashMap<String, String>,
}

/// Start looking: the first look is taken now and returned; the next ones arrive in `slot`, each with a
/// message to `hwnd`.
pub fn start(slot: Slot, hwnd: SendHwnd, msg: u32) -> (Sampler, Snap) {
    let stop = Arc::new(AtomicBool::new(false));
    let mut prev = Prev::default();
    let first = take(&mut prev);
    let flag = stop.clone();
    let _ = std::thread::Builder::new().name("launcher-procs".into()).spawn(move || {
        let mut wait = 350u64;
        loop {
            let mut left = wait;
            while left > 0 {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                let step = left.min(50);
                std::thread::sleep(Duration::from_millis(step));
                left -= step;
            }
            wait = 1000;
            let snap = take(&mut prev);
            if flag.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(mut g) = slot.lock() {
                *g = Some(snap);
            }
            unsafe {
                let _ = PostMessageW(Some(hwnd.hwnd()), msg, WPARAM(0), LPARAM(0));
            }
        }
    });
    (Sampler { stop }, first)
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Programs that must not be ended from here.
pub fn protected(exe: &str) -> bool {
    const KEEP: [&str; 24] = [
        "explorer.exe",
        "svchost.exe",
        "winlogon.exe",
        "csrss.exe",
        "wininit.exe",
        "services.exe",
        "lsass.exe",
        "dwm.exe",
        "taskkill.exe",
        "smss.exe",
        "fontdrvhost.exe",
        "sihost.exe",
        "ctfmon.exe",
        "runtimebroker.exe",
        "taskhostw.exe",
        "searchhost.exe",
        "startmenuexperiencehost.exe",
        "shellexperiencehost.exe",
        "lockapp.exe",
        "audiodg.exe",
        "conhost.exe",
        "msmpeng.exe",
        "system",
        "registry",
    ];
    let bytes = exe.as_bytes();
    let is_exe = bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".exe");
    if !is_exe || exe.len() > 100 || exe.starts_with('-') || KEEP.iter().any(|k| exe.eq_ignore_ascii_case(k)) {
        return true;
    }
    // this app itself (its file name is looked up once)
    static ME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let me = ME.get_or_init(|| {
        std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()))
    });
    me.as_deref().is_some_and(|m| exe.eq_ignore_ascii_case(m))
}

/// Look at every process once.
fn take(prev: &mut Prev) -> Snap {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    let times_ok = unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_ok();
    let (idle, total) = if times_ok { (ft(idle), ft(kernel) + ft(user)) } else { (0, 0) };
    let dsys = if prev.total > 0 { total.saturating_sub(prev.total) } else { 0 };
    let dcpu = if dsys > 0 { 1.0 - idle.saturating_sub(prev.idle) as f32 / dsys as f32 } else { 0.0 };
    let had_before = dsys > 0;

    let mut snap = Snap { cpu: dcpu.clamp(0.0, 1.0), ..Default::default() };
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
        snap.ram_total = m.ullTotalPhys;
        snap.ram_used = m.ullTotalPhys.saturating_sub(m.ullAvailPhys);
    }

    let mut by_name: HashMap<String, Proc> = HashMap::new();
    let mut ticks: HashMap<u32, u64> = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return snap };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut e).is_ok() {
            loop {
                let pid = e.th32ProcessID;
                let name = from_wide(&e.szExeFile);
                if pid > 4 && !name.is_empty() {
                    let key = name.to_ascii_lowercase();
                    let mut used = 0u64;
                    let mut mine = 0u64;
                    let mut path = String::new();
                    if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        let mut c: Counters = std::mem::zeroed();
                        c.cb = std::mem::size_of::<Counters>() as u32;
                        if K32GetProcessMemoryInfo(h, &mut c, std::mem::size_of::<Counters>() as u32) != 0 {
                            used = c.working_set as u64;
                        }
                        let (mut a, mut b, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
                        if GetProcessTimes(h, &mut a, &mut b, &mut k, &mut u) != 0 {
                            mine = k + u;
                        }
                        if !prev.paths.contains_key(&key) {
                            let mut buf = [0u16; 1024];
                            let mut len = buf.len() as u32;
                            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                                .is_ok()
                            {
                                path = String::from_utf16_lossy(&buf[..len as usize]);
                            }
                        }
                        let _ = CloseHandle(h);
                    }
                    if !path.is_empty() {
                        prev.paths.insert(key.clone(), path);
                    }
                    let mut share = 0.0f32;
                    if had_before {
                        if let Some(before) = prev.ticks.get(&pid) {
                            share = mine.saturating_sub(*before) as f32 / dsys as f32 * 100.0;
                        }
                    }
                    ticks.insert(pid, mine);
                    let p = by_name.entry(key).or_insert_with(|| Proc { name: name.clone(), locked: protected(&name), ..Default::default() });
                    p.mem += used;
                    p.cpu += share;
                    p.count += 1;
                }
                if Process32NextW(snapshot, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    for (key, mut p) in by_name {
        p.path = prev.paths.get(&key).cloned().unwrap_or_default();
        snap.procs.push(p);
    }
    prev.idle = idle;
    prev.total = total;
    prev.ticks = ticks;
    // forget the paths of programs that are gone
    if prev.paths.len() > 600 {
        prev.paths.clear();
    }
    snap
}

// -- showing them ---------------------------------------------------------------------------------------

/// How a program is called on screen: its file name without `.exe`, with a capital.
pub fn title(exe: &str) -> String {
    let stem = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// "412 MB", "1.3 GB".
pub fn mem_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if mb >= 10.0 {
        format!("{:.0} MB", mb)
    } else {
        format!("{:.1} MB", mb)
    }
}

/// "3.2%", "0%".
pub fn cpu_text(p: f32) -> String {
    if p >= 10.0 {
        format!("{:.0}%", p)
    } else if p >= 0.05 {
        format!("{:.1}%", p)
    } else {
        "0%".to_string()
    }
}

/// How well a name fits what was typed (higher is better; none = it does not fit).
pub fn score(name: &str, q: &str) -> Option<u32> {
    let n = name.to_lowercase();
    let n = n.strip_suffix(".exe").unwrap_or(&n);
    if q.is_empty() {
        return Some(0);
    }
    if n == q {
        return Some(1000);
    }
    if n.starts_with(q) {
        return Some(800u32.saturating_sub((n.len() - q.len()).min(100) as u32));
    }
    if let Some(at) = n.find(q) {
        return Some(600u32.saturating_sub(at.min(100) as u32));
    }
    // the letters in order, with gaps
    let mut it = n.chars();
    let mut gaps = 0u32;
    for c in q.chars() {
        let mut skipped = 0u32;
        loop {
            match it.next() {
                Some(x) if x == c => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(300u32.saturating_sub(gaps.min(250)))
}

/// The programs that fit what was typed, best first (equal fits: the one using more memory first). With
/// nothing typed: the ones using the most memory.
pub fn rank(procs: &[Proc], q: &str) -> Vec<usize> {
    let q = q.trim().to_lowercase();
    let mut v: Vec<(usize, u32)> = procs.iter().enumerate().filter_map(|(i, p)| score(&p.name, &q).map(|s| (i, s))).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(procs[b.0].mem.cmp(&procs[a.0].mem)).then(procs[a.0].name.cmp(&procs[b.0].name)));
    v.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, mem: u64) -> Proc {
        Proc { name: name.into(), mem, count: 1, ..Default::default() }
    }

    #[test]
    fn fits_are_ranked() {
        let list = vec![p("chrome.exe", 500), p("notepad.exe", 5), p("chromedriver.exe", 900), p("discord.exe", 300)];
        let r = rank(&list, "chrome");
        assert_eq!(list[r[0]].name, "chrome.exe");
        assert_eq!(list[r[1]].name, "chromedriver.exe");
        assert_eq!(r.len(), 2);
        let r = rank(&list, "cord");
        assert_eq!(list[r[0]].name, "discord.exe");
        let r = rank(&list, "cme");
        assert!(list[r[0]].name.starts_with("chrome"));
        assert!(rank(&list, "zzz").is_empty());
    }

    #[test]
    fn nothing_typed_is_by_memory() {
        let list = vec![p("a.exe", 1), p("b.exe", 3), p("c.exe", 2)];
        let r: Vec<&str> = rank(&list, "  ").into_iter().map(|i| list[i].name.as_str()).collect();
        assert_eq!(r, ["b.exe", "c.exe", "a.exe"]);
    }

    #[test]
    fn texts() {
        assert_eq!(title("chrome.exe"), "Chrome");
        assert_eq!(mem_text(412 * 1_048_576), "412 MB");
        assert_eq!(mem_text(1_395_864_371), "1.3 GB");
        assert_eq!(mem_text(3 * 1_048_576), "3.0 MB");
        assert_eq!(cpu_text(0.0), "0%");
        assert_eq!(cpu_text(3.24), "3.2%");
        assert_eq!(cpu_text(42.0), "42%");
    }
}
