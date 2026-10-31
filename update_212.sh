#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "pub struct GlassCfg" src/config.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---------------- config.rs ----------------
p='src/config.rs'; s=open(p).read()
s=sub(s,"    pub tabs: TabsCfg,\n}\n","    pub tabs: TabsCfg,\n    pub glass: GlassCfg,\n}\n")
s=sub(s,"""pub fn config_path() -> PathBuf {""","""/// The look of the Alcove glass edge (Settings → Glass). Sizes are in logical pixels.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassCfg {
    /// How far in from the edge the glass reaches.
    pub edge_width: f32,
    /// How far out the glass looks at its strongest (the warp).
    pub pull: f32,
    /// How sharply the bend is concentrated at the very edge (1 = even, 3 = only right at the edge).
    pub bend: f32,
    /// How solid the glass is at the edge (0.3 - 1).
    pub strength: f32,
    /// Extra warp in the rounded corners (0 - 1).
    pub corner: f32,
    /// Colour fringing: red, green and blue look slightly different distances out.
    pub fringe: f32,
    /// How far up the sides the glass is drawn before it fades out.
    pub reach: f32,
    /// How softly the glass starts at the outer edge.
    pub softness: f32,
    /// How see-through the bottom edge of the notch gets (0 = not at all).
    pub fade_amount: f32,
    /// How high up the bottom fade begins (1 = the usual height).
    pub fade_start: f32,
    /// How quickly the glass fades in (higher = quicker).
    pub glow_in: f32,
    /// Live glass: milliseconds between looks at the screen while media plays.
    pub live_ms: f32,
    /// Live glass: seconds for the edge to melt from one picture into the next.
    pub melt: f32,
    /// While the notch moves, the glass gets its finest detail once this fraction of the move is left.
    pub sharpen_at: f32,
}

impl Default for GlassCfg {
    fn default() -> Self {
        Self {
            edge_width: 76.0,
            pull: 22.0,
            bend: 1.5,
            strength: 0.88,
            corner: 0.3,
            fringe: 0.05,
            reach: 90.0,
            softness: 4.0,
            fade_amount: 0.8,
            fade_start: 1.7,
            glow_in: 9.0,
            live_ms: 150.0,
            melt: 0.12,
            sharpen_at: 0.12,
        }
    }
}

impl GlassCfg {
    fn clamped(&mut self) {
        self.edge_width = self.edge_width.clamp(20.0, 160.0);
        self.pull = self.pull.clamp(0.0, 60.0);
        self.bend = self.bend.clamp(0.8, 3.0);
        self.strength = self.strength.clamp(0.3, 1.0);
        self.corner = self.corner.clamp(0.0, 1.0);
        self.fringe = self.fringe.clamp(0.0, 0.2);
        self.reach = self.reach.clamp(30.0, 200.0);
        self.softness = self.softness.clamp(0.5, 12.0);
        self.fade_amount = self.fade_amount.clamp(0.0, 1.0);
        self.fade_start = self.fade_start.clamp(0.5, 3.0);
        self.glow_in = self.glow_in.clamp(2.0, 30.0);
        self.live_ms = self.live_ms.clamp(80.0, 500.0);
        self.melt = self.melt.clamp(0.03, 0.5);
        self.sharpen_at = self.sharpen_at.clamp(0.02, 0.5);
    }
}

pub fn config_path() -> PathBuf {""")
s=sub(s,"        self.home.widgets.truncate(4);\n","        self.glass.clamped();\n        self.home.widgets.truncate(4);\n")
s=sub(s,"""[tabs]
# Order of the header tabs""","""[glass]
# The Alcove glass edge (Settings → Glass). Sizes are in logical pixels; delete a line to get its default.
edge_width = 76.0
pull = 22.0
bend = 1.5
strength = 0.88
corner = 0.3
fringe = 0.05
reach = 90.0
softness = 4.0
fade_amount = 0.8
fade_start = 1.7
glow_in = 9.0
live_ms = 150.0
melt = 0.12
sharpen_at = 0.12

[tabs]
# Order of the header tabs""")
open(p,'w').write(s)

# ---------------- refract.rs ----------------
p='src/refract.rs'; s=open(p).read()
s=sub(s,"""/// Where the notch is, in window pixels""","""/// The look of the glass (sizes in logical pixels, scaled by `Shape::scale`). The defaults are the
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
        }
    }
}

/// Where the notch is, in window pixels""")
s=sub(s,"    pub radius: f32,\n    pub scale: f32,\n}","    pub radius: f32,\n    pub scale: f32,\n    pub tune: Tune,\n}")
# ring_sum
s=sub(s,"""    let pad = 32.0 * sc; // the glass pulls from up to this far outside the edge
    let band = (76.0 * sc + pad).ceil() as i32; // how far in from the edge the rim reaches""","""    let t = sh.tune;
    let pad = (t.pull * (1.0 + t.corner) + 4.0) * sc; // the glass pulls from up to this far outside the edge
    let band = (t.width * sc + pad).ceil() as i32; // how far in from the edge the rim reaches""")
