#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
# glow ring: wider and brighter
s=sub(s,"if let Some(g) = ring(outer, a_r, 7.0) {","if let Some(g) = ring(outer, a_r, 12.0) {")
s=sub(s,"""                                (0.55, Color::white(0.0)),
                                (0.85, Color::white(0.07 * k)),
                                (1.0, Color::white(0.20 * k)),""","""                                (0.45, Color::white(0.0)),
                                (0.80, Color::white(0.14 * k)),
                                (1.0, Color::white(0.40 * k)),""")
# lit rim: thicker and much brighter
s=sub(s,"if let Some(g) = ring(outer, a_r, 1.5) {","if let Some(g) = ring(outer, a_r, 2.0) {")
s=sub(s,"""                                (0.0, Color::white(0.10 * k)),
                                (0.5, Color::white(0.16 * k)),
                                (1.0, Color::white(0.50 * k)),""","""                                (0.0, Color::white(0.22 * k)),
                                (0.5, Color::white(0.34 * k)),
                                (1.0, Color::white(0.90 * k)),""")
# sheen: a stronger top highlight and bottom glow
s=sub(s,"""                                (0.0, Color::white(0.10 * k)),
                                (0.3, Color::white(0.0)),
                                (0.72, Color::white(0.0)),
                                (1.0, Color::white(0.09 * k)),""","""                                (0.0, Color::white(0.17 * k)),
                                (0.3, Color::white(0.0)),
                                (0.72, Color::white(0.0)),
                                (1.0, Color::white(0.16 * k)),""")
open(p,'w').write(s)
PYEOF
echo "Patched v187"
