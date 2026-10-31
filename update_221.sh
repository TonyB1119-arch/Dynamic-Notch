#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "Color::white(0.86)" src/alcove.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# the "nothing here yet" messages sit low in the notch, over the see-through bottom: brighter
p='src/alcove.rs'; s=open(p).read()
s=sub(s,"        &TextStyle::new(13.0, palette::TEXT_FAINT).center(),\n    );\n    // only keeps frames","        &TextStyle::new(13.0, Color::white(0.86)).center(),\n    );\n    // only keeps frames")
s=sub(s,"Color::white(0.62));\n    ui.p.text(\n        msg,","Color::white(0.8));\n    ui.p.text(\n        msg,")
open(p,'w').write(s)

p='src/ui.rs'; s=open(p).read()
s=sub(s,"self.p.icon(icon, Rect::new(r.cx() - is * 0.5, top, is, is), palette::TEXT_FAINT);","self.p.icon(icon, Rect::new(r.cx() - is * 0.5, top, is, is), Color::white(0.7));")
s=sub(s,"&TextStyle::new(12.5, palette::TEXT_DIM).center());\n    }\n}","&TextStyle::new(12.5, Color::white(0.86)).center());\n    }\n}")
open(p,'w').write(s)
PYEOF
echo "Patched v221"
