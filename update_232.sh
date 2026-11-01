#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fn draw_rim_to" src/gfx/mod.rs; then echo "already applied"; exit 0; fi
if ! grep -q "fn rim_near" src/app.rs; then echo "run update_225_226.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/gfx/mod.rs'; s=open(p).read()
s=sub(s,"""    pub fn draw_rim(&self, r: &RimPic, opacity: f32) {
        let a = opacity * self.alpha;""","""    pub fn draw_rim(&self, r: &RimPic, opacity: f32) {
        self.draw_rim_to(r, r.rect, opacity);
    }

    /// The same, drawn into `dst` (a picture made for an earlier size is stretched to the size the
    /// notch has now, so no gap shows at its edges).
    pub fn draw_rim_to(&self, r: &RimPic, dst: Rect, opacity: f32) {
        let a = opacity * self.alpha;""")
s=sub(s,"self.dc.DrawBitmap(&b, Some(&rf(r.rect)), a, mode, Some(&src), None);","self.dc.DrawBitmap(&b, Some(&rf(dst)), a, mode, Some(&src), None);")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"    rim_x: f32,\n","    rim_x: f32,\n    /// The notch's rectangle when the glass pictures were made (see `fit_rim`).\n    rim_at: Option<Rect>,\n    rim_old_at: Option<Rect>,\n    rim_last_at: Option<Rect>,\n")
s=sub(s,"            rim_x: 1.0,\n","            rim_x: 1.0,\n            rim_at: None,\n            rim_old_at: None,\n            rim_last_at: None,\n")
s=sub(s,"        self.rim_last = Some((geo, shape));\n","        self.rim_last = Some((geo, shape));\n        self.rim_last_at = Some(body);\n")
s=sub(s,"        let prev = self.refr_rim.take();\n","        let prev = self.refr_rim.take();\n        let prev_at = self.rim_at;\n")
s=sub(s,"                self.refr_rim = Some((key, pic));\n                // the notch has just stopped","                self.refr_rim = Some((key, pic));\n                self.rim_at = Some(body);\n                // the notch has just stopped")
s=sub(s,"                        self.rim_old = Some(old);\n                        self.rim_x = 0.0;\n","                        self.rim_old = Some(old);\n                        self.rim_old_at = prev_at;\n                        self.rim_x = 0.0;\n")
s=sub(s,"                            self.rim_old = self.refr_rim.take();\n","                            self.rim_old = self.refr_rim.take();\n                            self.rim_old_at = self.rim_at;\n                            self.rim_at = self.rim_last_at;\n")
s=sub(s,"""                        let a = a_glass * self.rim_k;
                        match &self.rim_old {""","""                        let a = a_glass * self.rim_k;
                        // a glass picture made a moment ago (lighter animations rebuild it less often)
                        // is stretched to the notch's size now, so nothing shows at its edges
                        let fit = |r: &gfx::RimPic, at: Option<Rect>| -> Rect {
                            match at {
                                Some(o) if o.w > 8.0 && o.h > 8.0 && ((o.w - body.w).abs() + (o.h - body.h).abs()) > 0.05 => {
                                    let (sx, sy) = (body.w / o.w, body.h / o.h);
                                    let (ocx, ncx) = (o.x + o.w * 0.5, body.x + body.w * 0.5);
                                    let r = r.rect;
                                    Rect::new(ncx + (r.x - ocx) * sx, body.bottom() + (r.y - o.bottom()) * sy, r.w * sx, r.h * sy)
                                }
                                _ => r.rect,
                            }
                        };
                        match &self.rim_old {""")
s=sub(s,"""                                p.draw_rim(old, a * (1.0 - t));
                                p.draw_rim(pic, a * t);
                            }
                            _ => p.draw_rim(pic, a),""","""                                p.draw_rim_to(old, fit(old, self.rim_old_at), a * (1.0 - t));
                                p.draw_rim_to(pic, fit(pic, self.rim_at), a * t);
                            }
                            _ => p.draw_rim_to(pic, fit(pic, self.rim_at), a),""")
open(p,'w').write(s)
PYEOF
echo "Patched v232"
