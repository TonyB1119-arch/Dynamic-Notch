#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "icon_square" src/config.rs; then echo "already applied"; exit 0; fi
if ! grep -q "glint_arc" src/alcove.rs; then echo "run update_244.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
def rw(p,f):
    s=open(p).read(); s=f(s); open(p,'w').write(s)

# ---- config: appearance.icon_square -----------------------------------------------------------
def cfg(s):
    s=sub(s,"""    pub mono_icons: bool,
""","""    pub mono_icons: bool,
    /// The soft badge behind each page icon is a rounded square (true) or a circle (false).
    pub icon_square: bool,
""")
    s=sub(s,"            mono_icons: true,\n","            mono_icons: true,\n            icon_square: true,\n")
    s=sub(s,"mono_icons = true\n","""mono_icons = true
# Shape of the soft badge behind each page icon: true = rounded square, false = circle
icon_square = true
""")
    return s
rw('src/config.rs',cfg)

# ---- ui: the switch ---------------------------------------------------------------------------
def ui(s):
    return sub(s,"""pub fn mono_icons() -> bool {
    MONO_ICONS.load(std::sync::atomic::Ordering::Relaxed)
}
""","""pub fn mono_icons() -> bool {
    MONO_ICONS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Icon badges are rounded squares (`[appearance] icon_square`) instead of circles.
static ICON_SQUARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_icon_square(on: bool) {
    ICON_SQUARE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn icon_square() -> bool {
    ICON_SQUARE.load(std::sync::atomic::Ordering::Relaxed)
}
""")
rw('src/ui.rs',ui)

def app(s):
    s=sub(s,"        crate::ui::set_mono_icons(cfg.appearance.mono_icons);\n","        crate::ui::set_mono_icons(cfg.appearance.mono_icons);\n        crate::ui::set_icon_square(cfg.appearance.icon_square);\n")
    s=sub(s,"                crate::ui::set_mono_icons(self.cfg.appearance.mono_icons);\n","                crate::ui::set_mono_icons(self.cfg.appearance.mono_icons);\n                crate::ui::set_icon_square(self.cfg.appearance.icon_square);\n")
    return s
rw('src/app.rs',app)

# ---- settings: Look tab toggle ------------------------------------------------------------------
def st(s):
    s=sub(s,"""        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");
""","""        toggle_row(ui, row_rect(body, 17), "Monochrome icons", a.mono_icons, "appearance", "mono_icons");
        toggle_row(ui, row_rect(body, 18), "Rounded-square icon badges (off = circles)", a.icon_square, "appearance", "icon_square");
""")
    s=sub(s,"        1 => 18.0,\n","        1 => 19.0,\n")
    return s
rw('src/modules/settings.rs',st)

# ---- gfx: arcs drawn from the realization cache -------------------------------------------------
def gfx(s):
    i=s.index("    pub fn arc(&self, cx: f32")
    a=s.index("        let end = start + sweep;\n        let h = width * 0.5;",i)
    b=s.index("        }) {\n            self.fill_geometry(&g, c);",a)
    s=s[:a]+"        if let Some(g) = self.path(|b| b.arc_stroke(cx, cy, radius, start, sweep, width)) {\n"+s[b+len("        }) {\n"):]
    s=sub(s,"""    /// Closed rounded-rect figure. Opposite windings punch holes (winding fill).
""","""    /// The outline of a stroked arc with round caps (`start` clockwise from 12 o'clock, `sweep` > 0).
    pub fn arc_stroke(&self, cx: f32, cy: f32, radius: f32, start: f32, sweep: f32, width: f32) {
        let end = start + sweep;
        let h = width * 0.5;
        let pt = |a: f32, r: f32| (cx + r * a.sin(), cy - r * a.cos());
        let large = sweep > std::f32::consts::PI;
        let ri = (radius - h).max(0.0);
        let (x, y) = pt(start, radius + h);
        self.move_to(x, y);
        let (x, y) = pt(end, radius + h);
        self.arc_to_ex(x, y, radius + h, true, large);
        let (x, y) = pt(end, ri);
        self.arc_to(x, y, h, true); // end cap
        let (x, y) = pt(start, ri);
        self.arc_to_ex(x, y, ri, false, large);
        let (x, y) = pt(start, radius + h);
        self.arc_to(x, y, h, true); // start cap
        self.close();
    }

    /// Closed rounded-rect figure. Opposite windings punch holes (winding fill).
""")
    s=sub(s,"""    /// Ring of `width` centered on a circle of `radius`.
""","""    /// `arc` for arcs that are drawn every frame (a badge's corner highlights): the outline is built
    /// once per size and angle (angles to 1/256 rad) and later frames only move and tint it.
    #[allow(clippy::too_many_arguments)]
    pub fn arc_cached(&self, cx: f32, cy: f32, radius: f32, start: f32, sweep: f32, width: f32, c: Color) {
        if sweep.abs() < 0.001 || c.a * self.alpha <= 0.001 {
            return;
        }
        if sweep.abs() >= std::f32::consts::TAU - 0.001 {
            self.stroke_circle(cx, cy, radius, width, c);
            return;
        }
        let (start, sweep) = if sweep < 0.0 { (start + sweep, -sweep) } else { (start, sweep) };
        let ang = |v: f32| (v * 256.0).round() as i32;
        let key = (31, q(radius), ang(start), ang(sweep), q(width));
        self.fill_cached(key, cx, cy, c, |b| {
            b.arc_stroke(0.0, 0.0, uq(key.1), key.2 as f32 / 256.0, key.3 as f32 / 256.0, uq(key.4))
        });
    }

    /// Ring of `width` centered on a circle of `radius`.
""")
    return s
