#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "reuse: Option<Snapshot>" src/gfx/mod.rs; then echo "179 already applied"; else
python3 - <<'P179'
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
P179
fi
if grep -q "dial_cy - 12.0, 38.0, 20.0" src/app.rs; then echo "180 already applied"; else
python3 - <<'P180'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        // these two have a status line under the value
        Some("battery" | "bluetooth") => Some(196.0),""","""        // the value sits inside the ring here too, so no extra room is needed under it
        Some("battery" | "bluetooth") => Some(150.0),""")
s=sub(s,"    let inside = dial && pk.key != Some(\"battery\");\n","    let inside = dial;\n")
s=sub(s,"crate::alcove::battery_glyph(ui, Rect::centered(body.cx(), dial_cy, 46.0, 24.0), frac, charging);","crate::alcove::battery_glyph(ui, Rect::centered(body.cx(), dial_cy - 12.0, 38.0, 20.0), frac, charging);")
# inside block: value for battery / bluetooth too
a0=s.index("        if inside {\n            // the value sits inside the ring, clear of the soft blur along the bottom edge")
a1=s.index("        return;\n    }\n    if crate::ui::look_alcove() {\n        // a round, quiet pill")
new='''        // the value sits inside the ring, clear of the soft blur along the bottom edge
        let pct = format!("{:.0}%", frac.clamp(0.0, 1.0) * 100.0);
        let r = Rect::new(body.cx() - 38.0, dial_cy + 3.0, 76.0, 22.0);
        if hint {
            ui.p.text("Switch output", r, &TextStyle::new(10.5, palette::TEXT_DIM).medium().center());
            return;
        }
        let mut big = match text {
            // "Charging · 45%": the bolt in the glyph already says charging, so only the number
            Some(t) if battery => t.split_once(" · ").map_or(pct, |(_, p)| p.to_string()),
            Some(t) => t.to_string(),
            None => pct,
        };
        // a long name (a Bluetooth device) is set smaller and cut short to stay inside the ring
        let size = if big.chars().count() <= 5 { 17.0 } else { 12.0 };
        let st = TextStyle::new(size, Color::white(0.95)).semibold().display().center();
        while big.chars().count() > 3 && ui.p.measure(&big, &st).0 > r.w {
            big.pop();
            if ui.p.measure(&big, &st).0 <= r.w {
                big.push('\\u{2026}');
                break;
            }
        }
        ui.p.text(&big, r, &st);
'''
s=s[:a0]+new+s[a1:]
open(p,'w').write(s)
P180
fi
if grep -q "fn bake_band" src/gfx/mod.rs; then echo "181 already applied"; else
python3 - <<'P181'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)

p='src/gfx/mod.rs'; s=open(p).read()
# Snapshot field
s=sub(s,"    bufs: Vec<ID2D1Bitmap1>,\n    px: (u32, u32),\n}\n","    bufs: Vec<ID2D1Bitmap1>,\n    px: (u32, u32),\n    /// The strip with its blur already blended in (see `Res::bake_band`).\n    baked: Option<(ID2D1Bitmap1, ID2D1BitmapBrush1)>,\n}\n")
s=sub(s,"                bufs,\n                px: (w, h),\n            })","                bufs,\n                px: (w, h),\n                baked: None,\n            })")
# Res fields
s=sub(s,"    realizations: std::cell::RefCell<HashMap<RKey, (ID2D1GeometryRealization, u64)>>,\n    frame: u64,\n}","    realizations: std::cell::RefCell<HashMap<RKey, (ID2D1GeometryRealization, u64)>>,\n    frame: u64,\n    /// The fade masks and the full-strip rectangle used by `bake_band`, made once per strip height.\n    band_mask: Option<(u32, Vec<ID2D1LinearGradientBrush>)>,\n    band_geo: Option<((u32, u32), ID2D1Geometry)>,\n}")
s=sub(s,"                realizations: std::cell::RefCell::new(HashMap::new()),\n                frame: 0,\n            })","                realizations: std::cell::RefCell::new(HashMap::new()),\n                frame: 0,\n                band_mask: None,\n                band_geo: None,\n            })")
# bake_band + painter method, placed before `impl Res {` that holds render_snapshot
marker="impl Res {\n    /// Draw `draw` into a new picture covering `region`"
add='''impl Res {
    /// Blend the strip's pictures (sharp, then lighter to heavier blur) into ONE picture. Each
    /// layer is laid down through a smooth vertical fade, so the blur grows steadily toward the
    /// rim with no slices and no steps, and the screen then draws a single picture. Called each
    /// time the strip is redrawn; the masks and the rectangle are made once per strip height.
    /// Returns false (and clears the baked picture) when something is missing.
    pub fn bake_band(&mut self, gfx: &Gfx, s: &mut Snapshot) -> bool {
        if s.bufs.len() != 4 || s.half.is_none() || s.soft.is_none() || s.far.is_none() {
            s.baked = None;
            return false;
        }
        let ok = self.bake_band_inner(gfx, s).is_some();
        if !ok {
            s.baked = None;
        }
        ok
    }

    fn bake_band_inner(&mut self, gfx: &Gfx, s: &mut Snapshot) -> Option<()> {
        let (w, h) = s.px;
        let dc = self.dc.clone();
        let ease = |a: f32, b: f32, x: f32| {
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        unsafe {
            if self.band_mask.as_ref().map(|m| m.0) != Some(h) {
                const N: usize = 40;
                let mut masks = Vec::with_capacity(4);
                for l in 0..4usize {
                    let mut stops = [D2D1_GRADIENT_STOP::default(); N];
                    for (j, st) in stops.iter_mut().enumerate() {
                        let u = j as f32 / (N - 1) as f32;
                        // how much of each picture a row takes: two neighbouring ones are blended so
                        // that together they add up to exactly `fade` of coverage
                        let pos = ease(0.0, 1.0, u) * 2.5;
                        let lo = pos.floor();
                        let mix = pos - lo;
                        let fade = 1.0 - 0.72 * ease(0.25, 1.0, u);
                        let under = fade * (1.0 - mix) / (1.0 - mix * fade).max(0.05);
                        let lo = lo as usize;
                        let a = if l == lo {
                            under
                        } else if l == lo + 1 && mix > 0.01 {
                            mix * fade
                        } else {
                            0.0
                        };
                        *st = D2D1_GRADIENT_STOP { position: u, color: D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a } };
                    }
                    let coll = ID2D1RenderTarget::CreateGradientStopCollection(
                        &dc,
                        &stops,
                        D2D1_GAMMA_2_2,
                        D2D1_EXTEND_MODE_CLAMP,
                    )
                    .ok()?;
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(0.0, 0.0), endPoint: v2(0.0, h as f32) };
                    masks.push(dc.CreateLinearGradientBrush(&props, None, &coll).ok()?);
                }
                self.band_mask = Some((h, masks));
            }
            if self.band_geo.as_ref().map(|g| g.0) != Some((w, h)) {
                let r = D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 };
                let geo: ID2D1Geometry = gfx.d2d_factory.CreateRectangleGeometry(&r).ok()?.cast().ok()?;
                self.band_geo = Some(((w, h), geo));
            }
            if s.baked.is_none() {
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                    colorContext: ManuallyDrop::new(None),
                };
                let bmp = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props).ok()?;
                let bprops = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
                };
                let brush = dc.CreateBitmapBrush(&bmp, Some(&bprops), None).ok()?;
                s.baked = Some((bmp, brush));
            }
            let (bmp, _) = s.baked.as_ref()?;
            let (_, masks) = self.band_mask.as_ref()?;
            let (_, geo) = self.band_geo.as_ref()?;
            let layers = [Some(&s.brush), s.half.as_ref(), s.soft.as_ref(), s.far.as_ref()];
            dc.SetTarget(bmp);
            dc.SetTransform(&Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 });
            dc.BeginDraw();
            dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            // a mask brush needs aliased drawing; a pixel-aligned rectangle has no edge to smooth
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            for (l, (layer, mask)) in layers.iter().zip(masks.iter()).enumerate() {
                let Some(b) = layer else { continue };
                let f = (1u32 << l) as f32;
                b.SetTransform(&Matrix3x2 { M11: f, M12: 0.0, M21: 0.0, M22: f, M31: 0.0, M32: 0.0 });
                b.SetOpacity(1.0);
                dc.FillGeometry(geo, *b, mask);
            }
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let ok = dc.EndDraw(None, None).is_ok();
            dc.SetTarget(None);
            ok.then_some(())
        }
    }
}

impl Snapshot {
    /// True once the blended strip picture exists.
    pub fn is_baked(&self) -> bool {
        self.baked.is_some()
    }
}

impl<'a> Painter<'a> {
    /// Draw the blended strip through `shape` (the notch outline, so the rounded corners are
    /// smooth), limited to the strip's own rectangle. One fill, whatever the strip height.
    pub fn snapshot_baked(&self, s: &Snapshot, shape: &ID2D1Geometry) -> bool {
        let Some((_, b)) = &s.baked else { return false };
        let a = self.alpha;
        if a <= 0.002 {
            return true;
        }
        let k = 1.0 / self.scale;
        let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: s.rect.x, M32: s.rect.y };
        self.push_clip(s.rect);
        unsafe {
            b.SetTransform(&m);
            b.SetOpacity(a);
            self.dc.FillGeometry(shape, b, None);
        }
        self.pop_clip();
        true
    }
}

'''
s=sub(s,marker,add+marker)
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"bottom_fade(&mut ui, b, body.bottom(), a_r, band_k, self.scale);","band_draw(&mut ui, b, body, a_r, a_ear, band_k, self.scale);",2)
s=sub(s,"""        }
    }

    /// 1 while the soft bottom edge is showing, else 0.""","""        }
        // blend the strip's blur levels into one picture, drawn later in a single fill
        if let Some(s) = self.snap_band.as_mut() {
            self.res.bake_band(&self.gfx, s);
        }
    }

    /// 1 while the soft bottom edge is showing, else 0.""")
