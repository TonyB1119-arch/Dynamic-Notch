//! Bluetooth connect / disconnect peeks with battery level ("AirPods Pro ·
//! Connected · 80%").
//!
//! A DeviceWatcher over paired Bluetooth and Bluetooth LE association
//! endpoints reports `IsConnected` changes (event driven, no polling). The
//! battery comes from the device's PnP property Windows fills in for hands-free
//! headsets and HID devices (the value Settings shows), with the GATT Battery
//! Service as a fallback for LE devices. Headsets often report it a few
//! seconds after connecting, so it is retried briefly, and refreshed every
//! few minutes while something is connected for the low-battery warning.
//!
//! The page lists nearby and paired devices. Windows only lets apps *pair*
//! (and un-pair) devices; connecting an already paired one is a best-effort
//! request, so the page offers the Windows Bluetooth settings when it fails.
//! The scan runs when the page is opened (and on the refresh button).

use std::any::Any;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::core::{Interface, GUID, HSTRING};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids,
};
use windows::Devices::Bluetooth::{
    BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
};
use windows::Devices::Enumeration::{
    DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DevicePairingResultStatus,
    DeviceUnpairingResultStatus, DeviceWatcher,
};
use windows::Devices::Radios::RadioKind;
use windows::Foundation::{IReference, TypedEventHandler};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows_collections::{IIterable, IMapView};
use windows_core::IInspectable;

use super::widgets::{scroll, switch};
use super::{CardSize, Cx, Module, ModuleId, Peek, Trailing};
use crate::bus::Bus;
use crate::gfx::{Icon, TextStyle};
use crate::sys::{radio, shell_open};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect};

const ID: ModuleId = "bluetooth";
const IS_CONNECTED: &str = "System.Devices.Aep.IsConnected";
const CONTAINER: &str = "System.Devices.Aep.ContainerId";
/// DEVPKEY_Bluetooth_Battery: percentage on the device's PnP nodes.
const BATTERY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";
const SIGNAL: &str = "System.Devices.Aep.SignalStrength";
const ROW_H: f32 = 34.0;
const HEADER_H: f32 = 30.0;
const LIST_H: f32 = 5.0 * ROW_H;
const RESCAN_AFTER: Duration = Duration::from_secs(45);
/// Battery arrives late on many headsets: retry after these delays (ms).
const BATTERY_RETRIES: [u64; 4] = [0, 1500, 3500, 7000];
const BATTERY_REFRESH: Duration = Duration::from_secs(5 * 60);
/// A battery reading this soon after connecting updates the connect peek.
const PEEK_WINDOW: Duration = Duration::from_millis(3500);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Audio,
    Mouse,
    Keyboard,
    Gamepad,
    Phone,
    Other,
}

impl Kind {
    fn icon(self) -> Icon {
        match self {
            Kind::Audio => Icon::Headphones,
            Kind::Mouse => Icon::Mouse,
            Kind::Keyboard => Icon::Keyboard,
            Kind::Gamepad => Icon::Gamepad,
            Kind::Phone => Icon::Phone,
            Kind::Other => Icon::Bluetooth,
        }
    }
}

enum BtMsg {
    Connected {
        id: String,
        name: String,
        kind: Kind,
    },
    /// Already connected when we started: tracked, but no peek.
    Present {
        id: String,
        name: String,
        kind: Kind,
    },
    Disconnected {
        id: String,
    },
    Battery {
        id: String,
        level: u8,
    },
    /// Result of a page scan.
    Found(Vec<Found>),
    ScanFailed(String),
    /// Result of a pair / connect / remove request.
    ActionDone {
        ok: bool,
        text: String,
        /// Offer the Windows Bluetooth settings as a fallback.
        settings_hint: bool,
    },
    Radio(Option<bool>),
}

#[derive(Clone, Debug)]
struct Found {
    id: String,
    name: String,
    kind: Kind,
    paired: bool,
    connected: bool,
    rssi: Option<i32>,
}

/// Requests from the page to its worker thread.
enum Ctl {
    Scan,
    Pair(String),
    Connect(String),
    Remove(String),
    Radio(bool),
    Quit,
}

struct Device {
    name: String,
    kind: Kind,
    connected_at: Instant,
    /// A battery reading before this instant updates the connect peek.
    peek_until: Option<Instant>,
    battery: Option<u8>,
    warned: bool,
}

