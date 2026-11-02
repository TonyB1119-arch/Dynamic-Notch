#!/bin/bash
set -e
cd "$(dirname "$0")"
if ! grep -q "icon_square" src/config.rs; then echo "already applied (or run update_245.sh first)"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
def rw(p,f):
    s=open(p).read(); s=f(s); open(p,'w').write(s)

# the squircle option is gone: badges are round
def cfg(s):
    s=sub(s,"    /// The soft badge behind each page icon is a rounded square (true) or a circle (false).\n    pub icon_square: bool,\n","")
    s=sub(s,"            icon_square: true,\n","")
    return sub(s,"# Shape of the soft badge behind each page icon: true = rounded square, false = circle\nicon_square = true\n","")
rw('src/config.rs',cfg)

rw('src/ui.rs',lambda s: sub(s,"""
/// Icon badges are rounded squares (`[appearance] icon_square`) instead of circles.
static ICON_SQUARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_icon_square(on: bool) {
    ICON_SQUARE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn icon_square() -> bool {
    ICON_SQUARE.load(std::sync::atomic::Ordering::Relaxed)
}
""",""))

rw('src/app.rs',lambda s: sub(sub(s,
 "        crate::ui::set_icon_square(cfg.appearance.icon_square);\n",""),
 "                crate::ui::set_icon_square(self.cfg.appearance.icon_square);\n",""))

rw('src/modules/settings.rs',lambda s: sub(sub(s,
 """        toggle_row(ui, row_rect(body, 18), "Rounded-square icon badges (off = circles)", a.icon_square, "appearance", "icon_square");\n""",""),
 "        1 => 19.0,\n","        1 => 18.0,\n"))

def alc(s):
    s=sub(s,"""/// two short white highlights on the top-right and bottom-left corners. A rounded square, or a
/// circle when `[appearance] icon_square` is off. It brightens under the pointer. Monochrome look only.""",
"""/// two faint white highlights on the top-right and bottom-left corners. It brightens a little under
/// the pointer. Monochrome look only.""")
    s=sub(s,"""    let side = c.w.min(c.h);
    let radius = if crate::ui::icon_square() { side * 0.30 } else { side * 0.5 };
    chip_body(ui, c, radius, h, 1.0);""","""    chip_body(ui, c, c.w.min(c.h) * 0.5, h, 1.0);""")
    s=sub(s,"    let peak = (0.22 + 0.10 * h) * k;","    let peak = (0.11 + 0.06 * h) * k;")
    s=sub(s,"""    // brightest in the middle and fades out quickly toward both ends; a rounded square has less
    // corner to light, so its layers run closer to the full quarter turn
    let round = radius >= c.w.min(c.h) * 0.5 - 0.01;
    let sweeps: [f32; 4] = if round { [1.2, 0.9, 0.6, 0.3] } else { [1.5, 1.12, 0.75, 0.38] };
""","""    // brightest in the middle and fades out quickly toward both ends
    let sweeps: [f32; 4] = [1.2, 0.9, 0.6, 0.3];
""")
    s=sub(s,"ui.p.arc_cached(cx, cy, r, mid - sw * 0.5, sw, 1.5, Color::white(peak));","ui.p.arc_cached(cx, cy, r, mid - sw * 0.5, sw, 1.3, Color::white(peak));")
    # tabs: the chosen one is a plain soft fill, no ring or highlights
    s=sub(s,"""/// A Wi-Fi / Bluetooth style tab. The chosen one is the badge (fill, ring, corner highlights, white
/// text); the others are plain, dim text on the notch itself.""","""/// A Wi-Fi / Bluetooth style tab. The chosen one is a soft fill with white text (no ring or
/// highlights); the others are plain, dim text on the notch itself.""")
    s=sub(s,"        chip_body(ui, rr, rr.h * 0.5, h, a);","        ui.p.fill_rounded(rr, rr.h * 0.5, Color::white((0.10 + 0.04 * h) * a));")
    s=sub(s,"    icon_chip(ui, c, 4);\n","")
    return s
rw('src/alcove.rs',alc)
PYEOF
echo "Patched v246"
