//! The glass edge: a snapshot of the desktop behind the notch, bent along the rim like thick glass.
//!
//! A snapshot of the screen is taken once when the notch opens (while the window is still small, so
//! it does not photograph itself). The rim is worked out from it on the CPU for the notch's current
//! size and uploaded as one small picture, drawn with a single call; it is only made again when the
//! size changes (while a page change resizes the notch, and never while nothing moves).

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

/// One colour (`shift` 16 red, 8 green, 0 blue) of the picture at a fractional position, smoothly
/// blended between the four nearest pixels (coordinates are clamped to the edge).
fn sample(s: &Shot, fx: f32, fy: f32, shift: u32) -> f32 {
    let (gx, gy) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |x: i32, y: i32| -> f32 {
        let xi = x.clamp(0, s.w as i32 - 1) as usize;
        let yi = y.clamp(0, s.h as i32 - 1) as usize;
        ((s.px[yi * s.w + xi] >> shift) & 255) as f32
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

/// Work out the rim along the sides and the bottom corners of the notch. Inside the rim each
/// pixel shows what lies further out (pulled in toward the edge, strongest at the very edge),
/// fading to nothing toward the inside of the notch.
pub fn rim(shot: &Shot, sh: Shape) -> Option<Rim> {
    let sc = sh.scale.max(0.5);
    // The rim fades out over a long, soft stretch so it melts into the black of the notch.
    let width = 64.0 * sc; // how far in from the edge the rim reaches
    let pull = 20.0 * sc; // how far out the strongest pixels look
    let r = sh.radius.max(2.0 * sc).min((sh.right - sh.left) * 0.5);
    let reach = r + 90.0 * sc; // how far up the sides it is drawn, fading out toward the top
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
    // Rows higher up only carry the rim along the two sides, so the middle is skipped there;
    // from `full_from` down the whole row is rim. The corners reach further in than the sides.
    let full_from = sh.bottom - width - 1.0;
    let side = (width.max(r) + 2.0).ceil() as usize;
    for j in 0..h {
        let y = (y0 + j as i32) as f32 + 0.5;
        let up = smooth(0.0, 0.5, (y - top) / reach);
        if up <= 0.0 {
            continue;
        }
        let (a_end, b_start) = if y >= full_from { (w, w) } else { (side.min(w), w.saturating_sub(side).max(side.min(w))) };
        for i in (0..a_end).chain(b_start..w) {
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
            // u: 1 at the edge, 0 at the inner end of the rim. The strength follows a smooth S, so
            // it starts and ends gently, with no visible line where the rim stops.
            let u = 1.0 - d / width;
            let alpha = 0.6 * u * u * (3.0 - 2.0 * u) * (d + 1.0).clamp(0.0, 1.0) * up;
            if alpha < 0.004 {
                continue;
            }
            let k = u * u.sqrt();
            let (ox, oy) = (nx * pull * k, ny * pull * k);
            // red, green and blue look a slightly different distance out, like light through glass
            let cr = sample(shot, x + ox, y + oy, 16);
            let cg = sample(shot, x + ox * 0.95, y + oy * 0.95, 8);
            let cb2 = sample(shot, x + ox * 0.9, y + oy * 0.9, 0);
            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
            px[j * w + i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);
        }
    }
    Some(Rim { x0, y0, w, h, px })
}
