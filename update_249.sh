#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
p = 'src/app.rs'
s = open(p).read()
if 'rim_cost * 10 / 3' in s:
    print('already applied')
    raise SystemExit
old = '(self.rim_cost * 3).clamp(Duration::from_millis(16), Duration::from_millis(45))'
new = '(self.rim_cost * 10 / 3).clamp(Duration::from_millis(18), Duration::from_millis(50))'
assert s.count(old) == 1, s.count(old)
s = s.replace(old, new)
s = s.replace('rebuilt at most every 45 ms', 'rebuilt at most every 50 ms')
open(p, 'w').write(s)
print('Patched v249')
PYEOF
