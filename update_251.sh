#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
p = 'src/app.rs'
s = open(p).read()
if 'SPLIT_GAP: f32 = 14.0' in s:
    print('already applied')
    raise SystemExit
assert 'SPLIT_GAP: f32 = 10.0' in s, 'run update_248.sh first'

def sub(a, b):
    global s
    assert s.count(a) == 1, (s.count(a), a)
    s = s.replace(a, b)

# small concave corners come back, with a wider gap so the two flares never meet
sub('SPLIT_GAP: f32 = 10.0', 'SPLIT_GAP: f32 = 14.0')
sub('        t.ear = 0.0;\n                    }', '        t.ear = 5.0;\n                    }')
sub('gfx::notch_path(ui.p, br, br.w * 0.5, 0.0)',
    'gfx::notch_path(ui.p, br, br.w * 0.5, a_ear.min(5.0))')
open(p, 'w').write(s)
print('Patched v251')
PYEOF