pub struct Bluetooth {
    tx: Option<Sender<Ev>>,
    devices: HashMap<String, Device>,
    ctl: Option<Sender<Ctl>>,
    found: Vec<Found>,
    scanning: bool,
    last_scan: Option<Instant>,
    /// Device id with a request in flight.
    busy: Option<String>,
    status: Option<(String, Instant, bool)>,
    settings_hint: bool,
    radio_on: Option<bool>,
    problem: Option<String>,
    scroll: f32,
}

impl Bluetooth {
    pub fn new() -> Self {
        Self {
            tx: None,
            devices: HashMap::new(),
            ctl: None,
            found: Vec::new(),
            scanning: false,
            last_scan: None,
            busy: None,
            status: None,
            settings_hint: false,
            radio_on: None,
            problem: None,
            scroll: 0.0,
        }
    }

    fn send(&self, c: Ctl) {
        if let Some(tx) = &self.ctl {
            let _ = tx.send(c);
        }
    }

    fn scan(&mut self) {
        if !self.scanning {
            self.scanning = true;
            self.last_scan = Some(Instant::now());
            self.send(Ctl::Scan);
        }
    }

    fn battery_of(&self, name: &str) -> Option<u8> {
        self.devices.values().find(|d| d.name == name).and_then(|d| d.battery)
    }
}

fn connect_peek(d: &Device) -> Peek {
    let sub = match d.battery {
        Some(b) => format!("Connected · {b}%"),
        None => "Connected".to_string(),
    };
    let p = Peek::new(d.kind.icon(), palette::BLUE, &d.name, sub).key("bluetooth").duration_ms(3500);
    match d.battery {
        Some(b) => p.trailing(Trailing::Battery { level: b as f32 / 100.0, charging: false }),
        None => p,
    }
}

