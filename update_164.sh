cd /workspaces/Notch
cat > /tmp/v164.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b):
    assert s.count(a)==1, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)

# --- prepare_band (pages and peeks) and band_k (no fade-in: it is simply there)
a=s.index("    /// Alcove: the bottom edge of an open page softens into a light blur.")
b=s.index("    /// While the notch opens or closes, and while a page slides in")
s=s[:a]+'''    /// Alcove: the bottom edge of the open notch (a page, or a bigger peek such as the volume dial
    /// or a message) softens into a light blur. Only the thin strip along the bottom is drawn a
    /// second time into a small picture (and only on frames that are drawn anyway), then laid over
    /// the live drawing in `bottom_fade`. It is there as soon as the notch is open, also while pages
    /// switch.
    fn prepare_band(&mut self, now: Instant) {
        let t = self.targets();
        let tw = self.page_width();
        let body = self.body_rect();
        let ap = &self.cfg.appearance;
        let on = ap.bottom_fade && ap.blur && crate::ui::look_alcove();
        let page_band = on
            && self.mode == Mode::Expanded
            && self.page != "settings"
            && self.anim.exp.value >= 0.88
            && (body.h - t.h).abs() < 40.0;
        let peek_band = on
            && self.mode == Mode::Peek
            && self.anim.peek.value >= 0.95
            && body.h >= 56.0
            && (body.h - t.h).abs() < 24.0
            && self.peek.as_ref().is_some_and(|a| a.swap.is_none());
        if !page_band && !peek_band {
            if self.band_since.is_some() {
                log!("bottom blur off: mode {:?} page {} exp {:.3} dh {:.1}", self.mode, self.page, self.anim.exp.value, body.h - t.h);
            }
            self.snap_band = None;
            self.band_since = None;
            return;
        }
        self.band_since.get_or_insert(now);
        let input = Input::default();
        let mut scratch = Effects::default();
        let (accent, page, scale) = (self.accent, self.page, self.scale);
        if page_band {
            let full = Rect::new(body.cx() - tw * 0.5, body.y, tw, t.h);
            let content =
                page_content(body.cx(), full.y, page_size(&self.cfg, &self.modules, &self.started, self.page));
            let h = BAND_H.min(full.h * 0.4);
            let region = Rect::new(full.x, full.bottom() - h, full.w, h);
            self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4D, |p| {
                let mut ui = Ui {
                    p,
                    input: &input,
                    fx: &mut scratch,
                    cfg: &self.cfg,
                    bus: &self.bus,
                    now,
                    accent,
                    hot: false,
                    interactive: false,
                };
                draw_body(&mut ui, content, &mut self.modules, &self.started, page);
            });
        } else if let Some(act) = &self.peek {
            let h = BAND_H.min(body.h * 0.4);
            let region = Rect::new(body.x, body.bottom() - h, body.w, h);
            let left = (act.until.saturating_duration_since(now).as_secs_f32()
                / act.peek.duration.as_secs_f32().max(0.1))
            .clamp(0.0, 1.0);
            self.snap_band = self.res.render_snapshot(&self.gfx, scale, region, 0xB4E, |p| {
                let mut ui = Ui {
                    p,
                    input: &input,
                    fx: &mut scratch,
                    cfg: &self.cfg,
                    bus: &self.bus,
                    now,
                    accent,
                    hot: false,
                    interactive: false,
                };
                draw_peek(&mut ui, body, &act.peek, left, None);
            });
        }
    }

    /// 1 while the soft bottom edge is showing, else 0.
    fn band_k(&self) -> f32 {
        if self.snap_band.is_some() {
            1.0
        } else {
            0.0
        }
    }

'''+s[b:]
s=sub(s,"let band_k = self.band_k(now);","let band_k = self.band_k();")

