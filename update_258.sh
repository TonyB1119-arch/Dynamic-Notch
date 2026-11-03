#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
s=open('src/app.rs').read()
s=rep(s,"    /// Open the pin picker panel above the dock.","    /// Load the dock's icons again (a new icon file was chosen).\n    pub fn dock_reload_icons(&mut self) {\n        if let Some(d) = self.dock.as_mut() {\n            d.reload_icons(&self.gfx);\n        }\n    }\n\n    /// Open the pin picker panel above the dock.")
open('src/app.rs','w').write(s)
print('ok')
PYEOF
echo "update_258 applied"
