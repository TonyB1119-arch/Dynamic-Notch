#!/bin/bash
set -e
cd "$(dirname "$0")"
if grep -q "rim_pool" src/gfx/mod.rs; then echo "already applied"; exit 0; fi
if ! grep -q "st == 1 && ox == 0.0" src/refract.rs; then echo "run update_219.sh first"; exit 1; fi
python3 - <<'PYEOF'
def sub(s,a,b,n=1):
    assert s.count(a)==n,(s.count(a),a[:90]); return s.replace(a,b)

# ---- springs: stop a hair sooner (far below what a screen can show) ----
p='src/anim.rs'; s=open(p).read()
s=sub(s,"Self { value, velocity: 0.0, target: value, stiffness: 0.0, damping: 0.0, eps: 0.01, pending: None };","Self { value, velocity: 0.0, target: value, stiffness: 0.0, damping: 0.0, eps: 0.02, pending: None };")
s=sub(s,"        s.eps = 0.0015;\n","        s.eps = 0.003;\n")
# the slow last stretch (under half a pixel a second) is not worth a frame each
s=sub(s,"self.velocity.abs() < self.eps * 10.0","self.velocity.abs() < self.eps * 25.0")
open(p,'w').write(s)

p='src/gfx/mod.rs'; s=open(p).read()
# ---- the pool of glass pictures ----
s=sub(s,"""    band_geo: Option<((u32, u32), ID2D1Geometry)>,
}
""","""    band_geo: Option<((u32, u32), ID2D1Geometry)>,
    /// Glass pictures that are no longer shown, kept to be filled again instead of allocating a new
    /// one for every rebuild (see `rim_pic`).
    rim_pool: RimPool,
}

/// Spare glass pictures: (width, height of the picture, the picture).
type RimPool = std::sync::Arc<std::sync::Mutex<Vec<(u32, u32, ID2D1Bitmap1)>>>;
""")
s=sub(s,"""                band_mask: None,
                band_geo: None,
            })""","""                band_mask: None,
                band_geo: None,
                rim_pool: RimPool::default(),
            })""")
s=sub(s,"""        self.layouts.clear();
        self.realizations.borrow_mut().clear();
""","""        self.layouts.clear();
        self.realizations.borrow_mut().clear();
        if let Ok(mut pool) = self.rim_pool.lock() {
            pool.clear();
        }
""")
a=s.index("/// The glass rim as one small picture (see `refract`)")
b=s.index("impl<'a> Painter<'a> {\n    /// Draw the glass rim")
s=s[:a]+'''/// The glass rim as one small picture (see `refract`), and where it sits in logical units.
pub struct RimPic {
    bmp: ID2D1Bitmap1,
    pub rect: Rect,
    step: usize,
    /// The part of the picture that holds the rim (the picture itself is rounded up to a multiple
    /// of 64, so one picture fits many slightly different sizes), and the picture's own size.
    used: (u32, u32),
    cap: (u32, u32),
    pool: RimPool,
}

impl Drop for RimPic {
    /// A picture that is no longer shown goes back to the pool to be filled again.
    fn drop(&mut self) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.retain(|e| (e.0, e.1) == self.cap);
            if pool.len() < 2 {
                pool.push((self.cap.0, self.cap.1, self.bmp.clone()));
            }
        }
    }
}

impl Res {
    /// Upload premultiplied BGRA pixels (`w` x `h`, at (`x0`, `y0`) in window pixels) as a picture.
    /// The picture is taken from the pool when one of the right size is spare, and only the rim's
    /// pixels are copied into it.
    pub fn rim_pic(&self, x0: i32, y0: i32, w: usize, h: usize, step: usize, px: &[u32], scale: f32) -> Option<RimPic> {
        if w == 0 || h == 0 || px.len() < w * h || w > 4096 || h > 4096 {
            return None;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: ManuallyDrop::new(None),
        };
        let up = |v: usize| (v.div_ceil(64) * 64) as u32;
        let cap = (up(w), up(h));
        let used = (w as u32, h as u32);
        unsafe {
            let data = px.as_ptr() as *const std::ffi::c_void;
            let pitch = (w * 4) as u32;
            let region = D2D_RECT_U { left: 0, top: 0, right: used.0, bottom: used.1 };
            let spare = self
                .rim_pool
                .lock()
                .ok()
                .and_then(|mut p| p.iter().position(|e| (e.0, e.1) == cap).map(|i| p.swap_remove(i).2));
            let bmp = match spare {
                Some(b) if b.CopyFromMemory(Some(&region), data, pitch).is_ok() => b,
                _ => {
                    let b = self.dc.CreateBitmap(D2D_SIZE_U { width: cap.0, height: cap.1 }, None, 0, &props).ok()?;
                    b.CopyFromMemory(Some(&region), data, pitch).ok()?;
                    b
                }
            };
            let k = step.max(1) as f32;
            Some(RimPic {
                bmp,
                rect: Rect::new(x0 as f32 / scale, y0 as f32 / scale, w as f32 * k / scale, h as f32 * k / scale),
                step,
                used,
                cap,
                pool: self.rim_pool.clone(),
            })
        }
    }
}

'''+s[b:]
s=sub(s,"""            self.dc.DrawBitmap(&b, Some(&rf(r.rect)), a, mode, None, None);
        }
    }
}
""","""            // only the part that holds the rim (the picture may be a little bigger)
            let src = D2D_RECT_F { left: 0.0, top: 0.0, right: r.used.0 as f32, bottom: r.used.1 as f32 };
            self.dc.DrawBitmap(&b, Some(&rf(r.rect)), a, mode, Some(&src), None);
        }
    }
}
""")
# ---- a page picture only needs the quarter-size step: skip the one nobody uses ----
s=sub(s,"""                for (i, slot) in brushes.iter_mut().enumerate() {
                    let (nw, nh) = (sw.div_ceil(2).max(1), sh.div_ceil(2).max(1));""","""                for (i, slot) in brushes.iter_mut().enumerate() {
                    if !deep && i >= 2 {
                        break; // the eighth-size step is only used by the bottom strip
                    }
                    let (nw, nh) = (sw.div_ceil(2).max(1), sh.div_ceil(2).max(1));""")
open(p,'w').write(s)
PYEOF
echo "Patched v220"
