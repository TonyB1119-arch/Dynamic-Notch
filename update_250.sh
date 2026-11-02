#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
p = 'src/app.rs'
s = open(p).read()
new = '(self.rim_cost * 10 / 3).clamp(Duration::from_millis(18), Duration::from_millis(50))'
old = '(self.rim_cost * 3).clamp(Duration::from_millis(16), Duration::from_millis(45))'
if new not in s:
    print('already back to the original wait')
    raise SystemExit
s = s.replace(new, old)
s = s.replace('rebuilt at most every 50 ms', 'rebuilt at most every 45 ms')
open(p, 'w').write(s)
print('Wait restored')
PYEOF
