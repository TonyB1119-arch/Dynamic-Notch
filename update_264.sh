#!/usr/bin/env bash
set -e
python3 - <<'PYEOF'
def edit(path,fn):
    s=open(path).read(); open(path,'w').write(fn(s))
def rep(s,a,b):
    if b in s: return s
    assert s.count(a)==1,(a,s.count(a))
    return s.replace(a,b)

def osd(s):
    s=rep(s,"use std::sync::atomic::{AtomicU32, Ordering};","use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};")
    s=rep(s,"static BRIGHT_PCT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);\n","""static BRIGHT_PCT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
/// What the default output is: 0 = the built-in speakers (or anything else), 1 = headphones / a
/// headset (AirPods and the like). Kept up to date by the worker; the media page shows it.
static OUT_KIND: AtomicU8 = AtomicU8::new(0);

/// True while the default output is a pair of headphones or a headset.
pub fn output_is_headphones() -> bool {
    OUT_KIND.load(Ordering::Relaxed) == 1
}
""")
    s=rep(s,"unsafe fn device_name(dev: &IMMDevice) -> String {","""/// PKEY_AudioEndpoint_FormFactor: 3 = headphones, 5 = headset.
const PKEY_FORM_FACTOR: PROPERTYKEY =
    PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e), pid: 0 };

/// Headphones or a headset: by what the device says it is, else by its name.
unsafe fn is_headphones(dev: &IMMDevice, name: &str) -> bool {
    if let Ok(store) = dev.OpenPropertyStore(STGM_READ) {
        if let Ok(mut pv) = store.GetValue(&PKEY_FORM_FACTOR) {
            let inner = &pv.Anonymous.Anonymous;
            let ff = if inner.vt == VT_UI4 { Some(inner.Anonymous.ulVal) } else { None };
            let _ = PropVariantClear(&mut pv);
            if let Some(ff) = ff {
                if ff == 3 || ff == 5 {
                    return true;
                }
            }
        }
    }
    let n = name.to_ascii_lowercase();
    ["headphone", "headset", "airpods", "earbuds", "earphone", "buds"].iter().any(|w| n.contains(w))
}

unsafe fn device_name(dev: &IMMDevice) -> String {""")
    s=rep(s,"    let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).ok()?;\n    vol.RegisterControlChangeNotify(events).ok()?;\n    bus.to_module(\n        ID,\n        OsdMsg::Device {\n            name: device_name(&dev),","    let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).ok()?;\n    vol.RegisterControlChangeNotify(events).ok()?;\n    let name = device_name(&dev);\n    OUT_KIND.store(is_headphones(&dev, &name) as u8, Ordering::Relaxed);\n    bus.to_module(\n        ID,\n        OsdMsg::Device {\n            name,")
    # next_output: remember each device's kind, show the new one at once, undo it when the switch fails
    s=rep(s,"    let mut devs: Vec<(String, String)> = Vec::new();","    let mut devs: Vec<(String, String, bool)> = Vec::new();")
    s=rep(s,"                    devs.push((id, device_name(&d)));","                    let name = device_name(&d);\n                    let hp = is_headphones(&d, &name);\n                    devs.push((id, name, hp));")
    s=rep(s,"    let pos = devs.iter().position(|(id, _)| Some(id) == cur.as_ref()).unwrap_or(devs.len() - 1);\n    let (id, name) = devs[(pos + 1) % devs.len()].clone();","    let pos = devs.iter().position(|(id, _, _)| Some(id) == cur.as_ref()).unwrap_or(devs.len() - 1);\n    let (id, name, hp) = devs[(pos + 1) % devs.len()].clone();\n    let before = OUT_KIND.swap(hp as u8, Ordering::Relaxed);")
    s=rep(s,"        let ok = set_default_output(&id);\n        bus.to_module","        let ok = set_default_output(&id);\n        if !ok {\n            OUT_KIND.store(before, Ordering::Relaxed);\n        }\n        bus.to_module")
    # native switch, the script only as a fallback
    s=rep(s,"fn set_default_output(id: &str) -> bool {\n    use std::os::windows::process::CommandExt;","""/// The same COM object the Sound control panel uses (no public API exists for this).
#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    unsafe fn GetMixFormat(&self, a: PCWSTR, b: *mut std::ffi::c_void) -> windows::core::HRESULT;
    unsafe fn GetDeviceFormat(&self, a: PCWSTR, b: i32, c: *mut std::ffi::c_void) -> windows::core::HRESULT;
    unsafe fn ResetDeviceFormat(&self, a: PCWSTR) -> windows::core::HRESULT;
    unsafe fn SetDeviceFormat(
        &self,
        a: PCWSTR,
        b: *mut std::ffi::c_void,
        c: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    unsafe fn GetProcessingPeriod(
        &self,
        a: PCWSTR,
        b: i32,
        c: *mut std::ffi::c_void,
        d: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    unsafe fn SetProcessingPeriod(&self, a: PCWSTR, b: *mut std::ffi::c_void) -> windows::core::HRESULT;
    unsafe fn GetShareMode(&self, a: PCWSTR, b: *mut std::ffi::c_void) -> windows::core::HRESULT;
    unsafe fn SetShareMode(&self, a: PCWSTR, b: *mut std::ffi::c_void) -> windows::core::HRESULT;
    unsafe fn GetPropertyValue(
        &self,
        a: PCWSTR,
        b: i32,
        c: *const PROPERTYKEY,
        d: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    unsafe fn SetPropertyValue(
        &self,
        a: PCWSTR,
        b: i32,
        c: *const PROPERTYKEY,
        d: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    unsafe fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> windows::core::HRESULT;
    unsafe fn SetEndpointVisibility(&self, a: PCWSTR, b: i32) -> windows::core::HRESULT;
}

/// Set the default output for every role directly: instant, with no process to start.
fn set_default_native(id: &str) -> bool {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let clsid = windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
        let Ok(pc) = CoCreateInstance::<_, IPolicyConfig>(&clsid, None, CLSCTX_ALL) else { return false };
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        (0..3).all(|role| pc.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), ERole(role)).is_ok())
    }
}

fn set_default_output(id: &str) -> bool {
    use std::os::windows::process::CommandExt;
    if set_default_native(id) {
        return true;
    }""")
    s=rep(s,"            OsdMsg::Device { name, level, muted } => {\n                self.device = name;","            OsdMsg::Device { name, level, muted } => {\n                cx.fx.redraw = true;\n                self.device = name;")
    s=rep(s,"            OsdMsg::Switched { name, ok } => {\n                self.switching = None;","            OsdMsg::Switched { name, ok } => {\n                cx.fx.redraw = true;\n                self.switching = None;")
    return s
