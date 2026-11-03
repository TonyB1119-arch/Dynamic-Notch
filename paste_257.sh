cat > /tmp/d.diff <<'DIFFEOF'
--- a/src/dock.rs
+++ b/src/dock.rs
@@ -14 +14 @@
-use windows::core::{w, BOOL, PCWSTR, PWSTR};
+use windows::core::{w, Interface, BOOL, PCWSTR, PWSTR};
@@ -16,3 +16,2 @@
-use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
-use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
-use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
+use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
+use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
@@ -57,0 +57,4 @@
+/// The pin picker panel (dips).
+const PICK_W: f32 = 340.0;
+const PICK_H: f32 = 400.0;
+const PICK_ROW: f32 = 38.0;
@@ -70,0 +74,2 @@
+    /// Cache key of its icon (built once, not on every frame).
+    key: String,
@@ -83,0 +89,4 @@
+    /// The pin picker: installed apps (Start menu), first visible row, open or not.
+    picker: bool,
+    apps: Vec<Item>,
+    scroll: usize,
@@ -119,0 +129,3 @@
+            picker: false,
+            apps: Vec::new(),
+            scroll: 0,
@@ -223 +235,2 @@
-            let win_h = base * mag + 2.0 * pad + 44.0 + self.foot() - FOOT;
+            let win_w = if self.picker { win_w.max(PICK_W + 16.0).min(room.max(PICK_W + 16.0)) } else { win_w };
+            let win_h = base * mag + 2.0 * pad + 44.0 + self.foot() - FOOT + if self.picker { PICK_H + 12.0 } else { 0.0 };
@@ -254 +267,3 @@
-        let mode = if self.hidden() && !full {
+        let mode = if self.picker {
+            1u8
+        } else if self.hidden() && !full {
@@ -335 +350 @@
-                items.push(Item {
+                items.push(with_key(Item {
@@ -342 +357,2 @@
-                });
+                    key: String::new(),
+                }));
@@ -362,2 +378 @@
-            let key = icon_key(it);
-            if !self.icons.contains_key(&key) {
+            if !self.icons.contains_key(&it.key) {
@@ -365 +380 @@
-                self.icons.insert(key, img);
+                self.icons.insert(it.key.clone(), img);
@@ -376,0 +392,3 @@
+        if self.picker {
+            self.load_picker_icons();
+        }
@@ -381 +399 @@
-        let radius = bar.h * 0.5;
+        let radius = bar.h * 0.36;
@@ -388 +406 @@
-            if self.hide_t > 0.98 { 0.0 } else { 1.0 },
+            if self.hide_t > 0.98 { 0.0 } else { op },
@@ -404 +422 @@
-            p.fill_rounded(bar, radius, Color::rgba(0.55, 0.57, 0.62, 0.03 + op * 0.40));
+            p.fill_rounded(bar, radius, Color::rgba(0.55, 0.57, 0.62, op * 0.30));
@@ -417 +435 @@
-                match self.icons.get(&icon_key(it)).and_then(|o| o.as_ref()) {
+                match self.icons.get(&it.key).and_then(|o| o.as_ref()) {
@@ -442,0 +461,42 @@
+            if self.picker {
+                p.set_local(1.0, 0.0, 0.0, 0.0, 0.0);
+                let pw = self.panel_rect();
+                p.fill_rounded(pw, 20.0, Color::rgba(0.09, 0.09, 0.11, 0.96));
+                let title = TextStyle::new(15.0, Color::white(0.95)).semibold();
+                p.text("Pin apps", Rect::new(pw.x + 16.0, pw.y + 10.0, 100.0, 28.0), &title);
+                let dim = TextStyle::new(11.0, Color::white(0.45));
+                p.text("scroll for more", Rect::new(pw.x + 96.0, pw.y + 10.0, 130.0, 28.0), &dim);
+                let done = Rect::new(pw.right() - 76.0, pw.y + 10.0, 64.0, 28.0);
+                p.fill_rounded(done, 14.0, Color::white(0.14));
+                p.text("Done", done, &TextStyle::new(12.0, Color::white(0.95)).semibold().center());
+                let vis = ((PICK_H - 56.0) / PICK_ROW) as usize;
+                if self.apps.is_empty() {
+                    p.text("No apps found", Rect::new(pw.x, pw.y + 60.0, pw.w, 30.0), &TextStyle::new(11.0, Color::white(0.45)).center());
+                }
+                let name_st = TextStyle::new(13.0, Color::white(0.92));
+                let pill_st = TextStyle::new(11.5, Color::white(0.9)).semibold().center();
+                let pill_on = TextStyle::new(11.5, Color::rgba(0.08, 0.08, 0.1, 1.0)).semibold().center();
+                for (k, it) in self.apps.iter().skip(self.scroll).take(vis).enumerate() {
+                    let row = Rect::new(pw.x + 8.0, pw.y + 52.0 + k as f32 * PICK_ROW, pw.w - 16.0, PICK_ROW - 2.0);
+                    if row.contains(self.mx_to, self.my) {
+                        p.fill_rounded(row, 10.0, Color::white(0.07));
+                    }
+                    let ir = Rect::new(row.x + 8.0, row.y + (row.h - 26.0) * 0.5, 26.0, 26.0);
+                    match self.icons.get(&it.key).and_then(|o| o.as_ref()) {
+                        Some(img) => p.image(img, ir, 0.0, 1.0),
+                        None => p.fill_rounded(ir, 7.0, Color::white(0.14)),
+                    }
+                    p.text(&it.label, Rect::new(row.x + 46.0, row.y, row.w - 46.0 - 80.0, row.h), &name_st);
+                    let on = self.pinned.iter().any(|q| {
+                        q.label.eq_ignore_ascii_case(&it.label) || q.path.eq_ignore_ascii_case(&it.path)
+                    });
+                    let pill = Rect::new(row.right() - 72.0, row.y + (row.h - 24.0) * 0.5, 64.0, 24.0);
+                    if on {
+                        p.fill_rounded(pill, 12.0, Color::white(0.92));
+                        p.text("Pinned", pill, &pill_on);
+                    } else {
+                        p.fill_rounded(pill, 12.0, Color::white(0.12));
+                        p.text("Pin", pill, &pill_st);
+                    }
+                }
+            }
@@ -457,0 +518,13 @@
+            WM_MOUSEWHEEL if self.picker => {
+                let d = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
+                let max = self.apps.len().saturating_sub(self.visible_rows()) as i32;
+                self.scroll = (self.scroll as i32 - d / 120 * 2).clamp(0, max) as usize;
+                self.draw(gfx);
+                Some(LRESULT(0))
+            }
+            WM_LBUTTONDOWN if self.picker => Some(LRESULT(0)),
+            WM_LBUTTONUP if self.picker => {
+                let (x, y) = self.point(lp);
+                self.picker_click(gfx, x, y);
+                Some(LRESULT(0))
+            }
@@ -507 +580 @@
-                if self.cfg.auto_hide && self.engage_to == 0.0 {
+                if self.cfg.auto_hide && self.engage_to == 0.0 && !self.picker {
@@ -573,0 +647,89 @@
+    // -- the pin picker -----------------------------------------------------------------------------
+
+    fn panel_rect(&self) -> Rect {
+        let w = PICK_W.min(self.win_w - 16.0);
+        Rect::new((self.win_w - w) * 0.5, 4.0, w, PICK_H)
+    }
+
+    fn visible_rows(&self) -> usize {
+        ((PICK_H - 56.0) / PICK_ROW) as usize
+    }
+
+    pub fn open_picker(&mut self, gfx: &Gfx) {
+        self.apps = scan_start_menu();
+        self.scroll = 0;
+        self.picker = true;
+        self.hide_t = 0.0;
+        self.hide_to = 0.0;
+        self.region_full = None;
+        self.place(gfx);
+        self.draw(gfx);
+    }
+
+    fn close_picker(&mut self, gfx: &Gfx) {
+        self.picker = false;
+        self.apps = Vec::new();
+        let items = &self.items;
+        self.icons.retain(|k, _| items.iter().any(|i| &i.key == k));
+        self.region_full = None;
+        self.place(gfx);
+        if self.cfg.auto_hide {
+            unsafe {
+                SetTimer(Some(self.hwnd), T_HIDE, 700, None);
+            }
+        }
+        self.draw(gfx);
+    }
+
+    fn load_picker_icons(&mut self) {
+        let dir = icons_dir();
+        let vis = self.visible_rows();
+        for it in self.apps.iter().skip(self.scroll).take(vis) {
+            if !self.icons.contains_key(&it.key) {
+                let img = load_icon(it, &dir);
+                self.icons.insert(it.key.clone(), img);
+            }
+        }
+    }
+
+    fn picker_click(&mut self, gfx: &Gfx, x: f32, y: f32) {
+        let pw = self.panel_rect();
+        if !pw.contains(x, y) || Rect::new(pw.right() - 76.0, pw.y + 10.0, 64.0, 28.0).contains(x, y) {
+            self.close_picker(gfx);
+            return;
+        }
+        let ly = pw.y + 52.0;
+        if y >= ly {
+            let k = self.scroll + ((y - ly) / PICK_ROW) as usize;
+            if (y - ly) / PICK_ROW < self.visible_rows() as f32 && k < self.apps.len() {
+                self.toggle_pin(gfx, k);
+            }
+        }
+    }
+
+    fn toggle_pin(&mut self, gfx: &Gfx, k: usize) {
+        let (name, path) = (self.apps[k].label.clone(), self.apps[k].path.clone());
+        let mut items = self.cfg.items.clone();
+        let before = items.len();
+        items.retain(|s| {
+            let p = parse_item(s);
+            !(p.label.eq_ignore_ascii_case(&name) || p.path.eq_ignore_ascii_case(&path))
+        });
+        if items.len() == before {
+            let target =
+                resolve_lnk(&path).filter(|t| t.to_ascii_lowercase().ends_with(".exe")).unwrap_or(path);
+            items.push(format!("{name}|{target}"));
+        }
+        self.cfg.items = items;
+        crate::config::set_value("dock", "items", &items_value(&self.cfg.items));
+        self.pinned = self.cfg.items.iter().filter(|s| !s.trim().is_empty()).map(|s| parse_item(s)).collect();
+        self.refresh_items();
+        self.place(gfx);
+        self.draw(gfx);
+    }
+
+    /// The pinned list as the dock currently has it.
+    pub fn items_cfg(&self) -> Vec<String> {
+        self.cfg.items.clone()
+    }
+
@@ -712 +874,6 @@
-    Item { label, path, exe, png: (!png.is_empty()).then(|| png.to_string()), pinned: true, hwnd: 0 }
+    with_key(Item { label, path, exe, png: (!png.is_empty()).then(|| png.to_string()), pinned: true, hwnd: 0, key: String::new() })
+}
+
+fn with_key(mut it: Item) -> Item {
+    it.key = icon_key(&it);
+    it
@@ -914,2 +1081 @@
-    let arr: Vec<String> = items.iter().map(|s| format!("'{}'", s.replace('\'', ""))).collect();
-    crate::config::set_value("dock", "items", &format!("[{}]", arr.join(", ")));
+    crate::config::set_value("dock", "items", &items_value(items));
@@ -919,2 +1085,7 @@
-/// A file picker for programs and shortcuts.
-fn pick_app(owner: HWND) -> Option<String> {
+fn items_value(items: &[String]) -> String {
+    let arr: Vec<String> = items.iter().map(|s| format!("'{}'", s.replace('\'', ""))).collect();
+    format!("[{}]", arr.join(", "))
+}
+
+/// The program a shortcut points to ("" when it has none, as with Store apps).
+fn resolve_lnk(path: &str) -> Option<String> {
@@ -922,10 +1093,33 @@
-        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
-        let spec = [COMDLG_FILTERSPEC { pszName: w!("Programs and shortcuts"), pszSpec: w!("*.exe;*.lnk") }];
-        let _ = dlg.SetFileTypes(&spec);
-        let _ = dlg.SetTitle(w!("Add an app to the dock"));
-        dlg.Show(Some(owner)).ok()?;
-        let item = dlg.GetResult().ok()?;
-        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
-        let out = name.to_string().ok();
-        CoTaskMemFree(Some(name.0 as *const _));
-        out
+        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
+        let pf: IPersistFile = link.cast().ok()?;
+        let wp = wstr(path);
+        pf.Load(PCWSTR(wp.as_ptr()), STGM_READ).ok()?;
+        let mut buf = [0u16; 520];
+        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
+        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
+        (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
+    }
+}
+
+/// Every app shortcut in the Start menu, sorted by name.
+fn scan_start_menu() -> Vec<Item> {
+    fn walk(dir: &Path, depth: u32, out: &mut Vec<(String, String)>) {
+        let Ok(rd) = std::fs::read_dir(dir) else { return };
+        for e in rd.flatten() {
+            let p = e.path();
+            if p.is_dir() {
+                if depth < 3 {
+                    walk(&p, depth + 1, out);
+                }
+            } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("lnk")) {
+                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
+                let l = name.to_ascii_lowercase();
+                if ["uninstall", "readme", "documentation", "release notes", "license", " help"]
+                    .iter()
+                    .any(|w| l.contains(w))
+                {
+                    continue;
+                }
+                out.push((name, p.to_string_lossy().to_string()));
+            }
+        }
@@ -932,0 +1127,14 @@
+    let mut found: Vec<(String, String)> = Vec::new();
+    for (var, tail) in [("ProgramData", r"Microsoft\Windows\Start Menu\Programs"), ("APPDATA", r"Microsoft\Windows\Start Menu\Programs")] {
+        if let Ok(base) = std::env::var(var) {
+            walk(&Path::new(&base).join(tail), 0, &mut found);
+        }
+    }
+    found.sort_by_key(|f| f.0.to_ascii_lowercase());
+    found.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
+    found
+        .into_iter()
+        .map(|(name, path)| {
+            with_key(Item { label: name, path, exe: String::new(), png: None, pinned: false, hwnd: 0, key: String::new() })
+        })
+        .collect()
@@ -946 +1154 @@
-        let add = wstr("Add an app…");
+        let add = wstr("Add or remove apps…");
@@ -967,6 +1175 @@
-            if let Some(p) = pick_app(hwnd) {
-                if !items.iter().any(|s| same_path(s, &p)) {
-                    items.push(format!("{}|{}", nice_name(&p), p));
-                    save_items(&items);
-                }
-            }
+            crate::window::with_app(|a| a.dock_open_picker());
DIFFEOF
patch --no-backup-if-mismatch -s src/dock.rs < /tmp/d.diff && rm -f /tmp/d.diff
python3 - <<'PYEOF'
def edit(path,fn):
    s=open(path).read(); open(path,'w').write(fn(s))
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)
def app(s):
    s=rep(s,"    pub fn dock_items(&self) -> Vec<String> {\n        self.cfg.dock.items.clone()\n    }","    pub fn dock_items(&self) -> Vec<String> {\n        match self.dock.as_ref() {\n            Some(d) => d.items_cfg(),\n            None => self.cfg.dock.items.clone(),\n        }\n    }\n\n    /// Open the pin picker panel above the dock.\n    pub fn dock_open_picker(&mut self) {\n        if let Some(d) = self.dock.as_mut() {\n            d.open_picker(&self.gfx);\n        }\n    }")
    return s
edit('src/app.rs',app)
print("ok")
PYEOF
