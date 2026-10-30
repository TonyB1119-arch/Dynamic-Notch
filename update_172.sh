cd /workspaces/Notch
cat > /tmp/v172.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
# volume and brightness: the speaker / sun sits in the upper half of the ring, the value in the lower half
s=sub(s,"""    let ir =
        if dial { Rect::centered(body.cx(), dial_cy, 40.0, 40.0) } else { Rect::new(body.x + pad, row_y, 24.0, 24.0) };""","""    let inside = dial && pk.key != Some("battery");
    let ir = if inside {
        Rect::centered(body.cx(), dial_cy - 13.0, 26.0, 26.0)
    } else if dial {
        Rect::centered(body.cx(), dial_cy, 40.0, 40.0)
    } else {
        Rect::new(body.x + pad, row_y, 24.0, 24.0)
    };""")
s=sub(s,"""        let label = Rect::new(body.x + 8.0, dial_cy + 60.0, body.w - 16.0, 30.0);
        // the big line, and a small one under it""","""        if inside {
            // the value sits inside the ring, clear of the soft blur along the bottom edge
            let big = text.map_or_else(|| format!("{:.0}%", frac.clamp(0.0, 1.0) * 100.0), str::to_string);
            let r = Rect::new(body.cx() - 36.0, dial_cy + 3.0, 72.0, 22.0);
            if hint {
                ui.p.text("Switch output", r, &TextStyle::new(10.5, palette::TEXT_DIM).medium().center());
            } else {
                ui.p.text(&big, r, &TextStyle::new(17.0, Color::white(0.95)).semibold().display().center());
            }
            return;
        }
        let label = Rect::new(body.x + 8.0, dial_cy + 60.0, body.w - 16.0, 30.0);
        // the big line, and a small one under it""")
# the notice no longer has a line of text under the ring
s=sub(s,'''        Some("osd-volume" | "osd-brightness") if cfg.general.hud_ring => Some(172.0),''','''        Some("osd-volume" | "osd-brightness") if cfg.general.hud_ring => Some(150.0),''')
open(p,'w').write(s)
PYEOF
python3 /tmp/v172.py && echo patched