s=sub(s,"/// How tall the soft-blur strip along the bottom of the open notch is.","""/// The soft bottom edge: the blended strip drawn in one fill through the notch outline; if that
/// picture could not be made, the older slice by slice drawing takes over.
#[allow(clippy::too_many_arguments)]
fn band_draw(ui: &mut Ui, snap: &Snapshot, body: Rect, radius: f32, ear: f32, k: f32, scale: f32) {
    if snap.is_baked() {
        if let Some(g) = gfx::notch_path(ui.p, body, radius, ear) {
            if ui.p.snapshot_baked(snap, &g) {
                return;
            }
        }
    }
    bottom_fade(ui, snap, body.bottom(), radius, k, scale);
}

/// How tall the soft-blur strip along the bottom of the open notch is.""")
open(p,'w').write(s)
P181
fi
if grep -q "band_failed" src/app.rs; then echo "182 already applied"; else
python3 - <<'P182'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"    snap_band: Option<Snapshot>,\n","    snap_band: Option<Snapshot>,\n    /// Set if the blended bottom strip could not be made: the notch then stays solid, no see-through edge.\n    band_failed: bool,\n")
s=sub(s,"            snap_band: None,\n","            snap_band: None,\n            band_failed: false,\n")
s=sub(s,"        let on = ap.bottom_fade && ap.blur && crate::ui::look_alcove();\n","        let on = ap.bottom_fade && ap.blur && crate::ui::look_alcove() && !self.band_failed;\n")
s=sub(s,"""        if let Some(s) = self.snap_band.as_mut() {
            self.res.bake_band(&self.gfx, s);
        }""","""        let baked = match self.snap_band.as_mut() {
            Some(s) => self.res.bake_band(&self.gfx, s),
            None => true,
        };
        if !baked {
            log!("bottom blur could not be built: staying solid");
            self.band_failed = true;
            self.snap_band = None;
            self.band_since = None;
        }""")
