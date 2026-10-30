cd /workspaces/Notch
cat > /tmp/v170.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
s=sub(s,"""        ui.p.snapshot_level(snap, lo as u8, fade);
        if mix > 0.01 {""","""        // two layers laid over each other must add up to exactly `fade` of coverage, with the upper
        // one carrying `mix` of the colour; otherwise a step shows each time the pair moves on
        let under = fade * (1.0 - mix) / (1.0 - mix * fade).max(0.05);
        ui.p.snapshot_level(snap, lo as u8, under);
        if mix > 0.01 {""")
open(p,'w').write(s)
PYEOF
python3 /tmp/v170.py && echo patched
