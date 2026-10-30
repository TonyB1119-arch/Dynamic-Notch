#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:80]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()

# --- unlock: the ring starts in the middle and slides left as the words arrive
s=sub(s,"""    // the ring sits on the left, the words beside it
    let cx = body.x + 82.0;
""","""    // the ring starts in the middle and slides to the left as "Hello" arrives
    let cx = lerp(body.cx(), body.x + 82.0, smoothstep(0.44, 0.64, t));
""")

# --- room under the text of a popup, so the soft bottom edge only meets empty glass
s=sub(s,"/// How tall the soft-blur strip along the bottom of the open notch is.","""/// Extra height under the text of a popup while the soft bottom edge is on, so that edge only
/// ever meets empty glass and never blurs a line of text.
fn band_room(cfg: &Config) -> f32 {
    if crate::ui::look_alcove() && cfg.appearance.bottom_fade && cfg.appearance.blur {
        40.0
    } else {
        0.0
    }
}

/// How tall the soft-blur strip along the bottom of the open notch is.""")
s=sub(s,"t.h = base_h + 58.0;","t.h = base_h + 58.0 + band_room(&self.cfg);")
s=sub(s,"t.h = base_h + 24.0;","t.h = base_h + 24.0 + band_room(&self.cfg);")
s=sub(s,"t.h = base_h + 92.0;","t.h = base_h + 92.0 + band_room(&self.cfg);")
s=sub(s,"t.h = base_h + 46.0;","t.h = base_h + 46.0 + band_room(&self.cfg);")
s=sub(s,"t.h = base_h + 34.0;","t.h = base_h + 34.0 + band_room(&self.cfg);")
# the content keeps its place at the top; the extra room is left empty at the bottom
s=sub(s,"let inner = Rect::new(body.x + 22.0, body.y + 8.0, body.w - 44.0, body.h - 16.0 - if rail { 12.0 } else { 0.0 });",
        "let room = band_room(ui.cfg);\n    let inner = Rect::new(body.x + 22.0, body.y + 8.0, body.w - 44.0, body.h - 16.0 - room - if rail { 12.0 } else { 0.0 });")
s=sub(s,"    let row_y = body.bottom() - 36.0;\n","    let row_y = body.bottom() - 36.0 - band_room(ui.cfg);\n")
open(p,'w').write(s)
PYEOF
echo "Patched v185"
