#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "straight edges stretch" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/refract.rs'; s=open(p).read()
s=sub(s,"            let mut p = pull * t.dir * (1.0 + t.corner * corner);\n","""            // stretch mode: the straight edges stretch nearly as much as the corners do
            // (corner = 0 along them, so they used to get no boost at all)
            let cw = if t.dir < 0.0 { corner.max(0.85) } else { corner };
            let mut p = pull * t.dir * (1.0 + t.corner * cw);
""")
open(p,'w').write(s)
PYEOF
echo "Patched v237"
