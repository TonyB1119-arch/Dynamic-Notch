#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)

# ---------------- gfx/mod.rs
p='src/gfx/mod.rs'; s=open(p).read()
s=sub(s,"""    pub rect: Rect,
    pub key: u64,
    pub born: std::time::Instant,
}
""","""    pub rect: Rect,
    pub key: u64,
    pub born: std::time::Instant,
    /// The pictures behind the brushes (full size, then half, quarter, eighth) and the size in
    /// device pixels, so a strip that is redrawn often reuses them instead of allocating again.
    bufs: Vec<ID2D1Bitmap1>,
    px: (u32, u32),
}
""")
s=sub(s,"""        key: u64,
        draw: impl FnOnce(&mut Painter),
    ) -> Option<Snapshot> {""","""        key: u64,
        reuse: Option<Snapshot>,
        deep: bool,
        draw: impl FnOnce(&mut Painter),
    ) -> Option<Snapshot> {""")
# fast path + helper
s=sub(s,"""        unsafe {
            let bmp = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props).ok()?;
            dc.SetTarget(&bmp);""","""        let ident = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };
        // one halving step: `src` averaged down into `dst`
        let down = |src: &ID2D1Bitmap1, dst: &ID2D1Bitmap1, nw: u32, nh: u32| -> bool {
            let Ok(from) = src.cast::<ID2D1Bitmap>() else { return false };
            unsafe {
                dc.SetTarget(dst);
                dc.SetTransform(&ident);
                dc.BeginDraw();
                dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
                let d = D2D_RECT_F { left: 0.0, top: 0.0, right: nw as f32, bottom: nh as f32 };
                dc.DrawBitmap(&from, Some(&d), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
                let ok = dc.EndDraw(None, None).is_ok();
                dc.SetTarget(None);
                ok
            }
        };
        unsafe {
            // the same size as last time: draw into the pictures that already exist
            if let Some(mut old) = reuse.filter(|r| r.px == (w, h) && r.bufs.len() == 4) {
                dc.SetTarget(&old.bufs[0]);
                dc.BeginDraw();
                {
                    let mut p = Painter::new(dc.clone(), gfx, &mut *self, scale, -x0, -y0);
                    p.clear();
                    draw(&mut p);
                }
                let ok = dc.EndDraw(None, None).is_ok();
                dc.SetTarget(None);
                if !ok {
                    return None;
                }
                let (mut sw, mut sh) = (w, h);
                for i in 1..4 {
                    let (nw, nh) = (sw.div_ceil(2).max(1), sh.div_ceil(2).max(1));
                    if !down(&old.bufs[i - 1], &old.bufs[i], nw, nh) {
                        return None;
                    }
                    sw = nw;
                    sh = nh;
                }
                old.rect = Rect::new(x0 / scale, y0 / scale, w as f32 / scale, h as f32 / scale);
                old.key = key;
                old.born = std::time::Instant::now();
                return Some(old);
            }
            let bmp = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props).ok()?;
            dc.SetTarget(&bmp);""")
# new level building
a0=s.index("            // Halve the picture three times (each step averages 2x2 pixels) and keep every step:")
a1=s.index("                born: std::time::Instant::now(),\n            })\n        }\n    }\n}\n", a0)
a1_end=a1+len("                born: std::time::Instant::now(),\n            })\n        }\n    }\n}\n")
new='''            // Halve the picture three times (each step averages 2x2 pixels). Stretched back up with a
            // smooth filter each step reads as a blur, a stronger one the smaller it is, for a
            // fraction of a real blur's cost. A page picture only ever needs the quarter-size step
            // (`deep` keeps all three, for the bottom strip); the others are let go straight away.
            let smooth = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            };
            let mut bufs: Vec<ID2D1Bitmap1> = Vec::new();
            let mut brushes: [Option<ID2D1BitmapBrush1>; 3] = [None, None, None];
            let built = (|| -> Option<()> {
                let mut src: ID2D1Bitmap1 = bmp.clone();
                let (mut sw, mut sh) = (w, h);
                if deep {
                    bufs.push(bmp.clone());
                }
                for (i, slot) in brushes.iter_mut().enumerate() {
                    let (nw, nh) = (sw.div_ceil(2).max(1), sh.div_ceil(2).max(1));
                    let small = dc.CreateBitmap(D2D_SIZE_U { width: nw, height: nh }, None, 0, &props).ok()?;
                    if !down(&src, &small, nw, nh) {
                        return None;
                    }
                    if deep || i == 1 {
                        *slot = Some(dc.CreateBitmapBrush(&small, Some(&smooth), None).ok()?);
                    }
                    if deep {
                        bufs.push(small.clone());
                    }
                    src = small;
                    sw = nw;
                    sh = nh;
                }
                Some(())
            })();
            if built.is_none() {
                bufs.clear();
                brushes = [None, None, None];
            }
            let [half, soft, far] = brushes;
            Some(Snapshot {
                brush,
                soft,
                half,
                far,
                rect: Rect::new(x0 / scale, y0 / scale, w as f32 / scale, h as f32 / scale),
                key,
                born: std::time::Instant::now(),
                bufs,
                px: (w, h),
            })
        }
    }
}
'''
s=s[:a0]+new+s[a1_end:]
open(p,'w').write(s)

# ---------------- app.rs
p='src/app.rs'; s=open(p).read()
s=sub(s,"self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4D, |p| {","self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4D, self.snap_band.take(), true, |p| {")
s=sub(s,"self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4E, |p| {","self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4E, self.snap_band.take(), true, |p| {")
s=sub(s,"self.snap_all = self.res.render_snapshot(&self.gfx, scale, region, key, |p| {","self.snap_all = self.res.render_snapshot(&self.gfx, scale, region, key, None, false, |p| {")
s=sub(s,"let snap = self.res.render_snapshot(&self.gfx, scale, region, key, |p| {","let snap = self.res.render_snapshot(&self.gfx, scale, region, key, None, false, |p| {")
# lit edge: only while the size is not changing (a realized shape per size would pile up mid-resize)
s=sub(s,"if glass && glass_k > 0.0 && body.h >= 56.0 && a_glass > 0.01 {","if glass && glass_k > 0.0 && settled && body.h >= 56.0 && a_glass > 0.01 {")
open(p,'w').write(s)
PYEOF
echo "Patched v179"
