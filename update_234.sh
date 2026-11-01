#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "never folds" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/refract.rs'; s=open(p).read()
s=sub(s,"            let p = pull * t.dir * (1.0 + t.corner * corner);\n","""            let mut p = pull * t.dir * (1.0 + t.corner * corner);
            if t.dir < 0.0 {
                // stretching only works while the picture keeps its order across the rim; past this
                // limit it folds over and the text reads backwards, so the stretch never folds
                let steepest = if even_bend { 1.5 } else { t.bend.max(1.0) };
                p = p.max(-0.92 * width / steepest);
            }
""")
open(p,'w').write(s)
PYEOF
echo "Patched v234"