# band_draw: no fallback
a0=s.index("/// The soft bottom edge: the blended strip drawn in one fill")
a1=s.index("/// How tall the soft-blur strip along the bottom of the open notch is.")
s=s[:a0]+'''/// The soft bottom edge: the blended strip drawn in one fill through the notch outline.
#[allow(clippy::too_many_arguments)]
fn band_draw(ui: &mut Ui, snap: &Snapshot, body: Rect, radius: f32, ear: f32) {
    if let Some(g) = gfx::notch_path(ui.p, body, radius, ear) {
        ui.p.snapshot_baked(snap, &g);
    }
}

'''+s[a1:]
# remove the old slice drawing
b0=s.index("/// The soft blur along the bottom edge (Alcove): the content in the last strip")
b1=s.index("/// The open notch. In the Alcove look the page dots sit on top of it")
s=s[:b0]+s[b1:]
s=sub(s,"band_draw(&mut ui, b, body, a_r, a_ear, band_k, self.scale);","band_draw(&mut ui, b, body, a_r, a_ear);",2)
open(p,'w').write(s)

p='src/gfx/mod.rs'; s=open(p).read()
c0=s.index("    /// A snapshot at blur level `lvl`: 0 sharp, 1 light, 2 medium, 3 heavy")
c1=s.index("            self.dc.FillRectangle(&rf(s.rect), b);\n        }\n    }\n", c0)+len("            self.dc.FillRectangle(&rf(s.rect), b);\n        }\n    }\n")
s=s[:c0].rstrip('\n')+"\n"+s[c1:]
open(p,'w').write(s)

s=open(p).read()
s=sub(s,"impl<'a> Painter<'a> {\n}\n\n","")
open(p,'w').write(s)
P182
fi
echo "Patched: memory fix, battery/bluetooth inside the ring, smooth bottom blur"
