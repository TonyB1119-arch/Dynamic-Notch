#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "fn prepare_pictures" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()

s=sub(s,'''    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_band(now);
        self.prepare_rim(now);
        let a_exp''','''    fn prepare_snapshots(&mut self, now: Instant) {
        self.prepare_pictures(now);
        self.prepare_band(now);
        self.prepare_rim(now);
    }

    fn prepare_pictures(&mut self, now: Instant) {
        let a_exp''')

s=sub(s,'''        if self.page_from.is_some() && self.snap_band.is_some() && self.mode == Mode::Expanded {
            return;
        }
        let t = self.targets();''','''        let t = self.targets();''')

s=sub(s,'''            let h = BAND_H.min(full.h * 0.4);
            let region = Rect::new(full.x, full.bottom() - h, full.w, h);
            self.snap_band''','''            let h = BAND_H.min(full.h * 0.4);
            let region = Rect::new(full.x, full.bottom() - h, full.w, h);
            let trans = self.page_from.map(|(from, dir)| (from, dir, self.anim.page.value.clamp(0.0, 1.0)));
            let tabs = if trans.is_some() { self.tab_list() } else { Vec::new() };
            let pages = self.snap_pages.as_ref().map(|(a, b)| (a, b));
            self.snap_band''')
s=sub(s,'''                draw_body(&mut ui, content, &mut self.modules, &self.started, page);
            });
        } else if let Some(act) = &self.peek {''','''                match trans {
                    Some(_) => draw_expanded_core(
                        &mut ui,
                        full,
                        &mut self.modules,
                        &self.started,
                        &tabs,
                        page,
                        trans,
                        &self.indicators,
                        pages,
                    ),
                    None => draw_body(&mut ui, content, &mut self.modules, &self.started, page),
                }
            });
        } else if let Some(act) = &self.peek {''')
s=sub(s,"let band = if band_k > 0.01 && self.mode == Mode::Expanded && self.page_from.is_none() {",
        "let band = if band_k > 0.01 && self.mode == Mode::Expanded {")
open(p,'w').write(s)
PYEOF
echo "Patched v191"
