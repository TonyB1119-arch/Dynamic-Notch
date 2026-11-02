#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fn par_rows" src/refract.rs; then echo "already applied"; exit 0; fi
if ! grep -q "deep_spans" src/refract.rs; then echo "run update_238.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()
# the two passes over the deep blurred middle run side by side on a big picture
s=sub(s,'''                let mut hb = vec![[0.0_f32; 3]; rows * cols];
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
''','''                let mut hb = vec![[0.0_f32; 3]; rows * cols];
                let last = shot.w as i32 - 1;
                let threads = if rows * cols >= PAR_MIN / 2 { workers() } else { 1 };
                par_rows(&mut hb, cols, threads, |ri0, chunk| {
                    for (dr, row) in chunk.chunks_mut(cols).enumerate() {
                        let ri = ri0 + dr;
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
                });
''')
s=sub(s,'''                // down: the four wanted rows blended, then the glass over it
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
''','''                // down: the four wanted rows blended, then the glass over it
                par_rows(&mut px, w, threads, |j0, chunk| {
                    for (dj, out) in chunk.chunks_mut(w).enumerate() {
                        let j = j0 + dj;
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
                });
''')
anchor="/// Pictures smaller than this are worked out on the one thread"
s=sub(s,anchor,'''
/// Run `f(first_row, rows)` on pieces of `data` (rows of `width` items) side by side; one piece on
/// this thread when `threads` is 1.
fn par_rows<T: Send>(data: &mut [T], width: usize, threads: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = data.len() / width.max(1);
    if threads <= 1 || rows < 2 {
        f(0, data);
        return;
    }
    let per = rows.div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        for (n, chunk) in data.chunks_mut(per * width).enumerate() {
            scope.spawn(move || f(n * per, chunk));
        }
    });
}
'''.lstrip("\n")+"\n"+anchor)
open(p,'w').write(s)
PYEOF
echo "Patched v240"
