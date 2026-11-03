#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
notes=[]
def get(p): return open(p).read()
def sub(s,a,b,tag):
    if b in s: return s
    if s.count(a)!=1:
        notes.append(tag); return s
    return s.replace(a,b)

# ---- config.rs -------------------------------------------------------------------------------
c=get('src/config.rs'); c0=c
c=sub(c,"    pub rim_bright: f32,\n","    pub rim_bright: f32,\n    /// The bottom of the dock curves outward and flows into the screen edge, like the notch does at the top.\n    pub melt: bool,\n","cfg field")
c=sub(c,"            rim_bright: 0.35,\n","            rim_bright: 0.35,\n            melt: false,\n","cfg default")
c=sub(c,"rim_bright = 0.35\n","rim_bright = 0.35\n# Let the bottom of the dock curve outward and melt into the screen edge (true / false)\nmelt = false\n","cfg template")

# ---- settings.rs -----------------------------------------------------------------------------
t=get('src/modules/settings.rs'); t0=t
t=sub(t,'            "rim_bright",\n        );\n    }\n','            "rim_bright",\n        );\n        toggle_row(ui, row_rect(body, 10), "Melt into the screen edge", d.melt, "dock", "melt");\n    }\n',"settings row")
t=sub(t,"        8 => 10.0,\n","        8 => 11.0,\n","settings rows")

# ---- dock.rs ---------------------------------------------------------------------------------
d=get('src/dock.rs'); d0=d
d=sub(d,"    fn foot(&self) -> f32 {\n        if self.cfg.auto_hide {","    fn foot(&self) -> f32 {\n        if self.cfg.melt {\n            0.0\n        } else if self.cfg.auto_hide {","foot")
d=sub(d,"let lift = if self.cfg.auto_hide { 0 } else","let lift = if self.cfg.auto_hide || self.cfg.melt { 0 } else","lift")
d=sub(d,"self.foot() - FOOT + if self.picker","self.foot().max(FOOT) - FOOT + if self.picker","win_h")
d=sub(d,"        if old.hide_taskbar != cfg.hide_taskbar {","        if old.melt != cfg.melt {\n            self.region_full = None;\n        }\n        if old.hide_taskbar != cfg.hide_taskbar {","configure")
d=sub(d,"        let s = self.scale;\n        let (l, t, r, b) = if mode == 1 {","        let s = self.scale;\n        let e = self.ear();\n        let (l, t, r, b) = if mode == 1 {","region e")
d=sub(d,"((bar.x * s).floor() as i32, hpx - (4.0 * s).ceil() as i32, (bar.right() * s).ceil() as i32, hpx)","(((bar.x - e) * s).floor() as i32, hpx - (4.0 * s).ceil() as i32, ((bar.right() + e) * s).ceil() as i32, hpx)","region 2")
d=sub(d,"((bar.x * s).floor() as i32, (bar.y * s).floor() as i32, (bar.right() * s).ceil() as i32, hpx)","(((bar.x - e) * s).floor() as i32, (bar.y * s).floor() as i32, ((bar.right() + e) * s).ceil() as i32, hpx)","region 0")
d=sub(d,"    /// The bar behind the icons, from the row's current layout.\n","    /// How far the curved feet of the melted dock reach out to each side (0 when it is off).\n    fn ear(&self) -> f32 {\n        if !self.cfg.melt {\n            return 0.0;\n        }\n        let (base, _, _, pad) = self.sizes();\n        ((base + 2.0 * pad) * 0.26).clamp(10.0, 26.0)\n    }\n\n    /// The bar behind the icons, from the row's current layout.\n","ear fn")
d=sub(d,"Rect::new(bar.x * sc, (bar.y + dy) * sc, bar.w * sc, bar.h * sc),","Rect::new(bar.x * sc, (bar.y + dy) * sc, bar.w * sc, (bar.h + if self.cfg.melt { radius } else { 0.0 }) * sc),","backdrop")
d=sub(d,"            let panel = self.panel_rect();\n","            let panel = self.panel_rect();\n            let ear = self.ear();\n","draw ear")
d=sub(d,"                p.stroke_rounded(bar.inset(rw * 0.5), (radius - rw * 0.5).max(0.0), rw, Color::white(self.cfg.rim_bright));\n","                if self.cfg.melt {\n                    if let Some(g) = melt_ring(&p, bar, radius, ear, rw) {\n                        p.fill_geometry(&g, Color::white(self.cfg.rim_bright));\n                    }\n                } else {\n                    p.stroke_rounded(bar.inset(rw * 0.5), (radius - rw * 0.5).max(0.0), rw, Color::white(self.cfg.rim_bright));\n                }\n","rim")
ring='''/// The white rim of the melted dock as one filled outline: round top corners, straight sides and
/// concave feet that flare out and end flat on the screen edge (the notch's shape, upside down).
/// The outline continues below the window, so no line shows along the screen edge.
fn melt_ring(p: &Painter, bar: Rect, r: f32, e: f32, w: f32) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Geometry> {
    let (x0, x1, t, b) = (bar.x, bar.right(), bar.y, bar.bottom());
    let h = w * 0.5;
    let e = e.min(bar.h * 0.5).max(h + 0.5);
    let r = r.min(bar.w * 0.5).min(bar.h - e).max(h + 0.5);
    p.path(|g| {
        g.move_to(x0 - e, b - h);
        g.arc_to(x0 - h, b - e, e - h, false);
        g.line_to(x0 - h, t + r);
        g.arc_to(x0 + r, t - h, r + h, true);
        g.line_to(x1 - r, t - h);
        g.arc_to(x1 + h, t + r, r + h, true);
        g.line_to(x1 + h, b - e);
        g.arc_to(x1 + e, b - h, e - h, false);
        g.line_to(x1 + e, b + h);
        g.arc_to(x1 - h, b - e, e + h, true);
        g.line_to(x1 - h, t + r);
        g.arc_to(x1 - r, t + h, r - h, false);
        g.line_to(x0 + r, t + h);
        g.arc_to(x0 + h, t + r, r - h, false);
        g.line_to(x0 + h, b - e);
        g.arc_to(x0 - e, b + h, e + h, true);
        g.close();
    })
}

'''
if "fn melt_ring" not in d:
    d=sub(d,"// -- open windows",ring+"// -- open windows","melt_ring")

if notes:
    print("NOTHING CHANGED - these spots in your code look different:")
    for n in notes: print("  -",n)
else:
    open('src/config.rs','w').write(c); open('src/modules/settings.rs','w').write(t); open('src/dock.rs','w').write(d)
    print("ok")
PYEOF
echo "update_268 done"
