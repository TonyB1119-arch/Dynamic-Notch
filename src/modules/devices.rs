//! USB drive peek: a short notice when a drive is plugged in or removed.
//! Purely event driven: the shell forwards Windows' device message.

use super::{Cx, Module, ModuleId, Peek, SystemEvent};
use crate::gfx::Icon;
use crate::util::palette;

const ID: ModuleId = "devices";
pub struct Devices;

impl Devices {
    pub fn new() -> Self {
        Self
    }
}

impl Module for Devices {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Devices"
    }
    fn icon(&self) -> Icon {
        Icon::Package
    }

    fn on_system(&mut self, ev: &SystemEvent, cx: &mut Cx) {
        let SystemEvent::Command { verb, args } = ev else { return };
        match verb.as_str() {
            "usb" => {
                let drive = args.first().cloned().unwrap_or_default();
                let arrived = args.get(1).is_some_and(|a| a == "in");
                let (title, color) =
                    if arrived { ("Drive connected", palette::BLUE) } else { ("Drive removed", palette::TEXT_DIM) };
                cx.fx.peek(Peek::new(Icon::Folder, color, title, drive).key("devices").duration_ms(3000));
            }
            _ => {}
        }
    }
}
