#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "sharp_zone" src/refract.rs; then echo "already applied"; exit 0; fi
if [ -f src/dock.rs ]; then echo "run update_216.sh first"; exit 1; fi
if grep -q "fn crop_for" src/refract.rs; then echo "update_217 is already in; run update_218.sh on its own"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/refract.rs'; s=open(p).read()
# Shot remembers where it sits in the window
s=sub(s,"""pub struct Shot {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}""","""pub struct Shot {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
    /// Where the copy's top-left corner lies in the notch window (0, 0 for a copy of all of it).
    pub ox: i32,
    pub oy: i32,
    /// A copy of the whole window (a live refresh copies only the part the glass can read).
    pub full: bool,
    /// For a partial copy: the notch shape it was made for.
    pub made_for: Option<Shape>,
}""")
s=sub(s,"""/// Copy the screen area at (`x`, `y`) of size `w` x `h` (screen pixels). None if it cannot be had.
pub fn grab(x: i32, y: i32, w: i32, h: i32) -> Option<Shot> {
""","""/// Copy the screen area at (`x`, `y`) of size `w` x `h` (screen pixels): the whole notch window.
/// None if it cannot be had.
pub fn grab(x: i32, y: i32, w: i32, h: i32) -> Option<Shot> {
    grab_at(x, y, w, h, 0, 0).map(|mut s| {
        s.full = true;
        s
    })
}

/// Copy a piece of the screen that sits at (`ox`, `oy`) in the notch window.
fn grab_at(x: i32, y: i32, w: i32, h: i32, ox: i32, oy: i32) -> Option<Shot> {
""")
s=sub(s,"shot = Some(Shot { w: w as usize, h: h as usize, px });","shot = Some(Shot { w: w as usize, h: h as usize, px, ox, oy, full: false, made_for: None });")
# reading pixels: window coordinates minus the copy's origin
s=sub(s,"    let (gx, gy) = (fx - 0.5, fy - 0.5);\n","    let (gx, gy) = (fx - s.ox as f32 - 0.5, fy - s.oy as f32 - 0.5);\n",2)
# ring_sum
a=s.index("    let x0 = ((sh.left - pad).floor() as i32).clamp(0, s.w as i32);")
b=s.index("    h\n}\n",a)+len("    h\n}\n")
s=s[:a]+'''    let (sx0, sy0) = (s.ox, s.oy);
    let (sx1, sy1) = (s.ox + s.w as i32, s.oy + s.h as i32);
    let x0 = ((sh.left - pad).floor() as i32).clamp(sx0, sx1);
    let x1 = ((sh.right + pad).ceil() as i32).clamp(sx0, sx1);
    let y0 = ((sh.bottom - r - t.reach * sc - pad).floor() as i32).clamp(sy0, sy1);
    let y1 = ((sh.bottom + pad).ceil() as i32).clamp(sy0, sy1);
    let mid = t.mid > 0.01 && ((sh.band > 0.0 && sh.fade > 0.004) || sh.clear > 0.004);
    let int_top = if sh.clear > 0.004 { sh.top } else { sh.bottom - sh.band };
    let (y0, full_from) = if mid {
        (y0.min(((int_top - pad).floor() as i32).clamp(sy0, sy1)), (sh.bottom - band as f32).min(int_top).floor() as i32)
    } else {
        (y0, (sh.bottom - band as f32).floor() as i32)
    };
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |p: u32| h = (h ^ (p & 0x00FF_FFFF) as u64).wrapping_mul(0x0000_0100_0000_01b3);
    for y in (y0..y1).step_by(2) {
        let ly = (y - sy0) as usize;
        let row = &s.px[ly * s.w..(ly + 1) * s.w];
        if y >= full_from {
            row[(x0 - sx0) as usize..(x1 - sx0) as usize].iter().step_by(3).for_each(|&p| mix(p));
        } else {
            let left_end = (x0 + band).min(x1);
            let right_start = (x1 - band).max(left_end);
            row[(x0 - sx0) as usize..(left_end - sx0) as usize].iter().step_by(3).for_each(|&p| mix(p));
            row[(right_start - sx0) as usize..(x1 - sx0) as usize].iter().step_by(3).for_each(|&p| mix(p));
        }
    }
    h
}
'''+s[b:]
# rim bounds
s=sub(s,"""    let x0 = ((sh.left - 2.0).floor() as i32).max(0);
    let x1 = ((sh.right + 2.0).ceil() as i32).min(shot.w as i32);
    let y0 = (top_y.floor() as i32).max(0);
    let y1 = ((sh.bottom + 2.0).ceil() as i32).min(shot.h as i32);""","""    let x0 = ((sh.left - 2.0).floor() as i32).max(shot.ox);
    let x1 = ((sh.right + 2.0).ceil() as i32).min(shot.ox + shot.w as i32);
    let y0 = (top_y.floor() as i32).max(shot.oy);
    let y1 = ((sh.bottom + 2.0).ceil() as i32).min(shot.oy + shot.h as i32);""")
