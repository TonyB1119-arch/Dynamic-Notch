#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "refr_dead" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- refract.rs: stronger bend, colour split, no white highlight
p='src/refract.rs'; s=open(p).read()
s=sub(s,"    let width = 15.0 * sc; // how far in from the edge the rim reaches\n    let pull = 12.0 * sc; // how far out the strongest pixels look\n",
        "    let width = 26.0 * sc; // how far in from the edge the rim reaches\n    let pull = 40.0 * sc; // how far out the strongest pixels look\n")
a0=s.index("            let t = (d / width).clamp(0.0, 1.0);\n")
a1=s.index("            px[j * w + i] = out;\n",a0)+len("            px[j * w + i] = out;\n")
new='''            let t = (d / width).clamp(0.0, 1.0);
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
'''
s=s[:a0]+new+s[a1:]
s=sub(s,"/// pixel shows what lies a little further out (pulled in toward the edge, strongest at the very\n/// edge) with a soft highlight, fading to nothing toward the inside of the notch.","/// pixel shows what lies further out (pulled in toward the edge, strongest at the very edge),\n/// fading to nothing toward the inside of the notch.")
open(p,'w').write(s)

# ---------------- app.rs: a fresh copy on every open, and no rim once the window behind changed
p='src/app.rs'; s=open(p).read()
s=sub(s,"    refr_idle: Option<Instant>,\n    refr_retry: Option<Instant>,\n","    refr_retry: Option<Instant>,\n    /// The window in front when the copy was taken; if another comes forward the copy is out of date.\n    refr_fg: isize,\n    refr_dead: bool,\n")
s=sub(s,"            refr_idle: None,\n            refr_retry: None,\n","            refr_retry: None,\n            refr_fg: 0,\n            refr_dead: false,\n")
a0=s.index("    fn prepare_rim(&mut self, now: Instant) {")
a1=s.index("        if !self.settled {\n            return;\n        }\n        let Some(shot) = self.refr_shot.as_ref() else { return };",a0)
new='''    fn prepare_rim(&mut self, now: Instant) {
        if !(self.cfg.appearance.glass_look && crate::ui::look_alcove()) || self.mode == Mode::Idle {
            // nothing is kept while the notch is closed: the next opening takes a fresh copy
            self.refr_shot = None;
            self.refr_rim = None;
            self.refr_dead = false;
            return;
        }
        let fg = unsafe { GetForegroundWindow() };
        if self.refr_shot.is_some() && fg != self.hwnd && fg.0 as isize != self.refr_fg {
            // another window came to the front: the copy no longer matches what is behind the
            // notch, so the rim goes rather than show an old picture (until the next opening)
            self.refr_shot = None;
            self.refr_rim = None;
            self.refr_dead = true;
        }
        if self.refr_shot.is_none()
            && !self.refr_dead
            && self.refr_retry.is_none_or(|t| now.duration_since(t) > Duration::from_secs(2))
        {
            let (x, y, w, h) = self.win_px;
            self.refr_shot = crate::refract::grab(x, y, w, h);
            self.refr_retry = Some(now);
            self.refr_fg = fg.0 as isize;
            if self.refr_shot.is_none() {
                log!("glass edge: the screen could not be copied");
            }
        }
'''
s=s[:a0]+new+s[a1:]
open(p,'w').write(s)
PYEOF
echo "Patched v190"
