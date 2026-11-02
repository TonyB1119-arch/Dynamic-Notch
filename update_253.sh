#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "DockCfg" src/config.rs; then echo "already applied"; exit 0; fi
if [ ! -f src/dock.rs ]; then echo "run update_252.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s, a, b, n=1):
    assert s.count(a) == n, (s.count(a), a[:80])
    return s.replace(a, b)

def rw(p, f):
    s = open(p).read()
    open(p, 'w').write(f(s))

# ---- config ------------------------------------------------------------------------------
def cfg(s):
    s = sub(s, "    pub glass: GlassCfg,\n}", "    pub glass: GlassCfg,\n    pub dock: DockCfg,\n}")
    s = sub(s, "        self.glass.clamped();\n", "        self.glass.clamped();\n        self.dock.clamped();\n")
    s = sub(s, "#[derive(Clone, Debug, Serialize, Deserialize)]\n#[serde(default)]\npub struct PortsCfg {", """/// The dock at the bottom of the screen.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DockCfg {
    pub enabled: bool,
    /// Icon size in logical pixels.
    pub icon_size: f32,
    /// How much an icon grows under the pointer (1.0 = not at all).
    pub magnify: f32,
    /// Also show the apps that are open but not pinned.
    pub show_running: bool,
    /// Gap between the dock and the bottom of the work area.
    pub bottom_margin: f32,
    /// "path", "Name|path" or "Name|path|icon.png".
    pub items: Vec<String>,
}

impl Default for DockCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            icon_size: 56.0,
            magnify: 1.6,
            show_running: true,
            bottom_margin: 6.0,
            items: vec![r"Explorer|%WINDIR%\\explorer.exe".to_string()],
        }
    }
}

impl DockCfg {
    fn clamped(&mut self) {
        self.icon_size = self.icon_size.clamp(32.0, 96.0);
        self.magnify = self.magnify.clamp(1.0, 2.2);
        self.bottom_margin = self.bottom_margin.clamp(0.0, 80.0);
        self.items.truncate(40);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PortsCfg {""")
    s = sub(s, "\n[glass]\n# The Alcove glass edge", """
[dock]
# A glass dock at the bottom of the screen: your pinned apps, then the apps that are open.
# To change an icon, put a .png named after the item (Chrome.png) or its exe (chrome.png)
# in %APPDATA%\\DynamicNotch\\dock-icons
enabled = true
icon_size = 56.0
magnify = 1.6
show_running = true
bottom_margin = 6.0
# Each item is "path", "Name|path" or "Name|path|icon.png". %VARS% work in paths.
items = ['Explorer|%WINDIR%\\explorer.exe']

[glass]
# The Alcove glass edge""")
    return s
rw('src/config.rs', cfg)

# ---- host: a second window on the same compositor ----------------------------------------------
def host(s):
    s = sub(s, "    fn build(hwnd: HWND, gfx: &Gfx, w: i32, h: i32, shared: Option<Compositor>)", """    /// Another window drawn with the compositor of the first one (the dock).
    pub fn new_shared(hwnd: HWND, gfx: &Gfx, w: i32, h: i32, compositor: Compositor) -> Result<Self> {
        Self::build(hwnd, gfx, w, h, Some(compositor))
    }

    fn build(hwnd: HWND, gfx: &Gfx, w: i32, h: i32, shared: Option<Compositor>)""")
    s = sub(s, "unsafe fn create_swapchain(", """impl Host {
    /// The blur behind a bar with all four corners round (pixel coordinates).
    pub fn set_backdrop_rounded(&mut self, body_px: Rect, radius_px: f32, opacity: f32) {
        let visible = opacity > 0.002;
        if visible != self.backdrop_visible {
            let _ = self.backdrop.SetIsVisible(visible);
            self.backdrop_visible = visible;
        }
        if !visible {
            return;
        }
        let r = radius_px.max(0.0);
        let now = [body_px.x, body_px.y, body_px.w, body_px.h, r + opacity * 1000.0];
        if now == self.backdrop_last {
            return;
        }
        self.backdrop_last = now;
        let _ = self.backdrop_geom.SetOffset(v2(body_px.x, body_px.y));
        let _ = self.backdrop_geom.SetSize(v2(body_px.w, body_px.h));
        let _ = self.backdrop_geom.SetCornerRadius(v2(r, r));
        let _ = self.backdrop.SetOpacity(opacity);
    }
}

unsafe fn create_swapchain(""")
    return s
