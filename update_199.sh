#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "bh \* 1.5" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""                        let from = 1.0 - bh / body.h;
                        let low = bg_alpha * (1.0 - 0.62 * band_k * bd_k);
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[(0.0, Color::black(bg_alpha)), (from, Color::black(bg_alpha)), (1.0, Color::black(low))],
                        );""","""                        // it starts higher up and follows a gentle S, so there is no point where the
                        // black visibly begins to thin out
                        let from = 1.0 - bh * 1.5 / body.h;
                        let drop = 0.66 * band_k * bd_k;
                        let at = |t: f32| {
                            let e = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
                            Color::black(bg_alpha * (1.0 - drop * e))
                        };
                        let span = 1.0 - from;
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[
                                (0.0, Color::black(bg_alpha)),
                                (from, Color::black(bg_alpha)),
                                (from + span * 0.25, at(0.25)),
                                (from + span * 0.5, at(0.5)),
                                (from + span * 0.75, at(0.75)),
                                (1.0, at(1.0)),
                            ],
                        );""")
open(p,'w').write(s)
p='src/refract.rs'; s=open(p).read()
s=sub(s,"let width = 64.0 * sc;","let width = 76.0 * sc;")
s=sub(s,"let pull = 20.0 * sc;","let pull = 22.0 * sc;")
s=sub(s,"let fade = u * u * (0.5 + 0.5 * u);","let fade = u * u * (1.5 - 0.5 * u);")
s=sub(s,"let alpha = (0.8 + 0.15 * corner) * fade","let alpha = (0.88 + 0.1 * corner) * fade")
open(p,'w').write(s)
PYEOF
echo "Patched v199"
