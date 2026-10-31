#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
p='src/app.rs'; s=open(p).read()
a0=s.index("                    // the lit edge: a fine bright line down the sides and around the bottom corners")
end="                    p.pop_clip();\n"
a1=s.index(end,a0)+len(end)
new='''                    // thick glass edge: a soft glow along the inside of the rim and a lit rim on top of
                    // it, both growing toward the bottom corners (drawn once the notch has settled)
                    let k = a_glass * glass_k;
                    let outer = Rect::new(body.x, body.y - a_r - 2.0, body.w, body.h + a_r + 2.0);
                    let ring = |r: Rect, radius: f32, width: f32| {
                        p.path(|b| {
                            b.rounded_rect(r, radius, true);
                            let inner = r.inset(width);
                            if inner.w > 0.0 && inner.h > 0.0 {
                                b.rounded_rect(inner, (radius - width).max(0.0), false);
                            }
                        })
                    };
                    p.push_clip(Rect::new(body.x - 1.0, body.y + a_ear * 0.8, body.w + 2.0, body.h));
                    if let Some(g) = ring(outer, a_r, 7.0) {
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[
                                (0.0, Color::white(0.0)),
                                (0.55, Color::white(0.0)),
                                (0.85, Color::white(0.07 * k)),
                                (1.0, Color::white(0.20 * k)),
                            ],
                        );
                    }
                    if let Some(g) = ring(outer, a_r, 1.5) {
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[
                                (0.0, Color::white(0.10 * k)),
                                (0.5, Color::white(0.16 * k)),
                                (1.0, Color::white(0.50 * k)),
                            ],
                        );
                    }
                    p.pop_clip();
'''
s=s[:a0]+new+s[a1:]
open(p,'w').write(s)
PYEOF
echo "Patched v186"
