cd /workspaces/Notch
cat > /tmp/v168.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/gfx/mod.rs'; s=rd(p)
s=sub(s,"""                dc.CreateBitmapBrush(&src, Some(&bprops), None).ok()
            })();""","""                // stretched back up with a smooth (cubic) filter, so the blur has no blocky edges
                let smooth = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                };
                dc.CreateBitmapBrush(&src, Some(&smooth), None).ok()
            })();""")
open(p,'w').write(s)
PYEOF
python3 /tmp/v168.py && echo patched
