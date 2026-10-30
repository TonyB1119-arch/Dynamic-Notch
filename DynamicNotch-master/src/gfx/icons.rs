//! Icon set, drawn entirely as vectors: evenly weighted strokes with round
//! caps on a 24-unit grid, and filled shapes for the media transport. The
//! recurring motif is the beam, a short vertical rounded bar, the same shape
//! the gauges and rings are made of (equalizer, signal, volume, keys, stats). Crisp at any DPI, and no font or
//! asset ships with the binary.

#![allow(dead_code)]

use super::{q, uq, Painter};
use crate::util::{Color, Rect};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    // vector
    Play,
    Pause,
    Next,
    Prev,
    Sparkle,
    Check,
    Close,
    Plus,
    Minus,
    Stop,
    // glyphs
    Music,
    Timer,
    Clipboard,
    Download,
    Mic,
    Camera,
    Bell,
    Home,
    Settings,
    Folder,
    Copy,
    Trash,
    Image,
    File,
    Link,
    Refresh,
    Send,
    Info,
    Bolt,
    Globe,
    Keyboard,
    Package,
    Volume,
    Mute,
    Brightness,
    Headphones,
    Bluetooth,
    Mouse,
    Gamepad,
    Phone,
    Pulse,
    Wifi,
    Cloud,
    Lock,
    Edit,
    Chat,
    Control,
    Unlock,
    Moon,
    Grid,
    Cpu,
    Ram,
}

/// Every icon is a glyph from the Phosphor set (see `pack.rs`), filled from cached geometry.
pub fn draw(p: &mut Painter, icon: Icon, r: Rect, c: Color) {
    // the Phosphor glyphs fill more of their box than the old hand-drawn set did
    let s = r.w.min(r.h) * 0.94;
    let b = r.center_square(s);
    let key = (150u8, icon as i32, q(s), 0, 0);
    p.fill_cached(key, b.x, b.y, c, |pb| {
        let k = uq(key.2) / super::pack::GRID;
        let d = super::pack::data(icon);
        let mut i = 0;
        while i < d.len() {
            match d[i] as i32 {
                0 => {
                    pb.move_to(d[i + 1] * k, d[i + 2] * k);
                    i += 3;
                }
                1 => {
                    pb.line_to(d[i + 1] * k, d[i + 2] * k);
                    i += 3;
                }
                2 => {
                    pb.cubic_to(d[i + 1] * k, d[i + 2] * k, d[i + 3] * k, d[i + 4] * k, d[i + 5] * k, d[i + 6] * k);
                    i += 7;
                }
                _ => {
                    pb.close();
                    i += 1;
                }
            }
        }
    });
}

