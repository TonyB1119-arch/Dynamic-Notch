#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "dashed_rounded" src/gfx/mod.rs; then echo "already applied"; exit 0; fi
if ! grep -q "arc_cached" src/gfx/mod.rs; then echo "run update_245.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
def rw(p,f):
    s=open(p).read(); s=f(s); open(p,'w').write(s)

DASH = """
/// The dashes of a rounded-rectangle outline as little quads (`half` = half the line width). The
/// dash count is rounded so the pattern closes evenly instead of ending in a stub.
fn dash_quads(w: f32, h: f32, rad: f32, dash: f32, gap: f32, half: f32) -> Vec<[(f32, f32); 4]> {
    let rad = rad.min(w * 0.5).min(h * 0.5).max(0.0);
    let (a, b) = (w - 2.0 * rad, h - 2.0 * rad);
    let arc = std::f32::consts::FRAC_PI_2 * rad;
    let total = 2.0 * (a + b) + 4.0 * arc;
    if total < dash + gap {
        return Vec::new();
    }
    let n = (total / (dash + gap)).round().max(1.0) as usize;
    let step = total / n as f32;
    let segs = [(a, 0), (arc, 1), (b, 2), (arc, 3), (a, 4), (arc, 5), (b, 6), (arc, 7)];
    let at = |s: f32| -> (f32, f32) {
        let mut s = s.rem_euclid(total);
        for (len, k) in segs {
            if s <= len {
                let t = if rad > 0.0 { s / rad } else { 0.0 };
                let pi = std::f32::consts::PI;
                let on = |cx: f32, cy: f32, t: f32| (cx + rad * t.sin(), cy - rad * t.cos());
                return match k {
                    0 => (rad + s, 0.0),
                    1 => on(w - rad, rad, t),
                    2 => (w, rad + s),
                    3 => on(w - rad, h - rad, pi * 0.5 + t),
                    4 => (w - rad - s, h),
                    5 => on(rad, h - rad, pi + t),
                    6 => (0.0, h - rad - s),
                    _ => on(rad, rad, pi * 1.5 + t),
                };
            }
            s -= len;
        }
        (rad, 0.0)
    };
    (0..n)
        .map(|i| {
            let s0 = i as f32 * step + (step - dash) * 0.5;
            let (p0, p1) = (at(s0), at(s0 + dash));
            let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
            let l = (dx * dx + dy * dy).sqrt().max(1e-4);
            let (nx, ny) = (-dy / l * half, dx / l * half);
            [(p0.0 + nx, p0.1 + ny), (p1.0 + nx, p1.1 + ny), (p1.0 - nx, p1.1 - ny), (p0.0 - nx, p0.1 - ny)]
        })
        .collect()
}
"""

def gfx(s):
    s=sub(s,"""    /// Ring of `width` centered on a circle of `radius`.
""","""    /// A dashed outline in the style of a drop zone: short straight dashes along a rounded rectangle.
    /// Built once per size (all dashes in one cached shape), so a frame costs a single draw.
    pub fn dashed_rounded(&self, r: Rect, radius: f32, width: f32, c: Color) {
        if c.a * self.alpha <= 0.001 || r.w <= 8.0 || r.h <= 8.0 {
            return;
        }
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5);
        let key = (32, q(r.w), q(r.h), q(rad), q(width));
        self.fill_cached(key, r.x, r.y, c, |b| {
            for d in dash_quads(uq(key.1), uq(key.2), uq(key.3), 7.0, 6.0, uq(key.4) * 0.5) {
                b.move_to(d[0].0, d[0].1);
                b.line_to(d[1].0, d[1].1);
                b.line_to(d[2].0, d[2].1);
                b.line_to(d[3].0, d[3].1);
                b.close();
            }
        });
    }

    /// Ring of `width` centered on a circle of `radius`.
""")
    s=sub(s,"pub type RKey = (u8, i32, i32, i32, i32);\n","pub type RKey = (u8, i32, i32, i32, i32);\n"+DASH)
    return s
rw('src/gfx/mod.rs',gfx)

def sh(s):
    s=sub(s,"""            ui.p.fill_rounded(zone, 14.0, accent.with_a(0.08));
            ui.p.stroke_rounded(zone.inset(0.75), 14.0, 1.5, accent.with_a(0.7));
""","""            // monochrome look: a quiet white fill and small white dashes instead of a solid coloured line
            let tint = if crate::ui::mono_icons() { Color::white(1.0) } else { accent };
            ui.p.fill_rounded(zone, 14.0, tint.with_a(0.05));
            ui.p.dashed_rounded(zone.inset(1.5), 12.5, 1.6, tint.with_a(0.75));
""")
    s=sub(s,"""            ui.p.fill_rounded(below, 26.0, palette::YELLOW.with_a(0.10));
            ui.p.stroke_rounded(below.inset(0.75), 26.0, 1.5, palette::YELLOW.with_a(0.7));
            ui.p.text("Drop to park files here", below, &TextStyle::new(15.0, palette::YELLOW).semibold().center());
""","""            let tint = if crate::ui::mono_icons() { Color::white(1.0) } else { palette::YELLOW };
            ui.p.fill_rounded(below, 26.0, tint.with_a(0.05));
            ui.p.dashed_rounded(below.inset(1.5), 24.5, 1.6, tint.with_a(0.75));
            ui.p.text("Drop to park files here", below, &TextStyle::new(15.0, tint).semibold().center());
""")
    return s
rw('src/modules/shelf.rs',sh)
PYEOF
echo "Patched v247"
