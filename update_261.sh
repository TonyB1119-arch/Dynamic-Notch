#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def edit(path,fn):
    s=open(path).read(); open(path,'w').write(fn(s))
def app(s):
    return s.replace(", crate::refract::Tune::from(&self.cfg.glass));",");")
edit('src/app.rs',app)
def st(s):
    s=s.replace('        toggle_row(ui, row_rect(body, 8), "Liquid glass edge", d.glass, "dock", "glass");\n','')
    s=s.replace("        8 => 9.0,\n","        8 => 8.0,\n")
    return s
edit('src/modules/settings.rs',st)
def cfg(s):
    s=s.replace("            opacity: 0.42,\n","            opacity: 0.30,\n").replace("opacity = 0.42\n","opacity = 0.30\n")
    return s
edit('src/config.rs',cfg)
print('ok')
PYEOF
echo "update_261 applied"
