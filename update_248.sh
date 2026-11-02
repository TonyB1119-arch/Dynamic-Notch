#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "SPLIT_GAP" src/app.rs; then echo "already applied"; exit 0; fi
python3 - <<'PYEOF'
p = 'src/app.rs'
L = open(p).read().split('\n')

def find(text, start=0):
    hits = [i for i in range(start, len(L)) if text in L[i]]
    assert len(hits) >= 1, text
    return hits[0]

# 1. the bubbles sit 10 px from the notch
i = find('fn split_rects')
j = find('(s + 8.0)', i)
assert j - i < 8, 'split_rects'
L[j] = L[j].replace('(s + 8.0)', '(s + SPLIT_GAP)')
i = find('let side = if self.split_media()')
assert '+ 8.0' in L[i]
L[i] = L[i].replace('+ 8.0', '+ SPLIT_GAP')

# 2. while the media is split, the notch has no flared ears
i = find('t.w = if self.split_media() { base_w }')
pad = L[i][:len(L[i]) - len(L[i].lstrip())]
L[i + 1:i + 1] = [
    pad + 'if self.split_media() {',
    pad + '    t.ear = 0.0;',
    pad + '}',
]

# 3. and neither do its two bubbles
i = find('(lb, &act.left, true), (rb, &act.right, false)')
assert 'a_ear.min(6.0)' in L[i + 1]
L[i + 1] = L[i + 1].replace('a_ear.min(6.0)', '0.0')

L += [
    '',
    '/// Alcove music: the clear space between the notch and each of its two side bubbles.',
    'const SPLIT_GAP: f32 = 10.0;',
    '',
]
open(p, 'w').write('\n'.join(L))
PYEOF
echo "Patched v248"
