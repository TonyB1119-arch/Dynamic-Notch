#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "halo_y" src/gfx/mod.rs; then echo "already applied"; exit 0; fi
if ! grep -q "rim_pool" src/gfx/mod.rs; then echo "run update_220.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/gfx/mod.rs'; s=open(p).read()
s=sub(s,"""    rim_pool: RimPool,
}
""","""    rim_pool: RimPool,
    /// Text whose middle lies below this height (logical units) is over the see-through glass at the
    /// bottom of the notch, and gets a thin dark outline so it stays readable (None = no outline).
    halo_y: Option<f32>,
}
""")
s=sub(s,"                rim_pool: RimPool::default(),\n            })","                rim_pool: RimPool::default(),\n                halo_y: None,\n            })")
s=sub(s,"""    /// Drop cached bitmaps that are not used right now""","""    /// Set where the see-through glass begins for the text that is drawn next (see `halo_y`).
    pub fn set_halo(&mut self, y: Option<f32>) {
        self.halo_y = y;
    }

    /// Drop cached bitmaps that are not used right now""")
s=sub(s,"""        if let Some(l) = self.layout(s, style, r.w, r.h) {
            self.draw_layout(&l, r.x, r.y, style.color);
        }
    }
""","""        if let Some(l) = self.layout(s, style, r.w, r.h) {
            // over the glass the screen behind can be any colour: a thin dark outline keeps the text
            // readable on light and busy backgrounds alike
            if self.res.halo_y.is_some_and(|y| r.y + r.h * 0.5 > y) {
                let halo = Color::black(0.6 * style.color.a);
                let d = 0.9;
                for (dx, dy) in [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d)] {
                    self.draw_layout(&l, r.x + dx, r.y + dy, halo);
                }
            }
            self.draw_layout(&l, r.x, r.y, style.color);
        }
    }
""")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"""    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_pictures(now);
        self.prepare_band(now);""","""    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_pictures(now);
        // (the blurred strip is drawn without the outline: it is a soft copy of the page anyway)
        self.res.set_halo(None);
        self.prepare_band(now);""")
s=sub(s,"""    fn prepare_pictures(&mut self, now: Instant) {""","""    /// Where the see-through glass begins at the bottom of a notch whose bottom edge is at `bottom` and
    /// that is `h` tall: text below that line gets a thin dark outline (see `Res::set_halo`). With
    /// the whole panel see-through (opacity below 100 %) all of it does.
    fn halo_y(&self, bottom: f32, h: f32) -> Option<f32> {
        let ap = &self.cfg.appearance;
        let opaque = if ap.blur { ap.expanded_opacity } else { ap.expanded_opacity.max(0.95) };
        if 1.0 - opaque > 0.05 {
            return Some(f32::MIN);
        }
        let bhh = BAND_H.min(h * 0.4);
        let band_on = ap.bottom_fade && ap.blur && crate::ui::look_alcove() && self.page != "settings" && h > bhh * 2.0;
        band_on.then(|| bottom - bhh * self.cfg.glass.fade_start.clamp(0.5, 3.0) * 0.6)
    }

    fn prepare_pictures(&mut self, now: Instant) {""")
s=sub(s,"""        if want_all {
            // the height is left out of the key""","""        self.res.set_halo(self.halo_y(full.bottom(), full.h));
        if want_all {
            // the height is left out of the key""")
s=sub(s,"""        let body = self.body_rect();
        let scale = self.scale;
        let ap = {""","""        let body = self.body_rect();
        self.res.set_halo(self.halo_y(body.bottom(), body.h));
        let scale = self.scale;
        let ap = {""")
open(p,'w').write(s)
PYEOF
echo "Patched v222"
