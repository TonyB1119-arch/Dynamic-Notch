#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "pub stretch: bool" src/config.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/config.rs'; s=open(p).read()
s=sub(s,"""    pub sharp_zone: f32,
}
""","""    pub sharp_zone: f32,
    /// Stretch the glass edge like iOS Liquid Glass: the picture under the glass is pulled out toward
    /// the edge (it can fold over right at the rim) instead of the picture outside being pulled in.
    pub stretch: bool,
}
""")
s=sub(s,"            sharp_zone: 4.0,\n        }","            sharp_zone: 4.0,\n            stretch: false,\n        }")
s=sub(s,"sharp_zone = 4.0\n","sharp_zone = 4.0\n# true = the glass edge stretches the picture under it toward the edge (iOS Liquid Glass look);\n# false = it pulls in the picture from outside the notch (the original look)\nstretch = false\n")
open(p,'w').write(s)

p='src/refract.rs'; s=open(p).read()
s=sub(s,"    pub sharp: f32,\n}\n","    pub sharp: f32,\n    /// 1 pulls the picture from outside the notch in; -1 stretches the picture under it outward.\n    pub dir: f32,\n}\n")
s=sub(s,"            sharp: g.sharp_zone.clamp(0.3, 4.0),\n","            sharp: g.sharp_zone.clamp(0.3, 4.0),\n            dir: if g.stretch { -1.0 } else { 1.0 },\n")
s=sub(s,"            let p = pull * (1.0 + t.corner * corner);\n","            let p = pull * t.dir * (1.0 + t.corner * corner);\n")
open(p,'w').write(s)

p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"""        let b = row_rect(body, 18);
        let style = ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM);""","""        toggle_row(ui, row_rect(body, 18), "Stretch edge (iOS look)", g.stretch, "glass", "stretch");
        let b = row_rect(body, 19);
        let style = ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM);""")
s=sub(s,"        _ => 19.0,\n","        _ => 20.0,\n")
open(p,'w').write(s)
PYEOF
echo "Patched v233"