rw('src/gfx/mod.rs',gfx)

# ---- alcove: one badge for every page, and the Wi-Fi / Bluetooth selected tab ---------------------
def alc(s):
    i=s.index("pub fn icon_chip(")
    # start of its doc comment
    ds=s.rfind("\n\n",0,i)+2
    e=s.index("/// A page title:",i)
    new='''/// The soft badge behind a page's icon, the same on every page: a quiet fill, a hairline ring and
/// two short white highlights on the top-right and bottom-left corners. A rounded square, or a
/// circle when `[appearance] icon_square` is off. It brightens under the pointer. Monochrome look only.
pub fn icon_chip(ui: &mut Ui, c: Rect, key: usize) {
    if !crate::ui::mono_icons() {
        return;
    }
    let h = ui.glide(gkey("alcove-chip-glint", key), if ui.hovered(c) { 1.0 } else { 0.0 });
    if h > 0.01 && h < 0.99 {
        ui.animate();
    }
    let side = c.w.min(c.h);
    let radius = if crate::ui::icon_square() { side * 0.30 } else { side * 0.5 };
    chip_body(ui, c, radius, h, 1.0);
}

/// The badge itself: fill, hairline ring and the two corner highlights, all scaled by `k` (0 - 1).
/// `h` is the pointer glow. The highlights are arcs of the corner circles, so they follow a round
/// badge, a rounded square or the end of a capsule alike, and come from the realization cache.
fn chip_body(ui: &mut Ui, c: Rect, radius: f32, h: f32, k: f32) {
    let radius = radius.min(c.w * 0.5).min(c.h * 0.5);
    ui.p.fill_rounded(c, radius, Color::white((0.07 + 0.05 * h) * k));
    ui.p.stroke_rounded(c.inset(0.5), radius - 0.5, 1.0, Color::white((0.045 + 0.05 * h) * k));
    let peak = (0.22 + 0.10 * h) * k;
    if peak <= 0.004 {
        return;
    }
    // each highlight is a few arcs laid over each other, shorter and shorter, so the light is
    // brightest in the middle and fades out quickly toward both ends; a rounded square has less
    // corner to light, so its layers run closer to the full quarter turn
    let round = radius >= c.w.min(c.h) * 0.5 - 0.01;
    let sweeps: [f32; 4] = if round { [1.2, 0.9, 0.6, 0.3] } else { [1.5, 1.12, 0.75, 0.38] };
    let r = (radius - 0.5).max(1.0);
    let quarter = std::f32::consts::FRAC_PI_4;
    let corners = [(c.right() - radius, c.y + radius, quarter), (c.x + radius, c.bottom() - radius, quarter + std::f32::consts::PI)];
    for (cx, cy, mid) in corners {
        for sw in sweeps {
            ui.p.arc_cached(cx, cy, r, mid - sw * 0.5, sw, 1.5, Color::white(peak));
        }
    }
}

/// A Wi-Fi / Bluetooth style tab. The chosen one is the badge (fill, ring, corner highlights, white
/// text); the others are plain, dim text on the notch itself. Without monochrome icons it is the
/// ordinary white-when-chosen `chip`.
pub fn tab_chip(ui: &mut Ui, r: Rect, label: &str, icon: Option<Icon>, on: bool) -> bool {
    if !crate::ui::mono_icons() {
        return chip(ui, r, label, icon, on);
    }
    let (h, p) = ui.touch(r, "alcove-tab-touch");
    let a = ui.glide(crate::ui::rect_key(r, "alcove-tab"), if on { 1.0 } else { 0.0 });
    let rr = r.inset(1.2 * p);
    if a < 0.99 {
        ui.p.fill_rounded(rr, rr.h * 0.5, Color::white(0.05 * h * (1.0 - a)));
    }
    if a > 0.01 {
        chip_body(ui, rr, rr.h * 0.5, h, a);
    }
    let fg = Color::white(0.52 + 0.14 * h * (1.0 - a)).lerp(Color::white(0.97), a);
    let st = TextStyle::new(12.5, fg).semibold();
    let mut x = r.x + 13.0;
    if let Some(i) = icon {
        ui.p.icon(i, Rect::new(x, r.cy() - 7.0, 14.0, 14.0), fg);
        x += 20.0;
    } else {
        let (tw, _) = ui.p.measure(label, &st);
        x = r.x + ((r.w - tw) * 0.5).max(6.0);
    }
    ui.p.text(label, Rect::new(x, r.y, (r.right() - x).max(0.0), r.h), &st);
    ui.clicked(r)
}

'''
    s=s[:ds]+new+s[e:]
    # Wi-Fi / Bluetooth status icon gets the same badge
    s=sub(s,"""    let c = Rect::new(r.x + 7.0, r.y + 7.0, d, d);
    ui.p.icon(icon, c.inset(d * 0.2), tint);
    let mut right = r.right() - 12.0;""","""    let c = Rect::new(r.x + 7.0, r.y + 7.0, d, d);
    icon_chip(ui, c, 4);
    ui.p.icon(icon, c.inset(d * 0.2), tint);
    let mut right = r.right() - 12.0;""")
    return s
