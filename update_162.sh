cd /workspaces/Notch
cat > /tmp/v162.py <<'PYEOF'
def rd(p): return open(p).read()
def sub(s,a,b):
    assert s.count(a)==1, (s.count(a), a[:80])
    return s.replace(a,b)
p='src/app.rs'; s=rd(p)
s=sub(s,"""        if !on {
            self.snap_band = None;
            self.band_since = None;
            return;
        }""","""        if !on {
            if self.band_since.is_some() {
                log!(
                    "bottom blur off: mode {:?} paging {} page {} exp {:.3} dh {:.1} exp_w {:.1}",
                    self.mode,
                    self.page_from.is_some(),
                    self.page,
                    self.anim.exp.value,
                    body.h - t.h,
                    body.w - tw
                );
            }
            self.snap_band = None;
            self.band_since = None;
            return;
        }""")
s=sub(s,"""            draw_body(&mut ui, content, &mut self.modules, &self.started, page);
        });
    }

    /// 0..1: how far""","""            draw_body(&mut ui, content, &mut self.modules, &self.started, page);
        });
        if self.snap_band.is_none() {
            log!("bottom blur: the strip picture could not be made");
        }
    }

    /// 0..1: how far""")
s=sub(s,"            self.trim_at = None;\n            self.res.trim_bitmaps();","            self.trim_at = None;\n            if self.snap_band.is_some() {\n                log!(\"trim while the bottom blur is showing\");\n            }\n            self.res.trim_bitmaps();")
open(p,'w').write(s)
PYEOF
python3 /tmp/v162.py && echo patched
