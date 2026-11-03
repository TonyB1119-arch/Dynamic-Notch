#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
s=open('src/modules/media.rs').read()
s=rep(s,"fn output_button(&mut self, ui: &mut Ui, r: Rect, fg: Color) {","fn output_button(&mut self, ui: &mut Ui, r: Rect, fg: Color, scale: f32) {")
s=rep(s,"let size = r.w.min(r.h) * 0.5 * (1.0 - 0.14 * p) * (1.0 + 0.03 * h);","let size = r.w.min(r.h) * scale * (1.0 - 0.14 * p) * (1.0 + 0.03 * h);")
s=rep(s,"self.output_button(ui, Rect::centered(cx + 130.0, cy, 40.0, 40.0), Color::white(0.9));","self.output_button(ui, Rect::centered(cx + 152.0, cy, 44.0, 40.0), Color::white(0.97), 0.85);")
s=rep(s,"let out = Rect::centered(next.cx() + small + 8.0, cy, small, small);\n        self.output_button(ui, out, palette::TEXT);","let out = Rect::centered(next.cx() + small + 14.0, cy, small, small);\n        self.output_button(ui, out, palette::TEXT, 0.5);")
open('src/modules/media.rs','w').write(s)
print('ok')
PYEOF
echo "update_265 applied"
