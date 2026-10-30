#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b):
    assert s.count(a)==1,(s.count(a),a[:70]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"        let glass = ap.glass_look && crate::ui::look_alcove();\n","""        let glass = ap.glass_look && crate::ui::look_alcove();
        // the sheen and the lit edge step aside while a page is changing and come back as it lands
        let glass_k = match self.page_from {
            Some(_) => 1.0 - (self.anim.page.value.clamp(0.0, 1.0) * std::f32::consts::PI).sin(),
            None => 1.0,
        };
""")
s=sub(s,"                        let k = a_glass;\n","                        let k = a_glass * glass_k;\n")
s=sub(s,"if glass && body.h >= 56.0 && a_glass > 0.01 {","if glass && body.h >= 56.0 && a_glass * glass_k > 0.01 {")
s=sub(s,"Color::white(0.17 * a_glass));","Color::white(0.17 * a_glass * glass_k));")
s=sub(s,"Color::white(0.05 * a_glass));","Color::white(0.05 * a_glass * glass_k));")
open(p,'w').write(s)
PYEOF
echo "Patched v176"
