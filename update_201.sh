#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "animate_slow" src/modules/mod.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---- Effects / Ui: a second, calmer kind of "keep drawing"
p='src/modules/mod.rs'; s=open(p).read()
s=sub(s,"""    /// Need another frame right away (caret blink, spinners...).
    pub animate: bool,""","""    /// Need another frame right away (caret blink, spinners...).
    pub animate: bool,
    /// Need more frames, but only a slow, gentle animation is running (a floating icon, a
    /// scanning spinner): half the frames are enough and nobody can tell the difference.
    pub animate_slow: bool,""")
open(p,'w').write(s)
p='src/ui.rs'; s=open(p).read()
s=sub(s,"""    pub fn animate(&mut self) {
        self.fx.animate = true;
    }
""","""    pub fn animate(&mut self) {
        self.fx.animate = true;
    }

    /// Like `animate`, for slow, gentle motion: drawn at about 30 frames a second.
    pub fn animate_slow(&mut self) {
        self.fx.animate_slow = true;
    }
""")
open(p,'w').write(s)

# ---- the gentle animations
p='src/alcove.rs'; s=open(p).read()
s=sub(s,"""    if ui.hovered(r) {
        ui.animate();
    }
}""","""    if ui.hovered(r) {
        ui.animate_slow();
    }
}""")
s=sub(s,"""    if spinning {
        ui.animate();""","""    if spinning {
        ui.animate_slow();""")
open(p,'w').write(s)
p='src/modules/wifi.rs'; s=open(p).read()
s=sub(s,"""        if self.scanning {
            ui.animate();""","""        if self.scanning {
            ui.animate_slow();""")
open(p,'w').write(s)
p='src/modules/bluetooth.rs'; s=open(p).read()
s=sub(s,"""        if self.scanning {
            ui.animate();""","""        if self.scanning {
            ui.animate_slow();""")
open(p,'w').write(s)
p='src/modules/weather.rs'; s=open(p).read()
s=sub(s,"""        if self.loading {
            ui.animate();""","""        if self.loading {
            ui.animate_slow();""")
open(p,'w').write(s)
p='src/modules/downloads.rs'; s=open(p).read()
s=sub(s,"""                if a.progress.is_none() {
                    ui.animate();""","""                if a.progress.is_none() {
                    ui.animate_slow();""")
open(p,'w').write(s)

# ---- app.rs
p='src/app.rs'; s=open(p).read()
s=sub(s,"    bd_move: Instant,\n","""    bd_move: Instant,
    /// The last frame only asked for a slow animation: every other frame is skipped.
    slow_only: bool,
    /// What the bottom strip picture was made from, and when. It is only made again when that
    /// changes or after 50 ms, not on every frame of a page that just sits there.
    band_sig: Option<BandSig>,
    band_at: Instant,
""")
s=sub(s,"            bd_move: Instant::now(),\n","            bd_move: Instant::now(),\n            slow_only: false,\n            band_sig: None,\n            band_at: Instant::now(),\n")
s=sub(s,"type RimKey = (i32, i32, i32, i32, i32);","type RimKey = (i32, i32, i32, i32, i32);\n/// Kind (page or peek), the page, the strip's rectangle in quarter pixels, a page change running.\ntype BandSig = (u8, ModuleId, i32, i32, i32, i32, bool);")

# request_frame: anything but a slow animation wants full speed again
s=sub(s,"""    fn request_frame(&mut self) {
        self.dirty = true;
        self.pacer.start();
    }""","""    fn request_frame(&mut self) {
        self.dirty = true;
        self.slow_only = false;
        self.pacer.start();
    }""")
# apply_effects
s=sub(s,"""        if fx.redraw || fx.animate || fx.layout_changed {
            self.request_frame();
        }""","""        if fx.redraw || fx.animate || fx.layout_changed {
            self.request_frame();
        } else if fx.animate_slow {
            // only a slow animation: frames keep coming, every other one is skipped in `frame`
            self.dirty = true;
            self.slow_only = true;
            self.pacer.start();
        }""")
