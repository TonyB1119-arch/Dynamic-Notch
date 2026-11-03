//! Global hotkeys from config strings like "Ctrl+Alt+Space".

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::config::Hotkeys as HotkeyCfg;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Ai,
    Timer,
    Clipboard,
    PlayPause,
    Dictate,
    Launcher,
    /// One of the user's combos (its place in `[combos] items`).
    Combo(usize),
}

const BASE_ID: i32 = 0x4E00;

pub struct Registered {
    pub id: i32,
    pub action: Action,
}

pub fn parse(s: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut mods = MOD_NOREPEAT;
    let mut vk: Option<u32> = None;
    // split on '+' but allow "+" itself as the final key ("Ctrl+Shift++" is unusual; skip)
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" | "meta" => mods |= MOD_WIN,
            _ => vk = Some(key_code(&lower)?),
        }
    }
    vk.map(|v| (mods, v))
}

fn key_code(k: &str) -> Option<u32> {
    let named = match k {
        "space" => VK_SPACE,
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" | "escape" => VK_ESCAPE,
        "backspace" => VK_BACK,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "insert" | "ins" => VK_INSERT,
        "delete" | "del" => VK_DELETE,
        "`" | "backtick" => VK_OEM_3,
        "-" | "minus" => VK_OEM_MINUS,
        "=" | "equals" => VK_OEM_PLUS,
        "[" => VK_OEM_4,
        "]" => VK_OEM_6,
        ";" => VK_OEM_1,
        "'" => VK_OEM_7,
        "," | "comma" => VK_OEM_COMMA,
        "." | "period" => VK_OEM_PERIOD,
        "/" | "slash" => VK_OEM_2,
        "\\" | "backslash" => VK_OEM_5,
        "playpause" | "mediaplaypause" => VK_MEDIA_PLAY_PAUSE,
        _ => VIRTUAL_KEY(0),
    };
    if named.0 != 0 {
        return Some(named.0 as u32);
    }
    let b = k.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_alphanumeric()) {
        return Some(b[0].to_ascii_uppercase() as u32);
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(VK_F1.0 as u32 + n - 1);
        }
    }
    None
}

/// Register all configured hotkeys. Returns what succeeded and a list of
/// human-readable failures (bad syntax or already taken by another app).
pub fn register_all(hwnd: HWND, cfg: &HotkeyCfg) -> (Vec<Registered>, Vec<String>) {
    unregister_all(hwnd);
    let mut ok = Vec::new();
    let mut errors = Vec::new();
    let list = [
        (Action::Toggle, &cfg.toggle),
        (Action::Ai, &cfg.ai),
        (Action::Timer, &cfg.timer),
        (Action::Clipboard, &cfg.clipboard),
        (Action::PlayPause, &cfg.play_pause),
        (Action::Dictate, &cfg.dictate),
        (Action::Launcher, &cfg.launcher),
    ];
    for (i, (action, s)) in list.iter().enumerate() {
        if s.trim().is_empty() {
            continue;
        }
        match parse(s) {
            Some((mods, vk)) => {
                let id = BASE_ID + i as i32;
                if unsafe { RegisterHotKey(Some(hwnd), id, mods, vk) }.is_ok() {
                    ok.push(Registered { id, action: *action });
                } else {
                    errors.push(format!("{s} is already in use"));
                }
            }
            None => errors.push(format!("can't parse hotkey \"{s}\"")),
        }
    }
    (ok, errors)
}

pub fn unregister_all(hwnd: HWND) {
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
            errors.push(format!("combo \"{item}\" has no |"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        let (m, k) = parse("Ctrl+Alt+Space").unwrap();
        assert!(m.contains(MOD_CONTROL) && m.contains(MOD_ALT));
        assert_eq!(k, VK_SPACE.0 as u32);
        assert_eq!(parse("win+shift+n").unwrap().1, b'N' as u32);
        assert_eq!(parse("Alt+F12").unwrap().1, VK_F12.0 as u32);
        assert!(parse("Ctrl+Nope").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn combos_need_a_modifier() {
        assert!(check_combo("Win+B").is_ok());
        assert!(check_combo("Ctrl+Alt+G").is_ok());
        assert!(check_combo("F9").is_ok());
        assert!(check_combo("B").is_err());
        assert!(check_combo("Win+Nope").is_err());
    }
}
