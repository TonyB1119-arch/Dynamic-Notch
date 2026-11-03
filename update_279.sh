#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
notes = []
def get(p): return open(p).read()
def swap(s, a, b, tag, once=True):
    if s.count(a) != 1:
        notes.append(tag); return s
    return s.replace(a, b)

if "pub size: f32" not in get('src/config.rs'):
    print("NOTHING CHANGED - install updates 271 to 278 first"); raise SystemExit

# ---- config.rs ---------------------------------------------------------------------------------------
cf = get('src/config.rs')
if "pub struct CombosCfg" not in cf:
    cf = swap(cf, "    pub launcher: LauncherCfg,\n}\n", "    pub launcher: LauncherCfg,\n    pub combos: CombosCfg,\n}\n", "config: field")
    cf = swap(cf, "        self.launcher.clamped();\n", "        self.launcher.clamped();\n        self.combos.items.truncate(24);\n", "config: clamp")
    cf = swap(cf, "/// The quick launcher (hotkey: `[hotkeys] launcher`).", '''/// Combos: your own hotkeys that open things.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CombosCfg {
    /// "Win+B|browser": the keys, a bar, and what to open (an app, file, folder, link or `browser`).
    pub items: Vec<String>,
}

/// The quick launcher (hotkey: `[hotkeys] launcher`).''', "config: struct")
    cf = swap(cf, "[glass]", "[combos]\n# Your own hotkeys that open things, easiest to change in Settings (Combos tab).\n# Each one is 'keys|what to open'. \"browser\" means your default browser.\n# For example: items = ['Win+B|browser', 'Ctrl+Alt+N|notepad.exe']\nitems = []\n\n[glass]", "config: template")

# ---- dock.rs: share the %VAR% expansion --------------------------------------------------------------------
dk = get('src/dock.rs')
if "pub fn expand_env" not in dk:
    dk = swap(dk, "fn expand_env(s: &str) -> String {", "pub fn expand_env(s: &str) -> String {", "dock: expand_env")

# ---- hotkeys.rs --------------------------------------------------------------------------------------------
hk = get('src/hotkeys.rs')
if "register_combos" not in hk:
    hk = swap(hk, "    Launcher,\n}\n", "    Launcher,\n    /// One of the user's combos (its place in `[combos] items`).\n    Combo(usize),\n}\n", "hotkeys: enum")
    hk = swap(hk, "pub fn unregister_all(hwnd: HWND) {\n    for i in 0..8 {\n        unsafe {\n            let _ = UnregisterHotKey(Some(hwnd), BASE_ID + i);\n        }\n    }\n}\n",
'''pub fn unregister_all(hwnd: HWND) {
    for i in 0..8 {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), BASE_ID + i);
        }
    }
    for i in 0..MAX_COMBOS {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), COMBO_BASE + i as i32);
        }
    }
}

const COMBO_BASE: i32 = BASE_ID + 32;
const MAX_COMBOS: usize = 24;

/// Is this a key combination a combo may use? It needs Ctrl, Alt, Shift or Win (or be an F key), so a
/// combo can't take over a plain letter.
pub fn check_combo(keys: &str) -> Result<(), &'static str> {
    let Some((mods, vk)) = parse(keys) else {
        return Err("Can't read that key combination (try Win+B or Ctrl+Alt+G)");
    };
    let has_mod = mods.0 & (MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0 | MOD_WIN.0) != 0;
    let f_key = (VK_F1.0 as u32..=VK_F24.0 as u32).contains(&vk);
    if has_mod || f_key {
        Ok(())
    } else {
        Err("Add Ctrl, Alt, Shift or Win to the keys")
    }
}

/// Register the user's combos ("keys|what to open"). Returns what worked and what did not.
pub fn register_combos(hwnd: HWND, items: &[String]) -> (Vec<Registered>, Vec<String>) {
    let mut ok = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().take(MAX_COMBOS).enumerate() {
        let Some((keys, _)) = item.split_once('|') else {
            errors.push(format!("combo \\"{item}\\" has no |"));
            continue;
        };
        let keys = keys.trim();
        if let Err(e) = check_combo(keys) {
            errors.push(format!("{keys}: {e}"));
            continue;
        }
        let (mods, vk) = parse(keys).unwrap_or((MOD_NOREPEAT, 0));
        let id = COMBO_BASE + i as i32;
        if unsafe { RegisterHotKey(Some(hwnd), id, mods, vk) }.is_ok() {
            ok.push(Registered { id, action: Action::Combo(i) });
        } else {
            errors.push(format!("{keys} is already in use"));
        }
    }
    (ok, errors)
}

/// Open what a combo points at: `browser` is the default browser, anything else is opened as typed
/// (an app, file, folder or link; %VARIABLES% work).
pub fn open_target(target: &str, search_url: &str) {
    let t = target.trim();
    if t.is_empty() {
        return;
    }
    if t.eq_ignore_ascii_case("browser") || t.eq_ignore_ascii_case("default browser") {
        // the start page of the search engine opens the default browser
        let site = search_url
            .strip_prefix("https://")
            .or_else(|| search_url.strip_prefix("http://"))
            .and_then(|r| r.split('/').next())
            .filter(|h| !h.is_empty())
            .map_or("https://www.google.com".to_string(), |h| format!("https://{h}"));
        crate::sys::shell_open(&site);
    } else {
        crate::sys::shell_open(&crate::dock::expand_env(t));
    }
}
''', "hotkeys: unregister")
    hk = swap(hk, "        assert!(parse(\"\").is_none());\n    }\n", "        assert!(parse(\"\").is_none());\n    }\n\n    #[test]\n    fn combos_need_a_modifier() {\n        assert!(check_combo(\"Win+B\").is_ok());\n        assert!(check_combo(\"Ctrl+Alt+G\").is_ok());\n        assert!(check_combo(\"F9\").is_ok());\n        assert!(check_combo(\"B\").is_err());\n        assert!(check_combo(\"Win+Nope\").is_err());\n    }\n", "hotkeys: test")

