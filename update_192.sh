#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "real bottom edge, also while" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,'''            && self.anim.exp.value >= 0.88
            && (body.h - t.h).abs() < 40.0;''','''            && self.anim.exp.value >= 0.88
            && body.h >= 56.0;''')
s=sub(s,'''            let h = BAND_H.min(full.h * 0.4);
            let region = Rect::new(full.x, full.bottom() - h, full.w, h);
            let trans''','''            // the strip sits on the notch's real bottom edge, also while it is still growing or
            // shrinking to the next page's size, and is as wide as the wider of the two
            let h = BAND_H.min(body.h * 0.4);
            let (rx, rr) = (full.x.min(body.x), full.right().max(body.right()));
            let region = Rect::new(rx, body.bottom() - h, rr - rx, h);
            let trans''')
open(p,'w').write(s)
PYEOF
echo "Patched v192"
