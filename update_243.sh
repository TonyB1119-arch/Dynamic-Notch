#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "mono_icons" src/config.rs; then echo "already applied"; exit 0; fi
if ! grep -q "linear fan" src/refract.rs; then echo "run update_242.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------------------------------------------------------- config
p='src/config.rs'; s=open(p).read()
s=sub(s,"    pub lite_motion: bool,\n","    pub lite_motion: bool,\n    /// Icons are drawn in white or grey whatever colour a page gives them (no coloured tints).\n    pub mono_icons: bool,\n")
s=sub(s,"            lite_motion: true,\n","            lite_motion: true,\n            mono_icons: true,\n")
s=sub(s,"lite_motion = true\n","lite_motion = true\n# Monochrome icons: every icon is white or grey, no coloured tints or halos. false = coloured icons\nmono_icons = true\n")
s=sub(s,"    pub stretch: bool,\n}\n","""    pub stretch: bool,
    /// A thin bright line along the very edge of the glass, like the light catching it (0 - 1).
    pub edge_light: f32,
    /// A light frost over the glass (0 - 1).
    pub tint: f32,
    /// Stretch mode: how far the stretch leans sideways toward the ends of the bottom edge (radians).
    pub fan: f32,
    /// Stretch mode: how long the stretch may be, relative to the longest one that never folds over.
    pub stretch_len: f32,
    /// How much the glass dims when the picture behind it is bright, so white text stays clear (0 - 1).
    pub readable: f32,
}
""")
s=sub(s,"            stretch: false,\n        }","            stretch: false,\n            edge_light: 0.35,\n            tint: 0.0,\n            fan: 0.7,\n            stretch_len: 1.0,\n            readable: 0.4,\n        }")
s=sub(s,"        self.sharp_zone = self.sharp_zone.clamp(0.3, 4.0);\n    }","""        self.sharp_zone = self.sharp_zone.clamp(0.3, 4.0);
        self.edge_light = self.edge_light.clamp(0.0, 1.0);
        self.tint = self.tint.clamp(0.0, 1.0);
        self.fan = self.fan.clamp(0.0, 1.2);
        self.stretch_len = self.stretch_len.clamp(0.5, 1.6);
        self.readable = self.readable.clamp(0.0, 1.0);
    }""")
s=sub(s,"stretch = false\n","""stretch = false
# a thin bright line along the very edge of the glass (0 - 1)
edge_light = 0.35
# a light frost over the glass (0 - 1)
tint = 0.0
# stretch mode: how far the stretch leans sideways toward the ends of the bottom edge (0 - 1.2)
fan = 0.7
# stretch mode: how long the stretch may be (1 = the longest that never folds over, up to 1.6)
stretch_len = 1.0
# the glass dims a little when what is behind it is bright, so white text stays clear (0 - 1)
readable = 0.4
""")
open(p,'w').write(s)

# ---------------------------------------------------------------- ui flag
p='src/ui.rs'; s=open(p).read()
s=sub(s,"/// The accent of the Bloom look (iOS blue).\n","""/// Monochrome icons (`[appearance] mono_icons`): any coloured icon is drawn white.
static MONO_ICONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_mono_icons(on: bool) {
    MONO_ICONS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn mono_icons() -> bool {
    MONO_ICONS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The accent of the Bloom look (iOS blue).
""")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"        crate::ui::set_look(&cfg.appearance.look);\n","        crate::ui::set_look(&cfg.appearance.look);\n        crate::ui::set_mono_icons(cfg.appearance.mono_icons);\n")
s=sub(s,"                crate::ui::set_look(&self.cfg.appearance.look);\n","                crate::ui::set_look(&self.cfg.appearance.look);\n                crate::ui::set_mono_icons(self.cfg.appearance.mono_icons);\n")
open(p,'w').write(s)

# ---------------------------------------------------------------- icons
p='src/gfx/icons.rs'; s=open(p).read()
s=sub(s,"""pub fn draw(p: &mut Painter, icon: Icon, r: Rect, c: Color) {
""","""pub fn draw(p: &mut Painter, icon: Icon, r: Rect, c: Color) {
    // monochrome: a coloured icon is drawn white (greys, whites and dark ink stay as they are)
    let c = if crate::ui::mono_icons() && c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b) > 0.12 {
        Color::white(c.a)
    } else {
        c
    };
""")
open(p,'w').write(s)

