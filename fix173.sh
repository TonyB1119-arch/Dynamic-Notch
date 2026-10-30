#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
p='src/app.rs'; s=open(p).read()
for a,b in [("    notch_height: f32,\n}\n\nstruct Targets","    notch_height: f32,\n    glass_look: bool,\n}\n\nstruct Targets"),
 ("                notch_height: a.notch_height,\n            }\n        };","                notch_height: a.notch_height,\n                glass_look: a.glass_look,\n            }\n        };")]:
    assert s.count(a)==1,a; s=s.replace(a,b)
open(p,'w').write(s)
PYEOF
echo "Fixed"
