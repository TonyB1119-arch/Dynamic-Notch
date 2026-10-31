#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "middle_warp" src/config.rs; then echo "already applied"; exit 0; fi
if ! grep -q "pub struct GlassCfg" src/config.rs; then echo "run update_212.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- config.rs ----------------
p='src/config.rs'; s=open(p).read()
s=sub(s,"""    /// How softly the glass starts at the outer edge.
    pub softness: f32,
""","""    /// How softly the glass starts at the outer edge.
    pub softness: f32,
    /// Glass across the middle of the notch, wherever it is even a little see-through (0 = off).
    pub middle: f32,
    /// How much the middle glass bends what is behind it (0 = clear, 1 = like the edge).
    pub middle_warp: f32,
""")
s=sub(s,"            softness: 4.0,\n","            softness: 4.0,\n            middle: 0.85,\n            middle_warp: 0.5,\n")
s=sub(s,"        self.softness = self.softness.clamp(0.5, 12.0);\n","        self.softness = self.softness.clamp(0.5, 12.0);\n        self.middle = self.middle.clamp(0.0, 1.0);\n        self.middle_warp = self.middle_warp.clamp(0.0, 1.0);\n")
s=sub(s,"softness = 4.0\nfade_amount","softness = 4.0\nmiddle = 0.85\nmiddle_warp = 0.5\nfade_amount")
open(p,'w').write(s)

# ---------------- refract.rs ----------------
p='src/refract.rs'; s=open(p).read()
s=sub(s,"    pub reach: f32,\n    pub soft: f32,\n}","    pub reach: f32,\n    pub soft: f32,\n    pub mid: f32,\n    pub mid_warp: f32,\n}")
s=sub(s,"            soft: g.softness.clamp(0.5, 12.0),\n","            soft: g.softness.clamp(0.5, 12.0),\n            mid: g.middle.clamp(0.0, 1.0),\n            mid_warp: g.middle_warp.clamp(0.0, 1.0),\n")
s=sub(s,"    pub scale: f32,\n    pub tune: Tune,\n}","""    pub scale: f32,
    pub tune: Tune,
    /// Top edge of the notch (window pixels).
    pub top: f32,
    /// Height of the see-through strip along the bottom, window pixels (0 = none), and how far the
    /// black thins out at its very bottom (0 - 1).
    pub band: f32,
    pub fade: f32,
    /// How see-through the whole notch is (0 = solid black).
    pub clear: f32,
}""")
# ring_sum: the middle is read as well
s=sub(s,"""    let full_from = (sh.bottom - band as f32).floor() as i32;
    let mut h = 0xcbf2""","""    let mid = t.mid > 0.01 && ((sh.band > 0.0 && sh.fade > 0.004) || sh.clear > 0.004);
    let int_top = if sh.clear > 0.004 { sh.top } else { sh.bottom - sh.band };
    let (y0, full_from) = if mid {
        (y0.min(((int_top - pad).floor() as i32).clamp(0, s.h as i32)), (sh.bottom - band as f32).min(int_top).floor() as i32)
    } else {
        (y0, (sh.bottom - band as f32).floor() as i32)
    };
    let mut h = 0xcbf2""")
