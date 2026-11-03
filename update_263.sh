#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def edit(path,fn):
    s=open(path).read(); open(path,'w').write(fn(s))
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
def cfg(s):
    s=rep(s,"    pub hide_apps: Vec<String>,\n","    pub hide_apps: Vec<String>,\n    /// The white rim around the dock: thickness in pixels (0 = none) and brightness (0 - 1).\n    pub rim_width: f32,\n    pub rim_bright: f32,\n")
    s=rep(s,"            hide_apps: vec![\"raycast.exe\".to_string()],\n","            hide_apps: vec![\"raycast.exe\".to_string()],\n            rim_width: 1.2,\n            rim_bright: 0.35,\n")
    s=rep(s,"        self.hide_apps.truncate(60);","        self.hide_apps.truncate(60);\n        self.rim_width = self.rim_width.clamp(0.0, 4.0);\n        self.rim_bright = self.rim_bright.clamp(0.0, 1.0);")
    s=rep(s,"hide_apps = ['raycast.exe']\n","hide_apps = ['raycast.exe']\n# The white rim around the dock: thickness (pixels, 0 = none) and brightness (0 - 1)\nrim_width = 1.2\nrim_bright = 0.35\n")
    return s
edit('src/config.rs',cfg)
def st(s):
    s=rep(s,'''            "bottom_margin",
        );
    }
''','''            "bottom_margin",
        );
        slider_row(
            ui,
            row_rect(body, 8),
            "Rim thickness",
            d.rim_width,
            (0.0, 4.0),
            format!("{:.1}", d.rim_width),
            "dock",
            "rim_width",
        );
        slider_row(
            ui,
            row_rect(body, 9),
            "Rim brightness",
            d.rim_bright,
            (0.0, 1.0),
            format!("{:.0}%", d.rim_bright * 100.0),
            "dock",
            "rim_bright",
        );
    }
''')
    s=rep(s,"        8 => 8.0,\n","        8 => 10.0,\n")
    return s
edit('src/modules/settings.rs',st)
print('ok')
PYEOF
echo "update_263 applied"
