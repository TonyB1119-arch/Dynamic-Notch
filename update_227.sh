#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "lite_motion" src/config.rs; then echo "already applied"; exit 0; fi
if ! grep -q "fn rim_near" src/app.rs; then echo "run update_225_226.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/config.rs'; s=open(p).read()
s=sub(s,"""    pub glass_live: bool,
""","""    pub glass_live: bool,
    /// Lighter animations: the glass edge is rebuilt less often (and coarser) while the notch moves,
    /// and the slow last stretch of a move uses fewer frames. Looks the same once the notch rests.
    /// Turn off for the full-rate animation.
    pub lite_motion: bool,
""")
s=sub(s,"""            glass_live: true,
""","""            glass_live: true,
            lite_motion: true,
""")
s=sub(s,"""glass_live = true
""","""glass_live = true
# Lighter animations: less CPU while the notch moves (the glass edge is refreshed less often, the slow
# end of a move uses fewer frames). false = the full-rate animation
lite_motion = true
""")
open(p,'w').write(s)

p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,"""            "max_fps",
        );
    }
""","""            "max_fps",
        );
        toggle_row(
            ui,
            row_rect(body, 16),
            "Lighter animations (less CPU)",
            a.lite_motion,
            "appearance",
            "lite_motion",
        );
    }
""")
s=sub(s,"        1 => 16.0,\n","        1 => 17.0,\n")
open(p,'w').write(s)

p='src/app.rs'; s=open(p).read()
s=sub(s,"    rim_geo: Option<(i32, i32, i32, i32, i32)>,\n","    rim_geo: Option<(i32, i32, i32, i32, i32)>,\n    /// When the glass edge was last rebuilt (lighter animations spread the rebuilds out).\n    rim_t: Option<Instant>,\n")
s=sub(s,"            rim_geo: None,\n","            rim_geo: None,\n            rim_t: None,\n")
s=sub(s,"""            && now.duration_since(self.last_frame) < Duration::from_micros(self.frame_gap_us() as u64 * 15 / 8)""","""            && now.duration_since(self.last_frame)
                < Duration::from_micros(self.frame_gap_us() as u64 * if self.cfg.appearance.lite_motion { 22 } else { 15 } / 8)""")
s=sub(s,"""        let step = fine;
        let _ = late;
        self.rim_geo = Some(geo);""","""        let step = fine;
        let _ = late;
        let lite = self.cfg.appearance.lite_motion;
        // lighter animations: while the size changes the glass edge is rebuilt at most every 45 ms and
        // the picture already there keeps showing in between (it is a few pixels behind at worst)
        if lite
            && !self.settled
            && !still
            && self.refr_rim.is_some()
            && self.rim_t.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(45))
        {
            self.rim_ok = true;
            return;
        }
        self.rim_geo = Some(geo);""")
s=sub(s,"""        const MOVING_ZONE: f32 = 0.55;""","""        let moving_zone: f32 = if lite { 0.3 } else { 0.55 };""")
s=sub(s,"tune.sharp > MOVING_ZONE && !still","tune.sharp > moving_zone && !still")
s=sub(s,"made.tune.sharp = MOVING_ZONE;","made.tune.sharp = moving_zone;")
s=sub(s,"""        if let Some(t) = t_rim {
            self.tm[6]""","""        self.rim_t = Some(now);
        if let Some(t) = t_rim {
            self.tm[6]""")
open(p,'w').write(s)
PYEOF
echo "Patched v227"
