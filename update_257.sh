#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def edit(path,fn):
    s=open(path).read(); open(path,'w').write(fn(s))
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
def app(s):
    s=rep(s,"    pub fn dock_items(&self) -> Vec<String> {\n        self.cfg.dock.items.clone()\n    }","    pub fn dock_items(&self) -> Vec<String> {\n        match self.dock.as_ref() {\n            Some(d) => d.items_cfg(),\n            None => self.cfg.dock.items.clone(),\n        }\n    }\n\n    /// Open the pin picker panel above the dock.\n    pub fn dock_open_picker(&mut self) {\n        if let Some(d) = self.dock.as_mut() {\n            d.open_picker(&self.gfx);\n        }\n    }")
    return s
edit('src/app.rs',app)
print("ok")
PYEOF
echo "update_257 applied"