# ---------------------------------------------------------------- the glass
p='src/refract.rs'; s=open(p).read()
s=sub(s,"    pub dir: f32,\n}\n","""    pub dir: f32,
    /// The bright line along the edge, the frost, the lean of the stretch, its length and the
    /// dimming for a bright background (see `GlassCfg`).
    pub edge_light: f32,
    pub tint: f32,
    pub fan: f32,
    pub len: f32,
    pub readable: f32,
}
""")
s=sub(s,"            dir: if g.stretch { -1.0 } else { 1.0 },\n","""            dir: if g.stretch { -1.0 } else { 1.0 },
            edge_light: g.edge_light.clamp(0.0, 1.0),
            tint: g.tint.clamp(0.0, 1.0),
            fan: g.fan.clamp(0.0, 1.2),
            len: g.stretch_len.clamp(0.5, 1.6),
            readable: g.readable.clamp(0.0, 1.0),
""")
s=sub(s,"let stretch_cap = -0.92 * width /","let stretch_cap = -0.92 * t.len * width /")
s=sub(s,"    const FAN: f32 = 0.7;\n","    let fan_a = t.fan;\n")
s=sub(s,"(FAN * s * ny).sin_cos()","(fan_a * s * ny).sin_cos()")
# the bright line along the edge
s=sub(s,"    let shade = |x: f32, y: f32| -> u32 {\n","""    // The light along the edge: a thin bright line right at the outer boundary, stronger in the
    // corners, fading out up the sides with the glass. `lift` lays that white over a finished pixel.
    let hw = 1.8 * sc;
    let light = |x: f32, y: f32| -> f32 {
        if t.edge_light <= 0.0 {
            return 0.0;
        }
        let (d, _, _, corner) = geom(x, y);
        if d >= hw || d <= -1.0 {
            return 0.0;
        }
        let f = 1.0 - d.max(0.0) / hw;
        let up = smooth(0.0, 0.5, (y - top) / reach);
        (t.edge_light * f * f.sqrt() * (1.0 + 0.6 * corner) * up).clamp(0.0, 1.0)
    };
    let lift = |p: u32, h: f32| -> u32 {
        let (m, w) = (1.0 - h, 255.0 * h);
        let c = |sft: u32| -> u32 { (((p >> sft) & 255) as f32 * m + w).round().min(255.0) as u32 };
        (c(24) << 24) | (c(16) << 16) | (c(8) << 8) | c(0)
    };
    let shade = |x: f32, y: f32| -> u32 {
""")
s=sub(s,"""        (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(rc) << 16) | (ch(gc) << 8) | ch(bc)
    };""","""        let out = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(rc) << 16) | (ch(gc) << 8) | ch(bc);
        let hl = light(x, y);
        if hl > 0.0 {
            lift(out, hl)
        } else {
            out
        }
    };""")
s=sub(s,"""        let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
        let a24 = ((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24;
        let col0 = x0 - shot.ox;""","""        let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
        let a24 = ((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24;
        // the edge light is the same along the whole row here
        let hl = light(xr, y);
        let col0 = x0 - shot.ox;""")
s=sub(s,"""                *o = a24 | (ch(((p >> 16) & 255) as f32) << 16) | (ch(((p >> 8) & 255) as f32) << 8) | ch((p & 255) as f32);
""","""                let v = a24 | (ch(((p >> 16) & 255) as f32) << 16) | (ch(((p >> 8) & 255) as f32) << 8) | ch((p & 255) as f32);
                *o = if hl > 0.0 { lift(v, hl) } else { v };
""")
s=sub(s,"""                    v |= ch(lo + (hi - lo) * ty) << sft;
                }
                *o = v;""","""                    v |= ch(lo + (hi - lo) * ty) << sft;
                }
                *o = if hl > 0.0 { lift(v, hl) } else { v };""")
