#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "shift: u32" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
import re
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/refract.rs'; s=open(p).read()

a0=s.index("/// One pixel of the picture, as red, green, blue")
a1=s.index("/// Work out the rim along the sides")
s=s[:a0]+'''/// One colour (`shift` 16 red, 8 green, 0 blue) of the picture at a fractional position, smoothly
/// blended between the four nearest pixels (coordinates are clamped to the edge).
fn sample(s: &Shot, fx: f32, fy: f32, shift: u32) -> f32 {
    let (gx, gy) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |x: i32, y: i32| -> f32 {
        let xi = x.clamp(0, s.w as i32 - 1) as usize;
        let yi = y.clamp(0, s.h as i32 - 1) as usize;
        ((s.px[yi * s.w + xi] >> shift) & 255) as f32
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

'''+s[a1:]

s,n=re.subn(r"(?:    // The pull must stay[^\n]*\n    // itself[^\n]*\n)?    let width = [^\n]*\n    let pull = [^\n]*\n",
"""    // The rim fades out over a long, soft stretch so it melts into the black of the notch.
    let width = 64.0 * sc; // how far in from the edge the rim reaches
    let pull = 28.0 * sc; // how far out the strongest pixels look
""",s)
assert n==1,n
s=sub(s,"let reach = r + 60.0 * sc;","let reach = r + 90.0 * sc;")

a0=s.index("    let (cl, cr, cb) = (sh.left + r, sh.right - r, sh.bottom - r);")
a1=s.index("    Some(Rim { x0, y0, w, h, px })")
s=s[:a0]+'''    let (cl, cr, cb) = (sh.left + r, sh.right - r, sh.bottom - r);
    let top = sh.bottom - reach;
    // Rows higher up only carry the rim along the two sides, so the middle is skipped there;
    // from `full_from` down the whole row is rim. The corners reach further in than the sides.
    let full_from = sh.bottom - width - 1.0;
    let side = (width.max(r) + 2.0).ceil() as usize;
    for j in 0..h {
        let y = (y0 + j as i32) as f32 + 0.5;
        let up = smooth(0.0, 0.5, (y - top) / reach);
        if up <= 0.0 {
            continue;
        }
        let (a_end, b_start) = if y >= full_from { (w, w) } else { (side.min(w), w.saturating_sub(side).max(side.min(w))) };
        for i in (0..a_end).chain(b_start..w) {
            let x = (x0 + i as i32) as f32 + 0.5;
            // distance inside the edge, and the direction pointing out of the notch
            let d;
            let nx;
            let ny;
            if y > cb && (x < cl || x > cr) {
                let cx = if x < cl { cl } else { cr };
                let (dx, dy) = (x - cx, y - cb);
                let dist = (dx * dx + dy * dy).sqrt().max(0.001);
                d = r - dist;
                nx = dx / dist;
                ny = dy / dist;
            } else {
                let (dl, dr, db) = (x - sh.left, sh.right - x, sh.bottom - y);
                if db <= dl && db <= dr {
                    d = db;
                    nx = 0.0;
                    ny = 1.0;
                } else if dl <= dr {
                    d = dl;
                    nx = -1.0;
                    ny = 0.0;
                } else {
                    d = dr;
                    nx = 1.0;
                    ny = 0.0;
                }
            }
            if d <= -1.0 || d >= width {
                continue;
            }
            // u: 1 at the edge, 0 at the inner end of the rim. The strength follows a smooth S, so
            // it starts and ends gently, with no visible line where the rim stops.
            let u = 1.0 - d / width;
            let alpha = 0.96 * u * u * (3.0 - 2.0 * u) * (d + 1.0).clamp(0.0, 1.0) * up;
            if alpha < 0.004 {
                continue;
            }
            let k = u * u.sqrt();
            let (ox, oy) = (nx * pull * k, ny * pull * k);
            // red, green and blue look a slightly different distance out, like light through glass
            let cr = sample(shot, x + ox, y + oy, 16);
            let cg = sample(shot, x + ox * 0.93, y + oy * 0.93, 8);
            let cb2 = sample(shot, x + ox * 0.86, y + oy * 0.86, 0);
            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
            px[j * w + i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);
        }
    }
'''+s[a1:]
open(p,'w').write(s)
s=open(p).read()
s=sub(s,"""//! Nothing here runs per frame. A snapshot of the screen is taken once when the notch opens (while
//! the window is still small, so it does not photograph itself); once the notch has settled the rim
//! is worked out on the CPU and uploaded as one small picture, drawn with a single call.""","""//! A snapshot of the screen is taken once when the notch opens (while the window is still small, so
//! it does not photograph itself). The rim is worked out from it on the CPU for the notch's current
//! size and uploaded as one small picture, drawn with a single call; it is only made again when the
//! size changes (while a page change resizes the notch, and never while nothing moves).""")
open(p,'w').write(s)
PYEOF
echo "Patched v196"