rw('src/alcove.rs',alc)

def ctl(s):
    return sub(s,"                if crate::alcove::chip(ui, b, label, Some(*icon), self.tab == i) {","                if crate::alcove::tab_chip(ui, b, label, Some(*icon), self.tab == i) {")
rw('src/modules/control.rs',ctl)

def gem(s):
    s=sub(s,"""        let badge = Rect::new(r.x + 20.0, r.y + 20.0, 46.0, 46.0);
        ui.p.icon(""","""        let badge = Rect::new(r.x + 20.0, r.y + 20.0, 46.0, 46.0);
        crate::alcove::icon_chip(ui, badge, 2);
        ui.p.icon(""")
    s=sub(s,"""            ui.p.icon(
                Icon::Sparkle,
                Rect::centered(cx, area.cy() - 34.0, 24.0, 24.0),""","""            crate::alcove::icon_chip(ui, Rect::centered(cx, area.cy() - 34.0, 38.0, 38.0), 3);
            ui.p.icon(
                Icon::Sparkle,
                Rect::centered(cx, area.cy() - 34.0, 22.0, 22.0),""")
    return s
rw('src/modules/gemini.rs',gem)

def tm(s):
    return sub(s,"""        ui.p.icon(Icon::Timer, Rect::new(inner.x, r.cy() - is * 0.5, is, is), color);
        let label_x = inner.x + is + 8.0;""","""        // in the Alcove look the icon sits on the same soft badge as the other pages
        let disc = crate::ui::look_alcove() && crate::ui::mono_icons();
        let lead = if disc { 26.0 } else { is };
        if disc {
            let c = Rect::new(inner.x, r.cy() - 13.0, 26.0, 26.0);
            crate::alcove::icon_chip(ui, c, 5);
            ui.p.icon(Icon::Timer, c.inset(5.0), color);
        } else {
            ui.p.icon(Icon::Timer, Rect::new(inner.x, r.cy() - is * 0.5, is, is), color);
        }
        let label_x = inner.x + lead + 8.0;""")
rw('src/modules/timer.rs',tm)
PYEOF
echo "Patched v245"
