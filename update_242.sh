#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "linear fan" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "const FAN" src/refract.rs; then echo "run update_239.sh first"; exit 1; fi
python3 - <<'PYEOF'
import re
p='src/refract.rs'; s=open(p).read()

new_a = """    // the linear fan: how far the stretch turns sideways at the very ends of the bottom edge
    // (radians), and the middle and half width it is measured from. Straight down in the middle,
    // turning more and more to the left on the left half and to the right on the right half.
    const FAN: f32 = 0.7;
    let (fan_c, fan_h) = ((sh.left + sh.right) * 0.5, ((sh.right - sh.left) * 0.5).max(1.0));
"""
new_b = """                // turn the direction sideways in step with the distance from the middle (and by less the
                // closer it already points sideways, so it still meets the straight sides)
                let s = ((x - fan_c) / fan_h).clamp(-1.0, 1.0);
                let (sn, cs) = (FAN * s * ny).sin_cos();
"""
# find the current fan/wave blocks by their first and last lines and swap them
a0 = s.index("    // the fan: how far") if "    // the fan: how far" in s else s.index("    // the waves: how far")
mark = "wave_k =" if "wave_k =" in s[a0:a0 + 900] else "fan_c"
a1 = s.index("\n", s.index(mark, a0)) + 1
s = s[:a0] + new_a + s[a1:]
b0 = s.index("                // turn the direction sideways")
b1 = s.index("\n", s.index("sin_cos();", b0)) + 1
s = s[:b0] + new_b + s[b1:]
assert s.count("const FAN") == 1 and "wave_k" not in s
open(p,'w').write(s)
PYEOF
echo "Patched v242"