# the crop a live refresh copies
s=sub(s,"fn smooth(","""/// The part of the notch window (x0, y0, x1, y1) the glass for `sh` can read: the notch itself and
/// a margin for how far the glass looks outside it. A live refresh copies only that.
fn crop_for(sh: Shape, w: i32, h: i32) -> (i32, i32, i32, i32) {
    let sc = sh.scale.max(0.5);
    let t = sh.tune;
    let m = ((t.pull * (1.0 + t.corner) + t.mid_blur + 6.0) * sc).ceil();
    let r = sh.radius.max(2.0 * sc).min((sh.right - sh.left) * 0.5);
    let mid = t.mid > 0.01 && ((sh.band > 0.0 && sh.fade > 0.004) || sh.clear > 0.004);
    let int_top = if sh.clear > 0.004 { sh.top } else { sh.bottom - sh.band };
    let rim_top = sh.bottom - r - t.reach * sc;
    let top = if mid { rim_top.min(int_top) } else { rim_top };
    let x0 = ((sh.left - m).floor() as i32).clamp(0, w);
    let x1 = ((sh.right + m).ceil() as i32).clamp(0, w);
    let y0 = ((top - m).floor() as i32).clamp(0, h);
    let y1 = ((sh.bottom + m).ceil() as i32).clamp(0, h);
    (x0, y0, x1, y1)
}

fn smooth(""")
s=sub(s,"""    let shot = grab(x, y, w, h)?;
    let sum = ring_sum(&shot, shape);""","""    // only the part of the screen the glass can read (a fraction of the window): far less to copy
    let (cx0, cy0, cx1, cy1) = crop_for(shape, w, h);
    let mut shot = grab_at(x + cx0, y + cy0, cx1 - cx0, cy1 - cy0, cx0, cy0)?;
    shot.made_for = Some(shape);
    let sum = ring_sum(&shot, shape);""")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"        self.rim_last = Some((geo, shape));\n","""        // a live copy covers only the notch at the size it had; if the size has changed, the whole
        // screen is copied again (once), so the new shape has pixels to read
        if !shot.full && shot.made_for != Some(shape) {
            self.refr_shot = None;
            self.refr_retry = None;
            self.request_frame();
            return;
        }
        self.rim_last = Some((geo, shape));
""")
open(p,'w').write(s)

p='src/config.rs'; s=open(p).read()
s=sub(s,"""    pub sharpen_at: f32,
}
""","""    pub sharpen_at: f32,
    /// How far in from the edge the glass is worked out pixel for pixel, in edge widths (4 = everywhere).
    pub sharp_zone: f32,
}
""")
s=sub(s,"            sharpen_at: 0.12,\n        }","            sharpen_at: 0.12,\n            sharp_zone: 4.0,\n        }")
s=sub(s,"        self.sharpen_at = self.sharpen_at.clamp(0.02, 0.5);\n","        self.sharpen_at = self.sharpen_at.clamp(0.02, 0.5);\n        self.sharp_zone = self.sharp_zone.clamp(0.3, 4.0);\n")
s=sub(s,"sharpen_at = 0.12\n","sharpen_at = 0.12\nsharp_zone = 4.0\n")
open(p,'w').write(s)

p='src/refract.rs'; s=open(p).read()
s=sub(s,"    pub mid_blur: f32,\n}\n","    pub mid_blur: f32,\n    pub sharp: f32,\n}\n")
s=sub(s,"            mid_blur: g.middle_blur.clamp(0.0, 6.0),\n","            mid_blur: g.middle_blur.clamp(0.0, 6.0),\n            sharp: g.sharp_zone.clamp(0.3, 4.0),\n")
s=sub(s,"""    let cell = if hybrid { 2_usize } else { 1 };
    let fine_d = if hybrid { width * 0.55 } else { f32::MAX };""","""    // (the top setting of the sharp zone means every pixel, so there are no blocks at all)
    let everywhere = t.sharp >= 3.95;
    let cell = if hybrid && !everywhere { 2_usize } else { 1 };
    let fine_d = if hybrid && !everywhere { width * t.sharp } else { f32::MAX };""")
open(p,'w').write(s)

p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"[(&str, f32, (f32, f32), String, &str); 17]","[(&str, f32, (f32, f32), String, &str); 18]")
s=sub(s,"""            ("Sharpen when left", g.sharpen_at, (0.02, 0.5), format!("{:.0}%", g.sharpen_at * 100.0), "sharpen_at"),
        ];""","""            ("Sharpen when left", g.sharpen_at, (0.02, 0.5), format!("{:.0}%", g.sharpen_at * 100.0), "sharpen_at"),
            (
                "Full-detail zone",
                g.sharp_zone,
                (0.3, 4.0),
                if g.sharp_zone >= 3.95 { "Everywhere".to_string() } else { format!("{:.1}x", g.sharp_zone) },
                "sharp_zone",
            ),
        ];""")
s=sub(s,"let b = row_rect(body, 17);","let b = row_rect(body, 18);")
s=sub(s,"                (\"sharpen_at\", d.sharpen_at),\n","                (\"sharpen_at\", d.sharpen_at),\n                (\"sharp_zone\", d.sharp_zone),\n")
s=sub(s,"        _ => 18.0,\n","        _ => 19.0,\n")
open(p,'w').write(s)
PYEOF
echo "Patched v217 + v218"
