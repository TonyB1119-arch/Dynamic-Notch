//! Breathe: a guided breathing ring made of ticks that grows while you
//! breathe in and shrinks while you breathe out. It only animates while this
//! page is open and running.

use std::time::Instant;

use crate::gfx::{Icon, TextStyle};
use crate::modules::widgets::segmented;
use crate::modules::{Module, ModuleId};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "breathe";

/// (name, seconds of: breathe in, hold, breathe out, hold)
const PATTERNS: [(&str, [f32; 4]); 3] =
    [("Box 4-4-4-4", [4.0, 4.0, 4.0, 4.0]), ("4-7-8", [4.0, 7.0, 8.0, 0.0]), ("Calm 5-5", [5.0, 0.0, 5.0, 0.0])];
const LABELS: [&str; 4] = ["Breathe in", "Hold", "Breathe out", "Hold"];

pub struct Breathe {
    pattern: usize,
    started: Option<Instant>,
}

impl Breathe {
    pub fn new() -> Self {
        Self { pattern: 0, started: None }
    }
}

/// Where we are in the cycle: (phase index, seconds into it, size 0..1, seconds left in the phase).
fn position(steps: &[f32; 4], t: f32) -> (usize, f32, f32, f32) {
    let cycle: f32 = steps.iter().sum();
    let mut t = t.rem_euclid(cycle.max(0.1));
    for (i, len) in steps.iter().enumerate() {
        if *len <= 0.0 {
            continue;
        }
        if t < *len {
            let k = t / len;
            let ease = k * k * (3.0 - 2.0 * k);
            let size = match i {
                0 => ease,
                1 => 1.0,
                2 => 1.0 - ease,
                _ => 0.0,
            };
            return (i, t, size, len - t);
        }
        t -= len;
    }
    (0, 0.0, 0.0, 0.0)
}

impl Module for Breathe {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Breathe"
    }
    fn icon(&self) -> Icon {
        Icon::Cloud
    }

    fn glance(&self) -> Option<(Icon, String)> {
        Some((Icon::Cloud, "Calm down".into()))
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        196.0
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        let steps = PATTERNS[self.pattern].1;
        let (cx, cy) = (r.x + 110.0, r.cy());
        let (phase, size, left, cycles) = match self.started {
            Some(t0) => {
                ui.animate();
                let el = ui.now.saturating_duration_since(t0).as_secs_f32();
                let (p, _, s, l) = position(&steps, el);
                let cycle: f32 = steps.iter().sum();
                (Some(p), s, l, (el / cycle.max(0.1)) as u32)
            }
            None => (None, 0.5, 0.0, 0),
        };
        // the ring breathes between 70 and 160 px
        let d = 70.0 + 90.0 * size;
        ui.ring(cx, cy, d.round(), 5.0, 1.0, palette::BLUE);
        match phase {
            Some(p) => {
                ui.p.text(
                    LABELS[p],
                    Rect::new(cx - 60.0, cy - 20.0, 120.0, 22.0),
                    &TextStyle::new(15.0, palette::TEXT).semibold().center(),
                );
                ui.p.text(
                    &format!("{:.0}", left.ceil().max(1.0)),
                    Rect::new(cx - 60.0, cy + 2.0, 120.0, 24.0),
                    &TextStyle::new(20.0, palette::TEXT_DIM).display().center(),
                );
            }
            None => ui.p.text(
                "Ready",
                Rect::new(cx - 60.0, cy - 11.0, 120.0, 22.0),
                &TextStyle::new(15.0, palette::TEXT_DIM).center(),
            ),
        }

        let x = r.x + 250.0;
        let w = (r.right() - x).max(200.0);
        ui.caption("PATTERN", Rect::new(x, r.y + 14.0, w, 16.0));
        let names: Vec<&str> = PATTERNS.iter().map(|p| p.0).collect();
        if let Some(i) =
            segmented(ui, Rect::new(x, r.y + 34.0, w.min(380.0), 32.0), &names, Some(self.pattern), palette::BLUE)
        {
            self.pattern = i;
            if self.started.is_some() {
                self.started = Some(ui.now);
            }
        }
        let main = Rect::new(x, r.y + 84.0, 130.0, 40.0);
        if self.started.is_some() {
            if ui.pill_button(main, "Stop", Some(Icon::Stop), ButtonStyle::filled(Color::white(0.1))) {
                self.started = None;
                ui.fx.redraw = true;
            }
            ui.p.text(
                &format!("{cycles} full breath{}", if cycles == 1 { "" } else { "s" }),
                Rect::new(main.right() + 16.0, main.y, 160.0, main.h),
                &TextStyle::new(13.0, palette::TEXT_DIM),
            );
        } else if ui.pill_button(main, "Start", Some(Icon::Play), ButtonStyle::inverted()) {
            self.started = Some(ui.now);
            ui.fx.redraw = true;
        }
        ui.p.text(
            "Follow the ring: it grows as you breathe in and shrinks as you breathe out.",
            Rect::new(x, r.y + 140.0, w, 36.0),
            &TextStyle::new(11.5, palette::TEXT_FAINT).wrap().top(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_positions() {
        let s = [4.0, 4.0, 4.0, 4.0];
        assert_eq!(position(&s, 0.0).0, 0);
        assert_eq!(position(&s, 5.0).0, 1);
        assert_eq!(position(&s, 9.0).0, 2);
        assert_eq!(position(&s, 13.0).0, 3);
        assert_eq!(position(&s, 16.5).0, 0);
        // holds with zero length are skipped
        assert_eq!(position(&[5.0, 0.0, 5.0, 0.0], 6.0).0, 2);
        assert!(position(&s, 2.0).2 > 0.0 && position(&s, 2.0).2 < 1.0);
    }
}
