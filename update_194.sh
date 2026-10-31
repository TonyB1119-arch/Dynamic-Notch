#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "rim_ok" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()

s=sub(s,"    refr_dead: bool,\n    /// When the blur strip",
"""    refr_dead: bool,
    /// The rim may be shown (right size, no page change running), how strongly it is shown now
    /// (it fades in and out instead of popping), and when that was last stepped.
    rim_ok: bool,
    rim_k: f32,
    rim_at: Instant,
    /// When the blur strip""")
s=sub(s,"            refr_dead: false,\n","            refr_dead: false,\n            rim_ok: false,\n            rim_k: 0.0,\n            rim_at: Instant::now(),\n")

s=sub(s,"""            self.refr_rim = None;
            self.refr_dead = false;
            return;
        }
        let fg =""","""            self.refr_rim = None;
            self.refr_dead = false;
            self.rim_ok = false;
            self.rim_k = 0.0;
            return;
        }
        let fg =""")
s=sub(s,"""        if !self.settled {
            return;
        }
        let Some(shot) = self.refr_shot.as_ref() else { return };
        let body = self.body_rect();
        if body.h < 56.0 {
            self.refr_rim = None;
            return;
        }""","""        // the rim is made as soon as the notch has reached its size, not only once every last
        // spring has come to rest (that tail made it appear a second late)
        let body = self.body_rect();
        let t = self.targets();
        let calm = self.page_from.is_none()
            && (self.anim.exp.value > 0.98 || self.anim.peek.value > 0.95)
            && (body.h - t.h).abs() < 1.0
            && (body.w - t.w).abs() < 1.0;
        if !calm {
            self.rim_ok = false;
            return;
        }
        let Some(shot) = self.refr_shot.as_ref() else {
            self.rim_ok = false;
            return;
        };
        if body.h < 56.0 {
            self.refr_rim = None;
            self.rim_ok = false;
            return;
        }""")
s=sub(s,"""        if self.refr_rim.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }""","""        if self.refr_rim.as_ref().is_some_and(|(k, _)| *k == key) {
            self.rim_ok = true;
            return;
        }""")
s=sub(s,"""                self.refr_rim = Some((key, pic));
            }
        }
    }
""","""                self.refr_rim = Some((key, pic));
            }
        }
        self.rim_ok = self.refr_rim.is_some();
    }
""")

s=sub(s,"""        let glass_k = if self.page_from.is_some() { 0.0 } else { 1.0 };
        let a_glass = self.anim.glass.value;
""","""        let glass_k = if self.page_from.is_some() { 0.0 } else { 1.0 };
        let a_glass = self.anim.glass.value;
        // the rim fades in over about a quarter of a second and out twice as fast
        let rim_to = if self.rim_ok && glass_k > 0.0 { 1.0 } else { 0.0 };
        let rim_dt = now.saturating_duration_since(self.rim_at).as_secs_f32().min(0.1);
        self.rim_at = now;
        let rim_rate = if rim_to > self.rim_k { 9.0 } else { 18.0 };
        self.rim_k += (rim_to - self.rim_k) * (1.0 - (-rim_rate * rim_dt).exp());
        if (rim_to - self.rim_k).abs() < 0.01 {
            self.rim_k = rim_to;
        }
        if self.rim_k != rim_to {
            fx.animate = true;
        }
""")
s=sub(s,"""                if glass && glass_k > 0.0 && settled && body.h >= 56.0 && a_glass > 0.01 {
                    // the glass edge: the screen behind, bent along the rim (see `prepare_rim`)
                    if let Some((_, pic)) = &self.refr_rim {
                        p.draw_rim(pic, a_glass * glass_k);""","""                if glass && self.rim_k > 0.003 && body.h >= 56.0 && a_glass > 0.01 {
                    // the glass edge: the screen behind, bent along the rim (see `prepare_rim`)
                    if let Some((_, pic)) = &self.refr_rim {
                        p.draw_rim(pic, a_glass * self.rim_k);""")
open(p,'w').write(s)
PYEOF
echo "Patched v194"
