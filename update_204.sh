#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "glass_live" src/config.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- config: the switch
p='src/config.rs'; s=open(p).read()
s=sub(s,"""    pub glass_look: bool,
""","""    pub glass_look: bool,
    /// Alcove: the glass edge keeps up with what is behind the notch (the screen is looked at again
    /// a few times a second while the notch is open). Turn off to save a little CPU.
    pub glass_live: bool,
""")
s=sub(s,"            glass_look: true,\n","            glass_look: true,\n            glass_live: true,\n")
s=sub(s,"glass_look = true\n","""glass_look = true
# Alcove: the glass edge keeps up with what is behind the notch (a little extra CPU while open;
# it also hides the open notch from screenshots and screen recordings)
glass_live = true
""")
open(p,'w').write(s)

# ---------------- settings: the row
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"""        0 => 14.0,
        1 => 14.0,""","""        0 => 14.0,
        1 => 15.0,""")
s=sub(s,"""            "appearance",
            "glass_look",
        );
""","""            "appearance",
            "glass_look",
        );
        toggle_row(
            ui,
            row_rect(body, 14),
            "Live glass (follows what is behind)",
            a.glass_live,
            "appearance",
            "glass_live",
        );
""")
open(p,'w').write(s)

# ---------------- window.rs: the wake-up message
p='src/window.rs'; s=open(p).read()
s=sub(s,"pub const WM_APP_DRAG: u32 = WM_APP + 5;\n","pub const WM_APP_DRAG: u32 = WM_APP + 5;\npub const WM_APP_GLASS: u32 = WM_APP + 6;\n")
open(p,'w').write(s)

# ---------------- refract.rs
p='src/refract.rs'; s=open(p).read()
s=sub(s,"""//! A snapshot of the screen is taken once when the notch opens (while the window is still small, so
//! it does not photograph itself). The rim is worked out from it on the CPU for the notch's current
//! size and uploaded as one small picture, drawn with a single call; it is only made again when the
//! size changes (while a page change resizes the notch, and never while nothing moves).
""","""//! A snapshot of the screen is taken when the notch opens (while the window is still small, so it
//! does not photograph itself). The rim is worked out from it on the CPU for the notch's current
//! size and uploaded as one small picture, drawn with a single call; it is only made again when the
//! size changes (while a page change resizes the notch, and never while nothing moves).
//!
//! Live glass: while the notch is open the screen is looked at again every so often, on a
//! background thread (`refresh`). The notch is hidden from screen copies for that (the window
//! is excluded from capture), so the copy shows only what is really behind it. A copy that turns
//! out identical to the last one is thrown away at once and costs nothing more.
""")
s=sub(s,"""                        let len = w as usize * h as usize;
                        let mut px = std::slice::from_raw_parts(bits as *const u32, len).to_vec();
                        // a screen copy has no alpha: make every pixel opaque
                        for p in px.iter_mut() {
                            *p |= 0xFF00_0000;
                        }
                        shot""","""                        let len = w as usize * h as usize;
                        // (a screen copy has no alpha; only the colours are ever read)
                        let px = std::slice::from_raw_parts(bits as *const u32, len).to_vec();
                        shot""")
s=sub(s,"#[derive(Clone, Copy)]\npub struct Shape {","#[derive(Clone, Copy, PartialEq)]\npub struct Shape {")
s=sub(s,"""fn smooth(a: f32, b: f32, x: f32) -> f32 {""","""/// A quick fingerprint of a screen copy (about one pixel in thirteen): equal fingerprints mean
/// "nothing worth redrawing changed".
pub fn checksum(s: &Shot) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &p in s.px.iter().step_by(13) {
        h = (h ^ (p & 0x00FF_FFFF) as u64).wrapping_mul(0x0000_0100_0000_01b3);
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

/// Copy the screen again and, when it differs from the copy with fingerprint `prev`, work out the
/// rim for `shape` as well. Meant for a background thread; None when the screen could not be copied.
pub fn refresh(x: i32, y: i32, w: i32, h: i32, prev: u64, shape: Shape) -> Option<Fetched> {
    let shot = grab(x, y, w, h)?;
    let sum = checksum(&shot);
    if sum == prev {
        return Some(Fetched::Same);
    }
    let made = rim(&shot, shape, 1);
    Some(Fetched::Fresh(Fresh { shot, sum, shape, rim: made }))
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {""")
open(p,'w').write(s)

