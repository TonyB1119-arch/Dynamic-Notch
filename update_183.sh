#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
# shared helper
s=sub(s,"fn draw_device_dial(ui: &mut Ui, body: Rect, pk: &Peek) {","""/// `s` cut short with an ellipsis so that it fits `w` wide in style `st`.
fn fit_text(ui: &Ui, mut s: String, st: &TextStyle, w: f32) -> String {
    if ui.p.measure(&s, st).0 <= w {
        return s;
    }
    while s.chars().count() > 1 {
        s.pop();
        let t = format!("{}\\u{2026}", s.trim_end());
        if ui.p.measure(&t, st).0 <= w {
            return t;
        }
    }
    s
}

fn draw_device_dial(ui: &mut Ui, body: Rect, pk: &Peek) {""")
# device dial: icon and name inside the ring
s=sub(s,"    let ic = Rect::centered(cx, cy, 40.0, 40.0);\n","    let ic = Rect::centered(cx, cy - 13.0, 26.0, 26.0);\n")
s=sub(s,"""        ui.p.line(ic.x + 4.0, ic.y + 4.0, ic.right() - 4.0, ic.bottom() - 4.0, 4.6, Color::black(0.95));
        ui.p.line(ic.x + 4.0, ic.y + 4.0, ic.right() - 4.0, ic.bottom() - 4.0, 2.2, tint);
    }
    ui.p.text(
        &pk.title,
        Rect::new(body.x + 10.0, cy + 58.0, body.w - 20.0, 28.0),
        &TextStyle::new(20.0, Color::white(0.95)).semibold().center(),
    );
    ui.p.text(
        &pk.subtitle,
        Rect::new(body.x + 10.0, cy + 88.0, body.w - 20.0, 20.0),
        &TextStyle::new(13.0, palette::TEXT_DIM).medium().center(),
    );
}""","""        ui.p.line(ic.x + 3.0, ic.y + 3.0, ic.right() - 3.0, ic.bottom() - 3.0, 3.6, Color::black(0.95));
        ui.p.line(ic.x + 3.0, ic.y + 3.0, ic.right() - 3.0, ic.bottom() - 3.0, 1.8, tint);
    }
    // the name sits inside the ring, clear of the soft blur along the bottom edge; whether the
    // device is connected is shown by the ring and the slash
    let r = Rect::new(cx - 38.0, cy + 3.0, 76.0, 22.0);
    let st = TextStyle::new(12.0, Color::white(0.95)).semibold().center();
    let name = fit_text(ui, pk.title.to_string(), &st, r.w);
    ui.p.text(&name, r, &st);
}""")
# use the helper in the HUD too
s=sub(s,"        let mut big = match text {","        let big = match text {")
a0=s.index("        while big.chars().count() > 3 && ui.p.measure(&big, &st).0 > r.w {")
a1=s.index("        ui.p.text(&big, r, &st);\n",a0)+len("        ui.p.text(&big, r, &st);\n")
s=s[:a0]+"        let big = fit_text(ui, big, &st, r.w);\n        ui.p.text(&big, r, &st);\n"+s[a1:]
open(p,'w').write(s)
PYEOF
echo "Patched v183"