#[allow(dead_code)]
fn draw_hand_drawn(p: &mut Painter, icon: Icon, r: Rect, c: Color) {
    if outline(p, icon, r, c) {
        return;
    }
    let s = r.w.min(r.h);
    let b = r.center_square(s);
    let (x, y) = (b.x, b.y);
    match icon {
        // Vector icons are built at the origin and cached on the GPU per size.
        Icon::Play => {
            let s = uq(q(s));
            p.fill_cached((10, q(s), 0, 0, 0), x, y, c, |pb| {
                let pts = [(s * 0.27, s * 0.16), (s * 0.27, s * 0.84), (s * 0.85, s * 0.50)];
                pb.rounded_poly(&pts, s * 0.09)
            });
        }
        Icon::Pause => {
            let w = s * 0.17;
            let h = s * 0.60;
            let gap = s * 0.15;
            let top = y + (s - h) * 0.5;
            let left = x + (s - (2.0 * w + gap)) * 0.5;
            p.fill_rounded(Rect::new(left, top, w, h), w * 0.32, c);
            p.fill_rounded(Rect::new(left + w + gap, top, w, h), w * 0.32, c);
        }
        Icon::Stop => {
            let q = s * 0.5;
            p.fill_rounded(b.center_square(q), q * 0.18, c);
        }
        Icon::Next | Icon::Prev => {
            let flip = icon == Icon::Prev;
            let s = uq(q(s));
            let kind = if flip { 12 } else { 11 };
            p.fill_cached((kind, q(s), 0, 0, 0), x, y, c, |pb| {
                for (x0, x1) in [(s * 0.07, s * 0.52), (s * 0.46, s * 0.91)] {
                    let (x0, x1) = if flip { (s - x0, s - x1) } else { (x0, x1) };
                    pb.rounded_poly(&[(x0, s * 0.23), (x0, s * 0.77), (x1, s * 0.50)], s * 0.06);
                }
            });
        }
        Icon::Sparkle => {
            let s = uq(q(s));
            p.fill_cached((13, q(s), 0, 0, 0), x, y, c, |pb| {
                // four-point stars with softly rounded tips
                for (cx, cy, rad) in [(s * 0.42, s * 0.56, s * 0.40), (s * 0.80, s * 0.20, s * 0.17)] {
                    let k = 0.30;
                    let pts = [
                        (cx, cy - rad),
                        (cx + rad * k, cy - rad * k),
                        (cx + rad, cy),
                        (cx + rad * k, cy + rad * k),
                        (cx, cy + rad),
                        (cx - rad * k, cy + rad * k),
                        (cx - rad, cy),
                        (cx - rad * k, cy - rad * k),
                    ];
                    pb.rounded_poly(&pts, rad * 0.16);
                }
            });
        }
        Icon::Check => {
            let w = s * 0.085;
            p.line(x + s * 0.22, y + s * 0.52, x + s * 0.42, y + s * 0.72, w, c);
            p.line(x + s * 0.42, y + s * 0.72, x + s * 0.80, y + s * 0.30, w, c);
        }
        Icon::Close => {
            let w = s * 0.082;
            p.line(x + s * 0.28, y + s * 0.28, x + s * 0.72, y + s * 0.72, w, c);
            p.line(x + s * 0.72, y + s * 0.28, x + s * 0.28, y + s * 0.72, w, c);
        }
        Icon::Plus => {
            let w = s * 0.09;
            p.line(x + s * 0.5, y + s * 0.24, x + s * 0.5, y + s * 0.76, w, c);
            p.line(x + s * 0.24, y + s * 0.5, x + s * 0.76, y + s * 0.5, w, c);
        }
        Icon::Minus => {
            let w = s * 0.09;
            p.line(x + s * 0.24, y + s * 0.5, x + s * 0.76, y + s * 0.5, w, c);
        }
        _ => {}
    }
}

/// Battery glyph drawn as vector so the fill level is exact.
pub fn battery(p: &mut Painter, r: Rect, level: f32, charging: bool, fill: Color) {
    // body occupies ~88% width, cap the rest
    let h = r.h.min(r.w * 0.5);
    let w = h * 2.05;
    let body = Rect::new(r.cx() - w * 0.5, r.cy() - h * 0.5, w * 0.9, h);
    let stroke = (h * 0.09).max(1.0);
    p.stroke_rounded(body.inset(stroke * 0.5), h * 0.3, stroke, Color::white(0.45));
    let cap = Rect::new(body.right() + stroke * 0.6, body.cy() - h * 0.18, w * 0.06, h * 0.36);
    p.fill_rounded(cap, cap.w * 0.5, Color::white(0.45));
    let inner = body.inset(stroke * 2.0);
    // the charge is five ticks, the last lit one partly
    let n = 5usize;
    let gap = (inner.h * 0.2).max(1.0);
    let cw = ((inner.w - gap * (n as f32 - 1.0)) / n as f32).max(1.0);
    let lvl = level.clamp(0.0, 1.0);
    let pos = (lvl * n as f32).max(if lvl > 0.0 { 0.7 } else { 0.0 });
    for i in 0..n {
        let t = (pos - i as f32).clamp(0.0, 1.0);
        let c = Color::white(0.14).lerp(crate::util::palette::BLUE.with_a(fill.a), t);
        p.fill_rounded(Rect::new(inner.x + i as f32 * (cw + gap), inner.y, cw, inner.h), cw.min(inner.h) * 0.3, c);
    }
    if charging {
        let bx = body.cx();
        let by = body.cy();
        // a slightly larger dark bolt underneath acts as the outline
        let bolt = |pb: &crate::gfx::PathBuilder, k: f32| {
            let pts = [
                (k * 0.15, -k),
                (-k * 0.55, k * 0.12),
                (-k * 0.02, k * 0.12),
                (-k * 0.15, k),
                (k * 0.55, -k * 0.12),
                (k * 0.02, -k * 0.12),
            ];
            pb.rounded_poly(&pts, k * 0.14);
        };
        let k = uq(q(h * 0.42));
        p.fill_cached((14, q(k), 0, 0, 0), bx, by, Color::black(0.85), |pb| bolt(pb, k * 1.32));
        p.fill_cached((15, q(k), 0, 0, 0), bx, by, Color::white(1.0), |pb| bolt(pb, k));
    }
}

