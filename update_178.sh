#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
def sub(s,a,b):
    assert s.count(a)==1,(s.count(a),a[:70]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
# no re-render of the strip while pages swap (it would show the NEW page's bottom under the old one)
s=sub(s,"""    fn prepare_band(&mut self, now: Instant) {
        let t = self.targets();""","""    fn prepare_band(&mut self, now: Instant) {
        if self.page_from.is_some() && self.snap_band.is_some() && self.mode == Mode::Expanded {
            return;
        }
        let t = self.targets();""")
# do not lay the strip over the page while pages swap: the live pages are drawn whole
s=sub(s,"let band = if band_k > 0.01 && self.mode == Mode::Expanded { self.snap_band.as_ref() } else { None };",
        "let band = if band_k > 0.01 && self.mode == Mode::Expanded && self.page_from.is_none() {\n                                self.snap_band.as_ref()\n                            } else {\n                                None\n                            };")
open(p,'w').write(s)
PYEOF
echo "Patched v178"