impl Module for Bluetooth {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Bluetooth"
    }
    fn icon(&self) -> Icon {
        Icon::Bluetooth
    }

    fn start(&mut self, cx: &mut Cx) {
        let (tx, rx) = channel();
        let bus = cx.bus.clone();
        let tx2 = tx.clone();
        let _ = std::thread::Builder::new().name("bluetooth".into()).spawn(move || worker(rx, tx2, bus));
        self.tx = Some(tx);
        let (ctx, crx) = channel();
        let bus = cx.bus.clone();
        let _ = std::thread::Builder::new().name("bluetooth-page".into()).spawn(move || page_worker(crx, bus));
        self.ctl = Some(ctx);
    }

    fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Ev::Quit);
        }
        if let Some(c) = self.ctl.take() {
            let _ = c.send(Ctl::Quit);
        }
        self.devices.clear();
        self.found.clear();
    }

    fn card(&self) -> Option<CardSize> {
        Some(CardSize::Small)
    }

    fn draw_card(&mut self, ui: &mut Ui, r: Rect) {
        if ui.card(r, true) {
            ui.fx.open_page = Some(ID);
        }
        let inner = r.inset_xy(12.0, 0.0);
        let mut names: Vec<&Device> = self.devices.values().collect();
        names.sort_by(|a, b| a.name.cmp(&b.name));
        let (icon, text, col) = match (names.first(), self.radio_on) {
            (Some(d), _) => {
                let extra = if names.len() > 1 { format!(" +{}", names.len() - 1) } else { String::new() };
                let batt = d.battery.map(|b| format!(" · {b}%")).unwrap_or_default();
                (d.kind.icon(), format!("{}{extra}{batt}", d.name), palette::TEXT)
            }
            (None, Some(false)) => (Icon::Bluetooth, "Bluetooth is off".to_string(), palette::TEXT_DIM),
            (None, _) => (Icon::Bluetooth, "No devices connected".to_string(), palette::TEXT_DIM),
        };
        ui.p.icon(icon, Rect::new(inner.x, r.cy() - 9.0, 18.0, 18.0), if names.is_empty() { palette::TEXT_FAINT } else { palette::BLUE });
        ui.p.text(
            &text,
            Rect::new(inner.x + 26.0, r.y, inner.w - 26.0, r.h),
            &TextStyle::new(12.0, col).medium(),
        );
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        HEADER_H + 8.0 + LIST_H
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        if self.last_scan.is_none_or(|t| t.elapsed() > RESCAN_AFTER) && !self.scanning && self.radio_on != Some(false)
        {
            self.scan();
        }
        let accent = palette::BLUE;

        // --- header ---------------------------------------------------------
        let head = Rect::new(r.x, r.y, r.w, HEADER_H);
        let sw = Rect::new(head.right() - 44.0, head.y, 44.0, head.h);
        let rb = Rect::new(sw.x - 32.0, head.y + 3.0, 24.0, 24.0);
        let connected = self.devices.len();
        let live_status = self.status.clone().filter(|(_, at, _)| at.elapsed() < Duration::from_secs(8));
        let hint_w = if self.settings_hint && live_status.is_some() { 136.0 } else { 0.0 };
        let (head_text, head_col) = match &live_status {
            Some((t, _, good)) => (t.clone(), if *good { palette::GREEN } else { palette::ORANGE }),
            None => (
                match (self.radio_on, connected) {
                    (Some(false), _) => "Bluetooth is off".to_string(),
                    (_, 0) => "No devices connected".to_string(),
                    (_, n) => format!("{n} connected"),
                },
                palette::TEXT,
            ),
        };
        ui.p.icon(Icon::Bluetooth, Rect::new(head.x + 2.0, head.cy() - 9.0, 18.0, 18.0), palette::TEXT_DIM);
        ui.p.text(
            &head_text,
            Rect::new(head.x + 28.0, head.y, (rb.x - head.x - 36.0 - hint_w).max(0.0), head.h),
            &TextStyle::new(13.0, head_col).medium(),
        );
        if hint_w > 0.0 {
            let b = Rect::new(rb.x - hint_w - 6.0, head.y + 3.0, hint_w, 24.0);
            if ui.pill_button(b, "Windows settings", None, ButtonStyle::filled(Color::white(0.1)).fg(palette::TEXT_DIM)) {
                shell_open("ms-settings:bluetooth");
            }
        }
        if live_status.is_none() && self.status.is_some() {
            self.status = None;
            self.settings_hint = false;
        }
        if self.scanning {
            ui.animate();
        }
        if ui.icon_button(rb, Icon::Refresh, ButtonStyle::default().scale(0.6).fg(palette::TEXT_DIM)) {
            self.scan();
        }
        if let Some(on) = self.radio_on {
            if switch(ui, sw, on, accent) {
                self.radio_on = Some(!on);
                self.send(Ctl::Radio(!on));
            }
        }

        // --- list -------------------------------------------------------------
        let area = Rect::new(r.x, head.bottom() + 8.0, r.w, LIST_H);
        if let Some(p) = self.problem.clone() {
            ui.empty_state(area, Icon::Bluetooth, &p);
        } else if self.radio_on == Some(false) {
            ui.empty_state(area, Icon::Bluetooth, "Turn Bluetooth on to find devices");
        } else if self.found.is_empty() {
            if self.scanning {
                ui.progress_indeterminate(Rect::new(area.cx() - 60.0, area.cy(), 120.0, 3.0), accent);
            } else {
                ui.empty_state(area, Icon::Bluetooth, "No devices found");
            }
        } else {
            let content_h = self.found.len() as f32 * ROW_H;
            scroll(ui, area, &mut self.scroll, content_h);
            ui.p.push_clip(area);
            let mut action: Option<Ctl> = None;
            for (i, f) in self.found.iter().enumerate() {
                let row = Rect::new(area.x, area.y + i as f32 * ROW_H - self.scroll, area.w, ROW_H - 4.0);
                if row.bottom() < area.y || row.y > area.bottom() {
                    continue;
                }
                let live = f.connected || self.devices.values().any(|d| d.name == f.name);
                ui.p.fill_rounded(row, 12.0, if live { accent.with_a(0.16) } else { palette::CARD });
                ui.p.icon(f.kind.icon(), Rect::new(row.x + 10.0, row.cy() - 9.0, 18.0, 18.0), if live { accent } else { palette::TEXT_DIM });
                let btn = Rect::new(row.right() - 100.0, row.cy() - 12.0, 90.0, 24.0);
                let trash = Rect::new(btn.x - 28.0, row.cy() - 12.0, 24.0, 24.0);
                let text_w = (if f.paired { trash.x } else { btn.x }) - row.x - 46.0;
                ui.p.text(
                    &f.name,
                    Rect::new(row.x + 38.0, row.y + 2.0, text_w, 16.0),
                    &TextStyle::new(13.0, palette::TEXT).medium(),
                );
                let sub = if live {
                    match self.battery_of(&f.name) {
                        Some(b) => format!("Connected · {b}%"),
                        None => "Connected".to_string(),
                    }
                } else if f.paired {
                    "Paired".to_string()
                } else {
                    "Available".to_string()
                };
                ui.p.text(
                    &sub,
                    Rect::new(row.x + 38.0, row.y + 17.0, text_w, 12.0),
                    &TextStyle::new(10.5, if live { palette::GREEN } else { palette::TEXT_FAINT }),
                );
                let busy_here = self.busy.as_deref() == Some(f.id.as_str());
                if busy_here {
                    ui.progress_indeterminate(Rect::new(btn.x + 10.0, btn.cy() - 1.5, btn.w - 20.0, 3.0), accent);
                } else if live {
                    ui.p.text(
                        "Connected",
                        btn,
                        &TextStyle::new(11.5, palette::GREEN).medium().center(),
                    );
                } else if f.paired {
                    if ui.pill_button(btn, "Connect", None, ButtonStyle::filled(accent.with_a(0.8)).fg(Color::white(1.0))) {
                        action = Some(Ctl::Connect(f.id.clone()));
                    }
                } else if ui.pill_button(btn, "Pair", None, ButtonStyle::filled(accent.with_a(0.8)).fg(Color::white(1.0))) {
                    action = Some(Ctl::Pair(f.id.clone()));
                }
                if f.paired && !busy_here && ui.icon_button(trash, Icon::Trash, ButtonStyle::default().scale(0.55).fg(palette::TEXT_FAINT)) {
                    action = Some(Ctl::Remove(f.id.clone()));
                }
            }
            ui.p.pop_clip();
            if let Some(a) = action {
                self.busy = match &a {
                    Ctl::Pair(id) | Ctl::Connect(id) | Ctl::Remove(id) => Some(id.clone()),
                    _ => None,
                };
                self.settings_hint = false;
                self.send(a);
            }
        }
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let msg = match msg.downcast::<BtMsg>() {
            Ok(m) => match *m {
                BtMsg::Found(list) => {
                    self.scanning = false;
                    self.problem = None;
                    self.found = list;
                    cx.fx.redraw = true;
                    return;
                }
                BtMsg::ScanFailed(e) => {
                    self.scanning = false;
                    self.problem = Some(e);
                    cx.fx.redraw = true;
                    return;
                }
                BtMsg::ActionDone { ok, text, settings_hint } => {
                    self.busy = None;
                    self.settings_hint = settings_hint;
                    self.status = Some((text, Instant::now(), ok));
                    // refresh the list so the new state shows
                    self.scanning = false;
                    self.scan();
                    cx.fx.redraw = true;
                    return;
                }
                BtMsg::Radio(r) => {
                    self.radio_on = r;
                    cx.fx.redraw = true;
                    return;
                }
                other => Box::new(other) as Box<dyn Any + Send>,
            },
            Err(m) => m,
        };
        self.on_device_message(msg, cx);
    }
}