# rim: whole function replaced
a=s.index("pub fn rim(shot: &Shot, sh: Shape, step: usize) -> Option<Rim> {")
endm="    Some(Rim { x0, y0, w, h, step: st, px })\n}\n"
b=s.index(endm,a)+len(endm)
new='''pub fn rim(shot: &Shot, sh: Shape, step: usize) -> Option<Rim> {
    let st = step.max(1);
    let sc = sh.scale.max(0.5);
    let t = sh.tune;
    let width = t.width * sc; // how far in from the edge the rim reaches
    let pull = t.pull * sc; // how far out the strongest pixels look
    let soft = t.soft * sc;
    let even_bend = (t.bend - 1.5).abs() < 0.001;
    let r = sh.radius.max(2.0 * sc).min((sh.right - sh.left) * 0.5);
    let reach = r + t.reach * sc; // how far up the sides it is drawn, fading out toward the top
    // The middle: wherever the notch is even a little see-through (the strip along the bottom, or
    // the whole notch when the panel opacity is below 100 %) the glass carries on across it.
    let mid = t.mid > 0.01 && ((sh.band > 0.0 && sh.fade > 0.004) || sh.clear > 0.004);
    let int_top = if sh.clear > 0.004 { sh.top } else { sh.bottom - sh.band };
    let top_y = if mid { (sh.bottom - reach).min(int_top) } else { sh.bottom - reach };
    let x0 = ((sh.left - 2.0).floor() as i32).max(0);
    let x1 = ((sh.right + 2.0).ceil() as i32).min(shot.w as i32);
    let y0 = (top_y.floor() as i32).max(0);
    let y1 = ((sh.bottom + 2.0).ceil() as i32).min(shot.h as i32);
    if x1 - x0 < 8 || y1 - y0 < 4 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let (w, h) = (w.div_ceil(st), h.div_ceil(st));
    let half = st as f32 * 0.5;
    let mut px = vec![0u32; w * h];
    let (cl, cr, cb) = (sh.left + r, sh.right - r, sh.bottom - r);
    let top = sh.bottom - reach;
    // Rows higher up only carry the rim along the two sides, so the middle is skipped there;
    // from `full_from` down the whole row is drawn. The corners reach further in than the sides.
    let full_from = if mid { (sh.bottom - width - 1.0).min(int_top) } else { sh.bottom - width - 1.0 };
    let side = ((width.max(r) + 2.0).ceil() as usize).div_ceil(st);
    let w2 = width + 70.0 * sc; // how far in the middle's gentle bend fades out
    let band = sh.band.max(1.0);
    let row = |j: usize, out: &mut [u32]| {
        let y = (y0 + (j * st) as i32) as f32 + half;
        let up = smooth(0.0, 0.5, (y - top) / reach);
        let in_mid = mid && y >= int_top;
        if up <= 0.0 && !in_mid {
            return;
        }
        // how much of the middle shows on this row: the black thins toward the bottom edge
        let a_row = if in_mid {
            let bt = ((y - (sh.bottom - sh.band)) / band).clamp(0.0, 1.0);
            let e = bt * bt * bt * (bt * (bt * 6.0 - 15.0) + 10.0);
            (sh.fade * e).max(sh.clear) * t.mid
        } else {
            0.0
        };
        let (a_end, b_start) = if y >= full_from { (w, w) } else { (side.min(w), w.saturating_sub(side).max(side.min(w))) };
        for i in (0..a_end).chain(b_start..w) {
            let x = (x0 + (i * st) as i32) as f32 + half;
            // distance inside the edge, and the direction pointing out of the notch
            let d;
            let nx;
            let ny;
            let mut corner = 0.0;
            if y > cb && (x < cl || x > cr) {
                let cx = if x < cl { cl } else { cr };
                let (dx, dy) = (x - cx, y - cb);
                let dist = (dx * dx + dy * dy).sqrt().max(0.001);
                d = r - dist;
                nx = dx / dist;
                ny = dy / dist;
                // 0 where the curve meets the straight edges, 1 in the middle of the curve (no seam)
                corner = (2.0 * dx.abs() * dy / (dist * dist)).clamp(0.0, 1.0);
            } else {
                let (dl, dr, db) = (x - sh.left, sh.right - x, sh.bottom - y);
                if db <= dl && db <= dr {
                    d = db;
                    nx = 0.0;
                    ny = 1.0;
                } else if dl <= dr {
                    d = dl;
                    nx = -1.0;
                    ny = 0.0;
                } else {
                    d = dr;
                    nx = 1.0;
                    ny = 0.0;
                }
            }
            if d <= -1.0 {
                continue;
            }
            let (mut alpha, mut ox, mut oy) = (0.0_f32, 0.0_f32, 0.0_f32);
            if d < width && up > 0.0 {
                // u: 1 at the edge, 0 at the inner end of the rim. The glass is strongest right at the
                // edge, and strongest of all in the corners, then eases away quickly so the black of the
                // notch takes over; the tail is smooth, with no visible line where the rim stops.
                let u = 1.0 - d / width;
                let fade = u * u * (1.5 - 0.5 * u);
                let base = (t.strength + 0.1 * corner).min(1.0);
                alpha = (base + (1.0 - base) * smooth(soft, 0.0, d)) * fade * (d + 1.0).clamp(0.0, 1.0) * up;
                let k = if even_bend { u * u.sqrt() } else { u.powf(t.bend) };
                let p = pull * (1.0 + t.corner * corner);
                ox = nx * p * k;
                oy = ny * p * k;
            }
            if a_row > 0.0 {
                // the middle lies under the rim; it bends gently toward the nearest edges
                let ai = a_row * (d + 1.0).clamp(0.0, 1.0);
                alpha += ai * (1.0 - alpha);
                if d > width && t.mid_warp > 0.0 {
                    let wi = smooth(width, width * 1.8, d);
                    let u2 = (1.0 - d / w2).clamp(0.0, 1.0);
                    let k2 = if even_bend { u2 * u2.sqrt() } else { u2.powf(t.bend) };
                    // a soft blend of the nearest edges' directions: no crease where they meet
                    let wl = 1.0 / (x - sh.left + 6.0).max(1.0).powi(3);
                    let wr = 1.0 / (sh.right - x + 6.0).max(1.0).powi(3);
                    let wb = 1.0 / (sh.bottom - y + 6.0).max(1.0).powi(3);
                    let inv = 1.0 / (wl + wr + wb);
                    let m = pull * t.mid_warp * k2 * wi;
                    ox += (wr - wl) * inv * m;
                    oy += wb * inv * m;
                }
            }
            if alpha < 0.004 {
                continue;
            }
            // red, green and blue look a slightly different distance out, like light through glass
            let cr = sample(shot, x + ox, y + oy, 16);
            let cg = sample(shot, x + ox * (1.0 - t.fringe), y + oy * (1.0 - t.fringe), 8);
            let cb2 = sample(shot, x + ox * (1.0 - 2.0 * t.fringe), y + oy * (1.0 - 2.0 * t.fringe), 0);
            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
            out[i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);
        }
    };
    // a big picture (the finest detail, or a wide middle) is split into bands of rows worked out
    // side by side; small ones are not worth the hand-over
    let threads = if w * h >= 60_000 { workers() } else { 1 };
    if threads > 1 {
        let per = h.div_ceil(threads);
        let row = &row;
        std::thread::scope(|scope| {
            for (n, chunk) in px.chunks_mut(per * w).enumerate() {
                scope.spawn(move || {
                    for (jj, out) in chunk.chunks_mut(w).enumerate() {
                        row(n * per + jj, out);
                    }
                });
            }
        });
    } else {
        for (j, out) in px.chunks_mut(w).enumerate() {
            row(j, out);
        }
    }
    Some(Rim { x0, y0, w, h, step: st, px })
}

/// How many threads a big rim may use (looked up once).
fn workers() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4))
}
'''
s=s[:a]+new+s[b:]
open(p,'w').write(s)

