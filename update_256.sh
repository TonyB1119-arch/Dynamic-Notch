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
    return rep(s,"    /// Start, update or remove the dock to match the settings.","""    /// What a right-click at `lp` landed on in the dock: (label, path, pinned).
    pub fn dock_menu_target(&self, lp: LPARAM) -> Option<(String, String, bool)> {
        self.dock.as_ref()?.menu_target(lp)
    }

    /// The pinned list as written in the settings.
    pub fn dock_items(&self) -> Vec<String> {
        self.cfg.dock.items.clone()
    }

    /// Start, update or remove the dock to match the settings.""")
edit('src/app.rs',app)
def cfg(s):
    s=rep(s,"    let mut section_seen = false;\n    for line in text.lines() {\n        let t = line.trim();\n","    let mut section_seen = false;\n    let mut skipping = false;\n    for line in text.lines() {\n        let t = line.trim();\n        if skipping {\n            skipping = !line.contains(']');\n            continue;\n        }\n")
    s=rep(s,"                out.push(format!(\"{key} = {value}\"));\n                done = true;\n                continue;","                out.push(format!(\"{key} = {value}\"));\n                done = true;\n                let rest = t.splitn(2, '=').nth(1).unwrap_or(\"\").trim();\n                skipping = rest.starts_with('[') && !rest.contains(']');\n                continue;")
    return s
edit('src/config.rs',cfg)
print("ok")
PYEOF
echo "update_256 applied"