impl Bluetooth {
    /// Watcher events (connect, disconnect, battery).
    fn on_device_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let Ok(m) = msg.downcast::<BtMsg>() else { return };
        let cfg = &cx.cfg.bluetooth;
        let now = Instant::now();
        match *m {
            BtMsg::Connected { id, name, kind } => {
                // dual-mode devices connect as Bluetooth and Bluetooth LE at once
                let twin =
                    self.devices.values().any(|d| d.name == name && now - d.connected_at < Duration::from_secs(10));
                let show = cfg.peek_on_connect && !twin;
                let d = Device {
                    name,
                    kind,
                    connected_at: now,
                    peek_until: show.then(|| now + PEEK_WINDOW),
                    battery: None,
                    warned: false,
                };
                if show {
                    cx.fx.peek(connect_peek(&d));
                }
                self.devices.insert(id, d);
            }
            BtMsg::Present { id, name, kind } => {
                let d = Device { name, kind, connected_at: now, peek_until: None, battery: None, warned: false };
                self.devices.insert(id, d);
            }
            BtMsg::Disconnected { id } => {
                if let Some(d) = self.devices.remove(&id) {
                    let twin = self.devices.values().any(|o| o.name == d.name);
                    if cfg.peek_on_disconnect && !twin {
                        cx.fx.peek(
                            Peek::new(d.kind.icon(), palette::TEXT_DIM, &d.name, "Disconnected")
                                .key("bluetooth")
                                .duration_ms(2200),
                        );
                    }
                }
            }
            BtMsg::Battery { id, level } => {
                let Some(d) = self.devices.get_mut(&id) else { return };
                let first = d.battery.is_none();
                d.battery = Some(level);
                let low = cfg.low_battery > 0 && level <= cfg.low_battery;
                if first && d.peek_until.is_some_and(|t| now < t) {
                    cx.fx.peek(connect_peek(d));
                    // the connect peek already shows the (red) level
                    d.warned |= low;
                } else if low && !d.warned {
                    d.warned = true;
                    cx.fx.peek(
                        Peek::new(
                            d.kind.icon(),
                            palette::RED,
                            format!("{} battery low", d.name),
                            format!("{level}% left"),
                        )
                        .trailing(Trailing::Battery { level: level as f32 / 100.0, charging: false })
                        .key("bluetooth-low")
                        .duration_ms(5000),
                    );
                }
                if level > cfg.low_battery.saturating_add(5) {
                    d.warned = false;
                }
            }
            // page results are handled in `on_message`
            BtMsg::Found(_) | BtMsg::ScanFailed(_) | BtMsg::ActionDone { .. } | BtMsg::Radio(_) => {}
        }
        cx.fx.redraw = true;
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

enum Ev {
    Added { id: String, name: String, connected: bool, container: Option<GUID> },
    Updated { id: String, connected: Option<bool> },
    Removed(String),
    Ready,
    Quit,
}

struct Endpoint {
    name: String,
    connected: bool,
    container: Option<GUID>,
}

fn worker(rx: Receiver<Ev>, tx: Sender<Ev>, bus: Bus) {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let watcher = match watch(tx) {
        Ok(w) => w,
        Err(e) => {
            crate::log!("bluetooth: device watcher unavailable: {e}");
            return;
        }
    };
    let mut eps: HashMap<String, Endpoint> = HashMap::new();
    // events before EnumerationCompleted are the initial state, not changes
    let mut ready = false;
    loop {
        let any_connected = eps.values().any(|e| e.connected);
        let ev = if any_connected {
            match rx.recv_timeout(BATTERY_REFRESH) {
                Ok(ev) => Some(ev),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(ev) => Some(ev),
                Err(_) => break,
            }
        };
        match ev {
            None => {
                for (id, e) in eps.iter().filter(|(_, e)| e.connected) {
                    read_battery(id.clone(), e.container, &[0], bus.clone());
                }
            }
            Some(Ev::Quit) => break,
            Some(Ev::Ready) => {
                ready = true;
                // track what is already connected (battery for the low warning)
                for (id, e) in eps.iter().filter(|(_, e)| e.connected) {
                    let kind = device_kind(id, &e.name);
                    bus.to_module(ID, BtMsg::Present { id: id.clone(), name: e.name.clone(), kind });
                    read_battery(id.clone(), e.container, &[0], bus.clone());
                }
            }
            Some(Ev::Added { id, name, connected, container }) => {
                let ep = Endpoint { name, connected: false, container };
                eps.insert(id.clone(), ep);
                if connected {
                    set_connected(&mut eps, &id, true, ready, &bus);
                }
            }
            Some(Ev::Updated { id, connected: Some(c) }) => set_connected(&mut eps, &id, c, ready, &bus),
            Some(Ev::Updated { .. }) => {}
            Some(Ev::Removed(id)) => {
                if eps.get(&id).is_some_and(|e| e.connected) {
                    set_connected(&mut eps, &id, false, ready, &bus);
                }
                eps.remove(&id);
            }
        }
    }
    let _ = watcher.Stop();
}

fn set_connected(eps: &mut HashMap<String, Endpoint>, id: &str, connected: bool, ready: bool, bus: &Bus) {
    let Some(e) = eps.get_mut(id) else { return };
    if e.connected == connected {
        return;
    }
    e.connected = connected;
    if !ready {
        return;
    }
    if connected {
        let kind = device_kind(id, &e.name);
        bus.to_module(ID, BtMsg::Connected { id: id.to_string(), name: e.name.clone(), kind });
        read_battery(id.to_string(), e.container, &BATTERY_RETRIES, bus.clone());
    } else {
        bus.to_module(ID, BtMsg::Disconnected { id: id.to_string() });
    }
}

fn watch(tx: Sender<Ev>) -> windows::core::Result<DeviceWatcher> {
    let aqs = format!(
        "({}) OR ({})",
        BluetoothDevice::GetDeviceSelectorFromPairingState(true)?,
        BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?
    );
    let props: IIterable<HSTRING> = vec![HSTRING::from(IS_CONNECTED), HSTRING::from(CONTAINER)].into();
    let w = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(aqs),
        &props,
        DeviceInformationKind::AssociationEndpoint,
    )?;
    let t = tx.clone();
    w.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, info| {
        if let Ok(info) = info.ok() {
            let p = info.Properties()?;
            let _ = t.send(Ev::Added {
                id: info.Id()?.to_string(),
                name: info.Name()?.to_string(),
                connected: prop::<bool>(&p, IS_CONNECTED).unwrap_or(false),
                container: prop::<GUID>(&p, CONTAINER),
            });
        }
        Ok(())
    }))?;
    let t = tx.clone();
    w.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, u| {
        if let Ok(u) = u.ok() {
            let connected = prop::<bool>(&u.Properties()?, IS_CONNECTED);
            let _ = t.send(Ev::Updated { id: u.Id()?.to_string(), connected });
        }
        Ok(())
    }))?;
    let t = tx.clone();
    w.Removed(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, u| {
        if let Ok(u) = u.ok() {
            let _ = t.send(Ev::Removed(u.Id()?.to_string()));
        }
        Ok(())
    }))?;
    w.EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(move |_, _| {
        let _ = tx.send(Ev::Ready);
        Ok(())
    }))?;
    w.Start()?;
    Ok(w)
}

