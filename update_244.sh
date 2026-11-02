#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "glint_arc" src/alcove.rs; then echo "already applied"; exit 0; fi
if ! grep -q "fn icon_chip" src/alcove.rs; then echo "run update_243.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/alcove.rs'; s=open(p).read()
s=sub(s,"""    ui.p.arc(cx, cy, rad - 0.5, -1.15, 2.3, 1.4, Color::white(0.34 + 0.30 * h));
    ui.p.arc(cx, cy, rad - 0.5, std::f32::consts::PI - 0.6, 1.2, 1.2, Color::white(0.10 + 0.08 * h));
}
""","""    // two short white highlights, one at the top right and one at the bottom left, only at those
    // corners. Each is a few arcs laid over each other, shorter and shorter, so the light is
    // brightest in the middle of the highlight and fades out quickly toward both ends.
    let peak = 0.22 + 0.10 * h;
    for centre in [std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4 + std::f32::consts::PI] {
        for sweep in [1.2_f32, 0.9, 0.6, 0.3] {
            glint_arc(ui, cx, cy, rad - 0.5, centre - sweep * 0.5, sweep, peak);
        }
    }
}

/// One layer of a corner highlight (angles run clockwise from straight up).
fn glint_arc(ui: &mut Ui, cx: f32, cy: f32, r: f32, start: f32, sweep: f32, a: f32) {
    ui.p.arc(cx, cy, r, start, sweep, 1.5, Color::white(a));
}
""")
# the full hairline ring is fainter now, so the light shows only at the two corners
s=sub(s,"    ui.p.stroke_circle(cx, cy, rad - 0.5, 1.0, Color::white(0.10 + 0.10 * h));\n","    ui.p.stroke_circle(cx, cy, rad - 0.5, 1.0, Color::white(0.045 + 0.05 * h));\n")
open(p,'w').write(s)
PYEOF
echo "Patched v244"