# frost and the dimming for a bright background, over the finished picture
s=sub(s,"    Some(Rim { x0, y0, w, h, step: st, px })\n","""    // Frost and dimming: a light veil of white over the glass, and a little less light when the
    // picture behind the notch is bright (white text then keeps its contrast).
    let dim = if t.readable > 0.0 && t.mid > 0.0 { backdrop_dim(shot, sh) * t.readable } else { 0.0 };
    let keep = ((1.0 - dim) * 256.0).round().clamp(0.0, 256.0) as u32;
    let veil = (t.tint * 0.30 * 256.0).round().clamp(0.0, 256.0) as u32;
    if keep < 256 || veil > 0 {
        par_rows(&mut px, w, threads, |_, chunk| {
            for p in chunk.iter_mut() {
                let a = *p >> 24;
                if a == 0 {
                    continue;
                }
                let add = (a * veil) >> 8;
                let c = |sft: u32| -> u32 { (((((*p >> sft) & 255) * keep) >> 8) + add).min(255) };
                *p = (a << 24) | (c(16) << 16) | (c(8) << 8) | c(0);
            }
        });
    }
    Some(Rim { x0, y0, w, h, step: st, px })
""")
s=sub(s,"/// Pictures smaller than this are worked out on the one thread","""/// How much to dim the glass for the picture behind the notch: 0 for a dark or middling one, up to
/// 0.45 for a bright one. Looks at a sparse grid of the copy over the notch.
fn backdrop_dim(shot: &Shot, sh: Shape) -> f32 {
    let (x0, x1) = ((sh.left as i32 - shot.ox).max(0), (sh.right as i32 - shot.ox).min(shot.w as i32));
    let (y0, y1) = ((sh.top as i32 - shot.oy).max(0), (sh.bottom as i32 - shot.oy).min(shot.h as i32));
    if x1 - x0 < 8 || y1 - y0 < 8 {
        return 0.0;
    }
    let (sx, sy) = (((x1 - x0) / 32).max(1) as usize, ((y1 - y0) / 16).max(1) as usize);
    let (mut sum, mut n) = (0.0_f32, 0u32);
    for y in (y0..y1).step_by(sy) {
        let row = &shot.px[y as usize * shot.w..(y as usize + 1) * shot.w];
        for x in (x0..x1).step_by(sx) {
            let p = row[x as usize];
            sum += 0.2126 * ((p >> 16) & 255) as f32 + 0.7152 * ((p >> 8) & 255) as f32 + 0.0722 * (p & 255) as f32;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    0.45 * smooth(0.55, 0.92, sum / (n as f32 * 255.0))
}

/// Pictures smaller than this are worked out on the one thread""")
open(p,'w').write(s)

