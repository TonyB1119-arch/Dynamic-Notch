#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
import re
def has(s,a): return s.count(a)==1
def swap(s,a,b):
    return s if b in s else s.replace(a,b)

notes=[]
# ---- app.rs: efficiency mode (EcoQoS) while the notch is idle ----------------------------------
s=open('src/app.rs').read()
A1="        self.trim_at = None;\n        let shown = !(self.user_hidden"
B1="        self.trim_at = None;\n        eco(false);\n        let shown = !(self.user_hidden"
A2="            if let Some(b) = before {"
B2="            eco(true);\n            if let Some(b) = before {"
ok=("eco(false);" in s and "eco(true);" in s) or (has(s,A1) and has(s,A2))
if ok:
    s=swap(s,A1,B1); s=swap(s,A2,B2)
    if "pub fn eco(" not in s:
        s=s.rstrip("\n")+'''

static ECO: AtomicBool = AtomicBool::new(false);

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
    open('src/app.rs','w').write(s)
    eco_ok=True
else:
    notes.append("app.rs idle efficiency mode skipped (code differs)")
    eco_ok=False

# ---- dock.rs ---------------------------------------------------------------------------------
d=open('src/dock.rs').read()
if "EXE_CACHE" not in d:
    a1="fn running_apps(hide: &[String]) -> Vec<(String, isize)> {"
    a2="    let mut paths: HashMap<u32, String> = HashMap::new();\n"
    a3="        out.push((path, h));\n    }\n    out\n}"
    if has(d,a1) and has(d,a2) and has(d,a3):
        d=d.replace(a1,"thread_local! {\n    /// pid -> exe path, kept between scans so each pass doesn't reopen every process.\n    static EXE_CACHE: std::cell::RefCell<HashMap<u32, String>> = std::cell::RefCell::new(HashMap::new());\n}\n\n"+a1)
        d=d.replace(a2,"    let mut paths: HashMap<u32, String> = EXE_CACHE.with(|c| std::mem::take(&mut *c.borrow_mut()));\n    paths.retain(|pid, p| !p.is_empty() && wins.iter().any(|w| w.1 == *pid));\n")
        d=d.replace(a3,"        out.push((path, h));\n    }\n    EXE_CACHE.with(|c| *c.borrow_mut() = paths);\n    out\n}")
    else:
        notes.append("dock.rs path cache skipped (code differs)")
if eco_ok and "crate::app::eco(false)" not in d:
    a="    fn start_anim(&mut self) {\n"
    if has(d,a): d=d.replace(a,a+"        crate::app::eco(false);\n")
    else: notes.append("dock.rs start_anim hook missing: efficiency mode left OFF for safety"); \
          open('src/app.rs','w').write(open('src/app.rs').read().replace("            eco(true);\n",""))
open('src/dock.rs','w').write(d)
print("ok"); [print("NOTE:",n) for n in notes]
PYEOF
echo "update_267 applied"