rw('src/host.rs', host)

rw('src/main.rs', lambda s: sub(s, "mod config;\n", "mod config;\nmod dock;\n"))

# ---- the app ---------------------------------------------------------------------------------------
def app(s):
    s = sub(s, "    trim_at: Option<Instant>,\n", "    trim_at: Option<Instant>,\n    dock: Option<crate::dock::Dock>,\n")
    s = sub(s, "            trim_at: None,\n", "            trim_at: None,\n            dock: None,\n")
    s = sub(s, "    pub fn start(&mut self) {\n        self.place();\n", "    pub fn start(&mut self) {\n        self.place();\n        self.dock_apply();\n")
    s = sub(s, "        self.check_fullscreen();\n        self.request_frame();\n    }\n\n    fn pick_monitor", "        self.dock_relocate();\n        self.check_fullscreen();\n        self.request_frame();\n    }\n\n    fn pick_monitor")
    s = sub(s, "                crate::ui::set_mono_icons(self.cfg.appearance.mono_icons);\n", "                crate::ui::set_mono_icons(self.cfg.appearance.mono_icons);\n                self.dock_apply();\n")
    s = sub(s, "            let _ = self.host.set_rendering_device(&self.gfx);\n", "            let _ = self.host.set_rendering_device(&self.gfx);\n            if let Some(d) = self.dock.as_mut() {\n                d.recover(&self.gfx);\n            }\n")
    s = sub(s, "    fn recover_device(&mut self) {", """    /// Messages for the dock's own window.
    pub fn dock_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let hidden = self.fs_hidden;
        let d = self.dock.as_mut()?;
        d.handle(&self.gfx, hidden, msg, wp, lp)
    }

    /// Start, update or remove the dock to match the settings.
    fn dock_apply(&mut self) {
        if !self.cfg.dock.enabled {
            self.dock = None;
            return;
        }
        if self.dock.is_none() {
            match crate::dock::Dock::new(&self.gfx, self.host.compositor.clone()) {
                Ok(d) => self.dock = Some(d),
                Err(e) => {
                    log!("dock failed to start: {e}");
                    return;
                }
            }
        }
        let (mon, scale) = (self.monitor, self.cfg.appearance.scale);
        if mon.is_invalid() {
            return;
        }
        if let Some(d) = self.dock.as_mut() {
            d.configure(&self.gfx, &self.cfg.dock, mon, scale);
        }
    }

    /// The screen or the monitor changed: put the dock where it belongs.
    fn dock_relocate(&mut self) {
        let (mon, scale) = (self.monitor, self.cfg.appearance.scale);
        if mon.is_invalid() {
            return;
        }
        if let Some(d) = self.dock.as_mut() {
            d.relocate(&self.gfx, mon, scale);
        }
    }

    fn recover_device(&mut self) {""")
    return s
rw('src/app.rs', app)

# ---- settings: a switch on the Look tab ---------------------------------------------------------------
def st(s):
    s = sub(s, """        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");\n""",
            """        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");
        let dock_on = self.cfg.dock.enabled;
        toggle_row(ui, row_rect(body, 18), "Dock at the bottom of the screen", dock_on, "dock", "enabled");\n""")
    return sub(s, "        1 => 18.0,\n", "        1 => 19.0,\n")
rw('src/modules/settings.rs', st)
PYEOF
echo "Patched v253 - now run ./release.sh"
