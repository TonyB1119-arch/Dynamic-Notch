#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "stretch_cap" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/refract.rs'; s=open(p).read()
# the middle warp bends the picture the old way (pulling in from outside), so it has no place in stretch mode
s=sub(s,"            mid_warp: g.middle_warp.clamp(0.0, 1.0),\n","            mid_warp: if g.stretch { 0.0 } else { g.middle_warp.clamp(0.0, 1.0) },\n")
# work out the stretch limit once per rim instead of once per pixel
s=sub(s,"    let even_bend = (t.bend - 1.5).abs() < 0.001;\n","""    let even_bend = (t.bend - 1.5).abs() < 0.001;
    // the most the picture may stretch before it would fold over (see the rim loop)
    let stretch_cap = -0.92 * width / if even_bend { 1.5 } else { t.bend.max(1.0) };
""")
s=sub(s,"""            if t.dir < 0.0 {
                // stretching only works while the picture keeps its order across the rim; past this
                // limit it folds over and the text reads backwards, so the stretch never folds
                let steepest = if even_bend { 1.5 } else { t.bend.max(1.0) };
                p = p.max(-0.92 * width / steepest);
            }
""","""            if t.dir < 0.0 {
                // stretching only works while the picture keeps its order across the rim; past this
                // limit it folds over and the text reads backwards, so the stretch never folds
                p = p.max(stretch_cap);
            }
""")
open(p,'w').write(s)
PYEOF
echo "Patched v235"
