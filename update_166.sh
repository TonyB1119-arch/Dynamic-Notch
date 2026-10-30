cd /workspaces/Notch
cat > /tmp/v166.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
# thinner slices: each step in strength is tiny, so no bands show on bright content like the camera
s=sub(s,"    const N: usize = 14;\n    let band = snap.rect;","    const N: usize = 40;\n    let band = snap.rect;")
open(p,'w').write(s)
PYEOF
python3 /tmp/v166.py && echo patched
