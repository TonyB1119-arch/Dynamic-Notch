//! Combos: your own hotkeys, caught by a keyboard hook before Windows or any program sees the keys.
//!
//! The hook runs on a thread of its own, so a busy notch can never make typing lag anywhere. The hook
//! function only compares a few numbers and posts a message to the notch window; it is installed only
//! while there is a combo (or a key is being recorded), and removed again when there is none.
//!
//! A key that starts a combo is swallowed: nothing else sees it, not even its release or the repeats of
//! a held key. When Win or Alt was held, a do-nothing key tap is added so the Start menu or an app's
//! menu bar does not open when the key is let go.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// The modifier bits (the same numbers `RegisterHotKey` uses).
pub const ALT: u32 = 1;
pub const CTRL: u32 = 2;
pub const SHIFT: u32 = 4;
pub const WIN: u32 = 8;

/// The id of the message that says a key combination was recorded (see `take_captured`).
pub const CAPTURE_ID: i32 = 0x4E60;

const M_INSTALL: u32 = WM_APP + 1;
const M_REMOVE: u32 = WM_APP + 2;
/// A key that does nothing at all, used to keep Start / menu bars from opening.
const VK_NOTHING: u16 = 0xE8;
const RECORD_MS: u64 = 12_000;

#[derive(Clone, Copy)]
struct Combo {
    mods: u32,
    vk: u32,
    id: i32,
}

static TABLE: Mutex<Vec<Combo>> = Mutex::new(Vec::new());
static CAPTURED: Mutex<Option<(u32, u32)>> = Mutex::new(None);
static APP: AtomicIsize = AtomicIsize::new(0);
static TID: AtomicU32 = AtomicU32::new(0);
static CAPTURING: AtomicBool = AtomicBool::new(false);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
/// The key that started a combo and has not been let go yet.
static HELD: AtomicU32 = AtomicU32::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

// -- control (called from the notch's main thread) ------------------------------------------------------

/// Replace the combos: (modifiers, key, message id). The hook is installed when there are any.
pub fn set_combos(app: HWND, list: Vec<(u32, u32, i32)>) {
    APP.store(app.0 as isize, Relaxed);
    if let Ok(mut t) = TABLE.lock() {
        *t = list.into_iter().map(|(mods, vk, id)| Combo { mods, vk, id }).collect();
    }
    sync();
}

/// Listen for the next key combination (for 12 seconds). The answer comes as a `Hotkey(CAPTURE_ID)` event.
pub fn start_capture(app: HWND) {
    APP.store(app.0 as isize, Relaxed);
    if let Ok(mut c) = CAPTURED.lock() {
        *c = None;
    }
    HELD.store(0, Relaxed);
    DEADLINE.store(now_ms() + RECORD_MS, Relaxed);
    CAPTURING.store(true, Relaxed);
    sync();
}

pub fn cancel_capture() {
    CAPTURING.store(false, Relaxed);
    sync();
}

/// Still waiting for keys?
pub fn capturing() -> bool {
    CAPTURING.load(Relaxed) && now_ms() < DEADLINE.load(Relaxed)
}

/// The recorded keys (modifiers, key), once.
pub fn take_captured() -> Option<(u32, u32)> {
    let got = CAPTURED.lock().ok().and_then(|mut c| c.take());
    sync();
    got
}

