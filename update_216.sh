#!/bin/bash
set -e
cd "$(dirname "$0")"
if [ ! -f src/dock.rs ]; then echo "already applied"; exit 0; fi
if ! grep -q "let hybrid = st == 1" src/refract.rs; then echo "run update_215.sh first"; exit 1; fi
python3 - <<'PYEOF'
import os
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ================= the dock goes =================
os.remove('src/dock.rs')
open('src/taskbar.rs','w').write('''//! Gives the Windows taskbar back if an earlier version (whose dock could hide it) was closed
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
''')

p='src/main.rs'; s=open(p).read()
s=sub(s,"mod dock;\n","")
s=sub(s,"mod tray;\n","mod taskbar;\nmod tray;\n")
s=sub(s,"    dock::recover_taskbar();\n","    taskbar::recover_taskbar();\n")
open(p,'w').write(s)

p='src/window.rs'; s=open(p).read()
a=s.index("/// The dock's window: same class and look as the notch's, a different job.\n")
b=s.index("\n}\n",a)+3
s=s[:a]+s[b:].lstrip("\n")
s=sub(s,"""    // The dock window has its own messages (and no tray menu).
    if hwnd.0 as isize == crate::dock::hwnd_value() {
        if let Some(Some(r)) = with_app(|app| app.dock_message(msg, wp, lp)) {
            return r;
        }
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
""","")
open(p,'w').write(s)

p='src/host.rs'; s=open(p).read()
s=sub(s,"""    /// A host that shares an existing compositor (the dock's window).
    pub fn new_shared(hwnd: HWND, gfx: &Gfx, w: i32, h: i32, compositor: Compositor) -> Result<Self> {
        Self::build(hwnd, gfx, w, h, Some(compositor))
    }

""","")
open(p,'w').write(s)

p='src/bloom.rs'; s=open(p).read()
a=s.index("    // Dock Mode: off, on, or peek (auto-hide)\n"); b=s.index("    // Notch Mode: always shown")
s=s[:a]+"""    // Glass edge: on or off
    let glass_on = ui.cfg.appearance.glass_look;
    if pill(ui, cell(1, 0), Icon::Control, "Glass Edge", if glass_on { "On" } else { "Off" }, false, false) {
        ui.fx.save_config.push(("appearance".into(), "glass_look".into(), (!glass_on).to_string()));
    }

"""+s[b:]
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"use crate::dock::{self, Dock};\n","")
s=sub(s,"    fs_hidden: bool,\n    dock: Option<Dock>,\n","    fs_hidden: bool,\n")
s=sub(s,"            dock: None,\n","")
s=sub(s,"        self.place();\n        self.sync_dock();\n        self.tray","        self.place();\n        self.tray")
s=sub(s,"                self.place();\n                self.sync_dock();\n","                self.place();\n")
a=s.index("    /// Create, update or remove the dock to match the settings.\n"); b=s.index("    pub fn shutdown(&mut self) {")
s=s[:a]+s[b:]
s=sub(s,"        self.dock = None;\n","")
s=sub(s,"                if let Some(d) = self.dock.as_mut() {\n                    d.explorer_restarted();\n                }\n","")
s=sub(s,"            if let Some(d) = self.dock.as_mut() {\n                d.set_hidden(hide);\n            }\n","")
s=sub(s,"            if let Some(d) = self.dock.as_mut() {\n                d.recover(&self.gfx);\n            }\n","")
open(p,'w').write(s)

p='src/config.rs'; s=open(p).read()
s=sub(s,"    pub dock: DockCfg,\n","")
a=s.index("/// The dock: pinned and running apps along the bottom edge"); b=s.index("#[derive(Clone, Debug, Default, Serialize, Deserialize)]\n#[serde(default)]\npub struct GreetingCfg")
s=s[:a]+s[b:]
s=sub(s,"        self.dock.size = self.dock.size.clamp(32.0, 80.0);\n        self.dock.magnify_amount = self.dock.magnify_amount.clamp(0.1, 1.0);\n","")
a=s.index("[dock]\n"); b=s.index("[greeting]\n")
s=s[:a]+s[b:]
# frame-rate cap
s=sub(s,"    pub glass_live: bool,\n","    pub glass_live: bool,\n    /// Most frames a second the notch draws while it moves (30 - 240). 60 is smooth and light; raise it on a fast display for\n    /// even smoother motion at a higher CPU cost.\n    pub max_fps: f32,\n")
s=sub(s,"            glass_live: true,\n","            glass_live: true,\n            max_fps: 60.0,\n")
s=sub(s,"        a.scale = a.scale.clamp(0.5, 2.5);\n","        a.scale = a.scale.clamp(0.5, 2.5);\n        a.max_fps = a.max_fps.clamp(30.0, 240.0);\n")
s=sub(s,"glass_live = true\n","glass_live = true\n# Most frames a second while the notch moves (30 - 240). 60 is smooth and light; raise it on a fast display\n# for smoother motion at a higher CPU cost.\nmax_fps = 60\n")
open(p,'w').write(s)

