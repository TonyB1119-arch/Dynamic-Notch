#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "is_playing" src/modules/mod.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---- Module: "is something playing?" (only the media module says yes)
p='src/modules/mod.rs'; s=open(p).read()
s=sub(s,"""    fn on_tick(&mut self, cx: &mut Cx) {}

    fn activity(&self) -> Option<Activity> {
        None
    }""","""    fn on_tick(&mut self, cx: &mut Cx) {}

    /// Media is playing right now (used to decide whether the glass edge needs to stay live).
    fn is_playing(&self) -> bool {
        false
    }

    fn activity(&self) -> Option<Activity> {
        None
    }""")
open(p,'w').write(s)
p='src/modules/media.rs'; s=open(p).read()
s=sub(s,"""    fn activity(&self) -> Option<Activity> {""","""    fn is_playing(&self) -> bool {
        self.playing()
    }

    fn activity(&self) -> Option<Activity> {""")
open(p,'w').write(s)

# ---- settings text + config comment
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,'"Live glass (follows what is behind)"','"Live glass while media plays"')
open(p,'w').write(s)
p='src/config.rs'; s=open(p).read()
s=sub(s,"""    /// Alcove: the glass edge keeps up with what is behind the notch (the screen is looked at again
    /// a few times a second while the notch is open). Turn off to save a little CPU.""","""    /// Alcove: while media is playing and the notch is open, the glass edge keeps up with what is
    /// behind the notch (the screen is looked at again a few times a second). Turn off to save CPU.""")
s=sub(s,"""# Alcove: the glass edge keeps up with what is behind the notch (a little extra CPU while open;
# it also hides the open notch from screenshots and screen recordings)""","""# Alcove: while media plays and the notch is open, the glass edge keeps up with what is behind it
# (a little extra CPU; during that time the notch is hidden from screenshots and screen recordings)""")
open(p,'w').write(s)

# ---- app.rs
p='src/app.rs'; s=open(p).read()
s=sub(s,"const LIVE_SLOW: Duration = Duration::from_millis(800);\n","""const LIVE_SLOW: Duration = Duration::from_millis(800);
/// Live glass carries on this long after the media stops (pausing a video for a moment must not
/// switch it off and on again).
const LIVE_LINGER: Duration = Duration::from_secs(6);
""")
s=sub(s,"    live_dead: bool,\n","    live_dead: bool,\n    /// When media was last seen playing.\n    play_seen: Option<Instant>,\n")
s=sub(s,"            live_dead: false,\n","            live_dead: false,\n            play_seen: None,\n")
s=sub(s,"""    /// Hide the notch from screen copies (or show it again).""","""    /// Media is playing, or stopped only a moment ago: the screen behind the notch may be moving.
    fn live_media(&mut self, now: Instant) -> bool {
        let playing = self.module_index("media").is_some_and(|i| self.started[i] && self.modules[i].is_playing());
        if playing {
            self.play_seen = Some(now);
        }
        self.play_seen.is_some_and(|t| now.saturating_duration_since(t) < LIVE_LINGER)
    }

    /// Hide the notch from screen copies (or show it again).""")
s=sub(s,"""        let want = self.cfg.appearance.glass_live && !self.live_dead;
        self.set_excl(want);""","""        let want = self.cfg.appearance.glass_live && !self.live_dead && self.live_media(now);
        self.set_excl(want);""")
s=sub(s,"""        self.live_same = 0;
        self.rim_old = None;
        self.rim_x = 1.0;
    }""","""        self.live_same = 0;
        self.rim_old = None;
        self.rim_x = 1.0;
        self.play_seen = None;
    }""")
s=sub(s,"""        self.live_done();
        if !self.live_armed() || self.live_rx.is_some() {
            return;
        }""","""        self.live_done();
        if !self.live_armed() || self.live_rx.is_some() {
            return;
        }
        if !self.live_media(now) {
            // the media has been quiet for a while: the picture behind is as good as it will get
            self.live_off();
            return;
        }""")
open(p,'w').write(s)
PYEOF
echo "Patched v205"
