#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)

# ---- app.rs: efficiency mode (EcoQoS) while the notch is idle ----------------------------------
s=open('src/app.rs').read()
helper='''static ECO: AtomicBool = AtomicBool::new(false);

/// Ask Windows to run this process on efficiency cores at low clocks (`true`) or normally (`false`).
/// Only on while nothing is moving, so animations never run throttled. Cheap to call repeatedly.
pub fn eco(on: bool) {
    if ECO.swap(on, Ordering::Relaxed) == on {
        return;
    }
    #[repr(C)]
    struct Throttle {
        version: u32,
        control: u32,
        state: u32,
    }
    let t = Throttle { version: 1, control: 1, state: u32::from(on) };
    unsafe {
        let _ = windows::Win32::System::Threading::SetProcessInformation(
            windows::Win32::System::Threading::GetCurrentProcess(),
            windows::Win32::System::Threading::PROCESS_INFORMATION_CLASS(4),
            &t as *const Throttle as *const core::ffi::c_void,
            std::mem::size_of::<Throttle>() as u32,
        );
    }
}

'''
if "pub fn eco(" not in s:
    i=s.index("/// This program's memory in MB")
    s=s[:i]+helper+s[i:]
s=rep(s,"        self.trim_at = None;\n        let shown = !(self.user_hidden","        self.trim_at = None;\n        eco(false);\n        let shown = !(self.user_hidden")
s=rep(s,"            if let Some(b) = before {","            eco(true);\n            if let Some(b) = before {")
open('src/app.rs','w').write(s)

# ---- dock.rs ---------------------------------------------------------------------------------
d=open('src/dock.rs').read()
d=rep(d,"/// The apps with a normal window right now: (exe path, one of its windows), topmost first.\nfn running_apps","thread_local! {\n    /// pid -> exe path, kept between scans so each pass doesn't reopen every process.\n    static EXE_CACHE: std::cell::RefCell<HashMap<u32, String>> = std::cell::RefCell::new(HashMap::new());\n}\n\n/// The apps with a normal window right now: (exe path, one of its windows), topmost first.\nfn running_apps")
d=rep(d,"    let mut paths: HashMap<u32, String> = HashMap::new();\n","    let mut paths: HashMap<u32, String> = EXE_CACHE.with(|c| std::mem::take(&mut *c.borrow_mut()));\n    paths.retain(|pid, p| !p.is_empty() && wins.iter().any(|w| w.1 == *pid));\n")
d=rep(d,"        out.push((path, h));\n    }\n    out\n}","        out.push((path, h));\n    }\n    EXE_CACHE.with(|c| *c.borrow_mut() = paths);\n    out\n}")
d=rep(d,"    fn start_anim(&mut self) {\n","    fn start_anim(&mut self) {\n        crate::app::eco(false);\n")
open('src/dock.rs','w').write(d)
print('ok')
PYEOF
echo "update_266 applied"
