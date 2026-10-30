#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)
p='src/alcove.rs'; s=open(p).read()
s=sub(s,"""    if name.is_empty() {
        (230.0, 112.0)
    } else {
        ((70.0 + name.chars().count() as f32 * 22.0).clamp(250.0, 440.0), 158.0)
    }""","""    // the ring on the left, "Hello" and the name beside it: wide rather than tall, so nothing
    // sits in the soft blur along the bottom edge
    if name.is_empty() {
        (290.0, 150.0)
    } else {
        ((180.0 + name.chars().count() as f32 * 13.0).clamp(310.0, 470.0), 150.0)
    }""")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"                    t.h = base_h + if name.is_empty() { 172.0 } else { 184.0 };\n","                    t.h = base_h + crate::alcove::hello_stage(name).1;\n")
s=sub(s,"""    let cx = body.cx();
    let cy = body.y + ui.cfg.appearance.notch_height + 66.0;
    let pop = smoothstep(0.0, 0.12, t);""","""    // the ring sits on the left, the words beside it
    let cx = body.x + 82.0;
    let cy = body.y + ui.cfg.appearance.notch_height + 66.0;
    let pop = smoothstep(0.0, 0.12, t);""")
a0=s.index('    // "Hello" first, the name a beat later\n    let hello = smoothstep(0.50, 0.62, t);')
a1=s.index("    ui.p.alpha = alpha;\n    if t < 0.95 {",a0)
new='''    // "Hello" first, the name a beat later, to the right of the ring
    let hello = smoothstep(0.50, 0.62, t);
    let x0 = body.x + 152.0;
    let tw = (body.right() - x0 - 22.0).max(40.0);
    let name = hello_name(&pk.title);
    let hy = if name.is_empty() { cy - 17.0 } else { cy - 38.0 };
    ui.p.alpha = alpha * hello * fade;
    ui.p.text(
        "Hello",
        Rect::new(x0 - 10.0 * (1.0 - hello), hy, tw, 34.0),
        &TextStyle::new(28.0, Color::white(0.97)).semibold().display(),
    );
    if !name.is_empty() {
        let nm = smoothstep(0.62, 0.74, t);
        ui.p.alpha = alpha * nm * fade;
        let mut size = 20.0;
        let (nw, _) = ui.p.measure(name, &TextStyle::new(size, Color::white(1.0)).semibold());
        size *= (tw / nw.max(1.0)).min(1.0);
        ui.p.text(
            name,
            Rect::new(x0 - 10.0 * (1.0 - nm), cy + 2.0, tw, 28.0),
            &TextStyle::new(size, Color::white(0.62)).semibold(),
        );
    }
'''
s=s[:a0]+new+s[a1:]
open(p,'w').write(s)
PYEOF
echo "Patched v184"