// ---------------------------------------------------------------------------
// Outline icons (SF Symbols look)
// ---------------------------------------------------------------------------

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

/// Pen that draws in a 24x24 grid scaled to the target square.
struct Pen<'p, 'g> {
    p: &'p mut Painter<'g>,
    x: f32,
    y: f32,
    u: f32,
    w: f32,
    c: Color,
}

impl Pen<'_, '_> {
    fn ln(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (x, y, u) = (self.x, self.y, self.u);
        self.p.line(x + x0 * u, y + y0 * u, x + x1 * u, y + y1 * u, self.w, self.c);
    }

    /// Connected line segments.
    fn poly(&self, pts: &[(f32, f32)], closed: bool) {
        for pair in pts.windows(2) {
            self.ln(pair[0].0, pair[0].1, pair[1].0, pair[1].1);
        }
        if closed && pts.len() > 2 {
            let (a, b) = (pts[pts.len() - 1], pts[0]);
            self.ln(a.0, a.1, b.0, b.1);
        }
    }

    /// Like `poly`, but every corner is rounded off with a small curve of
    /// radius `rad` (so no icon ends in a sharp point).
    fn rpoly(&self, pts: &[(f32, f32)], closed: bool, rad: f32) {
        let n = pts.len();
        if n < 3 {
            self.poly(pts, closed);
            return;
        }
        let mut out: Vec<(f32, f32)> = Vec::with_capacity(n * 4);
        for i in 0..n {
            let p = pts[i];
            if !closed && (i == 0 || i == n - 1) {
                out.push(p);
                continue;
            }
            let (a, b) = (pts[(i + n - 1) % n], pts[(i + 1) % n]);
            let (dx1, dy1, dx2, dy2) = (a.0 - p.0, a.1 - p.1, b.0 - p.0, b.1 - p.1);
            let l1 = (dx1 * dx1 + dy1 * dy1).sqrt().max(1e-3);
            let l2 = (dx2 * dx2 + dy2 * dy2).sqrt().max(1e-3);
            let r = rad.min(l1 * 0.5).min(l2 * 0.5);
            let s = (p.0 + dx1 / l1 * r, p.1 + dy1 / l1 * r);
            let e = (p.0 + dx2 / l2 * r, p.1 + dy2 / l2 * r);
            for k in 0..=3 {
                let t = k as f32 / 3.0;
                let u = 1.0 - t;
                out.push((
                    u * u * s.0 + 2.0 * u * t * p.0 + t * t * e.0,
                    u * u * s.1 + 2.0 * u * t * p.1 + t * t * e.1,
                ));
            }
        }
        self.poly(&out, closed);
    }

    fn ring(&self, cx: f32, cy: f32, r: f32) {
        self.p.stroke_circle(self.x + cx * self.u, self.y + cy * self.u, r * self.u, self.w, self.c);
    }

    fn dot(&self, cx: f32, cy: f32, r: f32) {
        self.p.fill_circle(self.x + cx * self.u, self.y + cy * self.u, r * self.u, self.c);
    }

    fn rr(&self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let r = Rect::new(self.x + x * self.u, self.y + y * self.u, w * self.u, h * self.u);
        self.p.stroke_rounded(r, radius * self.u, self.w, self.c);
    }

    /// Arc around (cx, cy): `start` in radians (0 = 12 o'clock, clockwise).
    fn arc(&self, cx: f32, cy: f32, r: f32, start: f32, sweep: f32) {
        self.p.arc(self.x + cx * self.u, self.y + cy * self.u, r * self.u, start, sweep, self.w, self.c);
    }