# ---------------- app.rs
p='src/app.rs'; s=open(p).read()
s=sub(s,"use crate::window::{WM_APP_BUS, WM_APP_DRAG, WM_APP_FOREGROUND, WM_APP_FRAME, WM_APP_TRAY};","use crate::window::{WM_APP_BUS, WM_APP_DRAG, WM_APP_FOREGROUND, WM_APP_FRAME, WM_APP_GLASS, WM_APP_TRAY};")
s=sub(s,"const AMBIENT_GAP: Duration = Duration::from_millis(12);\n","""const AMBIENT_GAP: Duration = Duration::from_millis(12);
/// Live glass: how often the screen behind the notch is looked at again, and how often once
/// it has not changed for a while.
const LIVE_FAST: Duration = Duration::from_millis(350);
const LIVE_SLOW: Duration = Duration::from_millis(800);
/// Seconds for the glass edge to melt from the old picture into a new one.
const LIVE_FADE: f32 = 0.4;
""")
s=sub(s,"    rim_geo: Option<(i32, i32, i32, i32, i32)>,\n","""    rim_geo: Option<(i32, i32, i32, i32, i32)>,
    /// The shape (and its quantised key) the current rim was made for.
    rim_last: Option<((i32, i32, i32, i32, i32), crate::refract::Shape)>,
    /// Live glass: a refresh running on a background thread.
    live_rx: Option<Receiver<Option<crate::refract::Fetched>>>,
    /// When the next refresh is due.
    live_next: Option<Instant>,
    /// How many refreshes in a row found nothing new.
    live_same: u8,
    /// Fingerprint of the current screen copy.
    live_sum: u64,
    /// This Windows cannot hide the notch from screen copies: no live glass.
    live_dead: bool,
    /// The notch is hidden from screen copies right now.
    excl: bool,
    /// The previous rim picture, while the new one melts in over it.
    rim_old: Option<(RimKey, gfx::RimPic)>,
    rim_x: f32,
""")
s=sub(s,"            rim_geo: None,\n","""            rim_geo: None,
            rim_last: None,
            live_rx: None,
            live_next: None,
            live_same: 0,
            live_sum: 0,
            live_dead: false,
            excl: false,
            rim_old: None,
            rim_x: 1.0,
""")
# message
s=sub(s,"""            WM_APP_BUS => {
                self.drain_bus();
                Some(LRESULT(0))
            }""","""            WM_APP_BUS => {
                self.drain_bus();
                Some(LRESULT(0))
            }
            WM_APP_GLASS => {
                self.live_done();
                Some(LRESULT(0))
            }""")
# on_wake
s=sub(s,"""        self.schedule_wake();
    }

    /// Arm a single Win32 timer for the earliest pending deadline.""","""        self.live_tick(now);
        self.schedule_wake();
    }

    /// Arm a single Win32 timer for the earliest pending deadline.""")
s=sub(s,"""        if let Some(t) = self.trim_at {
            consider(t);
        }
        if self.mode == Mode::Expanded && self.page == HOME {
            // the flip clock""","""        if let Some(t) = self.trim_at {
            consider(t);
        }
        if self.live_armed() {
            if let Some(t) = self.live_next {
                consider(t);
            }
            if self.live_rx.is_some() {
                // the answer normally arrives as a message; this is only a safety net
                consider(now + Duration::from_millis(60));
            }
        }
        if self.mode == Mode::Expanded && self.page == HOME {
            // the flip clock""")

# prepare_rim
s=sub(s,"""            self.refr_rim = None;
            self.refr_dead = false;
            self.rim_ok = false;
            self.rim_k = 0.0;
            return;
        }
        let fg = unsafe { GetForegroundWindow() };
        if self.refr_shot.is_some() && fg != self.hwnd && fg.0 as isize != self.refr_fg {""","""            self.refr_rim = None;
            self.refr_dead = false;
            self.rim_ok = false;
            self.rim_k = 0.0;
            self.live_off();
            return;
        }
        let fg = unsafe { GetForegroundWindow() };
        // (with live glass the copy is renewed anyway, so a new window in front is no reason to stop)
        if self.refr_shot.is_some() && !self.excl && fg != self.hwnd && fg.0 as isize != self.refr_fg {""")
