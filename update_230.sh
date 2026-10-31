#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "let mstep" src/app.rs; then echo "already applied"; exit 0; fi
if ! grep -q "lite_motion" src/config.rs; then echo "run update_227.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        let low = !self.settled && tune.sharp > moving_zone && !still;
        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, step + if low { RIM_LOW } else { 0 });""","""        let low = !self.settled && !still && (lite || tune.sharp > moving_zone);
        // lighter animations also work the moving glass out on every third pixel (about 4 times less
        // work, and it is drawn smoothed); the full picture follows once the notch rests
        let mstep = if low && lite { 3 } else { step };
        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, mstep + if low { RIM_LOW } else { 0 });""")
s=sub(s,"if let Some(rim) = crate::refract::rim(shot, made, step) {","if let Some(rim) = crate::refract::rim(shot, made, mstep) {")
open(p,'w').write(s)
PYEOF
echo "Patched v230"