    /// Point on a circle, same angle convention as `arc`.
    fn at(cx: f32, cy: f32, r: f32, a: f32) -> (f32, f32) {
        (cx + r * a.sin(), cy - r * a.cos())
    }

    /// `n` short rays around a center (sun, gear teeth).
    fn rays(&self, cx: f32, cy: f32, r0: f32, r1: f32, n: usize) {
        for i in 0..n {
            let a = i as f32 * (2.0 * PI / n as f32);
            let (x0, y0) = Self::at(cx, cy, r0, a);
            let (x1, y1) = Self::at(cx, cy, r1, a);
            self.ln(x0, y0, x1, y1);
        }
    }

    fn speaker(&self) {
        self.rpoly(&[(3.5, 9.5), (7.5, 9.5), (12.5, 5.2), (12.5, 18.8), (7.5, 14.5), (3.5, 14.5)], true, 1.3);
    }
}

/// Draw `icon` as an outline. Returns false for the icons that are drawn
/// as filled shapes elsewhere (media transport, sparkle, check, close, plus, minus).
fn outline(p: &mut Painter, icon: Icon, r: Rect, c: Color) -> bool {
    if matches!(
        icon,
        Icon::Play
            | Icon::Pause
            | Icon::Next
            | Icon::Prev
            | Icon::Stop
            | Icon::Sparkle
            | Icon::Check
            | Icon::Close
            | Icon::Plus
            | Icon::Minus
    ) {
        return false;
    }
    let s = r.w.min(r.h);
    let b = r.center_square(s);
    let pen = Pen { p, x: b.x, y: b.y, u: s / 24.0, w: (s * 0.09).max(1.3), c };
    match icon {
        Icon::Music => {
            // an equalizer: five beams of different heights
            for (x, top, bot) in
                [(4.5, 10.0, 14.0), (8.25, 6.0, 18.0), (12.0, 3.5, 20.5), (15.75, 7.5, 16.5), (19.5, 10.5, 13.5)]
            {
                pen.ln(x, top, x, bot);
            }
        }
        Icon::Timer => {
            pen.ring(12.0, 13.5, 7.6);
            pen.ln(10.0, 3.5, 14.0, 3.5);
            pen.ln(12.0, 3.5, 12.0, 5.9);
            pen.ln(12.0, 13.5, 15.0, 10.5);
            pen.ln(18.2, 7.6, 19.4, 6.4);
        }
        Icon::Clipboard => {
            pen.rr(5.0, 5.0, 14.0, 16.0, 3.0);
            pen.rr(8.8, 3.0, 6.4, 4.4, 2.0);
            pen.ln(8.5, 12.0, 15.5, 12.0);
            pen.ln(8.5, 16.0, 13.0, 16.0);
        }
        Icon::Download => {
            pen.ln(12.0, 4.0, 12.0, 15.0);
            pen.rpoly(&[(7.5, 10.8), (12.0, 15.2), (16.5, 10.8)], false, 1.4);
            pen.ln(5.0, 19.8, 19.0, 19.8);
        }
        Icon::Mic => {
            pen.rr(9.0, 3.5, 6.0, 11.0, 3.0);
            pen.arc(12.0, 11.0, 6.6, FRAC_PI_2, PI);
            pen.ln(12.0, 17.6, 12.0, 21.0);
            pen.ln(8.5, 21.0, 15.5, 21.0);
        }
        Icon::Camera => {
            pen.rr(3.0, 7.0, 18.0, 13.0, 3.5);
            pen.rpoly(&[(8.0, 7.0), (9.4, 4.4), (14.6, 4.4), (16.0, 7.0)], false, 1.3);
            pen.ring(12.0, 13.5, 3.6);
        }
        Icon::Bell => {
            pen.arc(12.0, 10.0, 6.0, -FRAC_PI_2, PI);
            pen.ln(6.0, 10.0, 5.2, 16.0);
            pen.ln(18.0, 10.0, 18.8, 16.0);
            pen.ln(4.2, 16.6, 19.8, 16.6);
            pen.arc(12.0, 18.6, 2.2, FRAC_PI_2, PI);
            pen.ln(12.0, 3.8, 12.0, 4.0);
        }
        Icon::Home => {
            pen.rpoly(&[(3.5, 11.6), (12.0, 4.0), (20.5, 11.6)], false, 2.2);
            pen.rpoly(&[(6.0, 10.2), (6.0, 19.8), (18.0, 19.8), (18.0, 10.2)], false, 2.4);
            pen.rpoly(&[(10.0, 19.8), (10.0, 14.6), (14.0, 14.6), (14.0, 19.8)], false, 1.6);
        }
        Icon::Settings => {
            pen.ring(12.0, 12.0, 3.2);
            pen.ring(12.0, 12.0, 7.2);
            pen.rays(12.0, 12.0, 8.2, 9.6, 8);
        }
        Icon::Folder => {
            pen.rr(3.0, 7.0, 18.0, 12.8, 3.0);
            pen.rpoly(&[(3.2, 10.0), (3.2, 6.8), (4.6, 5.0), (9.0, 5.0), (10.8, 7.0)], false, 1.4);
        }
        Icon::Copy => {
            pen.rr(9.0, 8.5, 11.0, 12.0, 2.6);
            pen.arc(6.6, 6.2, 2.6, -FRAC_PI_2, FRAC_PI_2);
            pen.ln(6.6, 3.6, 13.0, 3.6);
            pen.arc(13.0, 6.2, 2.6, 0.0, FRAC_PI_2);
            pen.ln(4.0, 6.2, 4.0, 13.6);
            pen.arc(6.6, 13.6, 2.6, PI, FRAC_PI_2);
            pen.ln(6.6, 16.2, 9.0, 16.2);
        }
        Icon::Trash => {
            pen.ln(4.5, 6.6, 19.5, 6.6);
            pen.rpoly(&[(9.4, 6.6), (9.4, 4.4), (14.6, 4.4), (14.6, 6.6)], false, 1.1);
            pen.rpoly(&[(6.4, 9.2), (7.4, 19.2), (16.6, 19.2), (17.6, 9.2)], false, 2.0);
            pen.ln(10.4, 11.8, 10.4, 16.4);
            pen.ln(13.6, 11.8, 13.6, 16.4);
        }
        Icon::Image => {
            pen.rr(3.5, 4.5, 17.0, 15.0, 3.5);
            pen.dot(9.0, 10.0, 1.5);
            pen.rpoly(&[(4.4, 18.2), (10.0, 13.0), (13.6, 16.4), (16.2, 14.2), (19.6, 17.6)], false, 1.6);
        }
        Icon::File => {
            pen.rr(5.0, 3.5, 14.0, 17.0, 3.0);
            pen.ln(8.6, 9.6, 15.4, 9.6);
            pen.ln(8.6, 13.2, 15.4, 13.2);
            pen.ln(8.6, 16.8, 12.6, 16.8);
        }
        Icon::Link => {
            // two chain links on a diagonal
            pen.arc(8.0, 16.0, 3.6, 3.0 * FRAC_PI_4, PI);
            pen.ln(10.55, 18.55, 13.4, 15.7);
            pen.ln(5.45, 13.45, 8.3, 10.6);
            pen.arc(16.0, 8.0, 3.6, 7.0 * FRAC_PI_4, PI);
            pen.ln(13.45, 5.45, 10.6, 8.3);
            pen.ln(18.55, 10.55, 15.7, 13.4);
            pen.ln(10.6, 13.4, 13.4, 10.6);
        }
        Icon::Refresh => {
            pen.arc(12.0, 12.0, 7.2, FRAC_PI_4, 1.5 * PI);
            let (tx, ty) = Pen::at(12.0, 12.0, 7.2, 1.75 * PI);
            pen.rpoly(&[(tx - 4.0, ty), (tx, ty), (tx, ty + 4.0)], false, 0.9);
        }
        Icon::Send => {
            pen.rpoly(&[(3.5, 11.0), (20.5, 3.5), (14.0, 20.5), (11.2, 13.2)], true, 1.8);
            pen.ln(11.2, 13.2, 20.5, 3.5);
        }
        Icon::Info => {
            pen.ring(12.0, 12.0, 9.0);
            pen.dot(12.0, 7.9, 1.1);
            pen.ln(12.0, 11.2, 12.0, 16.6);
        }
        Icon::Bolt => {
            pen.rpoly(&[(13.6, 3.0), (6.0, 13.4), (11.6, 13.4), (10.4, 21.0), (18.0, 10.6), (12.4, 10.6)], true, 1.5);
        }
        Icon::Globe => {
            pen.ring(12.0, 12.0, 9.0);
            pen.rr(7.6, 3.0, 8.8, 18.0, 4.4);
            pen.ln(3.0, 12.0, 21.0, 12.0);
            pen.ln(5.0, 7.6, 19.0, 7.6);
            pen.ln(5.0, 16.4, 19.0, 16.4);
        }
        Icon::Keyboard => {
            pen.rr(2.5, 6.5, 19.0, 11.0, 3.0);
            for x in [7.0, 10.3, 13.7, 17.0] {
                pen.ln(x, 9.3, x, 10.9);
            }
            pen.ln(8.0, 14.4, 16.0, 14.4);
        }
        Icon::Package => {
            pen.rpoly(&[(12.0, 3.4), (19.6, 7.6), (19.6, 16.4), (12.0, 20.6), (4.4, 16.4), (4.4, 7.6)], true, 1.8);
            pen.rpoly(&[(4.8, 7.8), (12.0, 11.8), (19.2, 7.8)], false, 1.2);
            pen.ln(12.0, 11.8, 12.0, 20.2);
        }
        Icon::Volume => {
            pen.speaker();
            // the level as three beams that grow
            pen.ln(15.4, 10.4, 15.4, 13.6);
            pen.ln(18.2, 8.2, 18.2, 15.8);
            pen.ln(21.0, 6.0, 21.0, 18.0);
        }
        Icon::Mute => {
            pen.speaker();
            pen.ln(16.2, 9.6, 21.0, 14.4);
            pen.ln(21.0, 9.6, 16.2, 14.4);
        }
        Icon::Brightness => {
            pen.ring(12.0, 12.0, 4.2);
            pen.rays(12.0, 12.0, 7.4, 9.8, 8);
        }
        Icon::Headphones => {
            pen.arc(12.0, 13.0, 7.5, -FRAC_PI_2, PI);
            pen.rr(4.5, 13.0, 3.6, 6.6, 1.8);
            pen.rr(15.9, 13.0, 3.6, 6.6, 1.8);
        }
        Icon::Bluetooth => {
            pen.rpoly(&[(6.5, 16.6), (17.5, 8.0), (12.0, 3.5), (12.0, 20.5), (17.5, 16.0), (6.5, 7.4)], false, 1.2);
        }
        Icon::Mouse => {
            pen.rr(6.5, 3.5, 11.0, 17.0, 5.5);
            pen.ln(12.0, 3.8, 12.0, 9.4);
        }
        Icon::Gamepad => {
            pen.rr(2.5, 7.5, 19.0, 10.0, 5.0);
            pen.ln(7.2, 10.6, 7.2, 14.4);
            pen.ln(5.3, 12.5, 9.1, 12.5);
            pen.dot(15.8, 11.4, 1.0);
            pen.dot(18.2, 13.6, 1.0);
        }
        Icon::Phone => {
            pen.rr(7.0, 2.6, 10.0, 18.8, 2.6);
            pen.ln(10.6, 18.0, 13.4, 18.0);
        }
        Icon::Pulse => {
            // load as a row of beams
            for (x, top) in [(5.0, 14.0), (9.7, 9.0), (14.3, 4.5), (19.0, 11.5)] {
                pen.ln(x, top, x, 20.0);
            }
        }
        Icon::Wifi => {
            // signal strength: beams that rise
            for (x, top) in [(5.0, 16.5), (9.7, 12.5), (14.3, 8.5), (19.0, 4.5)] {
                pen.ln(x, top, x, 20.0);
            }
        }
        Icon::Cloud => {
            pen.arc(6.8, 15.0, 3.5, PI, PI);
            pen.arc(11.5, 11.2, 5.2, -0.45 * PI, 1.0 * PI);
            pen.arc(16.8, 15.0, 3.5, 0.0, PI);
            pen.ln(6.8, 18.5, 16.8, 18.5);
        }
        Icon::Lock => {
            pen.rr(5.5, 10.5, 13.0, 10.0, 2.8);
            pen.arc(12.0, 10.5, 4.0, -FRAC_PI_2, PI);
            pen.dot(12.0, 15.4, 1.2);
        }
        Icon::Edit => {
            pen.rpoly(&[(15.6, 5.4), (18.6, 8.4), (9.4, 17.6), (5.4, 18.6), (6.4, 14.6)], true, 1.2);
            pen.ln(13.6, 7.4, 16.6, 10.4);
        }
        Icon::Unlock => {
            // padlock with the shackle swung open
            pen.rr(5.0, 11.0, 14.0, 10.0, 3.0);
            pen.dot(12.0, 16.0, 1.3);
            pen.ln(8.0, 11.0, 8.0, 8.4);
            pen.arc(12.0, 8.4, 4.0, -FRAC_PI_2, 0.62 * PI);
        }
        Icon::Control => {
            pen.rr(3.0, 4.5, 18.0, 6.5, 3.25);
            pen.dot(16.0, 7.75, 1.9);
            pen.rr(3.0, 13.0, 18.0, 6.5, 3.25);
            pen.dot(8.0, 16.25, 1.9);
        }
        Icon::Chat => {
            pen.rr(3.0, 4.5, 18.0, 12.5, 5.0);
            pen.rpoly(&[(8.2, 17.0), (7.2, 20.4), (11.8, 17.0)], false, 0.9);
        }
        Icon::Moon => {
            // a crescent: the outside of one circle and the inside of another
            pen.arc(12.0, 12.0, 8.5, 1.752, 4.349);
            pen.arc(15.5, 8.5, 7.0, 2.374, 3.107);
        }
        Icon::Grid => {
            pen.rr(4.0, 4.0, 6.5, 6.5, 2.0);
            pen.rr(13.5, 4.0, 6.5, 6.5, 2.0);
            pen.rr(4.0, 13.5, 6.5, 6.5, 2.0);
            pen.rr(13.5, 13.5, 6.5, 6.5, 2.0);
        }
        Icon::Cpu => {
            pen.rr(6.0, 6.0, 12.0, 12.0, 2.5);
            pen.rr(9.5, 9.5, 5.0, 5.0, 1.0);
            for v in [9.5, 14.5] {
                pen.ln(v, 3.0, v, 6.0);
                pen.ln(v, 18.0, v, 21.0);
                pen.ln(3.0, v, 6.0, v);
                pen.ln(18.0, v, 21.0, v);
            }
        }
        Icon::Ram => {
            pen.rr(2.5, 6.5, 19.0, 9.5, 1.8);
            pen.ln(6.0, 16.0, 6.0, 18.5);
            pen.ln(10.0, 16.0, 10.0, 18.5);
            pen.ln(14.0, 16.0, 14.0, 18.5);
            pen.ln(18.0, 16.0, 18.0, 18.5);
            pen.ln(6.5, 10.0, 6.5, 12.5);
            pen.ln(12.0, 10.0, 12.0, 12.5);
            pen.ln(17.5, 10.0, 17.5, 12.5);
        }
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------
// Weather glyphs: the same monoline strokes as the icon set, with the rain,
// the sun's rays and the lightning drawn as accent-coloured beams.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Storm,
}

/// A cloud made of three arcs and a base line, scaled by `k` around the middle
/// of the 24-unit grid and moved by (`ox`, `oy`).
fn sky_cloud(pen: &Pen, ox: f32, oy: f32, k: f32) {
    let tx = |x: f32| 12.0 + (x - 12.0) * k + ox;
    let ty = |y: f32| 12.0 + (y - 12.0) * k + oy;
    pen.arc(tx(6.8), ty(15.0), 3.5 * k, PI, PI);
    pen.arc(tx(11.5), ty(11.2), 5.2 * k, -0.45 * PI, PI);
    pen.arc(tx(16.8), ty(15.0), 3.5 * k, 0.0, PI);
    pen.ln(tx(6.8), ty(18.5), tx(16.8), ty(18.5));
}

/// A crescent: the outside of one circle with a bite taken out by a second.
fn sky_moon(pen: &Pen, cx: f32, cy: f32, r: f32) {
    let (c2x, c2y, r2) = (cx + 0.55 * r, cy - 0.45 * r, 0.82 * r);
    let (dx, dy) = (c2x - cx, c2y - cy);
    let d = (dx * dx + dy * dy).sqrt();
    let a = (r * r - r2 * r2 + d * d) / (2.0 * d);
    let phi = (a / r).clamp(-1.0, 1.0).acos();
    let psi = ((d - a) / r2).clamp(-1.0, 1.0).acos();
    let toward = dx.atan2(-dy); // direction of the bite, 0 = 12 o'clock, clockwise
    pen.arc(cx, cy, r, toward + phi, 2.0 * PI - 2.0 * phi);
    pen.arc(c2x, c2y, r2, toward + PI - psi, 2.0 * psi);
}

pub fn sky(p: &mut Painter, kind: Sky, r: Rect, night: bool, c: Color, accent: Color) {
    let s = r.w.min(r.h);
    let b = r.center_square(s);
    let mut pen = Pen { p, x: b.x, y: b.y, u: s / 24.0, w: (s * 0.085).max(1.3), c };
    let dim = c.alpha(0.5);
    // white line art; only water (rain, drizzle, snow) is drawn in the accent colour
    match kind {
        Sky::Clear => {
            if night {
                sky_moon(&pen, 12.0, 12.0, 7.4);
                pen.ln(18.6, 4.6, 18.6, 7.4);
                pen.ln(17.2, 6.0, 20.0, 6.0);
            } else {
                pen.ring(12.0, 12.0, 4.2);
                pen.rays(12.0, 12.0, 7.4, 10.0, 8);
            }
        }
        Sky::PartlyCloudy => {
            if night {
                sky_moon(&pen, 8.6, 8.8, 4.4);
            } else {
                pen.ring(8.4, 8.6, 2.8);
                for a in [-0.5 * PI, -0.75 * PI, -0.25 * PI, 0.0, 0.25 * PI] {
                    let (x0, y0) = Pen::at(8.4, 8.6, 5.0, a);
                    let (x1, y1) = Pen::at(8.4, 8.6, 6.8, a);
                    pen.ln(x0, y0, x1, y1);
                }
            }
            sky_cloud(&pen, 2.4, 3.2, 0.8);
        }
        Sky::Cloudy => {
            pen.c = dim;
            pen.arc(17.4, 9.0, 3.3, -0.4 * PI, 0.9 * PI);
            pen.c = c;
            sky_cloud(&pen, -1.4, 2.0, 0.88);
        }
        Sky::Fog => {
            sky_cloud(&pen, 0.0, -3.6, 0.86);
            pen.ln(6.0, 17.4, 18.0, 17.4);
            pen.ln(4.5, 20.4, 15.0, 20.4);
        }
        Sky::Drizzle | Sky::Rain => {
            sky_cloud(&pen, 0.0, -3.2, 0.9);
            pen.c = accent;
            let xs: &[f32] = if kind == Sky::Rain { &[8.0, 12.0, 16.0] } else { &[10.0, 14.0] };
            let len = if kind == Sky::Rain { 3.8 } else { 2.4 };
            for &x in xs {
                pen.ln(x + 0.6, 17.8, x + 0.6 - len * 0.34, 17.8 + len);
            }
        }
        Sky::Snow => {
            sky_cloud(&pen, 0.0, -3.2, 0.9);
            pen.c = accent;
            pen.dot(8.4, 18.4, 1.0);
            pen.dot(12.0, 21.0, 1.0);
            pen.dot(15.6, 18.4, 1.0);
        }
        Sky::Storm => {
            sky_cloud(&pen, 0.0, -3.6, 0.9);
            pen.rpoly(&[(13.0, 15.4), (10.2, 19.2), (13.8, 19.2), (11.0, 23.0)], false, 1.0);
            pen.c = accent;
            pen.ln(7.4, 17.6, 6.6, 19.8);
            pen.ln(17.0, 17.6, 16.2, 19.8);
        }
    }
}
