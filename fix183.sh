#!/bin/bash
set -e
cd "$(dirname "$0")"
python3 - <<'PYEOF'
p='src/app.rs'; s=open(p).read()
a="fn fit_text(ui: &Ui, mut s: String"
assert s.count(a)==1
open(p,'w').write(s.replace(a,"fn fit_text(ui: &mut Ui, mut s: String"))
PYEOF
echo "Fixed"
