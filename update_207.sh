#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "let prev = self.refr_rim.take();" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        self.refr_rim = None;
        if let Some(rim) = crate::refract::rim(shot, shape, step) {
            if let Some(pic) = self.res.rim_pic(rim.x0, rim.y0, rim.w, rim.h, rim.step, &rim.px, sc) {
                self.refr_rim = Some((key, pic));
            }
        }""","""        let prev = self.refr_rim.take();
        if let Some(rim) = crate::refract::rim(shot, shape, step) {
            if let Some(pic) = self.res.rim_pic(rim.x0, rim.y0, rim.w, rim.h, rim.step, &rim.px, sc) {
                self.refr_rim = Some((key, pic));
                // the notch has just stopped and the fine rim replaces the coarse one made while it
                // moved: the two melt into each other instead of one jumping to the other
                if let Some(old) = prev {
                    if old.0 .5 > key.5 && old.0 .0 == key.0 && old.0 .1 == key.1 && old.0 .2 == key.2 && old.0 .3 == key.3 && old.0 .4 == key.4 {
                        self.rim_old = Some(old);
                        self.rim_x = 0.0;
                    }
                }
            }
        }""")
open(p,'w').write(s)
PYEOF
echo "Patched v207"
