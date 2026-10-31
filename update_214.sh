#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "middle_blur" src/config.rs; then echo "already applied"; exit 0; fi
if ! grep -q "middle_warp" src/config.rs; then echo "run update_213.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- config.rs ----------------
p='src/config.rs'; s=open(p).read()
s=sub(s,"    pub middle_warp: f32,\n","    pub middle_warp: f32,\n    /// How much the middle glass softens what is behind it, in logical pixels (0 = sharp).\n    pub middle_blur: f32,\n")
s=sub(s,"            middle_warp: 0.5,\n","            middle_warp: 0.5,\n            middle_blur: 1.5,\n")
s=sub(s,"        self.middle_warp = self.middle_warp.clamp(0.0, 1.0);\n","        self.middle_warp = self.middle_warp.clamp(0.0, 1.0);\n        self.middle_blur = self.middle_blur.clamp(0.0, 6.0);\n")
s=sub(s,"middle_warp = 0.5\nfade_amount","middle_warp = 0.5\nmiddle_blur = 1.5\nfade_amount")
open(p,'w').write(s)

# ---------------- refract.rs ----------------
p='src/refract.rs'; s=open(p).read()
s=sub(s,"    pub mid_warp: f32,\n}","    pub mid_warp: f32,\n    pub mid_blur: f32,\n}")
s=sub(s,"            mid_warp: g.middle_warp.clamp(0.0, 1.0),\n","            mid_warp: g.middle_warp.clamp(0.0, 1.0),\n            mid_blur: g.middle_blur.clamp(0.0, 6.0),\n")
# one pixel, all three colours (for the soft middle)
s=sub(s,"/// Work out the rim along the sides","""/// The picture at a fractional position as red, green and blue, blended between the four nearest
/// pixels (coordinates are clamped to the edge).
fn sample_rgb(s: &Shot, fx: f32, fy: f32) -> [f32; 3] {
    let (gx, gy) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |x: i32, y: i32| -> u32 {
        let xi = x.clamp(0, s.w as i32 - 1) as usize;
        let yi = y.clamp(0, s.h as i32 - 1) as usize;
        s.px[yi * s.w + xi]
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut out = [0.0_f32; 3];
    for (n, shift) in [16u32, 8, 0].into_iter().enumerate() {
        let g = |p: u32| ((p >> shift) & 255) as f32;
        let top = g(a) + (g(b) - g(a)) * tx;
        let bot = g(c) + (g(d) - g(c)) * tx;
        out[n] = top + (bot - top) * ty;
    }
    out
}

/// Work out the rim along the sides""")
s=sub(s,"    let band = sh.band.max(1.0);\n    let row =","""    let band = sh.band.max(1.0);
    // deeper in, the outline the middle follows is rounder and rounder (bigger circles at the corners)
    let rb = (r + 70.0 * sc).min((sh.right - sh.left) * 0.5);
    let (cl2, cr2, cb2) = (sh.left + rb, sh.right - rb, sh.bottom - rb);
    let row =""")
a=s.index("            if a_row > 0.0 {\n                // the middle lies under the rim")
endm="            out[i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);\n"
b=s.index(endm,a)+len(endm)
new='''            let mut blur = 0.0_f32;
            if a_row > 0.0 {
                // the middle lies under the rim; it bends gently toward the nearest edges
                let ai = a_row * (d + 1.0).clamp(0.0, 1.0);
                alpha += ai * (1.0 - alpha);
                if d > width {
                    let dm;
                    let dirx;
                    let diry;
                    if y > cb2 && (x < cl2 || x > cr2) {
                        // around a corner: along circles, with the outline getting rounder inward
                        let cx = if x < cl2 { cl2 } else { cr2 };
                        let (dx, dy) = (x - cx, y - cb2);
                        let dist = (dx * dx + dy * dy).sqrt().max(0.001);
                        dm = d + ((rb - dist) - d) * smooth(0.0, width, d);
                        dirx = dx / dist;
                        diry = dy / dist;
                    } else {
                        // a soft blend of the nearest edges' directions: no crease where they meet
                        let wl = 1.0 / (x - sh.left + 6.0).max(1.0).powi(3);
                        let wr = 1.0 / (sh.right - x + 6.0).max(1.0).powi(3);
                        let wb = 1.0 / (sh.bottom - y + 6.0).max(1.0).powi(3);
                        let inv = 1.0 / (wl + wr + wb);
                        dm = d;
                        dirx = (wr - wl) * inv;
                        diry = wb * inv;
                    }
                    if t.mid_warp > 0.0 {
                        let wi = smooth(width, width * 1.8, d);
                        let u2 = (1.0 - dm / w2).clamp(0.0, 1.0);
                        let k2 = if even_bend { u2 * u2.sqrt() } else { u2.powf(t.bend) };
                        let m = pull * t.mid_warp * k2 * wi;
                        ox += dirx * m;
                        oy += diry * m;
                    }
                }
                // the middle is a little soft; the sharp rim keeps its fine detail
                blur = t.mid_blur * sc * smooth(width * 0.6, width * 1.6, d);
            }
            if alpha < 0.004 {
                continue;
            }
            let (cr, cg, cb2) = if blur > 0.25 {
                let (bx, by) = (x + ox, y + oy);
                let mut acc = [0.0_f32; 3];
                for (sx, sy) in [(-blur, -blur), (blur, -blur), (-blur, blur), (blur, blur)] {
                    let c = sample_rgb(shot, bx + sx, by + sy);
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                }
                (acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25)
            } else {
                // red, green and blue look a slightly different distance out, like light through glass
                (
                    sample(shot, x + ox, y + oy, 16),
                    sample(shot, x + ox * (1.0 - t.fringe), y + oy * (1.0 - t.fringe), 8),
                    sample(shot, x + ox * (1.0 - 2.0 * t.fringe), y + oy * (1.0 - 2.0 * t.fringe), 0),
                )
            };
            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
            out[i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);
'''
s=s[:a]+new+s[b:]
open(p,'w').write(s)

# ---------------- settings.rs ----------------
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"        _ => 17.0,","        _ => 18.0,")
s=sub(s,"(&str, f32, (f32, f32), String, &str); 16]","(&str, f32, (f32, f32), String, &str); 17]")
s=sub(s,"""            ("Bottom fade amount", g.fade_amount""","""            ("Middle blur", g.middle_blur, (0.0, 6.0), format!("{:.1}", g.middle_blur), "middle_blur"),
            ("Bottom fade amount", g.fade_amount""")
s=sub(s,"let b = row_rect(body, 16);","let b = row_rect(body, 17);")
s=sub(s,"                (\"middle_warp\", d.middle_warp),\n","                (\"middle_warp\", d.middle_warp),\n                (\"middle_blur\", d.middle_blur),\n")
open(p,'w').write(s)
PYEOF
echo "Patched v214"
