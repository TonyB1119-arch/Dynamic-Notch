cd /workspaces/Notch
cat > /tmp/v163.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b):
    assert s.count(a)==1, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
s=sub(s,"        let blur = if ap.blur && (see_through || band_k > 0.01) && settled { a_glass } else { 0.0 };\n","""        let blur = if ap.blur && (see_through || band_k > 0.01) && settled { a_glass } else { 0.0 };
        // The frosted layer is dark and strong; under the see-through bottom edge it is only
        // blended in lightly, so the page behind still shows through softly.
        let blur = if band_k > 0.01 && !see_through { blur * 0.4 } else { blur };
""")
open(p,'w').write(s)
PYEOF
python3 /tmp/v163.py && echo patched
