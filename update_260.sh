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
    s=rep(s,"    pub hide_apps: Vec<String>,\n}\n\nimpl Default for DockCfg","    pub hide_apps: Vec<String>,\n    /// The liquid glass edge, the same glass as the notch (see Settings → Glass).\n    pub glass: bool,\n}\n\nimpl Default for DockCfg")
    s=rep(s,"            hide_apps: vec![\"raycast.exe\".to_string()],\n        }","            hide_apps: vec![\"raycast.exe\".to_string()],\n            glass: true,\n        }")
    s=rep(s,"hide_apps = ['raycast.exe']\n","hide_apps = ['raycast.exe']\n# The liquid glass edge, the same glass as the notch\nglass = true\n")
    return s
edit('src/config.rs',cfg)
def st(s):
    s=rep(s,'        toggle_row(ui, row_rect(body, 3), "Show open apps", d.show_running, "dock", "show_running");','        toggle_row(ui, row_rect(body, 3), "Show open apps", d.show_running, "dock", "show_running");\n        toggle_row(ui, row_rect(body, 8), "Liquid glass edge", d.glass, "dock", "glass");')
    s=rep(s,"        8 => 8.0,\n","        8 => 9.0,\n")
    return s
edit('src/modules/settings.rs',st)
def app(s):
    return rep(s,"d.configure(&self.gfx, &self.cfg.dock, mon, scale);","d.configure(&self.gfx, &self.cfg.dock, mon, scale, crate::refract::Tune::from(&self.cfg.glass));")
edit('src/app.rs',app)
print('ok')
PYEOF
echo "update_260 applied"
