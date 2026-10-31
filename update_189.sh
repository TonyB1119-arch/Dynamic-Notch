#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "mod refract" src/main.rs; then echo "glass edge already applied"; exit 0; fi
cat > src/refract.rs <<'RSEOF'
//! The glass edge: a snapshot of the desktop behind the notch, bent along the rim like thick glass.
//!
//! Nothing here runs per frame. A snapshot of the screen is taken once when the notch opens (while
//! the window is still small, so it does not photograph itself); once the notch has settled the rim
//! is worked out on the CPU and uploaded as one small picture, drawn with a single call.

use std::ffi::c_void;

use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS, HGDIOBJ, ROP_CODE, SRCCOPY,
};

/// A piece of the screen as it looked at one moment: opaque BGRA pixels, `w` x `h`.
pub struct Shot {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Copy the screen area at (`x`, `y`) of size `w` x `h` (screen pixels). None if it cannot be had.
pub fn grab(x: i32, y: i32, w: i32, h: i32) -> Option<Shot> {
    if !(8..=4096).contains(&w) || !(8..=2400).contains(&h) {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let hdc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let out = match CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) => {
                let mut shot = None;
                if !bits.is_null() {
                    let old = SelectObject(hdc, HGDIOBJ(bmp.0));
                    let ok = BitBlt(hdc, 0, 0, w, h, Some(screen), x, y, ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0)).is_ok();
                    if ok {
                        let len = w as usize * h as usize;
                        let mut px = std::slice::from_raw_parts(bits as *const u32, len).to_vec();
                        // a screen copy has no alpha: make every pixel opaque
                        for p in px.iter_mut() {
                            *p |= 0xFF00_0000;
                        }
                        shot = Some(Shot { w: w as usize, h: h as usize, px });
                    }
                    SelectObject(hdc, old);
                }
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                shot
            }
            Err(_) => None,
        };
        let _ = DeleteDC(hdc);
        ReleaseDC(None, screen);
        out
    }
}