# ---------------- app.rs ----------------
p='src/app.rs'; s=open(p).read()
s=sub(s,"    rim_tune: Option<crate::refract::Tune>,\n","    rim_tune: Option<crate::refract::Tune>,\n    /// The strip / see-through settings the current rim was made with (quantised).\n    rim_extra: Option<(i32, i32, i32)>,\n")
s=sub(s,"            rim_tune: None,\n","            rim_tune: None,\n            rim_extra: None,\n")
s=sub(s,"""        let shape = crate::refract::Shape {
            left: body.x * sc,""","""        // where the notch is see-through: the strip along the bottom and, below 100 % panel opacity,
        // all of it (the middle glass follows that)
        let ap = &self.cfg.appearance;
        let bhh = BAND_H.min(body.h * 0.4);
        let band_on = ap.bottom_fade && ap.blur && self.page != "settings" && body.h > bhh * 2.0;
        let opaque = if ap.blur { ap.expanded_opacity } else { ap.expanded_opacity.max(0.95) };
        let band_px = if band_on { bhh * self.cfg.glass.fade_start.clamp(0.5, 3.0) * sc } else { 0.0 };
        let fade_to = if band_on { self.cfg.glass.fade_amount.clamp(0.0, 1.0) } else { 0.0 };
        let clear = (1.0 - opaque).clamp(0.0, 1.0);
        let extra = ((band_px * 4.0) as i32, (fade_to * 1000.0) as i32, (clear * 1000.0) as i32);
        if self.rim_extra != Some(extra) {
            self.rim_extra = Some(extra);
            self.refr_rim = None;
            self.rim_old = None;
            self.live_sum = 0;
        }
        let shape = crate::refract::Shape {
            top: body.y * sc,
            band: band_px,
            fade: fade_to,
            clear,
            left: body.x * sc,""")
# the glass now lies under the content (so buttons stay sharp over the see-through middle)
st="                if glass && self.rim_k > 0.003 && body.h >= 56.0 && a_glass > 0.01 {\n"
i=s.index(st); assert s.count(st)==1
endm="                }\n                p.pop_clip();\n"
j=s.index(endm,i)
block=s[i:j+len("                }\n")]
s=s[:i]+s[j+len("                }\n"):]
anchor="                p.push_clip(body);\n                let input = self.input.clone();\n"
assert s.count(anchor)==1
s=s.replace(anchor,"                // the glass lies under the content, so buttons and text stay sharp over it\n                p.push_clip(body);\n"+block+"                p.pop_clip();\n\n"+anchor)
open(p,'w').write(s)

# ---------------- settings.rs ----------------
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"        _ => 15.0,","        _ => 17.0,")
s=sub(s,"(&str, f32, (f32, f32), String, &str); 14]","(&str, f32, (f32, f32), String, &str); 16]")
s=sub(s,"""            ("Bottom fade amount", g.fade_amount""","""            ("Middle glass", g.middle, (0.0, 1.0), format!("{:.0}%", g.middle * 100.0), "middle"),
            ("Middle warp", g.middle_warp, (0.0, 1.0), format!("{:.0}%", g.middle_warp * 100.0), "middle_warp"),
            ("Bottom fade amount", g.fade_amount""")
s=sub(s,"let b = row_rect(body, 14);","let b = row_rect(body, 16);")
s=sub(s,"                (\"softness\", d.softness),\n","                (\"softness\", d.softness),\n                (\"middle\", d.middle),\n                (\"middle_warp\", d.middle_warp),\n")
open(p,'w').write(s)
PYEOF
echo "Patched v213"
