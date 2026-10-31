#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fn ring_sum" src/refract.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- refract.rs: fingerprint only the strip the rim reads; half-detail live rim
p='src/refract.rs'; s=open(p).read()
a=s.index("/// A quick fingerprint of a screen copy")
b=s.index("/// What a refresh brings back.")
s=s[:a]+"""/// A quick fingerprint of the part of the screen copy the rim actually reads: a thin ring around
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

"""+s[b:]
s=sub(s,"""/// Copy the screen again and, when it differs from the copy with fingerprint `prev`, work out the
/// rim for `shape` as well. Meant for a background thread; None when the screen could not be copied.""","""/// Copy the screen again and, when its ring differs from the one with fingerprint `prev`, work out
/// the rim for `shape` as well, at half detail (it is a soft gradient and this runs often). Meant
/// for a background thread; None when the screen could not be copied.""")
s=sub(s,"""    let sum = checksum(&shot);
    if sum == prev {
        return Some(Fetched::Same);
    }
    let made = rim(&shot, shape, 1);""","""    let sum = ring_sum(&shot, shape);
    if sum == prev {
        return Some(Fetched::Same);
    }
    let made = rim(&shot, shape, 2);""")
s=sub(s,"""//! out identical to the last one is thrown away at once and costs nothing more.""","""//! out identical to the last one (as far as the rim can see) is thrown away at once.""")
open(p,'w').write(s)

# ---------------- app.rs
p='src/app.rs'; s=open(p).read()
s=sub(s,"const LIVE_FAST: Duration = Duration::from_millis(350);\nconst LIVE_SLOW: Duration = Duration::from_millis(800);","const LIVE_FAST: Duration = Duration::from_millis(150);\nconst LIVE_SLOW: Duration = Duration::from_millis(600);")
s=sub(s,"const LIVE_FADE: f32 = 0.4;","const LIVE_FADE: f32 = 0.12;")
s=sub(s,"""            if let Some(shot) = &self.refr_shot {
                self.live_sum = crate::refract::checksum(shot);
            }
""","""            // (no fingerprint yet: the first refresh always counts as new)
            self.live_sum = 0;
""")
# step logic
s=sub(s,"""        let step = if self.rim_geo == Some(geo) { 1 } else { 2 };""","""        // (live glass keeps making half-detail rims, so that is what "fine" means while it is on)
        let fine = if self.live_armed() { 2 } else { 1 };
        let step = if self.rim_geo == Some(geo) { fine } else { 2 };""")
s=sub(s,"""        if step > 1 && self.rim_ok {
            // one more frame so the fine picture follows as soon as the notch has stopped""","""        if step > fine && self.rim_ok {
            // one more frame so the fine picture follows as soon as the notch has stopped""")
s=sub(s,"let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, 1);","let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, rim.step);")
# gentle thread
s=sub(s,"""            let out = crate::refract::refresh(x, y, w, h, prev, shape);""","""            // behind everything else: the notch's own drawing always comes first
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
            }
            let out = crate::refract::refresh(x, y, w, h, prev, shape);""")
open(p,'w').write(s)
PYEOF
echo "Patched v206"
