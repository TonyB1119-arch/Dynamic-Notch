#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b):
    assert s.count(a)==1,(s.count(a),a[:70]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""    let slide = if al { 28.0 } else { SLIDE };
    if t < SPLIT {
        let u = t / SPLIT;""","""    let slide = if al { 28.0 } else { SLIDE };
    // Alcove, with the pictures ready: the new page starts rising while the old one is still
    // fading, so there is never an empty moment between the two
    let overlap = al && snaps.is_some();
    let (old_end, new_start) = if overlap { (0.52, 0.26) } else { (SPLIT, SPLIT) };
    if t < old_end {
        let u = t / old_end;""")
s=sub(s,"""    } else {
        let u = (t - SPLIT) / (1.0 - SPLIT);""","""    }
    if t >= new_start {
        let u = (t - new_start) / (1.0 - new_start);""")
open(p,'w').write(s)
PYEOF
echo "Patched v174"
