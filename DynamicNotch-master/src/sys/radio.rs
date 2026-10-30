//! Wi-Fi and Bluetooth radio switches (`Windows.Devices.Radios`).
//!
//! Blocking calls: run them on a worker thread, never on the UI thread.

use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

fn init() {
    // already-initialised is fine: the error is ignored on purpose
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

fn find(kind: RadioKind) -> Option<Radio> {
    init();
    if Radio::RequestAccessAsync().ok()?.join().ok()? != RadioAccessStatus::Allowed {
        return None;
    }
    let radios = Radio::GetRadiosAsync().ok()?.join().ok()?;
    (0..radios.Size().ok()?).filter_map(|i| radios.GetAt(i).ok()).find(|r| r.Kind().ok() == Some(kind))
}

/// `Some(true)` when the radio is on, `None` when there is no such radio or access was denied.
pub fn is_on(kind: RadioKind) -> Option<bool> {
    Some(find(kind)?.State().ok()? == RadioState::On)
}

/// Turn a radio on or off. Returns whether Windows accepted the change.
pub fn set_on(kind: RadioKind, on: bool) -> bool {
    let Some(r) = find(kind) else { return false };
    let state = if on { RadioState::On } else { RadioState::Off };
    r.SetStateAsync(state).ok().and_then(|op| op.join().ok()).is_some_and(|s| s == RadioAccessStatus::Allowed)
}
