#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def edit(path,fn):
    s=open(path).read(); open(path,'w').write(fn(s))
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
def st(s):
    s=rep(s,'const TABS: [&str; 9] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Glass"];','const TABS: [&str; 10] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock", "Glass"];')
    s=rep(s,"        7 => 5.9,\n        _ => 25.0,","        7 => 5.9,\n        8 => 8.0,\n        _ => 25.0,")
    s=rep(s,"        1 => 22.0,","        1 => 18.0,")
    # remove the dock rows from the Look tab
    if "let dock_on = self.cfg.dock.enabled;" in s:
        a=s.index("        let dock_on = self.cfg.dock.enabled;")
        endm='"dock",\n            "opacity",\n        );\n'
        b=s.index(endm,a)+len(endm)
        s=s[:a]+s[b:]
    s=rep(s,"            7 => self.tabs(ui, body),\n            _ => self.glass(ui, body),","            7 => self.tabs(ui, body),\n            8 => self.dock(ui, body),\n            _ => self.glass(ui, body),")
    s=rep(s,"    fn glass(&mut self, ui: &mut Ui, body: Rect) {",'''    fn dock(&mut self, ui: &mut Ui, body: Rect) {
        let d = self.cfg.dock.clone();
        toggle_row(ui, row_rect(body, 0), "Show the dock", d.enabled, "dock", "enabled");
        toggle_row(ui, row_rect(body, 1), "Auto-hide", d.auto_hide, "dock", "auto_hide");
        toggle_row(ui, row_rect(body, 2), "Hide the Windows taskbar", d.hide_taskbar, "dock", "hide_taskbar");
        toggle_row(ui, row_rect(body, 3), "Show open apps", d.show_running, "dock", "show_running");
        slider_row(
            ui,
            row_rect(body, 4),
            "Icon size",
            d.icon_size,
            (32.0, 96.0),
            format!("{:.0}", d.icon_size),
            "dock",
            "icon_size",
        );
        slider_row(
            ui,
            row_rect(body, 5),
            "Magnification",
            d.magnify,
            (1.0, 2.2),
            format!("{:.1}x", d.magnify),
            "dock",
            "magnify",
        );
        slider_row(
            ui,
            row_rect(body, 6),
            "Transparency (higher = more solid)",
            d.opacity,
            (0.1, 1.0),
            format!("{:.0}%", d.opacity * 100.0),
            "dock",
            "opacity",
        );
        slider_row(
            ui,
            row_rect(body, 7),
            "Distance from the bottom",
            d.bottom_margin,
            (0.0, 40.0),
            format!("{:.0}", d.bottom_margin),
            "dock",
            "bottom_margin",
        );
    }

    fn glass(&mut self, ui: &mut Ui, body: Rect) {''')
    return s
edit('src/modules/settings.rs',st)
def tb(s):
    s=rep(s,"FindWindowW, ShowWindow, SW_HIDE, SW_SHOWNA}","FindWindowW, IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNA}")
    s=rep(s,"        let p = marker_path();\n        let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };\n        if !p.exists() {","        let p = marker_path();\n        // already hidden: nothing to do (this runs every few seconds)\n        if p.exists() && !taskbar_windows().iter().any(|b| IsWindowVisible(*b).as_bool()) {\n            return;\n        }\n        let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };\n        if !p.exists() {")
    return s
edit('src/taskbar.rs',tb)
print('ok')
PYEOF
echo "update_259 applied"
