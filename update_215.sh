#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "let hybrid = st == 1" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "middle_blur" src/config.rs; then echo "run update_214.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- refract.rs ----------------
p='src/refract.rs'; s=open(p).read()
a=s.index("    let row = |j: usize, out: &mut [u32]| {")
endm="    Some(Rim { x0, y0, w, h, step: st, px })\n}\n\n/// How many threads"
b=s.index(endm,a)
new='''    // Step 1 is sharp where it matters and half detail elsewhere: the glass bends the screen
    // hardest right along the edge, where fine text shows. Deeper in the rim is faint and the
    // middle is soft anyway, so each 2 x 2 block there is worked out once.
    let hybrid = st == 1;
    let cell = if hybrid { 2_usize } else { 1 };
    let fine_d = if hybrid { width * 0.55 } else { f32::MAX };
    // distance inside the edge, the direction pointing out of the notch, and the corner weight
    let geom = |x: f32, y: f32| -> (f32, f32, f32, f32) {
        if y > cb && (x < cl || x > cr) {
            let cx = if x < cl { cl } else { cr };
            let (dx, dy) = (x - cx, y - cb);
            let dist = (dx * dx + dy * dy).sqrt().max(0.001);
            // corner: 0 where the curve meets the straight edges, 1 in the middle of the curve (no seam)
            (r - dist, dx / dist, dy / dist, (2.0 * dx.abs() * dy / (dist * dist)).clamp(0.0, 1.0))
        } else {
            let (dl, dr, db) = (x - sh.left, sh.right - x, sh.bottom - y);
            if db <= dl && db <= dr {
                (db, 0.0, 1.0, 0.0)
            } else if dl <= dr {
                (dl, -1.0, 0.0, 0.0)
            } else {
                (dr, 1.0, 0.0, 0.0)
            }
        }
    };
    // one pixel of the picture (0 = nothing there)
    let shade = |x: f32, y: f32| -> u32 {
        let up = smooth(0.0, 0.5, (y - top) / reach);
        let in_mid = mid && y >= int_top;
        if up <= 0.0 && !in_mid {
            return 0;
        }
        // how much of the middle shows on this row: the black thins toward the bottom edge
        let a_row = if in_mid {
            let bt = ((y - (sh.bottom - sh.band)) / band).clamp(0.0, 1.0);
            let e = bt * bt * bt * (bt * (bt * 6.0 - 15.0) + 10.0);
            (sh.fade * e).max(sh.clear) * t.mid
        } else {
            0.0
        };
        let (d, nx, ny, corner) = geom(x, y);
        if d <= -1.0 {
            return 0;
        }
        let (mut alpha, mut ox, mut oy) = (0.0_f32, 0.0_f32, 0.0_f32);
        if d < width && up > 0.0 {
            // u: 1 at the edge, 0 at the inner end of the rim. The glass is strongest right at the
            // edge, and strongest of all in the corners, then eases away quickly so the black of the
            // notch takes over; the tail is smooth, with no visible line where the rim stops.
            let u = 1.0 - d / width;
            let fade = u * u * (1.5 - 0.5 * u);
            let base = (t.strength + 0.1 * corner).min(1.0);
            alpha = (base + (1.0 - base) * smooth(soft, 0.0, d)) * fade * (d + 1.0).clamp(0.0, 1.0) * up;
            let k = if even_bend { u * u.sqrt() } else { u.powf(t.bend) };
            let p = pull * (1.0 + t.corner * corner);
            ox = nx * p * k;
            oy = ny * p * k;
        }
        let mut blur = 0.0_f32;
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
            return 0;
        }
        let (rc, gc, bc) = if blur > 0.25 {
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
        (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(rc) << 16) | (ch(gc) << 8) | ch(bc)
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
        for ci in (0..a_end.div_ceil(cell)).chain(b_start / cell..cw) {
            let i0 = ci * cell;
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
    // a big picture is split into bands of rows worked out side by side; small ones are not worth
    // the hand-over
    let bands = h.div_ceil(cell);
    let threads = if w * h >= 60_000 { workers() } else { 1 };
    if threads > 1 {
        let per = bands.div_ceil(threads);
        let f = &band_fn;
        std::thread::scope(|scope| {
            for (n, chunk) in px.chunks_mut(per * cell * w).enumerate() {
                scope.spawn(move || {
                    for (jj, out) in chunk.chunks_mut(cell * w).enumerate() {
                        f(n * per + jj, out);
                    }
                });
            }
        });
    } else {
        for (bj, out) in px.chunks_mut(cell * w).enumerate() {
            band_fn(bj, out);
        }
    }
'''
s=s[:a]+new+s[b:]
s=sub(s,"    let made = rim(&shot, shape, 2);","    let made = rim(&shot, shape, 1);")
open(p,'w').write(s)

# ---------------- app.rs ----------------
p='src/app.rs'; s=open(p).read()
s=sub(s,"        let fine = if self.live_armed() { 2 } else { 1 };","        let fine = 1;")
s=sub(s,"        let step = if still || late { fine } else { 2 };","        // (the picture is sharp along the edge and half detail elsewhere, cheap enough to use always)\n        let step = fine;\n        let _ = late;")
open(p,'w').write(s)
PYEOF
echo "Patched v215"