s=sub(s,"""            self.refr_shot = crate::refract::grab(x, y, w, h);
            self.refr_retry = Some(now);""","""            self.refr_shot = crate::refract::grab(x, y, w, h);
            if let Some(shot) = &self.refr_shot {
                self.live_sum = crate::refract::checksum(shot);
            }
            self.refr_retry = Some(now);""")
s=sub(s,"""        if !open {
            self.rim_ok = false;
            return;
        }
        let Some(shot) = self.refr_shot.as_ref() else {""","""        if !open {
            self.rim_ok = false;
            self.live_off();
            return;
        }
        self.live_sync(now);
        let Some(shot) = self.refr_shot.as_ref() else {""")
s=sub(s,"""        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, step);
        if self.refr_rim.as_ref().is_some_and(|(k, _)| *k == key) {
            self.rim_ok = true;
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
""","""        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, step);
        let shape = crate::refract::Shape {
            left: body.x * sc,
            right: body.right() * sc,
            bottom: body.bottom() * sc,
            radius: r * sc,
            scale: sc,
        };
        self.rim_last = Some((geo, shape));
        if self.refr_rim.as_ref().is_some_and(|(k, _)| *k == key) {
            self.rim_ok = true;
            return;
        }
        self.refr_rim = None;
""")
# new methods, placed before band_k doc
s=sub(s,"""    /// 1 while the soft bottom edge is showing, else 0.
    fn band_k(&self) -> f32 {""","""    // ----- live glass -----------------------------------------------------------------------

    /// Live glass is switched on, the notch is hidden from screen copies and there is a copy.
    fn live_armed(&self) -> bool {
        self.excl && self.refr_shot.is_some() && self.cfg.appearance.glass_live
    }

    /// ... and the rim is showing, so a refresh would be seen.
    fn live_wanted(&self) -> bool {
        self.live_armed() && self.rim_ok
    }

    /// Hide the notch from screen copies (or show it again). Needs Windows 10 version 2004 or newer;
    /// when that fails live glass is simply off.
    fn set_excl(&mut self, on: bool) {
        if self.excl == on {
            return;
        }
        let how = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        match unsafe { SetWindowDisplayAffinity(self.hwnd, how) } {
            Ok(()) => self.excl = on,
            Err(e) => {
                if on {
                    log!("live glass: the notch cannot be hidden from screen copies here ({e}); not used");
                    self.live_dead = true;
                }
            }
        }
    }

    /// The notch is open: switch live glass on or off to match the setting and arm the first refresh.
    fn live_sync(&mut self, now: Instant) {
        let want = self.cfg.appearance.glass_live && !self.live_dead;
        self.set_excl(want);
        if !(want && self.excl) {
            self.live_next = None;
            self.live_rx = None;
            return;
        }
        if self.live_next.is_none() && self.live_rx.is_none() && self.refr_shot.is_some() {
            self.live_same = 0;
            self.live_next = Some(now + LIVE_FAST);
        }
    }

    /// The notch is closed (or the look is off): everything live stops and the notch shows in
    /// screen copies again.
    fn live_off(&mut self) {
        self.set_excl(false);
        self.live_next = None;
        self.live_rx = None;
        self.live_same = 0;
        self.rim_old = None;
        self.rim_x = 1.0;
    }

    /// Timer: pick up a finished refresh, and start the next one when it is due and the notch holds
    /// still (a refresh is made for one size; a moving notch would leave it out of date).
    fn live_tick(&mut self, now: Instant) {
        self.live_done();
        if !self.live_armed() || self.live_rx.is_some() {
            return;
        }
        if self.live_next.is_none_or(|t| t > now + Duration::from_millis(2)) {
            return;
        }
        let steady = self.rim_last.is_some_and(|(geo, _)| self.rim_geo == Some(geo));
        let (Some((_, shape)), true) = (self.rim_last, steady && self.rim_ok && self.page_from.is_none() && self.settled)
        else {
            self.live_next = Some(now + Duration::from_millis(120));
            return;
        };
        let (x, y, w, h) = self.win_px;
        let prev = self.live_sum;
        let hwnd = SendHwnd::new(self.hwnd);
        let (tx, rx) = std::sync::mpsc::channel();
        let job = std::thread::Builder::new().name("glass".into()).spawn(move || {
            let out = crate::refract::refresh(x, y, w, h, prev, shape);
            let _ = tx.send(out);
            unsafe {
                let _ = PostMessageW(Some(hwnd.hwnd()), WM_APP_GLASS, WPARAM(0), LPARAM(0));
            }
        });
        if job.is_ok() {
            self.live_rx = Some(rx);
            self.live_next = None;
        } else {
            self.live_next = Some(now + LIVE_SLOW);
        }
    }

    /// A refresh has finished: use the new copy, and melt the new rim in over the old one.
    fn live_done(&mut self) {
        let Some(rx) = self.live_rx.as_ref() else { return };
        let out = match rx.try_recv() {
            Ok(out) => out,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        self.live_rx = None;
        let fresh = match out {
            Some(crate::refract::Fetched::Fresh(f)) => Some(f),
            _ => None,
        };
        // a failed copy counts like "nothing new": it is simply tried again later
        self.live_same = if fresh.is_some() { 0 } else { (self.live_same + 1).min(8) };
        let gap = if self.live_same >= 3 { LIVE_SLOW } else { LIVE_FAST };
        self.live_next = Some(Instant::now() + gap);
        if let Some(f) = fresh {
            if self.live_wanted() {
                self.live_sum = f.sum;
                self.refr_shot = Some(f.shot);
                let sc = self.scale;
                // the rim made on the thread is only good if the notch kept its size meanwhile;
                // otherwise the next frame makes it again from the new copy
                if let (Some(rim), Some((geo, shape))) = (f.rim, self.rim_last) {
                    if shape == f.shape && self.page_from.is_none() {
                        if let Some(pic) = self.res.rim_pic(rim.x0, rim.y0, rim.w, rim.h, rim.step, &rim.px, sc) {
                            let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, 1);
                            self.rim_old = self.refr_rim.take();
                            self.rim_x = if self.rim_old.is_some() { 0.0 } else { 1.0 };
                            self.refr_rim = Some((key, pic));
                            self.request_frame();
                        }
                    }
                }
            }
        }
        self.schedule_wake();
    }

    /// 1 while the soft bottom edge is showing, else 0.
    fn band_k(&self) -> f32 {""")
