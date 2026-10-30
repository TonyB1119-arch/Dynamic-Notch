#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b):
    assert s.count(a)==1,(s.count(a),a[:70]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"if overlap { (0.52, 0.26) } else { (SPLIT, SPLIT) };","if overlap { (0.46, 0.38) } else { (SPLIT, SPLIT) };")
s=sub(s,"        ui.p.alpha = base * (1.0 - e);\n","        // overlapping: the old page drops away fast, so two similar pages never show as a double image\n        ui.p.alpha = base * if overlap { (1.0 - e) * (1.0 - e) } else { 1.0 - e };\n")
open(p,'w').write(s)
PYEOF
echo "Patched v175"
