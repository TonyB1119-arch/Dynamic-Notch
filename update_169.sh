cd /workspaces/Notch
cat > /tmp/v169.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)

# ---- gfx: a ladder of blur levels (half, quarter, eighth size), each stretched back with a smooth filter
p='src/gfx/mod.rs'; s=rd(p)
s=sub(s,"    soft: Option<ID2D1BitmapBrush1>,\n","    soft: Option<ID2D1BitmapBrush1>,\n    /// The same picture at half and at an eighth of the size (lighter and heavier blur).\n    half: Option<ID2D1BitmapBrush1>,\n    far: Option<ID2D1BitmapBrush1>,\n")
a=s.index("            // Halve the picture twice")
b=s.index("            Some(Snapshot {\n                brush,")
s=s[:a]+'''            // Halve the picture three times (each step averages 2x2 pixels) and keep every step:
            // half, quarter and eighth size. Stretched back up with a smooth filter each one reads
            // as a blur, a stronger one the smaller it is, for a fraction of a real blur's cost.
            let levels = (|| -> Option<Vec<ID2D1BitmapBrush1>> {
                let mut src: ID2D1Bitmap1 = bmp.clone();
                let (mut sw, mut sh) = (w, h);
                let ident = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 };
                let smooth = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                };
                let mut out = Vec::with_capacity(3);
                for _ in 0..3 {
                    let (nw, nh) = (sw.div_ceil(2).max(1), sh.div_ceil(2).max(1));
                    let small = dc.CreateBitmap(D2D_SIZE_U { width: nw, height: nh }, None, 0, &props).ok()?;
                    dc.SetTarget(&small);
                    dc.SetTransform(&ident);
                    dc.BeginDraw();
                    dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
                    let d = D2D_RECT_F { left: 0.0, top: 0.0, right: nw as f32, bottom: nh as f32 };
                    let from: ID2D1Bitmap = src.cast().ok()?;
                    dc.DrawBitmap(&from, Some(&d), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
                    let ok = dc.EndDraw(None, None).is_ok();
                    dc.SetTarget(None);
                    if !ok {
                        return None;
                    }
                    src = small;
                    sw = nw;
                    sh = nh;
                    out.push(dc.CreateBitmapBrush(&src, Some(&smooth), None).ok()?);
                }
                Some(out)
            })();
            let mut levels = levels.unwrap_or_default().into_iter();
            let (half, soft, far) = (levels.next(), levels.next(), levels.next());
'''+s[b:]
s=sub(s,"                brush,\n                soft,\n","                brush,\n                soft,\n                half,\n                far,\n")
s=sub(s,"""impl<'a> Painter<'a> {
    /// Like `snapshot`, but the soft (blurred) copy""","""impl<'a> Painter<'a> {
    /// A snapshot at blur level `lvl`: 0 sharp, 1 light, 2 medium, 3 heavy (the picture drawn from
    /// a half, quarter or eighth size copy). Falls back to the sharp picture if there is none.
    pub fn snapshot_level(&self, s: &Snapshot, lvl: u8, opacity: f32) {
        let brush = match lvl {
            0 => None,
            1 => s.half.as_ref(),
            2 => s.soft.as_ref(),
            _ => s.far.as_ref(),
        };
        let Some(b) = brush else {
            self.snapshot(s, 0.0, 0.0, opacity);
            return;
        };
        let a = opacity * self.alpha;
        if a <= 0.002 {
            return;
        }
        let k = (1u32 << lvl.min(3)) as f32 / self.scale;
        let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: s.rect.x, M32: s.rect.y };
        unsafe {
            b.SetTransform(&m);
            b.SetOpacity(a);
            self.dc.FillRectangle(&rf(s.rect), b);
        }
    }
}

impl<'a> Painter<'a> {
    /// Like `snapshot`, but the soft (blurred) copy""")
open(p,'w').write(s)

# ---- app: the bottom edge is a real gradient blur: sharp at the top of the strip, steadily heavier toward the rim
p='src/app.rs'; s=rd(p)
s=sub(s,"""        let soft = ease(0.0, 0.8, u) * k;
        let fade = 1.0 - 0.72 * ease(0.25, 1.0, u) * k;
        ui.p.push_clip(r);
        ui.p.snapshot(snap, 0.0, 0.0, (1.0 - soft) * fade);
        ui.p.snapshot_soft(snap, 0.0, 0.0, soft * fade);
        ui.p.pop_clip();""","""        // blur grows steadily toward the rim: each row blends two neighbouring levels
        let pos = ease(0.0, 1.0, u) * k * 2.5;
        let lo = pos.floor();
        let mix = pos - lo;
        let fade = 1.0 - 0.72 * ease(0.25, 1.0, u) * k;
        ui.p.push_clip(r);
        ui.p.snapshot_level(snap, lo as u8, fade);
        if mix > 0.01 {
            ui.p.snapshot_level(snap, lo as u8 + 1, mix * fade);
        }
        ui.p.pop_clip();""")
open(p,'w').write(s)
PYEOF
python3 /tmp/v169.py && echo patched