# frame(): skip
s=sub(s,"""        if !self.dirty && self.settled && now.duration_since(self.last_frame) < AMBIENT_GAP {
            return;
        }""","""        if !self.dirty && self.settled && now.duration_since(self.last_frame) < AMBIENT_GAP {
            return;
        }
        // a slow animation (floating icon, scanning spinner) is drawn at about 30 frames a second;
        // anything else that wants a frame (the pointer moving, a click, a spring) clears this flag
        if self.slow_only && self.settled && now.duration_since(self.last_frame) < Duration::from_millis(30) {
            return;
        }
        self.slow_only = false;""")

# prepare_band: not on every frame
s=sub(s,"""    fn prepare_band(&mut self, now: Instant) {
        let t = self.targets();""","""    /// True while the strip picture made a moment ago can be used again: same page, same place,
    /// no page change running, less than 50 ms old. Another frame is asked for so a fresh picture
    /// follows shortly after (nothing is left stale when the frames would stop here).
    fn band_fresh(&mut self, sig: BandSig, now: Instant) -> bool {
        let same = self.band_sig == Some(sig);
        self.band_sig = Some(sig);
        let ready = self.snap_band.as_ref().is_some_and(|b| b.is_baked());
        if same && ready && !sig.6 && now.saturating_duration_since(self.band_at) < Duration::from_millis(50) {
            self.request_frame();
            return true;
        }
        self.band_at = now;
        false
    }

    fn prepare_band(&mut self, now: Instant) {
        let t = self.targets();""")
s=sub(s,"""            self.snap_band = None;
            self.band_since = None;
            return;
        }
        self.band_since.get_or_insert(now);""","""            self.snap_band = None;
            self.band_since = None;
            self.band_sig = None;
            return;
        }
        self.band_since.get_or_insert(now);""")
s=sub(s,"""            let trans = self.page_from.map(|(from, dir)| (from, dir, self.anim.page.value.clamp(0.0, 1.0)));
            let tabs = if trans.is_some() { self.tab_list() } else { Vec::new() };""","""            let trans = self.page_from.map(|(from, dir)| (from, dir, self.anim.page.value.clamp(0.0, 1.0)));
            let q = |v: f32| (v * scale * 4.0).round() as i32;
            if self.band_fresh((1, page, q(region.x), q(region.y), q(region.w), q(region.h), trans.is_some()), now) {
                return;
            }
            let tabs = if trans.is_some() { self.tab_list() } else { Vec::new() };""")
s=sub(s,"""        let (accent, page, scale) = (self.accent, self.page, self.scale);
        if page_band {""","""        let (accent, page, scale) = (self.accent, self.page, self.scale);
        if peek_band {
            let h = BAND_H.min(body.h * 0.4);
            let q = |v: f32| (v * scale * 4.0).round() as i32;
            if self.band_fresh((2, "peek", q(body.x), q(body.bottom() - h), q(body.w), q(h), false), now) {
                return;
            }
        }
        if page_band {""")
s=sub(s,"""            log!("bottom blur could not be built: staying solid");
            self.band_failed = true;
            self.snap_band = None;
            self.band_since = None;""","""            log!("bottom blur could not be built: staying solid");
            self.band_failed = true;
            self.snap_band = None;
            self.band_since = None;
            self.band_sig = None;""")
open(p,'w').write(s)

# ---- doc comment back on prepare_band
p='src/app.rs'; s=open(p).read()
doc="""    /// Alcove: the bottom edge of the open notch (a page, or a bigger peek such as the volume dial
    /// or a message) softens into a light blur. Only the thin strip along the bottom is drawn a
    /// second time into a small picture (and only on frames that are drawn anyway), then laid over
    /// the live drawing in `bottom_fade`. It is there as soon as the notch is open, also while pages
    /// switch.
"""
assert s.count(doc)==1
s=s.replace(doc,"")
a="    fn prepare_band(&mut self, now: Instant) {"
assert s.count(a)==1
s=s.replace(a,doc+a)
open(p,'w').write(s)
PYEOF
echo "Patched v201"
