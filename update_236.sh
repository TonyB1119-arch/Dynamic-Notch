#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "rim_cost" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/app.rs'; s=open(p).read()
# 1. page switch: the new page starts while the old one is still fading, so there is no blank beat
s=sub(s,"    const SPLIT: f32 = 0.45;\n","")
s=sub(s,"if overlap { (0.46, 0.38) } else { (SPLIT, SPLIT) };","if overlap { (0.46, 0.38) } else { (0.5, 0.22) };")
s=sub(s,"} else { smoothstep(0.0, 0.8, u) };","} else { smoothstep(0.0, 0.45, u) };")
# 2. the tail of a move runs at 30 frames a second in every mode (lighter mode used to drop to about 20)
s=sub(s," * if self.cfg.appearance.lite_motion { 22 } else { 15 } / 8"," * 15 / 8")
# 3. glass rebuild rate follows how long a rebuild really takes on this PC
s=sub(s,"    rim_t: Option<Instant>,\n","    rim_t: Option<Instant>,\n    /// How long a glass rebuild takes lately (smoothed); sets how often the moving glass is redone.\n    rim_cost: Duration,\n")
s=sub(s,"            rim_t: None,\n","            rim_t: None,\n            rim_cost: Duration::from_millis(12),\n")
s=sub(s,"< Duration::from_millis(45))","< (self.rim_cost * 3).clamp(Duration::from_millis(16), Duration::from_millis(45)))")
s=sub(s,"        if let Some(rim) = crate::refract::rim(shot, made, mstep) {\n","""        let t_made = Instant::now();
        let built = crate::refract::rim(shot, made, mstep);
        self.rim_cost = (self.rim_cost * 3 + t_made.elapsed()) / 4;
        if let Some(rim) = built {
""")
# 4. the moving glass keeps a little more detail along the edge
s=sub(s,"let moving_zone: f32 = if lite { 0.3 } else { 0.55 };","let moving_zone: f32 = if lite { 0.4 } else { 0.55 };")
open(p,'w').write(s)

# 5. glass text a touch clearer: the middle is softened less
p='src/refract.rs'; s=open(p).read()
s=sub(s,"            mid_blur: g.middle_blur.clamp(0.0, 6.0),\n","            mid_blur: g.middle_blur.clamp(0.0, 6.0) * 0.7,\n")
open(p,'w').write(s)
PYEOF
echo "Patched v236"
