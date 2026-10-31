#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "rim_far" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---- refract.rs: the same picture, a bit faster (no edge clamping when all four pixels are inside)
p='src/refract.rs'; s=open(p).read()
s=sub(s,"""    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));""","""    let (a, b, c, d) = if x0 >= 0 && y0 >= 0 && (x0 as usize) + 1 < s.w && (y0 as usize) + 1 < s.h {
        // the usual case: all four pixels are inside the copy
        let i = y0 as usize * s.w + x0 as usize;
        let g = |p: u32| ((p >> shift) & 255) as f32;
        (g(s.px[i]), g(s.px[i + 1]), g(s.px[i + s.w]), g(s.px[i + s.w + 1]))
    } else {
        (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1))
    };""")
open(p,'w').write(s)

# ---- app.rs: the rim turns sharp during the last third of a move, not only after it
p='src/app.rs'; s=open(p).read()
s=sub(s,"    rim_geo: Option<(i32, i32, i32, i32, i32)>,\n","""    rim_geo: Option<(i32, i32, i32, i32, i32)>,
    /// How far (in logical px of width + height) the notch had to go when the current move began.
    rim_far: f32,
""")
s=sub(s,"            rim_geo: None,\n","            rim_geo: None,\n            rim_far: 0.0,\n")
s=sub(s,"""        let step = if self.rim_geo == Some(geo) { fine } else { 2 };
        self.rim_geo = Some(geo);""","""        let still = self.rim_geo == Some(geo);
        // how far the notch still has to go, and how far it had to go when this move began: in the
        // last third of a real move the rim is already made at full detail, so it is sharp by the
        // time the notch arrives (the work is spread over the slow end of the move)
        let left = (self.anim.w.target - self.anim.w.value).abs() + (self.anim.h.target - self.anim.h.value).abs();
        self.rim_far = if still { 0.0 } else { self.rim_far.max(left) };
        let late = self.rim_far >= 24.0 && left <= self.rim_far * 0.34;
        let step = if still || late { fine } else { 2 };
        self.rim_geo = Some(geo);""")
open(p,'w').write(s)
PYEOF
echo "Patched v208"
