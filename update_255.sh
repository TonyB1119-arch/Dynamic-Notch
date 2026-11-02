#!/usr/bin/env bash
set -e
# Dock update: auto-hide, rounder bar, transparency, hide taskbar, hide_apps. Run from the repo root after copying dock.rs over src/dock.rs
python3 - <<'PYEOF'
import re,sys
def edit(path,fn):
    s=open(path).read(); s2=fn(s); open(path,'w').write(s2)
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
def cfg(s):
    s=rep(s,"    pub items: Vec<String>,\n}\n\nimpl Default for DockCfg","    pub items: Vec<String>,\n    /// Slide the dock away until the pointer touches the bottom edge.\n    pub auto_hide: bool,\n    /// Hide the Windows taskbar while the dock runs.\n    pub hide_taskbar: bool,\n    /// How solid the dock is (0.1 = nearly clear, 1.0 = solid).\n    pub opacity: f32,\n    /// Programs that never show up in the dock.\n    pub hide_apps: Vec<String>,\n}\n\nimpl Default for DockCfg")
    s=rep(s,"            items: vec![r\"Explorer|%WINDIR%\\explorer.exe\".to_string()],\n        }","            items: vec![r\"Explorer|%WINDIR%\\explorer.exe\".to_string()],\n            auto_hide: false,\n            hide_taskbar: false,\n            opacity: 0.42,\n            hide_apps: vec![\"raycast.exe\".to_string()],\n        }")
    s=rep(s,"        self.items.truncate(40);","        self.opacity = self.opacity.clamp(0.1, 1.0);\n        self.hide_apps.truncate(60);\n        self.items.truncate(40);")
    s=rep(s,"bottom_margin = 6.0\n# Each item","bottom_margin = 6.0\n# Slide away until the pointer touches the bottom of the screen\nauto_hide = false\n# Hide the Windows taskbar while the dock is running (it comes back when the app closes)\nhide_taskbar = false\n# 0.1 = nearly clear, 1.0 = solid\nopacity = 0.42\n# Programs that never appear in the dock\nhide_apps = ['raycast.exe']\n# Each item")
    return s
edit('src/config.rs',cfg)
def st(s):
    s=rep(s,'''toggle_row(ui, row_rect(body, 18), "Dock at the bottom of the screen", dock_on, "dock", "enabled");
''','''toggle_row(ui, row_rect(body, 18), "Dock at the bottom of the screen", dock_on, "dock", "enabled");
        let (d_hide, d_tb, d_op) = (self.cfg.dock.auto_hide, self.cfg.dock.hide_taskbar, self.cfg.dock.opacity);
        toggle_row(ui, row_rect(body, 19), "Auto-hide the dock", d_hide, "dock", "auto_hide");
        toggle_row(ui, row_rect(body, 20), "Hide the Windows taskbar", d_tb, "dock", "hide_taskbar");
        slider_row(
            ui,
            row_rect(body, 21),
            "Dock transparency (higher = more solid)",
            d_op,
            (0.1, 1.0),
            format!("{:.0}%", d_op * 100.0),
            "dock",
            "opacity",
        );
''')
    s=rep(s,"        1 => 19.0,","        1 => 22.0,")
    return s
edit('src/modules/settings.rs',st)
def tb(s):
    s=rep(s,"fn taskbar_windows()",'''/// Hide the Windows taskbar (auto-hide on, windows hidden). The old state is remembered in a marker
/// file so `recover_taskbar` can undo this even after a crash. Safe to call again and again.
pub fn hide_taskbar() {
    unsafe {
        let p = marker_path();
        let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
        if !p.exists() {
            let old = SHAppBarMessage(4, &mut abd); // ABM_GETSTATE
            let _ = std::fs::write(&p, old.to_string());
        }
        let old = std::fs::read_to_string(&p).ok().and_then(|t| t.trim().parse::<isize>().ok()).unwrap_or(0);
        abd.lParam = LPARAM(old | 1); // ABS_AUTOHIDE
        SHAppBarMessage(10, &mut abd); // ABM_SETSTATE
        for b in taskbar_windows() {
            let _ = ShowWindow(b, SW_HIDE);
        }
    }
}

fn taskbar_windows()''')
    s=rep(s,"FindWindowW, ShowWindow, SW_SHOWNA}","FindWindowW, ShowWindow, SW_HIDE, SW_SHOWNA}")
    return s
edit('src/taskbar.rs',tb)
print("ok")
PYEOF
echo "update_255 applied"
