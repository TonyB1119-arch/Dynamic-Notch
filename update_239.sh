#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fan_h" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "deep_spans" src/refract.rs; then echo "run update_238.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()
# the stretch fans out along the bottom: the left part stretches left, the right part right, and only
# the middle straight down (the corners keep turning on from where the fan leaves off)
s=sub(s,"    let stretch_cap = -0.92 * width / if even_bend { 1.5 } else { t.bend.max(1.0) };\n","""    let stretch_cap = -0.92 * width / if even_bend { 1.5 } else { t.bend.max(1.0) };
    // the fan: how far the stretch turns sideways at the ends of the bottom edge (radians), and the
    // middle and half width it is measured from
    const FAN: f32 = 1.0;
    let (fan_c, fan_h) = ((sh.left + sh.right) * 0.5, ((sh.right - sh.left) * 0.5).max(1.0));
""")
s=sub(s,"""            ox = nx * p * k;
            oy = ny * p * k;
""","""            let (nx, ny) = if t.dir < 0.0 && ny > 0.0 {
                // turn the direction sideways by more the further from the middle (and by less the
                // closer it already points sideways, so it still meets the straight sides)
                let s = ((x - fan_c) / fan_h).clamp(-1.0, 1.0);
                let (sn, cs) = (FAN * s.signum() * smooth(0.0, 1.0, s.abs()) * ny).sin_cos();
                (nx * cs + ny * sn, ny * cs - nx * sn)
            } else {
                (nx, ny)
            };
            ox = nx * p * k;
            oy = ny * p * k;
""")
# no two pixels along the rim of a fanned row look alike, so the one-per-row shortcut is for normal glass only
s=sub(s,"""        let (mut xa, mut xb) = if db < width {
            (sh.left + db, sh.right - db)""","""        let (mut xa, mut xb) = if db < width {
            if t.dir < 0.0 {
                return (usize::MAX, 0);
            }
            (sh.left + db, sh.right - db)""")
open(p,'w').write(s)
PYEOF
echo "Patched v239"
