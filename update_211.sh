#!/bin/bash
set -e
cd "$(dirname "$0")"
if ! grep -q "let hl = if d < 5.0" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/refract.rs'; s=open(p).read()
# no bevel highlight (it read as a border)
s=sub(s,"""            let hl = if d < 5.0 * sc { 0.14 * smooth(-0.5, 1.2 * sc, d) * (1.0 - smooth(1.2 * sc, 5.0 * sc, d)) } else { 0.0 };
            let ch = |v: f32| ((v + (255.0 - v) * hl) * alpha).round().clamp(0.0, 255.0) as u32;""","""            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;""")
# back to the gentler bend: the stronger pull made text show twice along the edge
s=sub(s,"    let pull = 28.0 * sc; // how far out the strongest pixels look","    let pull = 22.0 * sc; // how far out the strongest pixels look")
s=sub(s,"""            // the bend is steepest right at the edge and eases away quickly (u^1.75): like light
            // wrapping around the rim of thick glass
            let k = u * u.sqrt() * u.sqrt().sqrt();
            let pull = pull * (1.0 + 0.2 * corner);""","""            let k = u * u.sqrt();
            let pull = pull * (1.0 + 0.3 * corner);""")
open(p,'w').write(s)
PYEOF
echo "Patched v211"