/// Install the hook when it is needed, remove it when it is not.
fn sync() {
    let wanted = CAPTURING.load(Relaxed) || TABLE.lock().map_or(false, |t| !t.is_empty());
    if wanted {
        let tid = ensure_thread();
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, M_INSTALL, WPARAM(0), LPARAM(0));
            }
        }
    } else {
        let tid = TID.load(Relaxed);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, M_REMOVE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn ensure_thread() -> u32 {
    let tid = TID.load(Relaxed);
    if tid != 0 {
        return tid;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    if std::thread::Builder::new().name("combo-keys".into()).spawn(move || thread_main(tx)).is_err() {
        return 0;
    }
    let tid = rx.recv_timeout(Duration::from_secs(2)).unwrap_or(0);
    TID.store(tid, Relaxed);
    tid
}

// -- the hook thread ---------------------------------------------------------------------------------------

fn thread_main(tx: Sender<u32>) {
    unsafe {
        let mut msg = MSG::default();
        // make the message queue exist before anyone posts to it
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        let _ = tx.send(GetCurrentThreadId());
        let mut hook: Option<HHOOK> = None;
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            match msg.message {
                M_INSTALL => {
                    if hook.is_none() {
                        HELD.store(0, Relaxed);
                        hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0).ok();
                    }
                }
                M_REMOVE => {
                    if let Some(h) = hook.take() {
                        let _ = UnhookWindowsHookEx(h);
                    }
                }
                _ => {}
            }
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == 0 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        // keys sent by programs (including our own do-nothing tap) pass untouched
        if k.flags.0 & 0x10 == 0 {
            let vk = k.vkCode;
            match wp.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if !is_modifier(vk) && on_down(vk) {
                        return LRESULT(1);
                    }
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    if vk != 0 && vk == HELD.load(Relaxed) {
                        HELD.store(0, Relaxed);
                        return LRESULT(1);
                    }
                }
                _ => {}
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// A key (not a modifier) went down. True when it belongs to us and must not go any further.
fn on_down(vk: u32) -> bool {
    // the repeats of a key that started a combo
    if vk == HELD.load(Relaxed) {
        return true;
    }
    let mods = held_mods();
    if CAPTURING.load(Relaxed) {
        if now_ms() > DEADLINE.load(Relaxed) {
            CAPTURING.store(false, Relaxed);
        } else if let Ok(mut c) = CAPTURED.try_lock() {
            *c = Some((mods, vk));
            CAPTURING.store(false, Relaxed);
            swallow(vk, mods);
            post(CAPTURE_ID);
            return true;
        } else {
            return false;
        }
    }
    let id = match TABLE.try_lock() {
        Ok(t) => t.iter().find(|c| c.vk == vk && c.mods == mods).map(|c| c.id),
        Err(_) => None,
    };
    match id {
        Some(id) => {
            swallow(vk, mods);
            post(id);
            true
        }
        None => false,
    }
}

fn swallow(vk: u32, mods: u32) {
    HELD.store(vk, Relaxed);
    if mods & (WIN | ALT) != 0 {
        // without this, letting go of Win (or Alt) would open the Start menu (or a menu bar)
        let tap = [tap_input(false), tap_input(true)];
        unsafe {
            SendInput(&tap, std::mem::size_of::<INPUT>() as i32);
        }
    }
}

fn tap_input(up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_NOTHING),
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn post(id: i32) {
    let app = APP.load(Relaxed);
    if app != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(app as *mut core::ffi::c_void)), WM_HOTKEY, WPARAM(id as usize), LPARAM(0));
        }
    }
}

fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

fn held_mods() -> u32 {
    let down = |vk: i32| unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 };
    let mut m = 0;
    if down(0x11) {
        m |= CTRL;
    }
    if down(0x12) {
        m |= ALT;
    }
    if down(0x10) {
        m |= SHIFT;
    }
    if down(0x5B) || down(0x5C) {
        m |= WIN;
    }
    m
}

// -- names -----------------------------------------------------------------------------------------------

/// "Ctrl+Alt+G" for a recorded combination; None for a key we have no name for.
pub fn describe(mods: u32, vk: u32) -> Option<String> {
    let key = key_name(vk)?;
    let mut s = String::new();
    for (bit, name) in [(CTRL, "Ctrl+"), (ALT, "Alt+"), (SHIFT, "Shift+"), (WIN, "Win+")] {
        if mods & bit != 0 {
            s.push_str(name);
        }
    }
    s.push_str(&key);
    Some(s)
}

fn key_name(vk: u32) -> Option<String> {
    let named = match vk {
        0x41..=0x5A | 0x30..=0x39 => return Some((vk as u8 as char).to_string()),
        0x70..=0x87 => return Some(format!("F{}", vk - 0x6F)),
        0x20 => "Space",
        0x0D => "Enter",
        0x09 => "Tab",
        0x1B => "Esc",
        0x08 => "Backspace",
        0x26 => "Up",
        0x28 => "Down",
        0x25 => "Left",
        0x27 => "Right",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x2D => "Insert",
        0x2E => "Delete",
        0xC0 => "`",
        0xBD => "-",
        0xBB => "=",
        0xDB => "[",
        0xDD => "]",
        0xBA => ";",
        0xDE => "'",
        0xBC => ",",
        0xBE => ".",
        0xBF => "/",
        0xDC => "\\",
        _ => return None,
    };
    Some(named.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_the_parser() {
        for (mods, vk) in [(WIN, 0x42u32), (CTRL | ALT, 0x47), (SHIFT, 0x70), (CTRL | SHIFT | WIN, 0x20), (ALT, 0xBF)] {
            let name = describe(mods, vk).unwrap();
            let (m, k) = crate::hotkeys::parse(&name).unwrap();
            assert_eq!((m.0 & 0xF, k), (mods, vk), "{name}");
        }
        assert_eq!(describe(WIN, 0x42).as_deref(), Some("Win+B"));
        assert!(describe(0, 0x6B).is_none());
    }
}
