#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fn creeping" src/app.rs; then echo "already applied"; exit 0; fi
if ! grep -q "fn halo_y" src/app.rs; then echo "run update_222.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

p='src/app.rs'; s=open(p).read()

# ---- the slow last stretch of a move ----
s=sub(s,"""            m |= s.step(dt);
        }
        m
    }
}
""","""            m |= s.step(dt);
        }
        m
    }

    /// True while every spring is creeping its last bit into place: within a hair of where it is
    /// going and moving by a fraction of a pixel a frame.
    fn creeping(&self) -> bool {
        let px = |s: &Spring| (s.value - s.target).abs() < 3.0 && s.velocity.abs() < 30.0;
        let un = |s: &Spring| (s.value - s.target).abs() < 0.03 && s.velocity.abs() < 0.3;
        px(&self.w)
            && px(&self.h)
            && px(&self.r)
            && px(&self.ear)
            && un(&self.glass)
            && un(&self.compact)
            && un(&self.peek)
            && un(&self.exp)
            && un(&self.shown)
            && un(&self.sec)
            && un(&self.split)
            && un(&self.page)
    }
}
""")
s=sub(s,"""    debug: bool,
}
""","""    debug: bool,
    /// The slow last stretch of a move (see `Anim::creeping`): every other frame is left out.
    tail: bool,
    /// DN_DEBUG timings of the current burst, in microseconds: [draw total, draw worst, frames drawn,
    /// pictures, bottom blur, glass check, glass rebuilds total, rebuild count, tail frames skipped].
    tm: [u64; 9],
}
""")
s=sub(s,"""            debug: std::env::var_os("DN_DEBUG").is_some(),
            cfg,""","""            debug: std::env::var_os("DN_DEBUG").is_some(),
            tail: false,
            tm: [0; 9],
            cfg,""")
s=sub(s,"""            return;
        }
        self.slow_only = false;
        let dt =""","""            return;
        }
        // the slow last stretch of a move: every spring is creeping the final bit into place, a
        // fraction of a pixel a frame, so every other frame is left out (about 30 a second; the
        // motion is too small to show it). Input or anything else that wants a frame ends it.
        if self.tail
            && !self.dirty
            && now.duration_since(self.last_frame) < Duration::from_micros(self.frame_gap_us() as u64 * 15 / 8)
        {
            self.tm[8] += 1;
            return;
        }
        self.slow_only = false;
        let dt =""")
s=sub(s,"""        let moving = self.anim.step(dt);
""","""        let moving = self.anim.step(dt);
        self.tail = moving && self.anim.creeping();
""")
# ---- DN_DEBUG timings ----
s=sub(s,"""        let fx = self.render(now);
        let animate = fx.animate;""","""        let t_draw = self.debug.then(Instant::now);
        let fx = self.render(now);
        if let Some(t) = t_draw {
            let us = t.elapsed().as_micros() as u64;
            self.tm[0] += us;
            self.tm[1] = self.tm[1].max(us);
            self.tm[2] += 1;
        }
        let animate = fx.animate;""")
s=sub(s,"""                self.stats = (None, 0, 0, 0);
""","""                self.stats = (None, 0, 0, 0);
                let tm = self.tm;
                let ms = |us: u64| us as f64 / 1000.0;
                log!(
                    "  timing: draw {:.2} ms avg, {:.2} ms worst | pictures {:.1} ms, bottom blur {:.1} ms, glass check {:.1} ms (totals) | {} glass rebuilds, {:.1} ms (total) | {} frames skipped in the slow tail",
                    ms(tm[0]) / tm[2].max(1) as f64,
                    ms(tm[1]),
                    ms(tm[3]),
                    ms(tm[4]),
                    ms(tm[5]),
                    tm[7],
                    ms(tm[6]),
                    tm[8]
                );
                self.tm = [0; 9];
""")
s=sub(s,"""        self.prepare_pictures(now);
        // (the blurred strip is drawn without the outline: it is a soft copy of the page anyway)
        self.res.set_halo(None);
        self.prepare_band(now);
        self.prepare_rim(now);""","""        let t0 = self.debug.then(Instant::now);
        self.prepare_pictures(now);
        let t1 = self.debug.then(Instant::now);
        // (the blurred strip is drawn without the outline: it is a soft copy of the page anyway)
        self.res.set_halo(None);
        self.prepare_band(now);
        let t2 = self.debug.then(Instant::now);
        self.prepare_rim(now);
        if let (Some(t0), Some(t1), Some(t2)) = (t0, t1, t2) {
            self.tm[3] += (t1 - t0).as_micros() as u64;
            self.tm[4] += (t2 - t1).as_micros() as u64;
            self.tm[5] += t2.elapsed().as_micros() as u64;
        }""")
s=sub(s,"""        let prev = self.refr_rim.take();
        if let Some(rim) = crate::refract::rim(shot, shape, step) {""","""        let prev = self.refr_rim.take();
        let t_rim = self.debug.then(Instant::now);
        if let Some(rim) = crate::refract::rim(shot, shape, step) {""")
s=sub(s,"""        self.rim_ok = self.refr_rim.is_some();
        if step > fine && self.rim_ok {""","""        if let Some(t) = t_rim {
            self.tm[6] += t.elapsed().as_micros() as u64;
            self.tm[7] += 1;
        }
        self.rim_ok = self.refr_rim.is_some();
        if step > fine && self.rim_ok {""")
open(p,'w').write(s)
PYEOF
echo "Patched v223"
