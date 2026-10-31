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

/// The look of the glass (sizes in logical pixels, scaled by `Shape::scale`). The defaults are the
/// look the glass has always had.
#[derive(Clone, Copy, PartialEq)]
pub struct Tune {
    pub width: f32,
    pub pull: f32,
    pub bend: f32,
    pub strength: f32,
    pub corner: f32,
    pub fringe: f32,
    pub reach: f32,
    pub soft: f32,
    pub mid: f32,
    pub mid_warp: f32,
    pub mid_blur: f32,
}

impl From<&crate::config::GlassCfg> for Tune {
    fn from(g: &crate::config::GlassCfg) -> Self {
        Self {
            width: g.edge_width.clamp(20.0, 160.0),
            pull: g.pull.clamp(0.0, 60.0),
            bend: g.bend.clamp(0.8, 3.0),
            strength: g.strength.clamp(0.3, 1.0),
            corner: g.corner.clamp(0.0, 1.0),
            fringe: g.fringe.clamp(0.0, 0.2),
            reach: g.reach.clamp(30.0, 200.0),
            soft: g.softness.clamp(0.5, 12.0),
            mid: g.middle.clamp(0.0, 1.0),
            mid_warp: g.middle_warp.clamp(0.0, 1.0),
            mid_blur: g.middle_blur.clamp(0.0, 6.0),
        }
    }
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
    pub tune: Tune,
    /// Top edge of the notch (window pixels).
    pub top: f32,
    /// Height of the see-through strip along the bottom, window pixels (0 = none), and how far the
    /// black thins out at its very bottom (0 - 1).
    pub band: f32,
    pub fade: f32,
    /// How see-through the whole notch is (0 = solid black).
    pub clear: f32,
}