fn prop<T>(map: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<T>
where
    T: windows::core::RuntimeType + 'static,
{
    map.Lookup(&HSTRING::from(key)).ok()?.cast::<IReference<T>>().ok()?.Value().ok()
}

fn device_kind(id: &str, name: &str) -> Kind {
    let hid = HSTRING::from(id);
    let by_class = if id.starts_with("BluetoothLE#") {
        BluetoothLEDevice::FromIdAsync(&hid).and_then(|op| op.join()).ok().and_then(|d| {
            let a = d.Appearance().ok()?;
            let k = match (a.Category().ok()?, a.SubCategory().ok()?) {
                (0x0F, 1) => Kind::Keyboard,
                (0x0F, 2) => Kind::Mouse,
                (0x0F, 3 | 4) => Kind::Gamepad,
                (0x01, _) => Kind::Phone,
                _ => Kind::Other,
            };
            let _ = d.Close();
            Some(k)
        })
    } else {
        BluetoothDevice::FromIdAsync(&hid).and_then(|op| op.join()).ok().and_then(|d| {
            let k = class_kind(d.ClassOfDevice().ok()?.RawValue().ok()?);
            let _ = d.Close();
            Some(k)
        })
    };
    match by_class {
        Some(k) if k != Kind::Other => k,
        _ => name_kind(name),
    }
}

/// Bluetooth class of device: major class in bits 8-12, minor in 2-7.
fn class_kind(cod: u32) -> Kind {
    let minor = (cod >> 2) & 0x3F;
    match (cod >> 8) & 0x1F {
        2 => Kind::Phone,
        4 => Kind::Audio,
        5 if minor & 0x10 != 0 => Kind::Keyboard,
        5 if minor & 0x20 != 0 => Kind::Mouse,
        5 if matches!(minor & 0x0F, 1 | 2) => Kind::Gamepad,
        _ => Kind::Other,
    }
}

fn name_kind(name: &str) -> Kind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["airpods", "buds", "headphone", "headset", "earphone", "beats", "wh-", "wf-", "speaker", "soundcore"]) {
        Kind::Audio
    } else if has(&["mouse", "trackpad", "mx master", "mx anywhere"]) {
        Kind::Mouse
    } else if has(&["keyboard", "keys"]) {
        Kind::Keyboard
    } else if has(&["controller", "gamepad", "xbox", "dualsense", "dualshock", "joy-con"]) {
        Kind::Gamepad
    } else if has(&["iphone", "phone", "galaxy", "pixel"]) {
        Kind::Phone
    } else {
        Kind::Other
    }
}