# ---- app.rs ------------------------------------------------------------------------------------------------
ap = get('src/app.rs')
if "Action::Combo" not in ap:
    ap = swap(ap, "            Action::Launcher => self.launcher_toggle(),\n", '''            Action::Launcher => self.launcher_toggle(),
            Action::Combo(i) => {
                if let Some((_, target)) = self.cfg.combos.items.get(i).and_then(|s| s.split_once('|')) {
                    hotkeys::open_target(target, &self.cfg.launcher.search_url);
                }
            }
''', "app: on_hotkey")
    ap = swap(ap, "        let (ok, errors) = hotkeys::register_all(self.hwnd, &self.cfg.hotkeys);\n        self.hotkeys = ok;\n",
"        let (ok, mut errors) = hotkeys::register_all(self.hwnd, &self.cfg.hotkeys);\n        let (combos, combo_errors) = hotkeys::register_combos(self.hwnd, &self.cfg.combos.items);\n        self.hotkeys = ok;\n        self.hotkeys.extend(combos);\n        errors.extend(combo_errors);\n", "app: register")

# ---- settings.rs ---------------------------------------------------------------------------------------------
st = get('src/modules/settings.rs')
if "fn combos(" not in st:
    st = swap(st, 'const TABS: [&str; 10] = ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock", "Glass"];',
              'const TABS: [&str; 11] =\n    ["General", "Look", "Home", "Modules", "Weather", "Gemini", "Notch", "Tabs", "Dock", "Glass", "Combos"];', "settings: tabs")
    st = swap(st, "    Key,\n    Model,\n}\n", "    Key,\n    Model,\n    ComboKeys,\n    ComboTarget,\n}\n", "settings: focus")
    st = swap(st, "    model: TextField,\n    focus: Focus,\n", "    model: TextField,\n    combo_keys: TextField,\n    combo_target: TextField,\n    focus: Focus,\n", "settings: fields")
    st = swap(st, "            model: TextField::new(80),\n", "            model: TextField::new(80),\n            combo_keys: TextField::new(40),\n            combo_target: TextField::new(260),\n", "settings: new")
    st = swap(st, "            Focus::Model => self.model.key(key, self.hwnd),\n            Focus::None => false,\n        };\n        if enter {\n",
'''            Focus::Model => self.model.key(key, self.hwnd),
            Focus::ComboKeys => self.combo_keys.key(key, self.hwnd),
            Focus::ComboTarget => self.combo_target.key(key, self.hwnd),
            Focus::None => false,
        };
        if enter && self.focus == Focus::ComboKeys {
            // Enter in the keys box moves on to the box for what to open
            self.focus = Focus::ComboTarget;
            cx.fx.layout_changed = true;
            return;
        }
        if enter && self.focus == Focus::ComboTarget {
            if let Some(list) = self.add_combo() {
                cx.fx.save_config.push(("combos".into(), "items".into(), list_value(&list)));
            }
            cx.fx.keyboard_focus = Some(false);
            self.focus = Focus::None;
            cx.fx.layout_changed = true;
            return;
        }
        if enter {
''', "settings: on_key")
    st = swap(st, "                Focus::None => {}\n            }\n            cx.fx.keyboard_focus = Some(false);", "                Focus::None | Focus::ComboKeys | Focus::ComboTarget => {}\n            }\n            cx.fx.keyboard_focus = Some(false);", "settings: enter arms")
    st = swap(st, "            8 => self.dock(ui, body),\n            _ => self.glass(ui, body),\n", "            8 => self.dock(ui, body),\n            9 => self.glass(ui, body),\n            _ => self.combos(ui, body),\n", "settings: dispatch")
    st = swap(st, "        8 => 16.0,\n", "        8 => 16.0,\n        10 => 14.0,\n", "settings: rows")
    st = swap(st, "    fn gemini(&mut self, ui: &mut Ui, body: Rect) {", '''    /// Check the two boxes and add the combo; returns the new list to save.
    fn add_combo(&mut self) -> Option<Vec<String>> {
        let keys = self.combo_keys.text().trim().to_string();
        let target = self.combo_target.text().trim().replace('|', " ");
        if let Err(e) = crate::hotkeys::check_combo(&keys) {
            self.say(e, false);
            return None;
        }
        if target.is_empty() {
            self.say("Say what it should open: an app, file, folder, link or browser", false);
            return None;
        }
        let mut items = self.cfg.combos.items.clone();
        // the same keys again replace the old combo
        items.retain(|s| !s.split_once('|').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(&keys)));
        if items.len() >= 12 {
            self.say("That is the most combos (12)", false);
            return None;
        }
        items.push(format!("{keys}|{target}"));
        self.combo_keys.clear();
        self.combo_target.clear();
        self.say("Combo added", true);
        Some(items)
    }

    fn combos(&mut self, ui: &mut Ui, body: Rect) {
        let accent = ui.accent;
        let row = row_rect(body, 0);
        ui.p.text(
            "Your own hotkeys. Example: Win+B opens your default browser (type browser).",
            Rect::new(row.x + 2.0, row.y, row.w, row.h),
            &TextStyle::new(11.5, palette::TEXT_DIM),
        );
        let row = row_rect(body, 1);
        let add = Rect::new(row.right() - 74.0, row.y + 2.0, 74.0, row.h - 4.0);
        let kf = Rect::new(row.x, row.y + 2.0, 150.0, row.h - 4.0);
        let tf = Rect::new(kf.right() + 8.0, row.y + 2.0, add.x - 8.0 - (kf.right() + 8.0), row.h - 4.0);
        if self.combo_keys.draw(ui, kf, "Win+B", accent, self.focus == Focus::ComboKeys) {
            self.focus = Focus::ComboKeys;
            ui.fx.keyboard_focus = Some(true);
        }
        if self.combo_target.draw(
            ui,
            tf,
            "browser, an app, a folder or a link",
            accent,
            self.focus == Focus::ComboTarget,
        ) {
            self.focus = Focus::ComboTarget;
            ui.fx.keyboard_focus = Some(true);
        }
        if ui.pill_button(add, "Add", None, ButtonStyle::inverted()) {
            if let Some(list) = self.add_combo() {
                put(ui, "combos", "items", list_value(&list));
            }
            self.focus = Focus::None;
            ui.fx.keyboard_focus = Some(false);
        }
        let items = self.cfg.combos.items.clone();
        if items.is_empty() {
            let row = row_rect(body, 2);
            ui.p.text(
                "No combos yet. Type keys like Ctrl+Alt+G, then what to open, then press Add.",
                Rect::new(row.x + 2.0, row.y, row.w, row.h),
                &TextStyle::new(11.5, palette::TEXT_FAINT),
            );
        }
        for (i, item) in items.iter().enumerate().take(12) {
            let row = row_rect(body, 2 + i);
            let (keys, target) = item.split_once('|').unwrap_or((item.as_str(), ""));
            label(ui, row, keys.trim());
            let x = row.x + 160.0;
            ui.p.text(
                &crate::util::one_line(target.trim()),
                Rect::new(x, row.y, row.right() - x - 48.0, row.h),
                &TextStyle::new(12.0, palette::TEXT_DIM),
            );
            let rm = Rect::new(row.right() - 36.0, row.y + 4.0, 28.0, row.h - 8.0);
            if ui.icon_button(rm, Icon::Close, ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
                let mut next = items.clone();
                next.remove(i);
                put(ui, "combos", "items", list_value(&next));
            }
        }
    }

    fn gemini(&mut self, ui: &mut Ui, body: Rect) {''', "settings: combos fn")

if notes:
    print("NOTHING CHANGED - these spots look different in your code:")
    for n in notes: print("  -", n)
else:
    open('src/config.rs', 'w').write(cf)
    open('src/dock.rs', 'w').write(dk)
    open('src/hotkeys.rs', 'w').write(hk)
    open('src/app.rs', 'w').write(ap)
    open('src/modules/settings.rs', 'w').write(st)
    print("ok")
PYEOF
echo "update_279 done"
