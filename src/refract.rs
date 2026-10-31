//! The glass edge: a snapshot of the desktop behind the notch, bent along the rim like thick glass.
//!
//! A snapshot of the screen is taken when the notch opens (while the window is still small, so it
//! does not photograph itself). The rim is worked out from it on the CPU for the notch's current
//! size and uploaded as one small picture, drawn with a single call; it is only made again when the
//! size changes (while a page change resizes the notch, and never while nothing moves).
//!
//! Live glass: while the notch is open the screen is looked at again every so often, on a
//! background thread (`refresh`). The notch is hidden from screen copies for that (the window
//! is excluded from capture), so the copy shows only what is really behind it. A copy that turns
//! out identical to the last one (as far as the rim can see) is thrown away at once.

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
                        // (a screen copy has no alpha; only the colours are ever read)
                        let px = std::slice::from_raw_parts(bits as *const u32, len).to_vec();
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
    /// Each picture pixel stands for `step` x `step` window pixels (1 = pixel for pixel).
    pub step: usize,
    pub px: Vec<u32>,
}

/// Where the notch is, in window pixels: left and right edges, the bottom edge and the radius of
/// its bottom corners; `scale` is device pixels per logical unit.
#[derive(Clone, Copy, PartialEq)]
pub struct Shape {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub radius: f32,
    pub scale: f32,
}

/// A quick fingerprint of the part of the screen copy the rim actually reads: a thin ring around
/// the notch (the strip along its sides and bottom, plus the reach of the glass pull), sampled
/// sparsely. Equal fingerprints mean "nothing the rim would show has changed", so a video playing
/// in the middle of the screen, or under the notch's inside, costs nothing.
pub fn ring_sum(s: &Shot, sh: Shape) -> u64 {
    let sc = sh.scale.max(0.5);
    let r = sh.radius.max(2.0 * sc).min((sh.right - sh.left) * 0.5);
    let pad = 32.0 * sc; // the glass pulls from up to this far outside the edge
    let band = (76.0 * sc + pad).ceil() as i32; // how far in from the edge the rim reaches
    let x0 = ((sh.left - pad).floor() as i32).clamp(0, s.w as i32);
    let x1 = ((sh.right + pad).ceil() as i32).clamp(0, s.w as i32);
    let y0 = ((sh.bottom - r - 90.0 * sc - pad).floor() as i32).clamp(0, s.h as i32);
    let y1 = ((sh.bottom + pad).ceil() as i32).clamp(0, s.h as i32);
    let full_from = (sh.bottom - band as f32).floor() as i32;
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |p: u32| h = (h ^ (p & 0x00FF_FFFF) as u64).wrapping_mul(0x0000_0100_0000_01b3);
    for y in (y0..y1).step_by(2) {
        let row = &s.px[y as usize * s.w..(y as usize + 1) * s.w];
        if y >= full_from {
            row[x0 as usize..x1 as usize].iter().step_by(3).for_each(|&p| mix(p));
        } else {
            let left_end = (x0 + band).min(x1);
            let right_start = (x1 - band).max(left_end);
            row[x0 as usize..left_end as usize].iter().step_by(3).for_each(|&p| mix(p));
            row[right_start as usize..x1 as usize].iter().step_by(3).for_each(|&p| mix(p));
        }
    }
    h
}

/// What a refresh brings back.
pub struct Fresh {
    pub shot: Shot,
    pub sum: u64,
    /// The notch shape the rim was made for.
    pub shape: Shape,
    pub rim: Option<Rim>,
}

pub enum Fetched {
    /// Looks exactly like the previous copy: nothing to do.
    Same,
    Fresh(Fresh),
}

