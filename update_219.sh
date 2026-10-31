#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "st == 1 && ox == 0.0" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "sharp_zone" src/refract.rs; then echo "run update_217_218.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/config.rs'; s=open(p).read()
s=sub(s,"            middle_warp: 0.5,\n","            middle_warp: 0.0,\n")
s=sub(s,"            middle_blur: 1.5,\n","            middle_blur: 0.0,\n")
s=sub(s,"middle_warp = 0.5\n","middle_warp = 0.0\n")
s=sub(s,"middle_blur = 1.5\n","middle_blur = 0.0\n")
open(p,'w').write(s)

p='src/refract.rs'; s=open(p).read()
# no warp: skip working out the bend directions altogether
s=sub(s,"            if d > width {\n                let dm;","            if d > width && t.mid_warp > 0.0 {\n                let dm;")
# clear middle (no warp, no blur): the pixel behind is simply copied, no blending
s=sub(s,"""        } else {
            // red, green and blue look a slightly different distance out, like light through glass
            (""","""        } else if st == 1 && ox == 0.0 && oy == 0.0 {
            // nothing bends it here, so the pixel behind is copied as it is (the same result as the
            // three blended reads below, without the blending)
            let xi = (x - shot.ox as f32).floor().clamp(0.0, shot.w as f32 - 1.0) as usize;
            let yi = (y - shot.oy as f32).floor().clamp(0.0, shot.h as f32 - 1.0) as usize;
            let p = shot.px[yi * shot.w + xi];
            (((p >> 16) & 255) as f32, ((p >> 8) & 255) as f32, (p & 255) as f32)
        } else {
            // red, green and blue look a slightly different distance out, like light through glass
            (""")
open(p,'w').write(s)
PYEOF
echo "Patched v219"
