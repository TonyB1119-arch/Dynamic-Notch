#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "u^1.75" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()
# a stronger, more lens-like warp: more pull, concentrated nearer the edge
s=sub(s,"    let pull = 22.0 * sc; // how far out the strongest pixels look","    let pull = 28.0 * sc; // how far out the strongest pixels look")
s=sub(s,"            let k = u * u.sqrt();\n            let pull = pull * (1.0 + 0.3 * corner);","            // the bend is steepest right at the edge and eases away quickly (u^1.75): like light\n            // wrapping around the rim of thick glass\n            let k = u * u.sqrt() * u.sqrt().sqrt();\n            let pull = pull * (1.0 + 0.2 * corner);")
# the outermost pixels are fully the bent picture (no dark fill showing through as a thin outline)
s=sub(s,"            let alpha = (0.88 + 0.1 * corner) * fade * smooth(-1.0, 2.5 * sc, d) * up;","""            let base = 0.88 + 0.1 * corner;
            let alpha = (base + (1.0 - base) * smooth(4.0 * sc, 0.0, d)) * fade * (d + 1.0).clamp(0.0, 1.0) * up;""")
# a thin bright bevel along the very edge, for the 3D look
s=sub(s,"            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;","""            let hl = if d < 5.0 * sc { 0.14 * smooth(-0.5, 1.2 * sc, d) * (1.0 - smooth(1.2 * sc, 5.0 * sc, d)) } else { 0.0 };
            let ch = |v: f32| ((v + (255.0 - v) * hl) * alpha).round().clamp(0.0, 255.0) as u32;""")
open(p,'w').write(s)

# the bottom edge melts further into what is behind
p='src/app.rs'; s=open(p).read()
s=sub(s,"let from = 1.0 - bh * 1.5 / body.h;","let from = 1.0 - bh * 1.7 / body.h;")
s=sub(s,"let drop = 0.66 * band_k * bd_k;","let drop = 0.8 * band_k * bd_k;")
open(p,'w').write(s)
PYEOF
echo "Patched v210"