p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,'const TABS: [&str; 10] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock", "Glass"];','const TABS: [&str; 9] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Glass"];')
s=sub(s,"        1 => 15.0,\n","        1 => 16.0,\n")
s=sub(s,"        8 => 8.6,\n","")
s=sub(s,"            8 => self.dock(ui, body),\n","")
a=s.index("    fn dock(&mut self, ui: &mut Ui, body: Rect) {"); b=s.index("    fn tabs(&mut self, ui: &mut Ui, body: Rect) {")
s=s[:a]+s[b:]
s=sub(s,'            "glass_live",\n        );\n    }\n',"""            "glass_live",
        );
        slider_row(
            ui,
            row_rect(body, 15),
            "Max frame rate",
            a.max_fps,
            (30.0, 240.0),
            format!("{:.0}", a.max_fps),
            "appearance",
            "max_fps",
        );
    }
""")
open(p,'w').write(s)

# ================= lower CPU =================
p='src/pacer.rs'; s=open(p).read()
s=sub(s,"use std::sync::atomic::{AtomicBool, Ordering};","use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};")
s=sub(s,"    quit: AtomicBool,\n    wake: isize, // HANDLE\n}","    quit: AtomicBool,\n    /// Shortest time between two frame messages, microseconds (the frame-rate cap; 0 = every refresh).\n    gap_us: AtomicU32,\n    wake: isize, // HANDLE\n}")
s=sub(s,"            quit: AtomicBool::new(false),\n            wake: ev.0 as isize,","            quit: AtomicBool::new(false),\n            gap_us: AtomicU32::new(0),\n            wake: ev.0 as isize,")
s=sub(s,"    /// Keep frames coming until `stop` is called.","""    /// Cap the frame rate: refresh ticks closer together than this are not turned into frames.
    pub fn set_gap_us(&self, us: u32) {
        self.shared.gap_us.store(us, Ordering::Relaxed);
    }

    /// Keep frames coming until `stop` is called.""")
s=sub(s,"        last = Instant::now();\n        // A frame message that was never consumed","""        last = Instant::now();
        // frame-rate cap: on a faster display some refreshes are simply skipped
        let gap = s.gap_us.load(Ordering::Relaxed) as u128;
        if gap > 0 && posted.elapsed().as_micros() < gap {
            continue;
        }
        // A frame message that was never consumed""")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"""    fn request_frame(&mut self) {
        self.dirty = true;
        self.slow_only = false;
        self.pacer.start();
    }
""","""    fn request_frame(&mut self) {
        self.dirty = true;
        self.slow_only = false;
        self.pacer.set_gap_us(self.frame_gap_us());
        self.pacer.start();
    }

    /// Shortest time between two frames, microseconds: the frame-rate cap from Settings → Look,
    /// a little under the refresh period so a 60 Hz screen is never held back.
    fn frame_gap_us(&self) -> u32 {
        let fps = self.cfg.appearance.max_fps.clamp(30.0, 240.0);
        (1_000_000.0 / fps * 0.8) as u32
    }
""")
s=sub(s,"""    fn frame(&mut self) {
        let now = Instant::now();
""","""    fn frame(&mut self) {
        let now = Instant::now();
        // (the pacer already leaves out refreshes beyond the cap; this catches the rest)
        if now.duration_since(self.last_frame).as_micros() < self.frame_gap_us() as u128 * 3 / 4 {
            return;
        }
""")
s=sub(s,"        let q4 = |v: f32| (v * sc * 4.0).round() as i32;","        // while the size changes the shape is followed to a whole pixel (far fewer rebuilds in the slow\n        // tail of a move); once it has settled, to a quarter pixel\n        let qs = if self.settled { 4.0 } else { 1.0 };\n        let q4 = |v: f32| (v * sc * qs).round() as i32;")
open(p,'w').write(s)

p='src/refract.rs'; s=open(p).read()
s=sub(s,"    let threads = if w * h >= 60_000 { workers() } else { 1 };","    let threads = if w * h >= PAR_MIN { workers() } else { 1 };")
s=sub(s,"/// How many threads a big rim may use (looked up once).","""/// Pictures smaller than this are worked out on the one thread (starting threads costs more CPU
/// than it saves); only a very big one is split up.
const PAR_MIN: usize = 400_000;

/// How many threads a big rim may use (looked up once).""")
open(p,'w').write(s)
PYEOF
echo "Patched v216"