/// Read the battery on a short-lived thread (GATT calls can take seconds),
/// retrying after each delay until a value shows up.
fn read_battery(id: String, container: Option<GUID>, delays: &'static [u64], bus: Bus) {
    let _ = std::thread::Builder::new().name("bluetooth-battery".into()).spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let mut waited = 0;
        for &at in delays {
            std::thread::sleep(Duration::from_millis(at - waited));
            waited = at;
            let level = container
                .and_then(pnp_battery)
                .or_else(|| id.starts_with("BluetoothLE#").then(|| gatt_battery(&id)).flatten());
            if let Some(level) = level {
                bus.to_module(ID, BtMsg::Battery { id, level: level.min(100) });
                return;
            }
        }
    });
}

/// The battery percentage Windows keeps on the device's PnP nodes.
fn pnp_battery(container: GUID) -> Option<u8> {
    let aqs = HSTRING::from(format!("System.Devices.ContainerId:=\"{{{container:?}}}\""));
    let props: IIterable<HSTRING> = vec![HSTRING::from(BATTERY)].into();
    let list = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &aqs,
        &props,
        DeviceInformationKind::Device,
    )
    .ok()?
    .join()
    .ok()?;
    (0..list.Size().ok()?).find_map(|i| prop::<u8>(&list.GetAt(i).ok()?.Properties().ok()?, BATTERY))
}

