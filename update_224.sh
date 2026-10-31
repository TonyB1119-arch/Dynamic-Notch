#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "let row_span" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "st == 1 && ox == 0.0" src/refract.rs; then echo "run update_219.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()

# ---- 1. one pixel = (what the glass does there) + (the colour that gives) ----
a=s.index("    let shade = |x: f32, y: f32| -> u32 {")
b=s.index("        let (rc, gc, bc) = if blur > 0.25 {")
seg=s[a:b]
assert seg.count("return 0;")==3, seg.count("return 0;")
seg=seg.replace("    let shade = |x: f32, y: f32| -> u32 {","    let params = |x: f32, y: f32| -> Option<(f32, f32, f32, f32)> {").replace("return 0;","return None;")
seg+="""        Some((alpha, ox, oy, blur))
    };
    let shade = |x: f32, y: f32| -> u32 {
        let Some((alpha, ox, oy, blur)) = params(x, y) else {
            return 0;
        };
"""
s=s[:a]+seg+s[b:]

# ---- 2. rows whose pixels all come out alike are filled in one go ----
a=s.index("    // a band of `cell` rows of the picture\n    let band_fn")
b=s.index("    // a big picture is split into bands of rows")
new='''    // Whole stretches of a row come out the same: the straight middle of the bottom edge, and the
    // inside of the notch. They are worked out once for the row instead of once per pixel (the
    // result is the same pixel for pixel). Only for the full-detail picture (one row per band) when
    // the middle is sharp.
    let fast = cell == 1 && st == 1 && t.mid_blur <= 0.0;
    // the columns [a, b) of the row at window height `y` that all look alike (empty when none)
    let row_span = |y: f32| -> (usize, usize) {
        let db = sh.bottom - y;
        let (mut xa, mut xb) = if db < width {
            (sh.left + db, sh.right - db)
        } else if t.mid_warp <= 0.0 {
            (sh.left + width, sh.right - width)
        } else {
            return (usize::MAX, 0);
        };
        if y > cb {
            xa = xa.max(cl);
            xb = xb.min(cr);
        }
        // a pixel's x is x0 + its column + 0.5; stay a pixel inside the limits
        let a = ((xa - x0 as f32 - 0.5).ceil() + 1.0).max(0.0);
        let b = ((xb - x0 as f32 - 0.5).floor() - 1.0).min(w as f32 - 1.0);
        if b < a {
            (usize::MAX, 0)
        } else {
            (a as usize, b as usize + 1)
        }
    };
    // fill the columns [a, b) of that row; false when this row needs the per-pixel way after all
    let fill_span = |y: f32, a: usize, b: usize, out: &mut [u32]| -> bool {
        let xr = x0 as f32 + ((a + b) / 2) as f32 + 0.5;
        let Some((alpha, ox, oy, blur)) = params(xr, y) else {
            return true; // nothing there, and the row is still empty
        };
        if ox != 0.0 || blur > 0.25 {
            return false;
        }
        let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
        let a24 = ((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24;
        let col0 = x0 - shot.ox;
        let last = shot.w as i32 - 1;
        let hmax = shot.h as i32 - 1;
        if oy == 0.0 {
            // nothing bends it: the pixel behind is copied as it is
            let ri = (y - shot.oy as f32).floor().clamp(0.0, shot.h as f32 - 1.0) as usize;
            let row = &shot.px[ri * shot.w..(ri + 1) * shot.w];
            for (k, o) in out[a..b].iter_mut().enumerate() {
                let p = row[(col0 + (a + k) as i32).clamp(0, last) as usize];
                *o = a24 | (ch(((p >> 16) & 255) as f32) << 16) | (ch(((p >> 8) & 255) as f32) << 8) | ch((p & 255) as f32);
            }
        } else {
            // pulled straight up or down: the same two rows of the screen for the whole stretch,
            // one pair for each of red, green and blue
            let src = |o: f32| -> (usize, usize, f32) {
                let gy = y + o - shot.oy as f32 - 0.5;
                let fl = gy.floor();
                ((fl as i32).clamp(0, hmax) as usize, (fl as i32 + 1).clamp(0, hmax) as usize, gy - fl)
            };
            let chans = [(src(oy), 16u32), (src(oy * (1.0 - t.fringe)), 8), (src(oy * (1.0 - 2.0 * t.fringe)), 0)];
            for (k, o) in out[a..b].iter_mut().enumerate() {
                let xi = (col0 + (a + k) as i32).clamp(0, last) as usize;
                let mut v = a24;
                for ((r0, r1, ty), sft) in chans {
                    let lo = ((shot.px[r0 * shot.w + xi] >> sft) & 255) as f32;
                    let hi = ((shot.px[r1 * shot.w + xi] >> sft) & 255) as f32;
                    v |= ch(lo + (hi - lo) * ty) << sft;
                }
                *o = v;
            }
        }
        true
    };
    // a band of `cell` rows of the picture
    let band_fn = |bj: usize, out: &mut [u32]| {
        let rows = out.len() / w;
        let j0 = bj * cell;
        let yc = y0 as f32 + (j0 * st) as f32 + (rows * st) as f32 * 0.5;
        let up = smooth(0.0, 0.5, (yc - top) / reach);
        if up <= 0.0 && !(mid && yc >= int_top) {
            return;
        }
        let cw = w.div_ceil(cell);
        let (a_end, b_start) = if yc >= full_from { (w, w) } else { (side.min(w), w.saturating_sub(side).max(side.min(w))) };
        let mut done = (usize::MAX, 0usize);
        if fast && rows == 1 && yc >= full_from {
            let (a, b) = row_span(yc);
            if a < b && fill_span(yc, a, b, out) {
                done = (a, b);
            }
        }
        for ci in (0..a_end.div_ceil(cell)).chain(b_start / cell..cw) {
            let i0 = ci * cell;
            if i0 >= done.0 && i0 < done.1 {
                continue;
            }
            let cols = cell.min(w - i0);
            let xc = x0 as f32 + (i0 * st) as f32 + (cols * st) as f32 * 0.5;
            if cell == 1 || geom(xc, yc).0 < fine_d {
                for rr in 0..rows {
                    for cc in 0..cols {
                        let x = (x0 + ((i0 + cc) * st) as i32) as f32 + half;
                        let y = (y0 + ((j0 + rr) * st) as i32) as f32 + half;
                        out[rr * w + i0 + cc] = shade(x, y);
                    }
                }
            } else {
                let v = shade(xc, yc);
                if v != 0 {
                    for rr in 0..rows {
                        for cc in 0..cols {
                            out[rr * w + i0 + cc] = v;
                        }
                    }
                }
            }
        }
    };
'''
s=s[:a]+new+s[b:]
open(p,'w').write(s)
PYEOF
echo "Patched v224"