/// Copy the screen again and, when its ring differs from the one with fingerprint `prev`, work out
/// the rim for `shape` as well, at half detail (it is a soft gradient and this runs often). Meant
/// for a background thread; None when the screen could not be copied.
pub fn refresh(x: i32, y: i32, w: i32, h: i32, prev: u64, shape: Shape) -> Option<Fetched> {
    let shot = grab(x, y, w, h)?;
    let sum = ring_sum(&shot, shape);
    if sum == prev {
        return Some(Fetched::Same);
    }
    let made = rim(&shot, shape, 2);
    Some(Fetched::Fresh(Fresh { shot, sum, shape, rim: made }))
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
    let (a, b, c, d) = if x0 >= 0 && y0 >= 0 && (x0 as usize) + 1 < s.w && (y0 as usize) + 1 < s.h {
        // the usual case: all four pixels are inside the copy
        let i = y0 as usize * s.w + x0 as usize;
        let g = |p: u32| ((p >> shift) & 255) as f32;
        (g(s.px[i]), g(s.px[i + 1]), g(s.px[i + s.w]), g(s.px[i + s.w + 1]))
    } else {
        (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1))
    };
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

/// Work out the rim along the sides and the bottom corners of the notch. Inside the rim each
/// pixel shows what lies further out (pulled in toward the edge, strongest at the very edge),
/// fading to nothing toward the inside of the notch.
///
/// `step` 1 gives the finest picture; 2 makes it on every other pixel each way (a quarter of the
/// work, drawn enlarged): the rim is a soft gradient, so it looks the same while it is moving.
pub fn rim(shot: &Shot, sh: Shape, step: usize) -> Option<Rim> {
    let st = step.max(1);
    let sc = sh.scale.max(0.5);
    // The rim fades out over a long, soft stretch so it melts into the black of the notch.
    let width = 76.0 * sc; // how far in from the edge the rim reaches
    let pull = 22.0 * sc; // how far out the strongest pixels look
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
    let (w, h) = (w.div_ceil(st), h.div_ceil(st));
    let half = st as f32 * 0.5;
    let mut px = vec![0u32; w * h];
    let (cl, cr, cb) = (sh.left + r, sh.right - r, sh.bottom - r);
    let top = sh.bottom - reach;
    // Rows higher up only carry the rim along the two sides, so the middle is skipped there;
    // from `full_from` down the whole row is rim. The corners reach further in than the sides.
    let full_from = sh.bottom - width - 1.0;
    let side = ((width.max(r) + 2.0).ceil() as usize).div_ceil(st);
    for j in 0..h {
        let y = (y0 + (j * st) as i32) as f32 + half;
        let up = smooth(0.0, 0.5, (y - top) / reach);
        if up <= 0.0 {
            continue;
        }
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
            if d <= -1.0 || d >= width {
                continue;
            }
            // u: 1 at the edge, 0 at the inner end of the rim. The glass is strongest right at the
            // edge, and strongest of all in the corners, then eases away quickly so the black of the
            // notch takes over; the tail is smooth, with no visible line where the rim stops.
            let u = 1.0 - d / width;
            let fade = u * u * (1.5 - 0.5 * u);
            let base = 0.88 + 0.1 * corner;
            let alpha = (base + (1.0 - base) * smooth(4.0 * sc, 0.0, d)) * fade * (d + 1.0).clamp(0.0, 1.0) * up;
            if alpha < 0.004 {
                continue;
            }
            let k = u * u.sqrt();
            let pull = pull * (1.0 + 0.3 * corner);
            let (ox, oy) = (nx * pull * k, ny * pull * k);
            // red, green and blue look a slightly different distance out, like light through glass
            let cr = sample(shot, x + ox, y + oy, 16);
            let cg = sample(shot, x + ox * 0.95, y + oy * 0.95, 8);
            let cb2 = sample(shot, x + ox * 0.9, y + oy * 0.9, 0);
            let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
            px[j * w + i] = (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(cr) << 16) | (ch(cg) << 8) | ch(cb2);
        }
    }
    Some(Rim { x0, y0, w, h, step: st, px })
}
