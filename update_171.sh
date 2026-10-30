cd /workspaces/Notch
cat > /tmp/v171.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/gfx/mod.rs'; s=rd(p)
# shrink with the wide, anti-aliased filter (not a plain 2x2 average), so the soft copies have no blocky edges
s=sub(s,"                    dc.DrawBitmap(&from, Some(&d), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);\n                    let ok = dc.EndDraw(None, None).is_ok();\n                    dc.SetTarget(None);\n                    if !ok {\n                        return None;\n                    }\n                    src = small;\n                    sw = nw;\n                    sh = nh;\n                    out.push(","                    dc.DrawBitmap(&from, Some(&d), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);\n                    let ok = dc.EndDraw(None, None).is_ok();\n                    dc.SetTarget(None);\n                    if !ok {\n                        return None;\n                    }\n                    src = small;\n                    sw = nw;\n                    sh = nh;\n                    out.push(")
open(p,'w').write(s)
PYEOF
python3 /tmp/v171.py && echo patched
