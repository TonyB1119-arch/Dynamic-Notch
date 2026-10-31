#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "rim_geo" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---- refract.rs: the rim can be made on a coarser grid (every `step`-th pixel) while moving
p='src/refract.rs'; s=open(p).read()
s=sub(s,"""    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Where the notch is""","""    pub w: usize,
    pub h: usize,
    /// Each picture pixel stands for `step` x `step` window pixels (1 = pixel for pixel).
    pub step: usize,
    pub px: Vec<u32>,
}

/// Where the notch is""")
s=sub(s,"pub fn rim(shot: &Shot, sh: Shape) -> Option<Rim> {","""///
/// `step` 1 gives the finest picture; 2 makes it on every other pixel each way (a quarter of the
/// work, drawn enlarged): the rim is a soft gradient, so it looks the same while it is moving.
pub fn rim(shot: &Shot, sh: Shape, step: usize) -> Option<Rim> {
    let st = step.max(1);""")
s=sub(s,"""    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut px = vec![0u32; w * h];""","""    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let (w, h) = (w.div_ceil(st), h.div_ceil(st));
    let half = st as f32 * 0.5;
    let mut px = vec![0u32; w * h];""")
s=sub(s,"    let side = (width.max(r) + 2.0).ceil() as usize;","    let side = ((width.max(r) + 2.0).ceil() as usize).div_ceil(st);")
s=sub(s,"        let y = (y0 + j as i32) as f32 + 0.5;","        let y = (y0 + (j * st) as i32) as f32 + half;")
s=sub(s,"            let x = (x0 + i as i32) as f32 + 0.5;","            let x = (x0 + (i * st) as i32) as f32 + half;")
s=sub(s,"    Some(Rim { x0, y0, w, h, px })","    Some(Rim { x0, y0, w, h, step: st, px })")
open(p,'w').write(s)

# ---- gfx: the picture covers `step` times its size; enlarged smoothly
p='src/gfx/mod.rs'; s=open(p).read()
s=sub(s,"""    bmp: ID2D1Bitmap1,
    pub rect: Rect,
}""","""    bmp: ID2D1Bitmap1,
    pub rect: Rect,
    step: usize,
}""")
s=sub(s,"pub fn rim_pic(&self, x0: i32, y0: i32, w: usize, h: usize, px: &[u32], scale: f32) -> Option<RimPic> {","pub fn rim_pic(&self, x0: i32, y0: i32, w: usize, h: usize, step: usize, px: &[u32], scale: f32) -> Option<RimPic> {")
s=sub(s,"            Some(RimPic { bmp, rect: Rect::new(x0 as f32 / scale, y0 as f32 / scale, w as f32 / scale, h as f32 / scale) })","""            let k = step.max(1) as f32;
            Some(RimPic {
                bmp,
                rect: Rect::new(x0 as f32 / scale, y0 as f32 / scale, w as f32 * k / scale, h as f32 * k / scale),
                step,
            })""")
s=sub(s,"""            self.dc.DrawBitmap(&b, Some(&rf(r.rect)), a, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);""","""            let mode = if r.step > 1 { D2D1_INTERPOLATION_MODE_LINEAR } else { D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR };
            self.dc.DrawBitmap(&b, Some(&rf(r.rect)), a, mode, None, None);""")
open(p,'w').write(s)

# ---- app.rs: coarse while the size changes, fine once it has settled
p='src/app.rs'; s=open(p).read()
s=sub(s,"type RimKey = (i32, i32, i32, i32, i32);","type RimKey = (i32, i32, i32, i32, i32, usize);")
s=sub(s,"    refr_rim: Option<(RimKey, gfx::RimPic)>,\n","    refr_rim: Option<(RimKey, gfx::RimPic)>,\n    /// The notch's shape at the previous frame (to tell whether it is still changing size).\n    rim_geo: Option<(i32, i32, i32, i32, i32)>,\n")
s=sub(s,"            refr_rim: None,\n","            refr_rim: None,\n            rim_geo: None,\n")
s=sub(s,"""        let key: RimKey = (q4(body.x), q4(body.w), q4(body.bottom()), q4(r), (sc * 100.0).round() as i32);
        if self.refr_rim""","""        let geo = (q4(body.x), q4(body.w), q4(body.bottom()), q4(r), (sc * 100.0).round() as i32);
        // while the size is still changing a coarse rim is enough (a quarter of the work); the
        // frame after it has stopped makes the fine one
        let step = if self.rim_geo == Some(geo) { 1 } else { 2 };
        self.rim_geo = Some(geo);
        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, step);
        if self.refr_rim""")
s=sub(s,"""        if let Some(rim) = crate::refract::rim(shot, shape) {
            if let Some(pic) = self.res.rim_pic(rim.x0, rim.y0, rim.w, rim.h, &rim.px, sc) {
                self.refr_rim = Some((key, pic));
            }
        }
        self.rim_ok = self.refr_rim.is_some();""","""        if let Some(rim) = crate::refract::rim(shot, shape, step) {
            if let Some(pic) = self.res.rim_pic(rim.x0, rim.y0, rim.w, rim.h, rim.step, &rim.px, sc) {
                self.refr_rim = Some((key, pic));
            }
        }
        self.rim_ok = self.refr_rim.is_some();
        if step > 1 && self.rim_ok {
            // one more frame so the fine picture follows as soon as the notch has stopped
            self.request_frame();
        }""")
open(p,'w').write(s)
PYEOF
echo "Patched v203"
