#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "bd_k" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"    rim_at: Instant,\n","    rim_at: Instant,\n    /// How much of the frosted layer behind the notch is let through (0 while the notch changes\n    /// size, because that layer is moved by Windows a moment later and would trail the edge) and\n    /// the body size at the last frame.\n    bd_k: f32,\n    bd_prev: (f32, f32),\n")
s=sub(s,"            rim_at: Instant::now(),\n","            rim_at: Instant::now(),\n            bd_k: 1.0,\n            bd_prev: (0.0, 0.0),\n")
s=sub(s,"""        if self.rim_k != rim_to {
            fx.animate = true;
        }
""","""        if self.rim_k != rim_to {
            fx.animate = true;
        }
        // while the notch changes size the frosted layer behind it would trail the edge as a second,
        // fainter outline: it is let go quickly, the bottom edge turns solid with it, and both come
        // back smoothly once the size holds still
        let moved = (body.w - self.bd_prev.0).abs() + (body.h - self.bd_prev.1).abs();
        self.bd_prev = (body.w, body.h);
        let bd_to = if moved > 1.0 { 0.0 } else { 1.0 };
        let bd_rate = if bd_to < self.bd_k { 30.0 } else { 9.0 };
        self.bd_k += (bd_to - self.bd_k) * (1.0 - (-bd_rate * rim_dt).exp());
        if (bd_to - self.bd_k).abs() < 0.01 {
            self.bd_k = bd_to;
        }
        if self.bd_k != bd_to {
            fx.animate = true;
        }
        let bd_k = self.bd_k;
""")
s=sub(s,"let low = bg_alpha * (1.0 - 0.74 * band_k);","let low = bg_alpha * (1.0 - 0.62 * band_k * bd_k);")
s=sub(s,"self.host.set_backdrop(bp, a_r * s, blur * self.anim.shown.value);","self.host.set_backdrop(bp, a_r * s, blur * self.anim.shown.value * bd_k);")
open(p,'w').write(s)

p='src/refract.rs'; s=open(p).read()
s=sub(s,"""            let d;
            let nx;
            let ny;
            if y > cb""","""            let d;
            let nx;
            let ny;
            let mut corner = 0.0;
            if y > cb""")
s=sub(s,"""                d = r - dist;
                nx = dx / dist;
                ny = dy / dist;
            } else {""","""                d = r - dist;
                nx = dx / dist;
                ny = dy / dist;
                corner = 1.0;
            } else {""")
s=sub(s,"""            // u: 1 at the edge, 0 at the inner end of the rim. The strength follows a smooth S, so
            // it starts and ends gently, with no visible line where the rim stops.
            let u = 1.0 - d / width;
            let alpha = 0.6 * u * u * (3.0 - 2.0 * u) * (d + 1.0).clamp(0.0, 1.0) * up;""","""            // u: 1 at the edge, 0 at the inner end of the rim. The glass is strongest right at the
            // edge, and strongest of all in the corners, then eases away quickly so the black of the
            // notch takes over; the tail is smooth, with no visible line where the rim stops.
            let u = 1.0 - d / width;
            let fade = u * u * (0.5 + 0.5 * u);
            let alpha = (0.8 + 0.15 * corner) * fade * (d + 1.0).clamp(0.0, 1.0) * up;""")
s=sub(s,"let (ox, oy) = (nx * pull * k, ny * pull * k);","let pull = pull * (1.0 + 0.3 * corner);\n            let (ox, oy) = (nx * pull * k, ny * pull * k);")
open(p,'w').write(s)
PYEOF
echo "Patched v198"