# ---------------------------------------------------------------- the little glass disc behind icons
p='src/alcove.rs'; s=open(p).read()
s=sub(s,"""/// A page title: a small round icon, the name and a dim note beside it.
pub fn title(ui: &mut Ui, r: Rect, icon: Icon, tint: Color, name: &str, note: &str) {
    let d = r.h.min(30.0);
    let c = Rect::new(r.x, r.cy() - d * 0.5, d, d);
    ui.p.icon(icon, c.inset(d * 0.2), tint);
""","""/// A small disc of frosted glass for an icon to sit on: a faint fill, a hairline ring and a bright
/// arc along the top where the light catches it. The ring and the arc brighten under the pointer.
/// Only in the monochrome look; nothing about it is coloured.
pub fn icon_chip(ui: &mut Ui, c: Rect, key: usize) {
    if !crate::ui::mono_icons() {
        return;
    }
    let h = ui.glide(gkey("alcove-chip-glint", key), if ui.hovered(c) { 1.0 } else { 0.0 });
    if h > 0.01 && h < 0.99 {
        ui.animate();
    }
    let (cx, cy, rad) = (c.cx(), c.cy(), c.w.min(c.h) * 0.5);
    ui.p.fill_circle(cx, cy, rad, Color::white(0.07 + 0.05 * h));
    ui.p.stroke_circle(cx, cy, rad - 0.5, 1.0, Color::white(0.10 + 0.10 * h));
    ui.p.arc(cx, cy, rad - 0.5, -1.15, 2.3, 1.4, Color::white(0.34 + 0.30 * h));
    ui.p.arc(cx, cy, rad - 0.5, std::f32::consts::PI - 0.6, 1.2, 1.2, Color::white(0.10 + 0.08 * h));
}

/// A page title: a small round icon, the name and a dim note beside it.
pub fn title(ui: &mut Ui, r: Rect, icon: Icon, tint: Color, name: &str, note: &str) {
    let d = r.h.min(30.0);
    let c = Rect::new(r.x, r.cy() - d * 0.5, d, d);
    icon_chip(ui, c, 0);
    ui.p.icon(icon, c.inset(d * 0.2), tint);
""")
s=sub(s,"""    let st = TextStyle::new(13.0, color).semibold();
    let (w, _) = ui.p.measure(text, &st);
    let r = Rect::new(right - w - 24.0, cy - 14.0, w + 24.0, 28.0);
    ui.p.fill_rounded(r, 14.0, color.with_a(0.18));""","""    // monochrome: a plain frosted label, white text, no coloured fill
    let mono = crate::ui::mono_icons();
    let (fill, color) = if mono { (Color::white(0.12), Color::white(0.92)) } else { (color.with_a(0.18), color) };
    let st = TextStyle::new(13.0, color).semibold();
    let (w, _) = ui.p.measure(text, &st);
    let r = Rect::new(right - w - 24.0, cy - 14.0, w + 24.0, 28.0);
    ui.p.fill_rounded(r, 14.0, fill);""")
s=sub(s,"""            // just the icon, in the notice's colour, on the plain black
            ui.p.icon(pk.icon, tile_r.inset(5.0), pk.accent.lerp(Color::white(1.0), 0.15));""","""            // just the icon on the plain black (on a little glass disc in the monochrome look)
            icon_chip(ui, tile_r.inset(1.0), 1);
            ui.p.icon(pk.icon, tile_r.inset(5.0), pk.accent.lerp(Color::white(1.0), 0.15));""")
open(p,'w').write(s)

# ---------------------------------------------------------------- settings
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"        1 => 17.0,\n","        1 => 18.0,\n")
s=sub(s,"        _ => 20.0,\n","        _ => 25.0,\n")
s=sub(s,"""            "appearance",
            "lite_motion",
        );
""","""            "appearance",
            "lite_motion",
        );
        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");
""")
s=sub(s,"let rows: [(&str, f32, (f32, f32), String, &str); 18] = [","let rows: [(&str, f32, (f32, f32), String, &str); 23] = [")
s=sub(s,"""                "sharp_zone",
            ),
        ];""","""                "sharp_zone",
            ),
            ("Edge highlight", g.edge_light, (0.0, 1.0), format!("{:.0}%", g.edge_light * 100.0), "edge_light"),
            ("Frost tint", g.tint, (0.0, 1.0), format!("{:.0}%", g.tint * 100.0), "tint"),
            ("Readability dim", g.readable, (0.0, 1.0), format!("{:.0}%", g.readable * 100.0), "readable"),
            ("Stretch lean", g.fan, (0.0, 1.2), format!("{:.2}", g.fan), "fan"),
            ("Stretch length", g.stretch_len, (0.5, 1.6), format!("{:.2}x", g.stretch_len), "stretch_len"),
        ];""")
s=sub(s,'toggle_row(ui, row_rect(body, 18), "Stretch edge (iOS look)"','toggle_row(ui, row_rect(body, 23), "Stretch edge (iOS look)"')
s=sub(s,"        let b = row_rect(body, 19);\n","        let b = row_rect(body, 24);\n")
s=sub(s,'                ("sharp_zone", d.sharp_zone),\n','''                ("sharp_zone", d.sharp_zone),
                ("edge_light", d.edge_light),
                ("tint", d.tint),
                ("readable", d.readable),
                ("fan", d.fan),
                ("stretch_len", d.stretch_len),
''')
open(p,'w').write(s)
PYEOF
echo "Patched v243"
