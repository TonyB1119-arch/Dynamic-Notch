#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fn rim_near" src/app.rs; then echo "already applied"; exit 0; fi
if ! grep -q "fn creeping" src/app.rs; then echo "run update_223.sh first"; exit 1; fi
# part 1 (v225): the timing log also shows the glass settings
if ! grep -q "full-detail zone {:.1}" src/app.rs; then
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""| {} frames skipped in the slow tail",
                    ms(tm[0]) / tm[2].max(1) as f64,""","""| {} frames skipped in the slow tail | glass: middle warp {:.2}, middle blur {:.2}, full-detail zone {:.1}",
                    ms(tm[0]) / tm[2].max(1) as f64,""")
s=sub(s,"""                    ms(tm[6]),
                    tm[8]
                );""","""                    ms(tm[6]),
                    tm[8],
                    self.cfg.glass.middle_warp,
                    self.cfg.glass.middle_blur,
                    self.cfg.glass.sharp_zone
                );""")
open(p,'w').write(s)
PYEOF
fi
# part 2 (v226): lighter glass while the notch moves, full liquid glass once it settles
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()

s=sub(s,"type RimKey = (i32, i32, i32, i32, i32, usize);\n","""type RimKey = (i32, i32, i32, i32, i32, usize);

/// A glass picture made while the notch moves carries 10 in its last field (reduced detail).
const RIM_LOW: usize = 10;

/// The two pictures belong to the same notch size (give or take a pixel).
fn rim_near(a: &RimKey, b: &RimKey) -> bool {
    // (whole pixels while moving, quarter pixels once settled)
    let (ua, ub) = (if a.5 >= RIM_LOW { 1 } else { 4 }, if b.5 >= RIM_LOW { 1 } else { 4 });
    let tol = if ua == ub { 0 } else { 4 };
    a.4 == b.4 && [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)].iter().all(|&(x, y)| (x * ub - y * ua).abs() <= tol)
}
""")

s=sub(s,"Some((old_key, old)) if old_key.0 == key.0 && old_key.1 == key.1 && old_key.2 == key.2 && old_key.3 == key.3 && old_key.4 == key.4 => {","Some((old_key, old)) if rim_near(old_key, key) => {")

s=sub(s,"""        let step = fine;
        let _ = late;
        self.rim_geo = Some(geo);
        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, step);""","""        let step = fine;
        let _ = late;
        self.rim_geo = Some(geo);
        // While the size is still changing the sharp zone is capped: the glass is rebuilt for nearly
        // every frame then, and the full-detail band along the edge is what costs most. The full
        // picture follows as soon as the notch stops and melts in over this one.
        const MOVING_ZONE: f32 = 0.55;
        let low = !self.settled && tune.sharp > MOVING_ZONE && !still;
        let key: RimKey = (geo.0, geo.1, geo.2, geo.3, geo.4, step + if low { RIM_LOW } else { 0 });""")
s=sub(s,"""        if let Some(rim) = crate::refract::rim(shot, shape, step) {""","""        let mut made = shape;
        if low {
            made.tune.sharp = MOVING_ZONE;
        }
        if let Some(rim) = crate::refract::rim(shot, made, step) {""")
s=sub(s,"if old.0 .5 > key.5 && old.0 .0 == key.0 && old.0 .1 == key.1 && old.0 .2 == key.2 && old.0 .3 == key.3 && old.0 .4 == key.4 {","if old.0 .5 > key.5 && rim_near(&old.0, &key) {")
s=sub(s,"""        if step > fine && self.rim_ok {""","""        if low && self.rim_ok {""")
open(p,'w').write(s)
PYEOF
echo "Patched v225 + v226"
