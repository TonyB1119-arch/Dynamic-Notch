#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "follows it: while a page change" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
p='src/app.rs'; s=open(p).read()
s=sub(s,"""        // the rim is made as soon as the notch has reached its size, not only once every last
        // spring has come to rest (that tail made it appear a second late)
        let body = self.body_rect();
        let t = self.targets();
        let calm = self.page_from.is_none()
            && (self.anim.exp.value > 0.98 || self.anim.peek.value > 0.95)
            && (body.h - t.h).abs() < 1.0
            && (body.w - t.w).abs() < 1.0;
        if !calm {""","""        // the rim is there as soon as the notch is open and follows it: while a page change
        // resizes the notch it is made again for every size on the way (a small picture), so it
        // moves with the page instead of waiting for it. Not while the notch itself opens or closes.
        let body = self.body_rect();
        let open = self.anim.exp.value > 0.98 || self.anim.peek.value > 0.95;
        if !open {""")
s=sub(s,"let rim_to = if self.rim_ok && glass_k > 0.0 { 1.0 } else { 0.0 };","let rim_to = if self.rim_ok { 1.0 } else { 0.0 };")
open(p,'w').write(s)
s=open(p).read()
s=sub(s,"""        // the sheen and the lit edge are not drawn at all while a page is changing, so a page
        // switch runs exactly the same with the glass look on or off
        let glass_k = if self.page_from.is_some() { 0.0 } else { 1.0 };
""","")
open(p,'w').write(s)
PYEOF
echo "Patched v195"
