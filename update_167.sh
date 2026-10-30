cd /workspaces/Notch
cat > /tmp/v167.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
# one slice per screen pixel row: the fade is then as smooth as the screen can show
s=sub(s,"    const N: usize = 40;\n    let band = snap.rect;","    let band = snap.rect;\n    let n = ((band.h * scale).round() as usize).clamp(8, 160);")
s=sub(s,"    let step = band.h / N as f32;","    let step = band.h / n as f32;")
s=sub(s,"    for i in 0..N {\n        let u = (i as f32 + 0.5) / N as f32;","    for i in 0..n {\n        let u = (i as f32 + 0.5) / n as f32;")
s=sub(s,"let y1 = if i + 1 == N {","let y1 = if i + 1 == n {")
open(p,'w').write(s)
PYEOF
python3 /tmp/v167.py && echo patched