# per frame fade progress
s=sub(s,"""        let bd_k = self.bd_k;
        let a_r = self.anim.r.value;""","""        if self.rim_old.is_some() {
            // a new rim is melting in over the old one (see `live_done`); 30 frames a second is plenty
            self.rim_x += rim_dt / LIVE_FADE;
            if self.rim_x >= 1.0 {
                self.rim_x = 1.0;
                self.rim_old = None;
            } else {
                fx.animate_slow = true;
            }
        }
        let bd_k = self.bd_k;
        let a_r = self.anim.r.value;""")
# draw
s=sub(s,"""                    if let Some((_, pic)) = &self.refr_rim {
                        p.draw_rim(pic, a_glass * self.rim_k);
                    }""","""                    if let Some((key, pic)) = &self.refr_rim {
                        let a = a_glass * self.rim_k;
                        match &self.rim_old {
                            // (only while the old picture still fits the notch's size)
                            Some((old_key, old)) if old_key.0 == key.0 && old_key.1 == key.1 && old_key.2 == key.2 && old_key.3 == key.3 && old_key.4 == key.4 => {
                                let t = self.rim_x.clamp(0.0, 1.0);
                                let t = t * t * (3.0 - 2.0 * t);
                                p.draw_rim(old, a * (1.0 - t));
                                p.draw_rim(pic, a * t);
                            }
                            _ => p.draw_rim(pic, a),
                        }
                    }""")
open(p,'w').write(s)
PYEOF
echo "Patched v204"
