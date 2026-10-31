#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "no seam" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()
# corner strength: a smooth bump along the arc (0 where the arc meets the straight sides, 1 in the
# middle of the curve) instead of an on/off switch that left a visible seam
s=sub(s,"""                nx = dx / dist;
                ny = dy / dist;
                corner = 1.0;""","""                nx = dx / dist;
                ny = dy / dist;
                // 0 where the curve meets the straight edges, 1 in the middle of the curve (no seam)
                corner = (2.0 * dx.abs() * dy / (dist * dist)).clamp(0.0, 1.0);""")
# outer edge: a soft ramp instead of a one pixel edge
s=sub(s,"let alpha = (0.88 + 0.1 * corner) * fade * (d + 1.0).clamp(0.0, 1.0) * up;","let alpha = (0.88 + 0.1 * corner) * fade * smooth(-1.0, 2.5 * sc, d) * up;")
open(p,'w').write(s)

# app.rs: full detail only for the very end of a move (about the last 12 %)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        // how far the notch still has to go, and how far it had to go when this move began: in the
        // last third of a real move the rim is already made at full detail, so it is sharp by the
        // time the notch arrives (the work is spread over the slow end of the move)""","""        // how far the notch still has to go, and how far it had to go when this move began: in the
        // last part of a real move (the last ~12 %, where it is slow) the rim is already made at full
        // detail, so it is sharp by the time the notch arrives""")
s=sub(s,"let late = self.rim_far >= 24.0 && left <= self.rim_far * 0.34;","let late = self.rim_far >= 24.0 && left <= self.rim_far * 0.12;")
open(p,'w').write(s)
PYEOF
echo "Patched v209"
