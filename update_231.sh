#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "let mstep = step;" src/app.rs; then echo "already applied"; exit 0; fi
if ! grep -q "let mstep" src/app.rs; then echo "run update_230.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        // lighter animations also work the moving glass out on every third pixel (about 4 times less
        // work, and it is drawn smoothed); the full picture follows once the notch rests
        let mstep = if low && lite { 3 } else { step };""","""        // (the moving glass keeps full pixel detail along the edge: coarser pictures looked pixelated)
        let mstep = step;""")
s=sub(s,"""        if low {
            made.tune.sharp = moving_zone;
        }""","""        if low {
            made.tune.sharp = moving_zone;
            if lite {
                // lighter animations: while the notch moves the middle is left clear (no bend, no blur,
                // about half the work); the full look melts in as soon as it rests
                made.tune.mid_warp = 0.0;
                made.tune.mid_blur = 0.0;
            }
        }""")
open(p,'w').write(s)
PYEOF
echo "Patched v231"
