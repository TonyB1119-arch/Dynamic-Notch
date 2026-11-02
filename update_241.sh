#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "wave_k" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "fan_h" src/refract.rs; then echo "run update_239.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()
# the stretch turns left, straight, right, straight, left ... in waves all along the bottom
s=sub(s,"""    // the fan: how far the stretch turns sideways at the ends of the bottom edge (radians), and the
    // middle and half width it is measured from
    const FAN: f32 = 1.0;
    let (fan_c, fan_h) = ((sh.left + sh.right) * 0.5, ((sh.right - sh.left) * 0.5).max(1.0));
""","""    // the waves: how far the stretch turns sideways at the most (radians), where they start from,
    // and how fast they repeat. A wave is never shorter than where the picture would fold over
    // sideways (the turn changes by less than the stretch gives, per pixel).
    const FAN: f32 = 1.0;
    let fan_c = (sh.left + sh.right) * 0.5;
    let wave_k = std::f32::consts::TAU / (110.0 * sc).max(7.0 * stretch_cap.abs());
""")
s=sub(s,"""                // turn the direction sideways by more the further from the middle (and by less the
                // closer it already points sideways, so it still meets the straight sides)
                let s = ((x - fan_c) / fan_h).clamp(-1.0, 1.0);
                let (sn, cs) = (FAN * s.signum() * smooth(0.0, 1.0, s.abs()) * ny).sin_cos();
""","""                // turn the direction sideways, left then right then left again as x moves along (and
                // by less the closer it already points sideways, so it still meets the straight sides)
                let (sn, cs) = (FAN * ((x - fan_c) * wave_k).sin() * ny).sin_cos();
""")
open(p,'w').write(s)
PYEOF
echo "Patched v241"
