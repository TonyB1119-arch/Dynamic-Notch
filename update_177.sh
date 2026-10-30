#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b):
    assert s.count(a)==1,(s.count(a),a[:70]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        // the sheen and the lit edge step aside while a page is changing and come back as it lands
        let glass_k = match self.page_from {
            Some(_) => 1.0 - (self.anim.page.value.clamp(0.0, 1.0) * std::f32::consts::PI).sin(),
            None => 1.0,
        };
""","""        // the sheen and the lit edge are not drawn at all while a page is changing, so a page
        // switch runs exactly the same with the glass look on or off
        let glass_k = if self.page_from.is_some() { 0.0 } else { 1.0 };
""")
s=sub(s,"                    if glass && body.h >= 56.0 {\n                        // a soft sheen","                    if glass && glass_k > 0.0 && body.h >= 56.0 {\n                        // a soft sheen")
s=sub(s,"if glass && body.h >= 56.0 && a_glass * glass_k > 0.01 {","if glass && glass_k > 0.0 && body.h >= 56.0 && a_glass > 0.01 {")
open(p,'w').write(s)
PYEOF
echo "Patched v177"