edit('src/modules/osd.rs',osd)

def media(s):
    s=rep(s,"    ly_at: Instant,\n}\n","    ly_at: Instant,\n    /// After a click on the output button: what it should show until the system confirms.\n    out_want: Option<(bool, Instant)>,\n}\n")
    s=rep(s,"            ly_at: Instant::now(),\n        }\n    }","            ly_at: Instant::now(),\n            out_want: None,\n        }\n    }")
    s=rep(s,"    fn toggle(&mut self) {","""    /// Headphones (true) or the built-in speakers (false), as far as the system says; right after a
    /// click the answer it is going to give.
    fn out_headphones(&self) -> bool {
        match self.out_want {
            Some((hp, t)) if t.elapsed() < Duration::from_secs(3) => hp,
            _ => super::osd::output_is_headphones(),
        }
    }

    /// The button that moves the sound to the next output device. The icon (a laptop for the
    /// built-in speakers, headphones for AirPods and the like) turns into the other one with a
    /// short fade and turn.
    fn output_button(&mut self, ui: &mut Ui, r: Rect, fg: Color) {
        let hp = self.out_headphones();
        let (h, p) = ui.touch(r, "output-button");
        ui.p.fill_rounded(r.inset(r.w * 0.07 * p), r.h * 0.5, Color::white(0.08 * h));
        let k = ui.glide(0x0A07_0001, if hp { 1.0 } else { 0.0 });
        let size = r.w.min(r.h) * 0.5 * (1.0 - 0.14 * p) * (1.0 + 0.03 * h);
        let (cx, cy) = (r.cx(), r.cy());
        let old = ui.p.alpha;
        // the laptop leaves downward as the headphones come up from below
        if k < 0.995 {
            let a = 1.0 - k;
            ui.p.alpha = old * a;
            draw_laptop(&mut ui.p, cx, cy + k * 5.0, size * (0.7 + 0.3 * a), fg);
        }
        if k > 0.005 {
            ui.p.alpha = old * k;
            let s = size * (0.7 + 0.3 * k);
            ui.p.icon(Icon::Headphones, Rect::new(cx - s * 0.5, cy + (1.0 - k) * 5.0 - s * 0.5, s, s), fg);
        }
        ui.p.alpha = old;
        if ui.clicked(r) {
            self.out_want = Some((!hp, Instant::now()));
            super::osd::request_next_output(ui.bus);
        }
    }

    fn toggle(&mut self) {""")
    s=rep(s,"""        if ui.icon_button(Rect::centered(cx + 76.0, cy, 44.0, 40.0), Icon::Next, side.fg(dim(self.snap.can_next))) {
            self.send(Cmd::Next);
        }
""","""        if ui.icon_button(Rect::centered(cx + 76.0, cy, 44.0, 40.0), Icon::Next, side.fg(dim(self.snap.can_next))) {
            self.send(Cmd::Next);
        }
        self.output_button(ui, Rect::centered(cx + 130.0, cy, 40.0, 40.0), Color::white(0.9));
""")
    s=rep(s,"""        if ui.icon_button(next, Icon::Next, style.fg(dim(self.snap.can_next))) {
            self.send(Cmd::Next);
        }
""","""        if ui.icon_button(next, Icon::Next, style.fg(dim(self.snap.can_next))) {
            self.send(Cmd::Next);
        }
        let out = Rect::centered(next.cx() + small + 8.0, cy, small, small);
        self.output_button(ui, out, palette::TEXT);
""")
    s=rep(s,"// ---------------------------------------------------------------------------\n// Worker\n","""/// A small laptop in the line style of the other icons: the screen, and the base under it.
fn draw_laptop(p: &mut crate::gfx::Painter, cx: f32, cy: f32, s: f32, c: Color) {
    let sw = (s * 0.085).max(1.3);
    let screen = Rect::new(cx - s * 0.30, cy - s * 0.34, s * 0.60, s * 0.44);
    p.stroke_rounded(screen.inset(sw * 0.5), s * 0.07, sw, c);
    p.fill_rounded(Rect::new(cx - s * 0.46, cy + s * 0.17, s * 0.92, s * 0.11), s * 0.055, c);
}

// ---------------------------------------------------------------------------
// Worker
""")
    return s
edit('src/modules/media.rs',media)
print('ok')
PYEOF
echo "update_264 applied"