/// GATT Battery Service (0x180F) / Battery Level (0x2A19).
fn gatt_battery(id: &str) -> Option<u8> {
    let dev = BluetoothLEDevice::FromIdAsync(&HSTRING::from(id)).ok()?.join().ok()?;
    let read = || -> Option<u8> {
        let services = dev
            .GetGattServicesForUuidWithCacheModeAsync(GattServiceUuids::Battery().ok()?, BluetoothCacheMode::Cached)
            .ok()?
            .join()
            .ok()?;
        if services.Status().ok()? != GattCommunicationStatus::Success {
            return None;
        }
        let service = services.Services().ok()?.GetAt(0).ok()?;
        let level = (|| {
            let chars = service
                .GetCharacteristicsForUuidWithCacheModeAsync(
                    GattCharacteristicUuids::BatteryLevel().ok()?,
                    BluetoothCacheMode::Cached,
                )
                .ok()?
                .join()
                .ok()?;
            let ch = chars.Characteristics().ok()?.GetAt(0).ok()?;
            let r = ch.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached).ok()?.join().ok()?;
            if r.Status().ok()? != GattCommunicationStatus::Success {
                return None;
            }
            DataReader::FromBuffer(&r.Value().ok()?).ok()?.ReadByte().ok()
        })();
        let _ = service.Close();
        level
    };
    let level = read();
    let _ = dev.Close();
    level
}

// ---------------------------------------------------------------------------
// Page worker: scan, pair, connect, remove, radio
// ---------------------------------------------------------------------------

fn es(e: windows::core::Error) -> String {
    e.to_string()
}

fn page_worker(rx: Receiver<Ctl>, bus: Bus) {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    bus.to_module(ID, BtMsg::Radio(radio::is_on(RadioKind::Bluetooth)));
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Ctl::Quit => return,
            Ctl::Scan => match discover() {
                Ok(list) => bus.to_module(ID, BtMsg::Found(list)),
                Err(e) => bus.to_module(ID, BtMsg::ScanFailed(e)),
            },
            Ctl::Pair(id) => {
                let (ok, text) = pair(&id);
                bus.to_module(ID, BtMsg::ActionDone { ok, text, settings_hint: false });
            }
            Ctl::Connect(id) => {
                let (ok, text) = connect_paired(&id);
                bus.to_module(ID, BtMsg::ActionDone { ok, text, settings_hint: !ok });
            }
            Ctl::Remove(id) => {
                let (ok, text) = remove(&id);
                bus.to_module(ID, BtMsg::ActionDone { ok, text, settings_hint: false });
            }
            Ctl::Radio(on) => {
                radio::set_on(RadioKind::Bluetooth, on);
                bus.to_module(ID, BtMsg::Radio(radio::is_on(RadioKind::Bluetooth)));
                if on {
                    std::thread::sleep(Duration::from_millis(2000));
                    match discover() {
                        Ok(list) => bus.to_module(ID, BtMsg::Found(list)),
                        Err(e) => bus.to_module(ID, BtMsg::ScanFailed(e)),
                    }
                }
            }
        }
    }
}

/// Paired and nearby devices (classic and LE), one entry per name.
fn discover() -> Result<Vec<Found>, String> {
    let sel = format!(
        "({}) OR ({}) OR ({}) OR ({})",
        BluetoothDevice::GetDeviceSelectorFromPairingState(true).map_err(es)?,
        BluetoothDevice::GetDeviceSelectorFromPairingState(false).map_err(es)?,
        BluetoothLEDevice::GetDeviceSelectorFromPairingState(true).map_err(es)?,
        BluetoothLEDevice::GetDeviceSelectorFromPairingState(false).map_err(es)?,
    );
    let props: IIterable<HSTRING> = vec![HSTRING::from(IS_CONNECTED), HSTRING::from(SIGNAL)].into();
    let list = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(sel),
        &props,
        DeviceInformationKind::AssociationEndpoint,
    )
    .map_err(es)?
    .join()
    .map_err(es)?;
    let mut out: Vec<Found> = Vec::new();
    for i in 0..list.Size().map_err(es)? {
        let Ok(info) = list.GetAt(i) else { continue };
        let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
        // LE advertisements without a name are noise
        if name.trim().is_empty() {
            continue;
        }
        let id = info.Id().map(|n| n.to_string()).unwrap_or_default();
        let paired = info.Pairing().and_then(|p| p.IsPaired()).unwrap_or(false);
        let p = info.Properties().ok();
        let connected = p.as_ref().and_then(|p| prop::<bool>(p, IS_CONNECTED)).unwrap_or(false);
        let rssi = p.as_ref().and_then(|p| prop::<i32>(p, SIGNAL));
        let f = Found { id, kind: name_kind(&name), name, paired, connected, rssi };
        let rank = |f: &Found| (f.connected as u8) * 2 + f.paired as u8;
        match out.iter_mut().find(|o| o.name == f.name) {
            Some(o) if rank(&f) > rank(o) => *o = f,
            Some(_) => {}
            None => out.push(f),
        }
    }
    out.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(b.rssi.unwrap_or(-200).cmp(&a.rssi.unwrap_or(-200)))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

