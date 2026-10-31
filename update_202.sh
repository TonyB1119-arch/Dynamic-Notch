#!/bin/bash
set -e
cd "$(dirname "$0")"
if ! grep -q "band_fresh" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""    /// What the bottom strip picture was made from, and when. It is only made again when that
    /// changes or after 50 ms, not on every frame of a page that just sits there.
    band_sig: Option<BandSig>,
    band_at: Instant,
""","")
s=sub(s,"            band_sig: None,\n            band_at: Instant::now(),\n","")
s=sub(s,"\n/// Kind (page or peek), the page, the strip's rectangle in quarter pixels, a page change running.\ntype BandSig = (u8, ModuleId, i32, i32, i32, i32, bool);","")
a=s.index("    /// True while the strip picture made a moment ago")
b=s.index("    /// Alcove: the bottom edge of the open notch")
s=s[:a]+s[b:]
s=sub(s,"            self.band_since = None;\n            self.band_sig = None;\n","            self.band_since = None;\n",2)
s=sub(s,"""            let q = |v: f32| (v * scale * 4.0).round() as i32;
            if self.band_fresh((1, page, q(region.x), q(region.y), q(region.w), q(region.h), trans.is_some()), now) {
                return;
            }
""","")
s=sub(s,"""        if peek_band {
            let h = BAND_H.min(body.h * 0.4);
            let q = |v: f32| (v * scale * 4.0).round() as i32;
            if self.band_fresh((2, "peek", q(body.x), q(body.bottom() - h), q(body.w), q(h), false), now) {
                return;
            }
        }
""","")
open(p,'w').write(s)
PYEOF
echo "Patched v202"
