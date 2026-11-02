#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "deep_spans" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()

# 1. beyond the reach of the middle's bend nothing moves: those rows are copied in one go too
s=sub(s,"""        } else if t.mid_warp <= 0.0 {
            (sh.left + width, sh.right - width)
        } else {
            return (usize::MAX, 0);
        };""","""        } else if t.mid_warp <= 0.0 {
            (sh.left + width, sh.right - width)
        } else if db > w2 + 1.0 {
            // beyond the reach of the middle's bend nothing moves: the same straight copy
            (sh.left + w2 + 1.0, sh.right - w2 - 1.0)
        } else {
            return (usize::MAX, 0);
        };""")

# 2. the deep part of a blurred middle in two cheap passes instead of four samples per pixel
NEW = '''    // The deep part of a blurred middle: the picture behind, softened by four taps at +-b. There
    // nothing bends it and the glass is the same along a row, so the four taps come out of two cheap
    // passes (along, then down) in place of four full samples for every pixel. The result is the
    // same as the per-pixel way (to rounding); the pixels are written here and skipped later.
    let mut deep_spans: Vec<(usize, usize)> = Vec::new();
    if cell == 1 && st == 1 && mid && t.mid_blur * sc > 0.25 && w > 8 {
        let bl = t.mid_blur * sc;
        let dmin = (if t.mid_warp > 0.0 { (width * 1.6).max(w2) } else { width * 1.6 }) + 2.0;
        let mut spans = vec![(0usize, 0usize); h];
        let mut alphas = vec![0.0_f32; h];
        let (mut ca, mut cb2) = (usize::MAX, 0usize);
        let (mut ra, mut rb) = (usize::MAX, 0usize);
        for j in 0..h {
            let y = (y0 + j as i32) as f32 + 0.5;
            let db = sh.bottom - y;
            if db < dmin {
                continue;
            }
            let (xa, xb) = if y > cb {
                let sv = r - dmin;
                let dy = y - cb;
                if sv <= 0.0 || dy >= sv {
                    continue;
                }
                let sx = (sv * sv - dy * dy).sqrt();
                (cl - sx, cr + sx)
            } else {
                (sh.left + dmin, sh.right - dmin)
            };
            let a = ((xa - x0 as f32 - 0.5).ceil() + 1.0).max(0.0);
            let b = ((xb - x0 as f32 - 0.5).floor() - 1.0).min(w as f32 - 1.0);
            if b < a + 4.0 {
                continue;
            }
            let (a, b) = (a as usize, b as usize + 1);
            // the row must really look alike across the span: same glass at both ends and the middle
            let at = |i: usize| params(x0 as f32 + i as f32 + 0.5, y);
            let Some((al, ox, oy, bk)) = at((a + b) / 2) else { continue };
            let same = |p: Option<(f32, f32, f32, f32)>| {
                p.is_some_and(|(a2, x2, y2, b2)| a2 == al && x2 == 0.0 && y2 == 0.0 && (b2 - bl).abs() < 1e-3)
            };
            if ox != 0.0 || oy != 0.0 || !same(at((a + b) / 2)) || !same(at(a)) || !same(at(b - 1)) {
                continue;
            }
            spans[j] = (a, b);
            alphas[j] = al;
            ca = ca.min(a);
            cb2 = cb2.max(b);
            ra = ra.min(j);
            rb = rb.max(j);
        }
        if ra != usize::MAX && cb2 > ca {
            // two taps each way, at +b and -b, each blended between its two pixels
            let taps = |b: f32| -> [(i32, f32); 4] {
                let mut o = [(0, 0.0); 4];
                for (n, q) in [-b, b].into_iter().enumerate() {
                    let fl = q.floor();
                    let f = q - fl;
                    o[n * 2] = (fl as i32, 0.5 * (1.0 - f));
                    o[n * 2 + 1] = (fl as i32 + 1, 0.5 * f);
                }
                o
            };
            let k = taps(bl);
            let gy = |j: usize| y0 + j as i32 - shot.oy;
            let src_row = |j: usize, n: usize| (gy(j) + k[n].0).clamp(0, shot.h as i32 - 1) as usize;
            let (mut lo, mut hi) = (usize::MAX, 0usize);
            for j in ra..=rb {
                if spans[j].0 < spans[j].1 {
                    for n in 0..4 {
                        lo = lo.min(src_row(j, n));
                        hi = hi.max(src_row(j, n));
                    }
                }
            }
            let cols = cb2 - ca;
            let rows = hi - lo + 1;
            if rows * cols <= 3_000_000 {
                // along: each needed row of the picture, blurred sideways, for the columns wanted
                let mut hb = vec![[0.0_f32; 3]; rows * cols];
                let last = shot.w as i32 - 1;
                for (ri, row) in hb.chunks_mut(cols).enumerate() {
                    let src = &shot.px[(lo + ri) * shot.w..(lo + ri + 1) * shot.w];
                    for (ci, o) in row.iter_mut().enumerate() {
                        let gx = x0 + (ca + ci) as i32 - shot.ox;
                        let mut acc = [0.0_f32; 3];
                        for (off, wgt) in k {
                            let p = src[(gx + off).clamp(0, last) as usize];
                            acc[0] += wgt * ((p >> 16) & 255) as f32;
                            acc[1] += wgt * ((p >> 8) & 255) as f32;
                            acc[2] += wgt * (p & 255) as f32;
                        }
                        *o = acc;
                    }
                }
                // down: the four wanted rows blended, then the glass over it
                for j in ra..=rb {
                    let (a, b) = spans[j];
                    if a >= b {
                        continue;
                    }
                    let al = alphas[j];
                    let a24 = ((al * 255.0).round().clamp(0.0, 255.0) as u32) << 24;
                    let r4: [&[[f32; 3]]; 4] = std::array::from_fn(|n| {
                        let ri = src_row(j, n) - lo;
                        &hb[ri * cols..(ri + 1) * cols]
                    });
                    let out = &mut px[j * w..(j + 1) * w];
                    for i in a..b {
                        let c = i - ca;
                        let mut acc = [0.0_f32; 3];
                        for n in 0..4 {
                            let wgt = k[n].1;
                            let v = r4[n][c];
                            acc[0] += wgt * v[0];
                            acc[1] += wgt * v[1];
                            acc[2] += wgt * v[2];
                        }
                        let ch = |v: f32| (v * al).round().clamp(0.0, 255.0) as u32;
                        out[i] = a24 | (ch(acc[0]) << 16) | (ch(acc[1]) << 8) | ch(acc[2]);
                    }
                }
                deep_spans = spans;
            }
        }
    }
'''
anchor = "    // a band of `cell` rows of the picture\n"
s=sub(s,anchor,NEW+anchor)
s=sub(s,"""        let mut done = (usize::MAX, 0usize);
        if fast && rows == 1 && yc >= full_from {""","""        let mut done = (usize::MAX, 0usize);
        if cell == 1 && rows == 1 {
            if let Some(&(a, b)) = deep_spans.get(bj) {
                if a < b {
                    done = (a, b);
                }
            }
        }
        if done.0 == usize::MAX && fast && rows == 1 && yc >= full_from {""")
open(p,'w').write(s)
PYEOF
echo "Patched v238"
