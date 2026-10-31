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
/// pixel shows what lies further out (pulled in toward the edge, strongest at the very edge),
/// fading to nothing toward the inside of the notch.
pub fn rim(shot: &Shot, sh: Shape) -> Option<Rim> {
    let sc = sh.scale.max(0.5);
    let width = 26.0 * sc; // how far in from the edge the rim reaches
    let pull = 40.0 * sc; // how far out the strongest pixels look
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
            let k = (1.0 - t).powf(1.6);
            // red, green and blue look a slightly different distance out, like light through glass
            let (cr, cg, cb2) = (
                sample(shot, x + nx * pull * k, y + ny * pull * k)[0],
                sample(shot, x + nx * pull * k * 0.88, y + ny * pull * k * 0.88)[1],
                sample(shot, x + nx * pull * k * 0.76, y + ny * pull * k * 0.76)[2],
            );
            let alpha = 0.96 * (1.0 - t).powf(0.9) * (d + 1.0).clamp(0.0, 1.0) * up;
            let ch = |v: f32| ((v * alpha).round().clamp(0.0, 255.0) as u32);
            px[j * w + i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);
        }
    }
    Some(Rim { x0, y0, w, h, px })
}
