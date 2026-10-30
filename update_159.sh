cd /workspaces/Notch
cat > /tmp/v159.py <<'PYEOF'
import re
def rd(p): return open(p).read()
def sub(s,a,b):
    assert s.count(a)==1, (s.count(a), a[:80])
    return s.replace(a,b)

# ---- gfx: a vertical gradient fill for a shape
p='src/gfx/mod.rs'; s=rd(p)
s=sub(s,"""    pub fn fill_rounded(&self, r: Rect, radius: f32, c: Color) {""","""    /// A shape filled with a vertical gradient from `y0` to `y1`; `stops` are (position 0..1, color).
    pub fn fill_geometry_vgrad(&self, g: &ID2D1Geometry, y0: f32, y1: f32, stops: &[(f32, Color)]) {
        if stops.len() < 2 || y1 - y0 < 0.5 {
            return;
        }
        let mut buf = [D2D1_GRADIENT_STOP::default(); 8];
        let n = stops.len().min(buf.len());
        for (slot, (pos, c)) in buf.iter_mut().zip(stops) {
            *slot = D2D1_GRADIENT_STOP { position: *pos, color: d2c(c.alpha(self.alpha)) };
        }
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(0.0, y0), endPoint: v2(0.0, y1) };
        unsafe {
            let Ok(coll) = ID2D1RenderTarget::CreateGradientStopCollection(
                &self.dc,
                &buf[..n],
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) else {
                return;
            };
            let Ok(brush) = self.dc.CreateLinearGradientBrush(&props, None, &coll) else { return };
            self.dc.FillGeometry(g, &brush, None);
        }
    }

    pub fn fill_rounded(&self, r: Rect, radius: f32, c: Color) {""")
open(p,'w').write(s)

# ---- app
p='src/app.rs'; s=rd(p)
s=sub(s,"    snap_band: Option<Snapshot>,\n    pacer: Pacer,","    snap_band: Option<Snapshot>,\n    /// When the blur strip first appeared (it fades in from here).\n    band_since: Option<Instant>,\n    pacer: Pacer,")
s=sub(s,"            snap_band: None,\n            pacer,","            snap_band: None,\n            band_since: None,\n            pacer,")
# prepare_band: needs frosted glass, tracks since
s=sub(s,"        let on = self.cfg.appearance.bottom_fade\n","        let on = self.cfg.appearance.bottom_fade\n            && self.cfg.appearance.blur\n")
s=sub(s,"""        if !on {
            self.snap_band = None;
            return;
        }
        let full = Rect::new(body.cx() - tw * 0.5, body.y, tw, t.h);""","""        if !on {
            self.snap_band = None;
            self.band_since = None;
            return;
        }
        self.band_since.get_or_insert(now);
        let full = Rect::new(body.cx() - tw * 0.5, body.y, tw, t.h);""")
s=sub(s,"""    /// While the notch opens or closes, and while a page slides in""","""    /// 0..1: how far the soft bottom edge has faded in (0 while it is off or the notch moves).
    fn band_k(&self, now: Instant) -> f32 {
        match (&self.snap_band, self.band_since) {
            (Some(_), Some(t0)) => {
                let x = (now.saturating_duration_since(t0).as_secs_f32() / 0.4).clamp(0.0, 1.0);
                x * x * (3.0 - 2.0 * x)
            }
            _ => 0.0,
        }
    }

    /// While the notch opens or closes, and while a page slides in""")
# read k before painter
s=sub(s,"        let page_w = self.page_width();\n        let bubble = self.bubble_rect();","        let page_w = self.page_width();\n        let band_k = self.band_k(now);\n        let bubble = self.bubble_rect();")
# body fill
s=sub(s,"""                if let Some(g) = gfx::notch_path(&p, body, a_r, a_ear) {
                    p.fill_geometry(&g, Color::black(bg_alpha));
                }

                p.push_clip(body);""","""                if let Some(g) = gfx::notch_path(&p, body, a_r, a_ear) {
                    if band_k > 0.01 && body.h > BAND_H * 2.0 {
                        // Alcove: the glass turns see-through toward the bottom edge
                        let from = 1.0 - BAND_H / body.h;
                        let low = bg_alpha * (1.0 - 0.62 * band_k);
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[(0.0, Color::black(bg_alpha)), (from, Color::black(bg_alpha)), (1.0, Color::black(low))],
                        );
                        fx.animate |= band_k < 1.0;
                    } else {
                        p.fill_geometry(&g, Color::black(bg_alpha));
                    }
                }

                p.push_clip(body);""")
