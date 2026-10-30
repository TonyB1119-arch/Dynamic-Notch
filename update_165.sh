cd /workspaces/Notch
cat > /tmp/v165.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
s=sub(s,"fn bottom_fade(ui: &mut Ui, snap: &Snapshot, bottom: f32, radius: f32, k: f32) {","fn bottom_fade(ui: &mut Ui, snap: &Snapshot, bottom: f32, radius: f32, k: f32, scale: f32) {")
s=sub(s,"""    let step = band.h / N as f32;
    let ease =""","""    let step = band.h / N as f32;
    // slice edges sit exactly on device pixels, so neighbouring slices meet without a seam
    let px = |v: f32| (v * scale).round() / scale;
    let ease =""")
s=sub(s,"""        let y0 = (band.y + step * i as f32).round();
        let y1 = if i + 1 == N { band.bottom() } else { (band.y + step * (i + 1) as f32).round() };""","""        let y0 = px(band.y + step * i as f32);
        let y1 = if i + 1 == N { band.bottom() } else { px(band.y + step * (i + 1) as f32) };""")
s=sub(s,"""        let r = Rect::new(band.x + inset, y0, band.w - inset * 2.0, y1 - y0);""","""        let (x0, x1) = (px(band.x + inset), px(band.right() - inset));
        let r = Rect::new(x0, y0, x1 - x0, y1 - y0);""")
s=sub(s,"bottom_fade(&mut ui, b, body.bottom(), a_r, band_k);","bottom_fade(&mut ui, b, body.bottom(), a_r, band_k, self.scale);",2)
open(p,'w').write(s)
PYEOF
python3 /tmp/v165.py && echo patched