/// The rim, ready to upload: premultiplied BGRA pixels, `w` x `h`, at (`x0`, `y0`) in window pixels.
pub struct Rim {
    pub x0: i32,
    pub y0: i32,
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Where the notch is, in window pixels: left and right edges, the bottom edge and the radius of
/// its bottom corners; `scale` is device pixels per logical unit.
#[derive(Clone, Copy)]
pub struct Shape {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub radius: f32,
    pub scale: f32,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One pixel of the picture, as red, green, blue in 0..255 (coordinates are clamped to the edge).
fn texel(s: &Shot, x: i32, y: i32) -> [f32; 3] {
    let xi = x.clamp(0, s.w as i32 - 1) as usize;
    let yi = y.clamp(0, s.h as i32 - 1) as usize;
    let p = s.px[yi * s.w + xi];
    [((p >> 16) & 255) as f32, ((p >> 8) & 255) as f32, (p & 255) as f32]
}

/// The picture at a fractional position, smoothly blended between the four nearest pixels.
fn sample(s: &Shot, fx: f32, fy: f32) -> [f32; 3] {
    let (gx, gy) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (texel(s, x0, y0), texel(s, x0 + 1, y0), texel(s, x0, y0 + 1), texel(s, x0 + 1, y0 + 1));
    let mut o = [0.0; 3];
    for i in 0..3 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bot = c[i] + (d[i] - c[i]) * tx;
        o[i] = top + (bot - top) * ty;
    }
    o
}

/// Work out the rim along the sides and the bottom corners of the notch. Inside the rim each
/// pixel shows what lies a little further out (pulled in toward the edge, strongest at the very
/// edge) with a soft highlight, fading to nothing toward the inside of the notch.
pub fn rim(shot: &Shot, sh: Shape) -> Option<Rim> {
    let sc = sh.scale.max(0.5);
    let width = 15.0 * sc; // how far in from the edge the rim reaches
    let pull = 12.0 * sc; // how far out the strongest pixels look
    let r = sh.radius.max(2.0 * sc).min((sh.right - sh.left) * 0.5);
    let reach = r + 60.0 * sc; // how far up the sides it is drawn, fading out toward the top
    let x0 = ((sh.left - 2.0).floor() as i32).max(0);
    let x1 = ((sh.right + 2.0).ceil() as i32).min(shot.w as i32);
    let y0 = ((sh.bottom - reach).floor() as i32).max(0);
    let y1 = ((sh.bottom + 2.0).ceil() as i32).min(shot.h as i32);
    if x1 - x0 < 8 || y1 - y0 < 4 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut px = vec![0u32; w * h];
    let (cl, cr, cb) = (sh.left + r, sh.right - r, sh.bottom - r);
    let top = sh.bottom - reach;
    for j in 0..h {
        let y = (y0 + j as i32) as f32 + 0.5;
        let up = smooth(0.0, 0.5, (y - top) / reach);
        for i in 0..w {
            let x = (x0 + i as i32) as f32 + 0.5;
            // distance inside the edge, and the direction pointing out of the notch
            let d;
            let nx;
            let ny;
            if y > cb && (x < cl || x > cr) {
                let cx = if x < cl { cl } else { cr };
                let (dx, dy) = (x - cx, y - cb);
                let dist = (dx * dx + dy * dy).sqrt().max(0.001);
                d = r - dist;
                nx = dx / dist;
                ny = dy / dist;
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
            if d <= -1.0 || d >= width {
                continue;
            }
            let t = (d / width).clamp(0.0, 1.0);
            let k = (1.0 - t) * (1.0 - t);
            let c = sample(shot, x + nx * pull * k, y + ny * pull * k);
            // a soft highlight hugging the edge
            let glow = 0.30 * (1.0 - t).powi(5) + 0.07 * k;
            let alpha = 0.88 * (1.0 - t).powf(1.4) * (d + 1.0).clamp(0.0, 1.0) * up;
            let mut out = 0u32;
            for (n, v) in [c[2], c[1], c[0]].into_iter().enumerate() {
                let lit = v + (255.0 - v) * glow;
                out |= ((lit * alpha).round().clamp(0.0, 255.0) as u32) << (8 * n);
            }
            out |= ((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24;
            px[j * w + i] = out;
        }
    }
    Some(Rim { x0, y0, w, h, px })
}
RSEOF
python3 - <<'PYEOF'
import os
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/main.rs'; s=open(p).read()
s=sub(s,"mod pacer;\n","mod pacer;\nmod refract;\n")
open(p,'w').write(s)

# ---- gfx: the rim picture
p='src/gfx/mod.rs'; s=open(p).read()
marker="impl Res {\n    /// Blend the strip's pictures"
add='''/// The glass rim as one small picture (see `refract`), and where it sits in logical units.
pub struct RimPic {
    bmp: ID2D1Bitmap1,
    pub rect: Rect,
}

impl Res {
    /// Upload premultiplied BGRA pixels (`w` x `h`, at (`x0`, `y0`) in window pixels) as a picture.
    pub fn rim_pic(&self, x0: i32, y0: i32, w: usize, h: usize, px: &[u32], scale: f32) -> Option<RimPic> {
        if w == 0 || h == 0 || px.len() < w * h || w > 4096 || h > 4096 {
            return None;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: ManuallyDrop::new(None),
        };
        unsafe {
            let size = D2D_SIZE_U { width: w as u32, height: h as u32 };
            let data = px.as_ptr() as *const std::ffi::c_void;
            let bmp = self.dc.CreateBitmap(size, Some(data), (w * 4) as u32, &props).ok()?;
            Some(RimPic { bmp, rect: Rect::new(x0 as f32 / scale, y0 as f32 / scale, w as f32 / scale, h as f32 / scale) })
        }
    }
}

impl<'a> Painter<'a> {
    /// Draw the glass rim: one picture, one call, pixel for pixel.
    pub fn draw_rim(&self, r: &RimPic, opacity: f32) {
        let a = opacity * self.alpha;
        if a <= 0.002 {
            return;
        }
        let Ok(b) = r.bmp.cast::<ID2D1Bitmap>() else { return };
        unsafe {
            self.dc.DrawBitmap(&b, Some(&rf(r.rect)), a, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
        }
    }
}

'''
s=sub(s,marker,add+marker)
open(p,'w').write(s)

# ---- app
p='src/app.rs'; s=open(p).read()
s=sub(s,"/// The few look settings the drawing code reads every frame","/// Cache key of the glass rim: position, size, corner radius and scale, in quarter pixels.\ntype RimKey = (i32, i32, i32, i32, i32);\n\n/// The few look settings the drawing code reads every frame")
s=sub(s,"    band_failed: bool,\n","""    band_failed: bool,
    /// Alcove glass edge: a copy of the screen behind the notch, taken as it opens, the rim worked
    /// out from it, when the notch last went idle, and when a copy was last tried.
    refr_shot: Option<crate::refract::Shot>,
    refr_rim: Option<(RimKey, gfx::RimPic)>,
    refr_idle: Option<Instant>,
    refr_retry: Option<Instant>,
""")
s=sub(s,"            band_failed: false,\n","            band_failed: false,\n            refr_shot: None,\n            refr_rim: None,\n            refr_idle: None,\n            refr_retry: None,\n")
s=sub(s,"                self.win_px = geom;\n","                self.win_px = geom;\n                self.refr_shot = None;\n                self.refr_rim = None;\n")
s=sub(s,"""    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_band(now);""","""    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_band(now);
        self.prepare_rim(now);""")
s=sub(s,"    /// 1 while the soft bottom edge is showing, else 0.","""    /// Alcove glass edge. The screen behind the notch is copied once, on the first frame after the
    /// notch leaves its resting size (the window is still small then, so the copy is clean), and
    /// let go a moment after the notch closes. Once the notch has settled, the rim is worked out
    /// from that copy and uploaded as one small picture; that is redone only if the size changes.
    fn prepare_rim(&mut self, now: Instant) {
        if !(self.cfg.appearance.glass_look && crate::ui::look_alcove()) {
            self.refr_shot = None;
            self.refr_rim = None;
            self.refr_idle = None;
            return;
        }
        if self.mode == Mode::Idle {
            let since = *self.refr_idle.get_or_insert(now);
            if now.duration_since(since) > Duration::from_millis(1500) {
                self.refr_shot = None;
                self.refr_rim = None;
            }
            return;
        }
        self.refr_idle = None;
        if self.refr_shot.is_none() && self.refr_retry.is_none_or(|t| now.duration_since(t) > Duration::from_secs(2)) {
            let (x, y, w, h) = self.win_px;
            self.refr_shot = crate::refract::grab(x, y, w, h);
            self.refr_retry = Some(now);
            if self.refr_shot.is_none() {
                log!("glass edge: the screen could not be copied");
            }
        }
        if !self.settled {
            return;
        }
        let Some(shot) = self.refr_shot.as_ref() else { return };
        let body = self.body_rect();
        if body.h < 56.0 {
            self.refr_rim = None;
            return;
        }
        let sc = self.scale;
        let r = self.anim.r.value.min(body.w * 0.5).min(body.h - self.anim.ear.value.min(body.h * 0.5)).max(0.0);
        let q4 = |v: f32| (v * sc * 4.0).round() as i32;
        let key: RimKey = (q4(body.x), q4(body.w), q4(body.bottom()), q4(r), (sc * 100.0).round() as i32);
        if self.refr_rim.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        self.refr_rim = None;
        let shape = crate::refract::Shape {
            left: body.x * sc,
            right: body.right() * sc,
            bottom: body.bottom() * sc,
            radius: r * sc,
            scale: sc,
        };
        if let Some(rim) = crate::refract::rim(shot, shape) {
            if let Some(pic) = self.res.rim_pic(rim.x0, rim.y0, rim.w, rim.h, &rim.px, sc) {
                self.refr_rim = Some((key, pic));
            }
        }
    }

    /// 1 while the soft bottom edge is showing, else 0.""")
# remove the old sheen
a0=s.index("                    if glass && glass_k > 0.0 && body.h >= 56.0 {\n                        // a soft sheen")
a1=s.index("                }\n\n                p.push_clip(body);\n",a0)
s=s[:a0]+s[a1:]
# replace the old rings with the rim picture
b0=s.index("                if glass && glass_k > 0.0 && settled && body.h >= 56.0 && a_glass > 0.01 {")
end="                p.pop_clip();\n            }\n        }\n        if let Err(e) = self.host.end_draw()"
b1=s.index(end,b0)
s=s[:b0]+'''                if glass && glass_k > 0.0 && settled && body.h >= 56.0 && a_glass > 0.01 {
                    // the glass edge: the screen behind, bent along the rim (see `prepare_rim`)
                    if let Some((_, pic)) = &self.refr_rim {
                        p.draw_rim(pic, a_glass * glass_k);
                    }
                }
'''+s[b1:]
open(p,'w').write(s)

# ---- settings label, config comment
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,'"Glass shine and lit edge",','"Glass edge (bends what is behind)",')
open(p,'w').write(s)
p='src/config.rs'; s=open(p).read()
s=sub(s,"/// Alcove: a soft sheen, a lit edge and a little inner glow on the open notch (liquid-glass look).","/// Alcove: the open notch's edge bends what is behind it, like thick glass.")
s=sub(s,"# Alcove: a soft sheen, a lit edge and a little inner glow (false = plain glass)","# Alcove: the notch's edge bends what is behind it (false = plain glass)")
open(p,'w').write(s)
PYEOF
echo "Patched v189: glass edge that bends what is behind the notch"