/// A quick fingerprint of the part of the screen copy the rim actually reads: a thin ring around
/// the notch (the strip along its sides and bottom, plus the reach of the glass pull), sampled
/// sparsely. Equal fingerprints mean "nothing the rim would show has changed", so a video playing
/// in the middle of the screen, or under the notch's inside, costs nothing.
pub fn ring_sum(s: &Shot, sh: Shape) -> u64 {
    let sc = sh.scale.max(0.5);
    let r = sh.radius.max(2.0 * sc).min((sh.right - sh.left) * 0.5);
    let t = sh.tune;
    let pad = (t.pull * (1.0 + t.corner) + 4.0) * sc; // the glass pulls from up to this far outside the edge
    let band = (t.width * sc + pad).ceil() as i32; // how far in from the edge the rim reaches
    let x0 = ((sh.left - pad).floor() as i32).clamp(0, s.w as i32);
    let x1 = ((sh.right + pad).ceil() as i32).clamp(0, s.w as i32);
    let y0 = ((sh.bottom - r - t.reach * sc - pad).floor() as i32).clamp(0, s.h as i32);
    let y1 = ((sh.bottom + pad).ceil() as i32).clamp(0, s.h as i32);
    let mid = t.mid > 0.01 && ((sh.band > 0.0 && sh.fade > 0.004) || sh.clear > 0.004);
    let int_top = if sh.clear > 0.004 { sh.top } else { sh.bottom - sh.band };
    let (y0, full_from) = if mid {
        (y0.min(((int_top - pad).floor() as i32).clamp(0, s.h as i32)), (sh.bottom - band as f32).min(int_top).floor() as i32)
    } else {
        (y0, (sh.bottom - band as f32).floor() as i32)
    };
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
    let made = rim(&shot, shape, 1);
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

/// The picture at a fractional position as red, green and blue, blended between the four nearest
/// pixels (coordinates are clamped to the edge).
fn sample_rgb(s: &Shot, fx: f32, fy: f32) -> [f32; 3] {
    let (gx, gy) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |x: i32, y: i32| -> u32 {
        let xi = x.clamp(0, s.w as i32 - 1) as usize;
        let yi = y.clamp(0, s.h as i32 - 1) as usize;
        s.px[yi * s.w + xi]
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut out = [0.0_f32; 3];
    for (n, shift) in [16u32, 8, 0].into_iter().enumerate() {
        let g = |p: u32| ((p >> shift) & 255) as f32;
        let top = g(a) + (g(b) - g(a)) * tx;
        let bot = g(c) + (g(d) - g(c)) * tx;
        out[n] = top + (bot - top) * ty;
    }
    out
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
    // deeper in, the outline the middle follows is rounder and rounder (bigger circles at the corners)
    let rb = (r + 70.0 * sc).min((sh.right - sh.left) * 0.5);
    let (cl2, cr2, cb2) = (sh.left + rb, sh.right - rb, sh.bottom - rb);
    // Step 1 is sharp where it matters and half detail elsewhere: the glass bends the screen
    // hardest right along the edge, where fine text shows. Deeper in the rim is faint and the
    // middle is soft anyway, so each 2 x 2 block there is worked out once.
    let hybrid = st == 1;
    let cell = if hybrid { 2_usize } else { 1 };
    let fine_d = if hybrid { width * 0.55 } else { f32::MAX };
    // distance inside the edge, the direction pointing out of the notch, and the corner weight
    let geom = |x: f32, y: f32| -> (f32, f32, f32, f32) {
        if y > cb && (x < cl || x > cr) {
            let cx = if x < cl { cl } else { cr };
            let (dx, dy) = (x - cx, y - cb);
            let dist = (dx * dx + dy * dy).sqrt().max(0.001);
            // corner: 0 where the curve meets the straight edges, 1 in the middle of the curve (no seam)
            (r - dist, dx / dist, dy / dist, (2.0 * dx.abs() * dy / (dist * dist)).clamp(0.0, 1.0))
        } else {
            let (dl, dr, db) = (x - sh.left, sh.right - x, sh.bottom - y);
            if db <= dl && db <= dr {
                (db, 0.0, 1.0, 0.0)
            } else if dl <= dr {
                (dl, -1.0, 0.0, 0.0)
            } else {
                (dr, 1.0, 0.0, 0.0)
            }
        }
    };
    // one pixel of the picture (0 = nothing there)
    let shade = |x: f32, y: f32| -> u32 {
        let up = smooth(0.0, 0.5, (y - top) / reach);
        let in_mid = mid && y >= int_top;
        if up <= 0.0 && !in_mid {
            return 0;
        }
        // how much of the middle shows on this row: the black thins toward the bottom edge
        let a_row = if in_mid {
            let bt = ((y - (sh.bottom - sh.band)) / band).clamp(0.0, 1.0);
            let e = bt * bt * bt * (bt * (bt * 6.0 - 15.0) + 10.0);
            (sh.fade * e).max(sh.clear) * t.mid
        } else {
            0.0
        };
        let (d, nx, ny, corner) = geom(x, y);
        if d <= -1.0 {
            return 0;
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
        let mut blur = 0.0_f32;
        if a_row > 0.0 {
            // the middle lies under the rim; it bends gently toward the nearest edges
            let ai = a_row * (d + 1.0).clamp(0.0, 1.0);
            alpha += ai * (1.0 - alpha);
            if d > width {
                let dm;
                let dirx;
                let diry;
                if y > cb2 && (x < cl2 || x > cr2) {
                    // around a corner: along circles, with the outline getting rounder inward
                    let cx = if x < cl2 { cl2 } else { cr2 };
                    let (dx, dy) = (x - cx, y - cb2);
                    let dist = (dx * dx + dy * dy).sqrt().max(0.001);
                    dm = d + ((rb - dist) - d) * smooth(0.0, width, d);
                    dirx = dx / dist;
                    diry = dy / dist;
                } else {
                    // a soft blend of the nearest edges' directions: no crease where they meet
                    let wl = 1.0 / (x - sh.left + 6.0).max(1.0).powi(3);
                    let wr = 1.0 / (sh.right - x + 6.0).max(1.0).powi(3);
                    let wb = 1.0 / (sh.bottom - y + 6.0).max(1.0).powi(3);
                    let inv = 1.0 / (wl + wr + wb);
                    dm = d;
                    dirx = (wr - wl) * inv;
                    diry = wb * inv;
                }
                if t.mid_warp > 0.0 {
                    let wi = smooth(width, width * 1.8, d);
                    let u2 = (1.0 - dm / w2).clamp(0.0, 1.0);
                    let k2 = if even_bend { u2 * u2.sqrt() } else { u2.powf(t.bend) };
                    let m = pull * t.mid_warp * k2 * wi;
                    ox += dirx * m;
                    oy += diry * m;
                }
            }
            // the middle is a little soft; the sharp rim keeps its fine detail
            blur = t.mid_blur * sc * smooth(width * 0.6, width * 1.6, d);
        }
        if alpha < 0.004 {
            return 0;
        }
        let (rc, gc, bc) = if blur > 0.25 {
            let (bx, by) = (x + ox, y + oy);
            let mut acc = [0.0_f32; 3];
            for (sx, sy) in [(-blur, -blur), (blur, -blur), (-blur, blur), (blur, blur)] {
                let c = sample_rgb(shot, bx + sx, by + sy);
                acc[0] += c[0];
                acc[1] += c[1];
                acc[2] += c[2];
            }
            (acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25)
        } else {
            // red, green and blue look a slightly different distance out, like light through glass
            (
                sample(shot, x + ox, y + oy, 16),
                sample(shot, x + ox * (1.0 - t.fringe), y + oy * (1.0 - t.fringe), 8),
                sample(shot, x + ox * (1.0 - 2.0 * t.fringe), y + oy * (1.0 - 2.0 * t.fringe), 0),
            )
        };
        let ch = |v: f32| (v * alpha).round().clamp(0.0, 255.0) as u32;
        (((alpha * 255.0).round().clamp(0.0, 255.0) as u32) << 24) | (ch(rc) << 16) | (ch(gc) << 8) | ch(bc)
    };
    // a band of `cell` rows of the picture
    let band_fn = |bj: usize, out: &mut [u32]| {
        let rows = out.len() / w;
        let j0 = bj * cell;
        let yc = y0 as f32 + (j0 * st) as f32 + (rows * st) as f32 * 0.5;
        let up = smooth(0.0, 0.5, (yc - top) / reach);
        if up <= 0.0 && !(mid && yc >= int_top) {
            return;
        }
        let cw = w.div_ceil(cell);
        let (a_end, b_start) = if yc >= full_from { (w, w) } else { (side.min(w), w.saturating_sub(side).max(side.min(w))) };
        for ci in (0..a_end.div_ceil(cell)).chain(b_start / cell..cw) {
            let i0 = ci * cell;
            let cols = cell.min(w - i0);
            let xc = x0 as f32 + (i0 * st) as f32 + (cols * st) as f32 * 0.5;
            if cell == 1 || geom(xc, yc).0 < fine_d {
                for rr in 0..rows {
                    for cc in 0..cols {
                        let x = (x0 + ((i0 + cc) * st) as i32) as f32 + half;
                        let y = (y0 + ((j0 + rr) * st) as i32) as f32 + half;
                        out[rr * w + i0 + cc] = shade(x, y);
                    }
                }
            } else {
                let v = shade(xc, yc);
                if v != 0 {
                    for rr in 0..rows {
                        for cc in 0..cols {
                            out[rr * w + i0 + cc] = v;
                        }
                    }
                }
            }
        }
    };
    // a big picture is split into bands of rows worked out side by side; small ones are not worth
    // the hand-over
    let bands = h.div_ceil(cell);
    let threads = if w * h >= PAR_MIN { workers() } else { 1 };
    if threads > 1 {
        let per = bands.div_ceil(threads);
        let f = &band_fn;
        std::thread::scope(|scope| {
            for (n, chunk) in px.chunks_mut(per * cell * w).enumerate() {
                scope.spawn(move || {
                    for (jj, out) in chunk.chunks_mut(cell * w).enumerate() {
                        f(n * per + jj, out);
                    }
                });
            }
        });
    } else {
        for (bj, out) in px.chunks_mut(cell * w).enumerate() {
            band_fn(bj, out);
        }
    }
    Some(Rim { x0, y0, w, h, step: st, px })
}

/// Pictures smaller than this are worked out on the one thread (starting threads costs more CPU
/// than it saves); only a very big one is split up.
const PAR_MIN: usize = 400_000;

/// How many threads a big rim may use (looked up once).
fn workers() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4))
}