# --- body fill follows the strip's own height
s=sub(s,"""                    if self.snap_band.is_some() && band_k < 1.0 {
                        fx.animate = true;
                    }
                    if band_k > 0.01 && body.h > BAND_H * 2.0 {
                        // Alcove: the glass turns see-through toward the bottom edge
                        let from = 1.0 - BAND_H / body.h;
                        let low = bg_alpha * (1.0 - 0.62 * band_k);""","""                    let bh = self.snap_band.as_ref().map_or(BAND_H, |b| b.rect.h);
                    if band_k > 0.01 && body.h > bh * 2.0 {
                        // Alcove: the glass turns see-through toward the bottom edge
                        let from = 1.0 - bh / body.h;
                        let low = bg_alpha * (1.0 - 0.74 * band_k);""")
s=sub(s,"                        fx.animate |= band_k < 1.0;\n","")
# --- the frosted layer: there with the strip, not only once settled
s=sub(s,"let blur = if ap.blur && (see_through || band_k > 0.01) && settled { a_glass } else { 0.0 };","let blur = if ap.blur && ((see_through && settled) || band_k > 0.01) { a_glass } else { 0.0 };")
# --- pages
s=sub(s,"let band = if band_k > 0.01 { self.snap_band.as_ref() } else { None };","let band = if band_k > 0.01 && self.mode == Mode::Expanded { self.snap_band.as_ref() } else { None };")
s=sub(s,"bottom_fade(&mut ui, b, full, band_k);","bottom_fade(&mut ui, b, body.bottom(), a_r, band_k);")
# --- peeks
s=sub(s,"""                        draw_peek(&mut ui, body, &pk.peek, left.clamp(0.0, 1.0), swap);
                        ui.p.reset_local();""","""                        let band = if band_k > 0.01 && self.mode == Mode::Peek && swap.is_none() {
                            self.snap_band.as_ref()
                        } else {
                            None
                        };
                        if let Some(b) = band {
                            ui.p.push_clip(Rect::new(0.0, -200.0, self.win_w, b.rect.y + 200.0));
                        }
                        draw_peek(&mut ui, body, &pk.peek, left.clamp(0.0, 1.0), swap);
                        if let Some(b) = band {
                            ui.p.pop_clip();
                            bottom_fade(&mut ui, b, body.bottom(), a_r, band_k);
                        }
                        ui.p.reset_local();""")
# --- the strip: a little taller and a little stronger
a=s.index("/// How tall the soft-blur strip along the bottom")
b=s.index("/// The open notch. In the Alcove look the page dots")
s=s[:a]+'''/// How tall the soft-blur strip along the bottom of the open notch is.
const BAND_H: f32 = 64.0;

/// The soft blur along the bottom edge (Alcove): the content in the last strip of the open notch
/// goes from sharp to a light blur and fades into the glass toward the rim. `snap` is the strip's
/// own picture, `bottom` the notch's bottom edge and `radius` its corner radius; `k` scales the
/// effect. Drawn as a few thin slices, which follow the rounded corners and keep it cheap.
fn bottom_fade(ui: &mut Ui, snap: &Snapshot, bottom: f32, radius: f32, k: f32) {
    const N: usize = 14;
    let band = snap.rect;
    let rad = radius.max(6.0);
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
        let dy = (y1 - (bottom - rad)).clamp(0.0, rad);
        let inset = rad - (rad * rad - dy * dy).max(0.0).sqrt();
        let r = Rect::new(band.x + inset, y0, band.w - inset * 2.0, y1 - y0);
        if r.w <= 1.0 || r.h <= 0.0 {
            continue;
        }
        let soft = ease(0.0, 0.8, u) * k;
        let fade = 1.0 - 0.72 * ease(0.25, 1.0, u) * k;
        ui.p.push_clip(r);
        ui.p.snapshot(snap, 0.0, 0.0, (1.0 - soft) * fade);
        ui.p.snapshot_soft(snap, 0.0, 0.0, soft * fade);
        ui.p.pop_clip();
    }
}

'''+s[b:]
open(p,'w').write(s)
PYEOF
python3 /tmp/v164.py && echo patched