s=sub(s,"let y0 = ((sh.bottom - r - 90.0 * sc - pad).floor() as i32).clamp(0, s.h as i32);","let y0 = ((sh.bottom - r - t.reach * sc - pad).floor() as i32).clamp(0, s.h as i32);")
# rim
s=sub(s,"""    let width = 76.0 * sc; // how far in from the edge the rim reaches
    let pull = 22.0 * sc; // how far out the strongest pixels look""","""    let t = sh.tune;
    let width = t.width * sc; // how far in from the edge the rim reaches
    let pull = t.pull * sc; // how far out the strongest pixels look
    let soft = t.soft * sc;
    let even_bend = (t.bend - 1.5).abs() < 0.001;""")
s=sub(s,"let reach = r + 90.0 * sc; // how far up","let reach = r + t.reach * sc; // how far up")
s=sub(s,"            let base = 0.88 + 0.1 * corner;\n            let alpha = (base + (1.0 - base) * smooth(4.0 * sc, 0.0, d))","            let base = (t.strength + 0.1 * corner).min(1.0);\n            let alpha = (base + (1.0 - base) * smooth(soft, 0.0, d))")
s=sub(s,"""            let k = u * u.sqrt();
            let pull = pull * (1.0 + 0.3 * corner);""","""            let k = if even_bend { u * u.sqrt() } else { u.powf(t.bend) };
            let pull = pull * (1.0 + t.corner * corner);""")
s=sub(s,"""            let cg = sample(shot, x + ox * 0.95, y + oy * 0.95, 8);
            let cb2 = sample(shot, x + ox * 0.9, y + oy * 0.9, 0);""","""            let cg = sample(shot, x + ox * (1.0 - t.fringe), y + oy * (1.0 - t.fringe), 8);
            let cb2 = sample(shot, x + ox * (1.0 - 2.0 * t.fringe), y + oy * (1.0 - 2.0 * t.fringe), 0);""")
open(p,'w').write(s)

# ---------------- app.rs ----------------
p='src/app.rs'; s=open(p).read()
s=sub(s,"""/// Live glass: how often the screen behind the notch is looked at again, and how often once
/// it has not changed for a while.
const LIVE_FAST: Duration = Duration::from_millis(150);
const LIVE_SLOW: Duration = Duration::from_millis(600);
""","")
s=sub(s,"""/// Seconds for the glass edge to melt from the old picture into a new one.
const LIVE_FADE: f32 = 0.12;
""","")
s=sub(s,"    rim_last: Option<((i32, i32, i32, i32, i32), crate::refract::Shape)>,\n","    rim_last: Option<((i32, i32, i32, i32, i32), crate::refract::Shape)>,\n    /// The glass settings the current rim was made with (a change makes it again).\n    rim_tune: Option<crate::refract::Tune>,\n")
s=sub(s,"            rim_last: None,\n","            rim_last: None,\n            rim_tune: None,\n")
s=sub(s,"""        let fg = unsafe { GetForegroundWindow() };
        // (with live glass the copy is renewed anyway""","""        // a change in Settings → Glass: the rim is made again with the new look
        let tune = crate::refract::Tune::from(&self.cfg.glass);
        if self.rim_tune != Some(tune) {
            self.rim_tune = Some(tune);
            self.refr_rim = None;
            self.rim_old = None;
            self.live_sum = 0;
        }
        let fg = unsafe { GetForegroundWindow() };
        // (with live glass the copy is renewed anyway""")
s=sub(s,"let late = self.rim_far >= 24.0 && left <= self.rim_far * 0.12;","let late = self.rim_far >= 24.0 && left <= self.rim_far * self.cfg.glass.sharpen_at.clamp(0.02, 0.5);")
s=sub(s,"            radius: r * sc,\n            scale: sc,\n        };","            radius: r * sc,\n            scale: sc,\n            tune,\n        };")
s=sub(s,"            self.live_next = Some(now + LIVE_FAST);","            self.live_next = Some(now + self.live_fast());")
s=sub(s,"        let gap = if self.live_same >= 3 { LIVE_SLOW } else { LIVE_FAST };","        let gap = if self.live_same >= 3 { self.live_fast() * 4 } else { self.live_fast() };")
s=sub(s,"            self.live_next = Some(now + LIVE_SLOW);","            self.live_next = Some(now + self.live_fast() * 4);")
s=sub(s,"""    /// The notch is open: switch live glass on or off""","""    /// Time between looks at the screen while live glass is busy (Settings → Glass).
    fn live_fast(&self) -> Duration {
        Duration::from_millis(self.cfg.glass.live_ms.clamp(80.0, 500.0) as u64)
    }

    /// The notch is open: switch live glass on or off""")
