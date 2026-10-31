#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "two rows of cards" src/modules/wifi.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)
# Wi-Fi and Bluetooth: two rows of networks / devices instead of three
p='src/modules/wifi.rs'; s=open(p).read()
s=sub(s,"const ALC_LIST: f32 = 186.0;","const ALC_LIST: f32 = 116.0; // two rows of cards, the rest scrolls")
open(p,'w').write(s)
p='src/modules/bluetooth.rs'; s=open(p).read()
s=sub(s,"            return 48.0 + 12.0 + 186.0;","            return 48.0 + 12.0 + ALC_LIST;")
s=sub(s,"let area = Rect::new(r.x, head.bottom() + 12.0, r.w, 186.0);","let area = Rect::new(r.x, head.bottom() + 12.0, r.w, ALC_LIST);")
i=s.index("\nconst ")
s=s[:i]+"\n/// Height of the list of devices: two rows of cards, the rest scrolls.\nconst ALC_LIST: f32 = 116.0;"+s[i:]
open(p,'w').write(s)
# glass edge: softer
p='src/refract.rs'; s=open(p).read()
s=sub(s,"let pull = 28.0 * sc;","let pull = 20.0 * sc;")
s=sub(s,"let alpha = 0.96 * u * u","let alpha = 0.6 * u * u")
s=sub(s,"x + ox * 0.93, y + oy * 0.93, 8)","x + ox * 0.95, y + oy * 0.95, 8)")
s=sub(s,"x + ox * 0.86, y + oy * 0.86, 0)","x + ox * 0.9, y + oy * 0.9, 0)")
open(p,'w').write(s)
PYEOF
echo "Patched v197"
