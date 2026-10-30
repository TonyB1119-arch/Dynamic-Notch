cd /workspaces/Notch
cat > /tmp/v161.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b):
    assert s.count(a)==1, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
# the strip no longer depends on the exact moment the springs stop (that last frame is the one that stays on screen)
s=sub(s,"            && (body.h - t.h).abs() < 3.0\n            && (body.w - tw).abs() < 3.0;","            && (body.h - t.h).abs() < 24.0;")
# keep frames coming until the fade-in has finished, also on its very first frame
s=sub(s,"                    if band_k > 0.01 && body.h > BAND_H * 2.0 {","                    if self.snap_band.is_some() && band_k < 1.0 {\n                        fx.animate = true;\n                    }\n                    if band_k > 0.01 && body.h > BAND_H * 2.0 {")
open(p,'w').write(s)
PYEOF
python3 /tmp/v161.py && echo patched