s=sub(s,"let rim_rate = if rim_to > self.rim_k { 9.0 } else { 18.0 };","let glow = self.cfg.glass.glow_in.clamp(2.0, 30.0);\n        let rim_rate = if rim_to > self.rim_k { glow } else { glow * 2.0 };")
s=sub(s,"self.rim_x += rim_dt / LIVE_FADE;","self.rim_x += rim_dt / self.cfg.glass.melt.clamp(0.03, 0.5);")
s=sub(s,"let from = 1.0 - bh * 1.7 / body.h;\n                        let drop = 0.8 * band_k * bd_k;","let from = 1.0 - bh * self.cfg.glass.fade_start.clamp(0.5, 3.0) / body.h;\n                        let drop = self.cfg.glass.fade_amount.clamp(0.0, 1.0) * band_k * bd_k;")
open(p,'w').write(s)

# ---------------- settings.rs ----------------
p='src/modules/settings.rs'; s=open(p).read()
s=sub(s,'const TABS: [&str; 9] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock"];','const TABS: [&str; 10] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock", "Glass"];')
s=sub(s,"        7 => 5.9,\n        _ => 8.6,","        7 => 5.9,\n        8 => 8.6,\n        _ => 15.0,")
s=sub(s,"            7 => self.tabs(ui, body),\n            _ => self.dock(ui, body),","            7 => self.tabs(ui, body),\n            8 => self.dock(ui, body),\n            _ => self.glass(ui, body),")
s=sub(s,"    fn dock(&mut self, ui: &mut Ui, body: Rect) {","""    /// Settings → Glass: one slider for each dial of the Alcove glass edge.
    fn glass(&mut self, ui: &mut Ui, body: Rect) {
        let g = self.cfg.glass.clone();
        // (label, value, range, shown, key)
        let rows: [(&str, f32, (f32, f32), String, &str); 14] = [
            ("Edge width", g.edge_width, (20.0, 160.0), format!("{:.0}", g.edge_width), "edge_width"),
            ("Warp strength", g.pull, (0.0, 60.0), format!("{:.0}", g.pull), "pull"),
            ("Bend curve", g.bend, (0.8, 3.0), format!("{:.1}", g.bend), "bend"),
            ("Edge opacity", g.strength, (0.3, 1.0), format!("{:.0}%", g.strength * 100.0), "strength"),
            ("Corner warp", g.corner, (0.0, 1.0), format!("{:.0}%", g.corner * 100.0), "corner"),
            ("Colour fringing", g.fringe, (0.0, 0.2), format!("{:.0}%", g.fringe * 500.0), "fringe"),
            ("Reach up the sides", g.reach, (30.0, 200.0), format!("{:.0}", g.reach), "reach"),
            ("Outer edge softness", g.softness, (0.5, 12.0), format!("{:.1}", g.softness), "softness"),
            ("Bottom fade amount", g.fade_amount, (0.0, 1.0), format!("{:.0}%", g.fade_amount * 100.0), "fade_amount"),
            ("Bottom fade start", g.fade_start, (0.5, 3.0), format!("{:.1}", g.fade_start), "fade_start"),
            ("Glow fade-in speed", g.glow_in, (2.0, 30.0), format!("{:.0}", g.glow_in), "glow_in"),
            ("Live refresh (ms)", g.live_ms, (80.0, 500.0), format!("{:.0}", g.live_ms), "live_ms"),
            ("Live melt time (s)", g.melt, (0.03, 0.5), format!("{:.2}", g.melt), "melt"),
            ("Sharpen when left", g.sharpen_at, (0.02, 0.5), format!("{:.0}%", g.sharpen_at * 100.0), "sharpen_at"),
        ];
        for (i, (text, val, range, shown, key)) in rows.into_iter().enumerate() {
            slider_row(ui, row_rect(body, i), text, val, range, shown, "glass", key);
        }
        let d = crate::config::GlassCfg::default();
        let b = row_rect(body, 14);
        let style = ButtonStyle::filled(Color::white(0.07)).fg(palette::TEXT_DIM);
        if ui.pill_button(Rect::new(b.x, b.y, 190.0, b.h), "Reset to defaults", None, style) {
            for (key, v) in [
                ("edge_width", d.edge_width),
                ("pull", d.pull),
                ("bend", d.bend),
                ("strength", d.strength),
                ("corner", d.corner),
                ("fringe", d.fringe),
                ("reach", d.reach),
                ("softness", d.softness),
                ("fade_amount", d.fade_amount),
                ("fade_start", d.fade_start),
                ("glow_in", d.glow_in),
                ("live_ms", d.live_ms),
                ("melt", d.melt),
                ("sharpen_at", d.sharpen_at),
            ] {
                put(ui, "glass", key, format!("{v:.2}"));
            }
        }
    }

    fn dock(&mut self, ui: &mut Ui, body: Rect) {""")
open(p,'w').write(s)
PYEOF
echo "Patched v212"