# draw branch
a=s.index("                            draw_expanded(\n                                &mut ui,\n                                full,\n                                body,")
b=s.index("                    ui.p.reset_local();",a)
blk=s[a:b]
new='''                            // the strip along the bottom is drawn from its own picture (sharp, then
                            // softer toward the rim), so the live page stops just above it
                            let band = if band_k > 0.01 { self.snap_band.as_ref() } else { None };
                            if let Some(b) = band {
                                ui.p.push_clip(Rect::new(0.0, -200.0, self.win_w, b.rect.y + 200.0));
                            }
                            draw_expanded(
                                &mut ui,
                                full,
                                body,
                                &mut self.modules,
                                &self.started,
                                &tabs,
                                self.page,
                                trans,
                                &self.indicators,
                                pages,
                            );
                            if let Some(b) = band {
                                ui.p.pop_clip();
                                bottom_fade(&mut ui, b, full, band_k);
                            }
                        }
                    }
'''
s=s[:a]+new+s[b:]
# blur backdrop
s=sub(s,"let blur = if ap.blur && see_through && settled { a_glass } else { 0.0 };","let blur = if ap.blur && (see_through || band_k > 0.01) && settled { a_glass } else { 0.0 };")
# bottom_fade rewrite
a=s.index("/// The soft blur along the bottom edge (Alcove)")
b=s.index("/// The open notch. In the Alcove look the page dots")
s=s[:a]+'''/// The soft blur along the bottom edge (Alcove): the content in the last strip of the open notch
/// goes from sharp to a light blur and fades into the glass toward the rim. `snap` is the strip's
/// own picture; `k` fades the whole effect in. Drawn as a few thin slices, which follow the rounded
/// corners and keep it cheap.
fn bottom_fade(ui: &mut Ui, snap: &Snapshot, full: Rect, k: f32) {
    const N: usize = 14;
    let band = snap.rect;
    let bottom = full.bottom();
    let step = band.h / N as f32;
    let ease = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for i in 0..N {
        let u = (i as f32 + 0.5) / N as f32;
        let y0 = (band.y + step * i as f32).round();
        let y1 = if i + 1 == N { band.bottom() } else { (band.y + step * (i + 1) as f32).round() };
        // keep inside the rounded bottom corners
        let dy = (y1 - (bottom - BAND_R)).clamp(0.0, BAND_R);
        let inset = BAND_R - (BAND_R * BAND_R - dy * dy).max(0.0).sqrt();
        let r = Rect::new(band.x + inset, y0, band.w - inset * 2.0, y1 - y0);
        if r.w <= 1.0 || r.h <= 0.0 {
            continue;
        }
        let soft = ease(0.0, 0.9, u) * k;
        let fade = 1.0 - 0.6 * ease(0.3, 1.0, u) * k;
        ui.p.push_clip(r);
        ui.p.snapshot(snap, 0.0, 0.0, (1.0 - soft) * fade);
        ui.p.snapshot_soft(snap, 0.0, 0.0, soft * fade);
        ui.p.pop_clip();
    }
}

'''+s[b:]
open(p,'w').write(s)

s=rd('src/app.rs')
s=sub(s,"(body.h - t.h).abs() < 1.5","(body.h - t.h).abs() < 3.0")
s=sub(s,"(body.w - tw).abs() < 1.5","(body.w - tw).abs() < 3.0")
s=re.sub(r"Build v\d+ ·","Build v159 ·",s) if False else s
open('src/app.rs','w').write(s)
PYEOF
python3 /tmp/v159.py && echo patched
