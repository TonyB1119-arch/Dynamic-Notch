cd /workspaces/Notch
cat > /tmp/v158.py <<'PYEOF'
import re
def rd(p): return open(p).read()
def sub(s,a,b):
    assert s.count(a)==1, (s.count(a), a[:70])
    return s.replace(a,b)

# ---- config
p='src/config.rs'; s=rd(p)
s=sub(s,"    pub motion_cache: bool,\n","    pub motion_cache: bool,\n    /// Alcove: the bottom edge of the open notch softens into a light blur.\n    pub bottom_fade: bool,\n")
s=sub(s,"            motion_cache: true,\n","            motion_cache: true,\n            bottom_fade: true,\n")
s=sub(s,"motion_cache = true\n","motion_cache = true\n# Alcove: the bottom edge of the open notch softens into a light blur\nbottom_fade = true\n")
open(p,'w').write(s)

# ---- settings
p='src/modules/settings.rs'; s=rd(p)
s=sub(s,"""            "appearance",
            "motion_cache",
        );
""","""            "appearance",
            "motion_cache",
        );
        toggle_row(
            ui,
            row_rect(body, 12),
            "Soft blur at the bottom edge",
            a.bottom_fade,
            "appearance",
            "bottom_fade",
        );
""")
s=sub(s,"        1 => 12.0,","        1 => 13.0,")
s=re.sub(r"Build v\d+ ·","Build v158 ·",s)
open(p,'w').write(s)

# ---- app
p='src/app.rs'; s=rd(p)
s=sub(s,"""    snap_pages: Option<(Snapshot, Snapshot)>,
    pacer: Pacer,""","""    snap_pages: Option<(Snapshot, Snapshot)>,
    /// The bottom strip of the open page, for the soft blur along the bottom edge (Alcove).
    snap_band: Option<Snapshot>,
    pacer: Pacer,""")
s=sub(s,"            snap_pages: None,\n            pacer,","            snap_pages: None,\n            snap_band: None,\n            pacer,")
s=sub(s,"""    fn prepare_snapshots(&mut self, now: Instant) {
        let a_exp = self.anim.exp.value;""","""    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_band(now);
        let a_exp = self.anim.exp.value;""")
s=sub(s,"""    /// While the notch opens or closes, and while a page slides in""","""    /// Alcove: the bottom edge of an open page softens into a light blur. Only the thin strip along
    /// the bottom is drawn a second time into a small picture (and only on frames that are drawn
    /// anyway), then laid over the live page in `bottom_fade`. Skipped while anything moves.
    fn prepare_band(&mut self, now: Instant) {
        let t = self.targets();
        let tw = self.page_width();
        let body = self.body_rect();
        let on = self.cfg.appearance.bottom_fade
            && crate::ui::look_alcove()
            && self.mode == Mode::Expanded
            && self.page_from.is_none()
            && self.page != "settings"
            && self.anim.exp.value >= 0.97
            && (body.h - t.h).abs() < 1.5
            && (body.w - tw).abs() < 1.5;
        if !on {
            self.snap_band = None;
            return;
        }
        let full = Rect::new(body.cx() - tw * 0.5, body.y, tw, t.h);
        let content = page_content(body.cx(), full.y, page_size(&self.cfg, &self.modules, &self.started, self.page));
        let h = BAND_H.min(full.h * 0.4);
        let region = Rect::new(full.x, full.bottom() - h, full.w, h);
        let input = Input::default();
        let mut scratch = Effects::default();
        let (accent, page, scale) = (self.accent, self.page, self.scale);
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
    }

    /// While the notch opens or closes, and while a page slides in""")
s=sub(s,"""                                &self.indicators,
                                pages,
                            );
                        }
                    }
                    ui.p.reset_local();""","""                                &self.indicators,
                                pages,
                            );
                            // Alcove: the soft blur along the bottom edge fades in once the notch has settled
                            let want = if self.snap_band.is_some() { 1.0 } else { 0.0 };
                            let k = ui.glide(gkey("alcove-band", 0), want);
                            if let (Some(b), true) = (&self.snap_band, k > 0.01) {
                                bottom_fade(&mut ui, b, full, k);
                            }
                        }
                    }
                    ui.p.reset_local();""")
s=sub(s,"""/// The open notch. In the Alcove look the page dots""","""/// How tall the soft-blur strip along the bottom of the open notch is.
const BAND_H: f32 = 56.0;
/// The bottom corner radius of the open Alcove notch (the strip follows it).
const BAND_R: f32 = 46.0;

/// The soft blur along the bottom edge (Alcove): content fades into a light blur and a little
/// darkness toward the rim. `snap` is the strip's own picture; `k` fades the whole effect in.
/// Drawn as a few thin slices so it follows the rounded corners and stays cheap.
fn bottom_fade(ui: &mut Ui, snap: &Snapshot, full: Rect, k: f32) {
    const N: usize = 10;
    let band = snap.rect;
    let bottom = full.bottom();
    let step = band.h / N as f32;
    let ease = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for pass in 0..3 {
        for i in 0..N {
            let u = (i as f32 + 0.5) / N as f32;
            let y = band.y + step * i as f32;
            // keep inside the rounded bottom corners
            let dy = (y + step - (bottom - BAND_R)).max(0.0).min(BAND_R);
            let inset = BAND_R - (BAND_R * BAND_R - dy * dy).max(0.0).sqrt();
            let r = Rect::new(band.x + inset, y, band.w - inset * 2.0, step + 0.5);
            if r.w <= 1.0 {
                continue;
            }
            match pass {
                0 => ui.p.fill_rect(r, Color::black(0.55 * ease(0.1, 1.0, u) * k)),
                1 => {
                    ui.p.push_clip(r);
                    ui.p.snapshot_soft(snap, 0.0, 0.0, 0.75 * ease(0.1, 1.0, u) * k);
                    ui.p.pop_clip();
                }
                _ => ui.p.fill_rect(r, Color::black(0.40 * ease(0.35, 1.0, u) * k)),
            }
        }
    }
}

/// The open notch. In the Alcove look the page dots""")
open(p,'w').write(s)
PYEOF
python3 /tmp/v158.py && echo patched
