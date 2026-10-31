#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "bd_move" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"    bd_prev: (f32, f32),\n","    bd_prev: (f32, f32),\n    /// When the notch last changed size.\n    bd_move: Instant,\n")
s=sub(s,"            bd_prev: (0.0, 0.0),\n","            bd_prev: (0.0, 0.0),\n            bd_move: Instant::now(),\n")
s=sub(s,"""        let bd_to = if moved > 1.0 { 0.0 } else { 1.0 };
        let bd_rate = if bd_to < self.bd_k { 30.0 } else { 9.0 };
        self.bd_k += (bd_to - self.bd_k) * (1.0 - (-bd_rate * rim_dt).exp());
        if (bd_to - self.bd_k).abs() < 0.01 {
            self.bd_k = bd_to;
        }
        if self.bd_k != bd_to {
            fx.animate = true;
        }""","""        // gone the very frame the size starts to change (a fade would still show the trailing
        // outline), and only back a moment after it holds still, as the layer needs a few frames
        // to catch up with the new size
        if moved > 0.25 {
            self.bd_move = now;
            self.bd_k = 0.0;
        }
        let still = now.saturating_duration_since(self.bd_move) > Duration::from_millis(140);
        let bd_to = if still { 1.0 } else { 0.0 };
        if bd_to > self.bd_k {
            self.bd_k += (bd_to - self.bd_k) * (1.0 - (-10.0 * rim_dt).exp());
        }
        if (bd_to - self.bd_k).abs() < 0.01 {
            self.bd_k = bd_to;
        }
        if self.bd_k != bd_to || !still {
            fx.animate = true;
        }""")
open(p,'w').write(s)
PYEOF
echo "Patched v200"