fn pair(id: &str) -> (bool, String) {
    let res = (|| -> Result<(bool, String), String> {
        let info = DeviceInformation::CreateFromIdAsync(&HSTRING::from(id)).map_err(es)?.join().map_err(es)?;
        let r = info.Pairing().map_err(es)?.PairAsync().map_err(es)?.join().map_err(es)?;
        let st = r.Status().map_err(es)?;
        Ok(if st == DevicePairingResultStatus::Paired || st == DevicePairingResultStatus::AlreadyPaired {
            (true, "Paired. Tap Connect to use it.".to_string())
        } else if st == DevicePairingResultStatus::AuthenticationFailure {
            (false, "Pairing failed: wrong code".to_string())
        } else if st == DevicePairingResultStatus::AuthenticationTimeout {
            (false, "Pairing timed out".to_string())
        } else if st == DevicePairingResultStatus::ConnectionRejected {
            (false, "The device rejected pairing".to_string())
        } else if st == DevicePairingResultStatus::PairingCanceled {
            (false, "Pairing cancelled".to_string())
        } else {
            (false, "Couldn't pair. Is the device in pairing mode?".to_string())
        })
    })();
    res.unwrap_or_else(|e| (false, e))
}

/// Best effort: ask Windows for the device's services, which makes it open a
/// link, then wait a few seconds for the connection to show up.
fn connect_paired(id: &str) -> (bool, String) {
    let hid = HSTRING::from(id);
    let res = (|| -> Result<bool, String> {
        let status = |connected: bool| connected;
        if id.starts_with("BluetoothLE#") {
            let d = BluetoothLEDevice::FromIdAsync(&hid).map_err(es)?.join().map_err(es)?;
            let _ = d.GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached).and_then(|op| op.join());
            let mut up = false;
            for _ in 0..8 {
                up = d.ConnectionStatus().is_ok_and(|s| s == BluetoothConnectionStatus::Connected);
                if up {
                    break;
                }
                std::thread::sleep(Duration::from_millis(700));
            }
            let _ = d.Close();
            Ok(status(up))
        } else {
            let d = BluetoothDevice::FromIdAsync(&hid).map_err(es)?.join().map_err(es)?;
            let _ = d.GetRfcommServicesWithCacheModeAsync(BluetoothCacheMode::Uncached).and_then(|op| op.join());
            let mut up = false;
            for _ in 0..8 {
                up = d.ConnectionStatus().is_ok_and(|s| s == BluetoothConnectionStatus::Connected);
                if up {
                    break;
                }
                std::thread::sleep(Duration::from_millis(700));
            }
            let _ = d.Close();
            Ok(status(up))
        }
    })();
    match res {
        Ok(true) => (true, "Connected".to_string()),
        Ok(false) => (false, "Windows didn't connect it from here".to_string()),
        Err(e) => (false, e),
    }
}

fn remove(id: &str) -> (bool, String) {
    let res = (|| -> Result<bool, String> {
        let info = DeviceInformation::CreateFromIdAsync(&HSTRING::from(id)).map_err(es)?.join().map_err(es)?;
        let r = info.Pairing().map_err(es)?.UnpairAsync().map_err(es)?.join().map_err(es)?;
        Ok(r.Status().map_err(es)? == DeviceUnpairingResultStatus::Unpaired)
    })();
    match res {
        Ok(true) => (true, "Device removed".to_string()),
        Ok(false) => (false, "Couldn't remove the device".to_string()),
        Err(e) => (false, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert_eq!(class_kind(0x240404), Kind::Audio); // wearable headset
        assert_eq!(class_kind(0x002540), Kind::Keyboard);
        assert_eq!(class_kind(0x002580), Kind::Mouse);
        assert_eq!(class_kind(0x002508), Kind::Gamepad);
        assert_eq!(class_kind(0x5a020c), Kind::Phone);
        assert_eq!(name_kind("Sony WH-1000XM5"), Kind::Audio);
        assert_eq!(name_kind("Xbox Wireless Controller"), Kind::Gamepad);
        assert_eq!(name_kind("Thing"), Kind::Other);
    }
}
