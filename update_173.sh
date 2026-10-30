#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
import re
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert s.count(a)==n, (s.count(a), a[:80])
    return s.replace(a,b)

# ---- config
p='src/config.rs'; s=rd(p)
s=sub(s,"    pub bottom_fade: bool,\n","    pub bottom_fade: bool,\n    /// Alcove: a soft sheen, a lit edge and a little inner glow on the open notch (liquid-glass look).\n    pub glass_look: bool,\n")
s=sub(s,"            bottom_fade: true,\n","            bottom_fade: true,\n            glass_look: true,\n")
s=sub(s,"bottom_fade = true\n","bottom_fade = true\n# Alcove: a soft sheen, a lit edge and a little inner glow (false = plain glass)\nglass_look = true\n")
open(p,'w').write(s)

# ---- settings
p='src/modules/settings.rs'; s=rd(p)
s=sub(s,"""            "appearance",
            "bottom_fade",
        );
""","""            "appearance",
            "bottom_fade",
        );
        toggle_row(
            ui,
            row_rect(body, 13),
            "Glass shine and lit edge",
            a.glass_look,
            "appearance",
            "glass_look",
        );
""")
s=sub(s,"        1 => 13.0,","        1 => 14.0,")
open(p,'w').write(s)

# ---- app
p='src/app.rs'; s=rd(p)
s=sub(s,"""                        p.fill_geometry(&g, Color::black(bg_alpha));
                    }
                }

                p.push_clip(body);""","""                        p.fill_geometry(&g, Color::black(bg_alpha));
                    }
                    if glass && body.h >= 56.0 {
                        // a soft sheen at the top and a little glow from below, like light in thick glass
                        let k = a_glass;
                        p.fill_geometry_vgrad(
                            &g,
                            body.y,
                            body.bottom(),
                            &[
                                (0.0, Color::white(0.10 * k)),
                                (0.3, Color::white(0.0)),
                                (0.72, Color::white(0.0)),
                                (1.0, Color::white(0.09 * k)),
                            ],
                        );
                    }
                }

                p.push_clip(body);""")
s=sub(s,"""                p.pop_clip();
            }
        }
        if let Err(e) = self.host.end_draw() {""","""                if glass && body.h >= 56.0 && a_glass > 0.01 {
                    // the lit edge: a fine bright line down the sides and around the bottom corners
                    let rim = Rect::new(body.x + 0.5, body.y - a_r - 2.0, body.w - 1.0, body.h + a_r + 1.5);
                    p.push_clip(Rect::new(body.x - 1.0, body.y + a_ear * 0.8, body.w + 2.0, body.h));
                    p.stroke_rounded(rim, a_r, 1.0, Color::white(0.17 * a_glass));
                    p.stroke_rounded(rim.inset(1.0), (a_r - 1.0).max(1.0), 1.0, Color::white(0.05 * a_glass));
                    p.pop_clip();
                }
                p.pop_clip();
            }
        }
        if let Err(e) = self.host.end_draw() {""")
# the switch, read once per frame next to the other appearance reads
s=sub(s,"        let see_through = ap.expanded_opacity < 0.97 || ap.collapsed_opacity < 0.97;\n","        let see_through = ap.expanded_opacity < 0.97 || ap.collapsed_opacity < 0.97;\n        let glass = ap.glass_look && crate::ui::look_alcove();\n")
open(p,'w').write(s)
PYEOF
echo "Patched v173 - now run ./release.sh"
