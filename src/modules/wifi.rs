//! Wi-Fi: see the networks around you and connect to them from the notch.
//!
//! Built on `Windows.Devices.WiFi`. Everything blocking (scan, connect) runs
//! on one worker thread. Networks Windows already knows connect without a
//! password; for a new secured network the page asks for it.
//!
//! The list refreshes when the page opens (at most every 30 s) and on the
//! refresh button; nothing scans in the background.

use std::any::Any;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Devices::Radios::RadioKind;
use windows::Devices::WiFi::{
    WiFiAccessStatus, WiFiAdapter, WiFiAvailableNetwork, WiFiConnectionStatus, WiFiReconnectionKind,
};
use windows::Networking::Connectivity::NetworkAuthenticationType;
use windows::Security::Credentials::PasswordCredential;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use super::widgets::{scroll, switch, TextField};
use super::{CardSize, Cx, Key, Module, ModuleId, Peek};
use crate::bus::Bus;
use crate::gfx::{Icon, TextStyle};
use crate::sys::radio;
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, Color, Rect, SendHwnd};

const ID: ModuleId = "control";
const ROW_H: f32 = 40.0;
const HEADER_H: f32 = 34.0;
const LIST_H: f32 = 5.0 * ROW_H;
/// Alcove: three rows of network cards.
const ALC_LIST: f32 = 116.0; // two rows of cards, the rest scrolls
const RESCAN_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
struct Net {
    ssid: String,
    bars: u8,
    secured: bool,
}

enum WMsg {
    List {
        nets: Vec<Net>,
        connected: Option<String>,
    },
    /// No adapter / access denied / scan failed.
    Problem(String),
    Connected(String),
    NeedPassword(String),
    Failed(String, String),
    Disconnected,
    Radio(Option<bool>),
}

enum Cmd {
    Scan,
    Connect { ssid: String, password: Option<String> },
    Disconnect,
    Radio(bool),
    Quit,
}

pub struct Wifi {
    tx: Option<Sender<Cmd>>,
    hwnd: SendHwnd,
    nets: Vec<Net>,
    connected: Option<String>,
    problem: Option<String>,
    scanning: bool,
    last_scan: Option<Instant>,
    /// SSID being connected right now.
    busy: Option<String>,
    status: Option<(String, Instant, bool)>,
    radio_on: Option<bool>,
    /// SSID waiting for a password.
    prompt: Option<String>,
    pw: TextField,
    scroll: f32,
}

impl Wifi {
    /// Is this message one of ours? (The control center routes by type.)
    pub(super) fn accepts(msg: &Box<dyn Any + Send>) -> bool {
        msg.is::<WMsg>()
    }

    pub fn new() -> Self {
        Self {
            tx: None,
            hwnd: SendHwnd(0),
            nets: Vec::new(),
            connected: None,
            problem: None,
            scanning: false,
            last_scan: None,
            busy: None,
            status: None,
            radio_on: None,
            prompt: None,
            pw: TextField::new(128).masked(),
            scroll: 0.0,
        }
    }

    fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }

    fn scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.last_scan = Some(Instant::now());
        self.send(Cmd::Scan);
    }

    fn say(&mut self, text: impl Into<String>, good: bool) {
        self.status = Some((text.into(), Instant::now(), good));
    }
}

impl Module for Wifi {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Wi-Fi"
    }
    fn icon(&self) -> Icon {
        Icon::Wifi
    }

    fn start(&mut self, cx: &mut Cx) {
        self.hwnd = cx.hwnd;
        let (tx, rx) = channel();
        let bus = cx.bus.clone();
        let _ = std::thread::Builder::new().name("wifi".into()).spawn(move || worker(rx, bus));
        self.tx = Some(tx);
        self.scan();
    }

    fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Cmd::Quit);
        }
        self.nets.clear();
        self.connected = None;
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let Ok(m) = msg.downcast::<WMsg>() else { return };
        match *m {
            WMsg::List { nets, connected } => {
                self.scanning = false;
                self.problem = None;
                self.nets = nets;
                self.connected = connected;
            }
            WMsg::Problem(p) => {
                self.scanning = false;
                self.busy = None;
                self.problem = Some(p);
            }
            WMsg::Connected(ssid) => {
                self.busy = None;
                self.prompt = None;
                self.pw.clear();
                cx.fx.keyboard_focus = Some(false);
                cx.fx.peek(Peek::new(Icon::Wifi, palette::GREEN, &ssid, "Connected").key("wifi").page(ID));
                self.say(format!("Connected to {ssid}"), true);
                self.connected = Some(ssid);
            }
            WMsg::NeedPassword(ssid) => {
                self.busy = None;
                self.pw.clear();
                self.prompt = Some(ssid);
                cx.fx.keyboard_focus = Some(true);
            }
            WMsg::Failed(ssid, why) => {
                self.busy = None;
                self.say(format!("{ssid}: {why}"), false);
            }
            WMsg::Disconnected => {
                self.connected = None;
                self.say("Disconnected", true);
            }
            WMsg::Radio(r) => self.radio_on = r,
        }
        cx.fx.redraw = true;
    }

    fn glance(&self) -> Option<(Icon, String)> {
        let text = match (&self.connected, self.radio_on) {
            (_, Some(false)) => "Off".to_string(),
            (Some(s), _) => crate::util::truncate_chars(s, 9),
            _ => "—".to_string(),
        };
        Some((Icon::Wifi, text))
    }

    fn card(&self) -> Option<CardSize> {
        Some(CardSize::Small)
    }

    fn draw_card(&mut self, ui: &mut Ui, r: Rect) {
        if ui.card(r, true) {
            ui.fx.open_page = Some(ID);
        }
        let inner = r.inset_xy(12.0, 0.0);
        let on = self.connected.is_some();
        let icon_col = if on { palette::TEXT } else { palette::TEXT_FAINT };
        ui.p.icon(Icon::Wifi, Rect::new(inner.x, r.cy() - 9.0, 18.0, 18.0), icon_col);
        let text = match (&self.connected, &self.problem, self.radio_on) {
            (Some(s), _, _) => s.clone(),
            (None, Some(_), _) => "Wi-Fi unavailable".to_string(),
            (None, None, Some(false)) => "Wi-Fi is off".to_string(),
            _ => "Not connected".to_string(),
        };
        ui.p.text(
            &text,
            Rect::new(inner.x + 26.0, r.y, inner.w - 26.0 - 24.0, r.h),
            &TextStyle::new(12.0, if on { palette::TEXT } else { palette::TEXT_DIM }).medium(),
        );
        if let Some(n) = self.connected.as_ref().and_then(|s| self.nets.iter().find(|n| &n.ssid == s)) {
            draw_bars(ui, Rect::new(inner.right() - 18.0, r.cy() - 8.0, 18.0, 16.0), n.bars, palette::TEXT);
        }
    }

    fn has_page(&self) -> bool {
        true
    }

    fn wants_keyboard(&self) -> bool {
        self.prompt.is_some()
    }

    fn page_height(&self, _w: f32) -> f32 {
        if crate::ui::look_alcove() {
            return 48.0 + 12.0 + ALC_LIST + if self.prompt.is_some() { 50.0 } else { 0.0 };
        }
        HEADER_H + 12.0 + LIST_H + if self.prompt.is_some() { 50.0 } else { 0.0 }
    }

    fn on_key(&mut self, key: Key, cx: &mut Cx) {
        let Some(ssid) = self.prompt.clone() else { return };
        if self.pw.key(key, self.hwnd) {
            let pw = self.pw.text();
            if !pw.is_empty() {
                self.busy = Some(ssid.clone());
                self.send(Cmd::Connect { ssid, password: Some(pw) });
                self.prompt = None;
                self.pw.clear();
                cx.fx.keyboard_focus = Some(false);
            }
        }
        cx.fx.layout_changed = true;
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        if self.last_scan.is_none_or(|t| t.elapsed() > RESCAN_AFTER) && !self.scanning && self.radio_on != Some(false) {
            self.scan();
        }
        let accent = palette::BLUE;
        if crate::ui::look_alcove() {
            let area = self.draw_alcove(ui, r);
            self.draw_prompt(ui, r, area);
            return;
        }

        // --- header ---------------------------------------------------------
        let head = Rect::new(r.x, r.y, r.w, HEADER_H);
        let sw = Rect::new(head.right() - 44.0, head.y, 44.0, head.h);
        let rb = Rect::new(sw.x - 32.0, head.y + 3.0, 24.0, 24.0);
        let live_status = self.status.clone().filter(|(_, at, _)| at.elapsed() < Duration::from_secs(5));
        let (head_text, head_col) = match &live_status {
            Some((t, _, good)) => (t.clone(), if *good { palette::GREEN } else { palette::ORANGE }),
            None => (
                match (&self.connected, self.radio_on) {
                    (_, Some(false)) => "Wi-Fi is off".to_string(),
                    (Some(s), _) => format!("Connected to {s}"),
                    _ => "Not connected".to_string(),
                },
                palette::TEXT,
            ),
        };
        if live_status.is_none() && self.status.is_some() {
            self.status = None;
        }
        ui.p.icon(Icon::Wifi, Rect::new(head.x + 2.0, head.cy() - 9.0, 18.0, 18.0), palette::TEXT_DIM);
        ui.p.text(
            &head_text,
            Rect::new(head.x + 28.0, head.y, rb.x - head.x - 36.0, head.h),
            &TextStyle::new(13.0, head_col).medium(),
        );
        if self.scanning {
            ui.animate();
        }
        if ui.icon_button(rb, Icon::Refresh, ButtonStyle::default().scale(0.6).fg(palette::TEXT_DIM)) {
            self.scan();
        }
        if let Some(on) = self.radio_on {
            if switch(ui, sw, on, accent) {
                self.radio_on = Some(!on);
                self.send(Cmd::Radio(!on));
            }
        }

        // tick strip under the header: sweeps while scanning, otherwise shows the signal of the network you are on
        let strip = Rect::new(r.x + 2.0, head.bottom() + 2.0, r.w - 4.0, 7.0);
        if self.scanning || self.busy.is_some() {
            ui.ticks_sweep(strip, Color::white(0.9));
        } else {
            let sig = self
                .connected
                .as_ref()
                .and_then(|s| self.nets.iter().find(|n| &n.ssid == s))
                .map_or(0.0, |n| n.bars as f32 / 4.0);
            ui.ticks(strip, sig, Color::white(0.9));
        }

        // --- body -----------------------------------------------------------
        let area = Rect::new(r.x, head.bottom() + 12.0, r.w, LIST_H);
        if let Some(p) = self.problem.clone() {
            ui.empty_state(area, Icon::Wifi, &p);
        } else if self.radio_on == Some(false) {
            ui.empty_state(area, Icon::Wifi, "Turn Wi-Fi on to see networks");
        } else if self.nets.is_empty() {
            if self.scanning {
                ui.progress_indeterminate(Rect::new(area.cx() - 60.0, area.cy(), 120.0, 3.0), accent);
            } else {
                ui.empty_state(area, Icon::Wifi, "No networks found");
            }
        } else {
            let content_h = self.nets.len() as f32 * ROW_H;
            scroll(ui, area, &mut self.scroll, content_h);
            ui.p.push_clip(area);
            let mut action: Option<Cmd> = None;
            for (i, n) in self.nets.iter().enumerate() {
                let row = Rect::new(area.x, area.y + i as f32 * ROW_H - self.scroll, area.w, ROW_H - 6.0);
                if row.bottom() < area.y || row.y > area.bottom() {
                    continue;
                }
                let is_conn = self.connected.as_deref() == Some(n.ssid.as_str());
                let rh = ui.hovered(row);
                ui.row_bg(row, rh, None);
                ui.ticks(Rect::new(row.x + 12.0, row.cy() - 5.0, 40.0, 10.0), n.bars as f32 / 4.0, Color::white(0.9));
                let btn = Rect::new(row.right() - 100.0, row.cy() - 12.0, 90.0, 24.0);
                let name_w = btn.x - row.x - 88.0;
                ui.p.text(
                    &n.ssid,
                    Rect::new(row.x + 60.0, row.y, name_w, row.h),
                    &TextStyle::new(13.0, palette::TEXT).medium(),
                );
                if n.secured {
                    let (tw, _) = ui.p.measure(&n.ssid, &TextStyle::new(13.0, palette::TEXT).medium());
                    ui.p.icon(
                        Icon::Lock,
                        Rect::new(row.x + 60.0 + tw.min(name_w) + 6.0, row.cy() - 6.0, 12.0, 12.0),
                        palette::TEXT_FAINT,
                    );
                }
                let busy_here = self.busy.as_deref() == Some(n.ssid.as_str());
                if busy_here {
                    ui.progress_indeterminate(Rect::new(btn.x + 10.0, btn.cy() - 1.5, btn.w - 20.0, 3.0), accent);
                } else {
                    // no buttons: the whole row is the control, its state is a quiet word on the right
                    let (word, col) = match (is_conn, rh) {
                        (true, true) => ("Disconnect", palette::TEXT_DIM),
                        (true, false) => ("Connected", accent),
                        (false, true) => ("Connect", palette::TEXT_DIM),
                        (false, false) => ("", palette::TEXT_FAINT),
                    };
                    ui.p.text(
                        word,
                        Rect::new(row.right() - 110.0, row.y, 98.0, row.h),
                        &TextStyle::new(12.0, col).medium().right(),
                    );
                    if ui.clicked(row) {
                        action = Some(if is_conn {
                            Cmd::Disconnect
                        } else {
                            Cmd::Connect { ssid: n.ssid.clone(), password: None }
                        });
                    }
                }
            }
            ui.p.pop_clip();
            match action {
                Some(Cmd::Connect { ssid, password }) => {
                    self.busy = Some(ssid.clone());
                    self.prompt = None;
                    self.send(Cmd::Connect { ssid, password });
                }
                Some(c) => self.send(c),
                None => {}
            }
        }

        // --- password prompt / status line -----------------------------------
        if let Some(ssid) = self.prompt.clone() {
            let bar = Rect::new(r.x, area.bottom() + 12.0, r.w, 36.0);
            let cancel = Rect::new(bar.right() - 34.0, bar.y + 2.0, 32.0, 32.0);
            let join = Rect::new(cancel.x - 76.0, bar.y + 4.0, 70.0, 28.0);
            let field = Rect::new(bar.x, bar.y + 2.0, join.x - bar.x - 8.0, 32.0);
            let hint = format!("Password for {ssid}");
            self.pw.draw(ui, field, &hint, accent, true);
            let can = !self.pw.is_empty();
            let style = if can {
                ButtonStyle::inverted()
            } else {
                ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_FAINT)
            };
            if ui.pill_button(join, "Join", None, style) && can {
                let pw = self.pw.text();
                self.busy = Some(ssid.clone());
                self.send(Cmd::Connect { ssid, password: Some(pw) });
                self.prompt = None;
                self.pw.clear();
                ui.fx.keyboard_focus = Some(false);
            }
            if ui.icon_button(cancel, Icon::Close, ButtonStyle::default().scale(0.5).fg(palette::TEXT_DIM)) {
                self.prompt = None;
                self.pw.clear();
                ui.fx.keyboard_focus = Some(false);
            }
        }
    }
}

/// Signal strength as six thin ticks that grow taller, `bars` of four lit.
fn draw_bars(ui: &mut Ui, r: Rect, bars: u8, color: Color) {
    let n = 6usize;
    let w = 2.0;
    let gap = (r.w - w * n as f32) / (n as f32 - 1.0);
    let lit = bars.min(4) as f32 / 4.0 * n as f32;
    for i in 0..n {
        let h = 4.0 + 2.0 * i as f32;
        let x = r.x + i as f32 * (w + gap);
        let c = Color::white(0.18).lerp(color, (lit - i as f32).clamp(0.0, 1.0));
        ui.p.fill_rounded(Rect::new(x, r.cy() + 7.0 - h, w, h), 1.0, c);
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

fn es(e: windows::core::Error) -> String {
    e.to_string()
}

fn open_adapter() -> Result<WiFiAdapter, String> {
    let access = WiFiAdapter::RequestAccessAsync().map_err(es)?.join().map_err(es)?;
    if access != WiFiAccessStatus::Allowed {
        return Err("Windows didn't allow Wi-Fi access (check Settings → Privacy → Location)".into());
    }
    let list = WiFiAdapter::FindAllAdaptersAsync().map_err(es)?.join().map_err(es)?;
    if list.Size().map_err(es)? == 0 {
        return Err("No Wi-Fi adapter found".into());
    }
    list.GetAt(0).map_err(es)
}

fn connected_ssid(ad: &WiFiAdapter) -> Option<String> {
    let profile = ad.NetworkAdapter().ok()?.GetConnectedProfileAsync().ok()?.join().ok()?;
    Some(profile.ProfileName().ok()?.to_string())
}

fn is_secured(n: &WiFiAvailableNetwork) -> bool {
    n.SecuritySettings()
        .and_then(|s| s.NetworkAuthenticationType())
        .map(|t| t != NetworkAuthenticationType::None)
        .unwrap_or(true)
}

fn scan(adapter: &mut Option<WiFiAdapter>, avail: &mut Vec<WiFiAvailableNetwork>, bus: &Bus) {
    let res = (|| -> Result<(Vec<Net>, Option<String>), String> {
        if adapter.is_none() {
            *adapter = Some(open_adapter()?);
        }
        let ad = adapter.as_ref().ok_or("no adapter")?;
        ad.ScanAsync().map_err(es)?.join().map_err(es)?;
        let list = ad.NetworkReport().map_err(es)?.AvailableNetworks().map_err(es)?;
        avail.clear();
        let mut nets: Vec<Net> = Vec::new();
        for i in 0..list.Size().map_err(es)? {
            let Ok(n) = list.GetAt(i) else { continue };
            let ssid = n.Ssid().map(|s| s.to_string()).unwrap_or_default();
            if ssid.is_empty() {
                continue;
            }
            let bars = n.SignalBars().unwrap_or(0);
            let secured = is_secured(&n);
            avail.push(n);
            match nets.iter_mut().find(|x| x.ssid == ssid) {
                Some(x) => x.bars = x.bars.max(bars),
                None => nets.push(Net { ssid, bars, secured }),
            }
        }
        nets.sort_by(|a, b| b.bars.cmp(&a.bars).then_with(|| a.ssid.cmp(&b.ssid)));
        Ok((nets, connected_ssid(ad)))
    })();
    match res {
        Ok((nets, connected)) => bus.to_module(ID, WMsg::List { nets, connected }),
        Err(e) => {
            // forget the adapter so the next scan starts fresh
            *adapter = None;
            bus.to_module(ID, WMsg::Problem(e));
        }
    }
}

fn connect(
    adapter: &Option<WiFiAdapter>,
    avail: &[WiFiAvailableNetwork],
    ssid: &str,
    password: Option<String>,
    bus: &Bus,
) -> bool {
    let Some(ad) = adapter.as_ref() else {
        bus.to_module(ID, WMsg::Failed(ssid.to_string(), "no Wi-Fi adapter".into()));
        return false;
    };
    // strongest access point with this name
    let Some(net) = avail
        .iter()
        .filter(|n| n.Ssid().is_ok_and(|s| s.to_string() == ssid))
        .max_by_key(|n| n.SignalBars().unwrap_or(0))
    else {
        bus.to_module(ID, WMsg::Failed(ssid.to_string(), "network not in range".into()));
        return false;
    };
    let secured = is_secured(net);
    let res = (|| -> Result<WiFiConnectionStatus, String> {
        let op = match &password {
            Some(pw) => {
                let cred = PasswordCredential::new().map_err(es)?;
                cred.SetPassword(&HSTRING::from(pw.as_str())).map_err(es)?;
                ad.ConnectWithPasswordCredentialAsync(net, WiFiReconnectionKind::Automatic, &cred).map_err(es)?
            }
            None => ad.ConnectAsync(net, WiFiReconnectionKind::Automatic).map_err(es)?,
        };
        op.join().map_err(es)?.ConnectionStatus().map_err(es)
    })();
    match res {
        Ok(WiFiConnectionStatus::Success) => {
            bus.to_module(ID, WMsg::Connected(ssid.to_string()));
            true
        }
        Ok(_) if password.is_none() && secured => {
            bus.to_module(ID, WMsg::NeedPassword(ssid.to_string()));
            false
        }
        Ok(WiFiConnectionStatus::InvalidCredential) => {
            bus.to_module(ID, WMsg::Failed(ssid.to_string(), "wrong password".into()));
            false
        }
        Ok(WiFiConnectionStatus::NetworkNotAvailable) => {
            bus.to_module(ID, WMsg::Failed(ssid.to_string(), "network not available".into()));
            false
        }
        Ok(WiFiConnectionStatus::Timeout) => {
            bus.to_module(ID, WMsg::Failed(ssid.to_string(), "timed out".into()));
            false
        }
        Ok(_) => {
            bus.to_module(ID, WMsg::Failed(ssid.to_string(), "couldn't connect".into()));
            false
        }
        Err(e) => {
            bus.to_module(ID, WMsg::Failed(ssid.to_string(), e));
            false
        }
    }
}

fn worker(rx: Receiver<Cmd>, bus: Bus) {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let mut adapter: Option<WiFiAdapter> = None;
    let mut avail: Vec<WiFiAvailableNetwork> = Vec::new();
    bus.to_module(ID, WMsg::Radio(radio::is_on(RadioKind::WiFi)));
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Quit => return,
            Cmd::Scan => scan(&mut adapter, &mut avail, &bus),
            Cmd::Connect { ssid, password } => {
                if connect(&adapter, &avail, &ssid, password, &bus) {
                    scan(&mut adapter, &mut avail, &bus);
                }
            }
            Cmd::Disconnect => {
                if let Some(ad) = &adapter {
                    let _ = ad.Disconnect();
                }
                bus.to_module(ID, WMsg::Disconnected);
                std::thread::sleep(Duration::from_millis(600));
                scan(&mut adapter, &mut avail, &bus);
            }
            Cmd::Radio(on) => {
                radio::set_on(RadioKind::WiFi, on);
                bus.to_module(ID, WMsg::Radio(radio::is_on(RadioKind::WiFi)));
                if on {
                    // the adapter needs a moment to come back
                    std::thread::sleep(Duration::from_millis(2500));
                    adapter = None;
                    scan(&mut adapter, &mut avail, &bus);
                }
            }
        }
    }
}

impl Wifi {
    /// Alcove: a status strip, then the networks as soft cards, two to a row.
    /// Returns the area the cards use.
    fn draw_alcove(&mut self, ui: &mut Ui, r: Rect) -> Rect {
        // one quiet white instead of blue: a connected network is simply the brightest tile
        let accent = Color::white(0.92);
        let live_status = self.status.clone().filter(|(_, at, _)| at.elapsed() < Duration::from_secs(5));
        let (head_text, head_col) = match &live_status {
            Some((t, _, good)) => (t.clone(), if *good { palette::GREEN } else { palette::ORANGE }),
            None => (
                match (&self.connected, self.radio_on) {
                    (_, Some(false)) => "Wi-Fi is off".to_string(),
                    (Some(s), _) => format!("Connected to {s}"),
                    _ => "Not connected".to_string(),
                },
                palette::TEXT,
            ),
        };
        if live_status.is_none() && self.status.is_some() {
            self.status = None;
        }
        let head = Rect::new(r.x, r.y, r.w, 48.0);
        let tint = if self.radio_on == Some(false) { Color::white(0.5) } else { accent };
        let (refresh, toggled) =
            crate::alcove::status_tile(ui, head, Icon::Wifi, tint, &head_text, head_col, self.radio_on, self.scanning);
        if refresh {
            self.scan();
        }
        if toggled {
            if let Some(on) = self.radio_on {
                self.radio_on = Some(!on);
                self.send(Cmd::Radio(!on));
            }
        }

        let area = Rect::new(r.x, head.bottom() + 12.0, r.w, ALC_LIST);
        if let Some(p) = self.problem.clone() {
            crate::alcove::empty(ui, area, Icon::Wifi, &p);
        } else if self.radio_on == Some(false) {
            crate::alcove::empty(ui, area, Icon::Wifi, "Turn Wi-Fi on to see networks");
        } else if self.nets.is_empty() {
            if self.scanning {
                crate::alcove::empty(ui, area, Icon::Wifi, "Looking for networks…");
            } else {
                crate::alcove::empty(ui, area, Icon::Wifi, "No networks found");
            }
        } else {
            let gap = 10.0;
            let pitch = 62.0;
            let cw = (area.w - gap) * 0.5;
            let rows = self.nets.len().div_ceil(2);
            let shown =
                crate::alcove::scroll_rows(ui, area, &mut self.scroll, rows as f32 * pitch - 8.0, pitch, "wifi-list");
            ui.p.push_clip(area);
            let mut action: Option<Cmd> = None;
            for (i, n) in self.nets.iter().enumerate() {
                let (c, rw) = (i % 2, i / 2);
                let card =
                    Rect::new(area.x + c as f32 * (cw + gap), area.y + rw as f32 * pitch - shown, cw, pitch - 8.0);
                if card.bottom() < area.y || card.y > area.bottom() {
                    continue;
                }
                let is_conn = self.connected.as_deref() == Some(n.ssid.as_str());
                let hover = ui.hovered(card) && ui.hovered(area);
                if hover {
                    ui.hot = true;
                }
                let bg = if is_conn {
                    Color::white(if hover { 0.26 } else { 0.2 })
                } else {
                    Color::white(if hover { 0.12 } else { 0.07 })
                };
                ui.p.fill_rounded(card, 26.0, bg);
                let ic = Rect::centered(card.x + 31.0, card.cy(), 38.0, 38.0);
                crate::alcove::signal_bars(
                    ui,
                    Rect::centered(ic.cx(), ic.cy(), 18.0, 15.0),
                    n.bars,
                    if is_conn { Color::white(1.0) } else { Color::white(0.7) },
                );
                let tx = card.x + 60.0;
                let tw = (card.right() - tx - 40.0).max(0.0);
                ui.p.text(
                    &n.ssid,
                    Rect::new(tx, card.y + 8.0, tw, 20.0),
                    &TextStyle::new(14.0, palette::TEXT).semibold(),
                );
                let (sub, subc) = if is_conn {
                    ("Connected", Color::white(0.9))
                } else if n.secured {
                    ("Secured", palette::TEXT_FAINT)
                } else {
                    ("Open", palette::TEXT_FAINT)
                };
                ui.p.text(sub, Rect::new(tx, card.y + 28.0, tw, 18.0), &TextStyle::new(12.0, subc));
                let busy_here = self.busy.as_deref() == Some(n.ssid.as_str());
                if busy_here {
                    ui.ring_spin(card.right() - 24.0, card.cy(), 20.0, 3.0, accent);
                    ui.animate();
                } else {
                    if n.secured && !is_conn {
                        ui.p.icon(
                            Icon::Lock,
                            Rect::centered(card.right() - 24.0, card.cy(), 13.0, 13.0),
                            palette::TEXT_FAINT,
                        );
                    }
                    if ui.clicked(card) && ui.hovered(area) {
                        action = Some(if is_conn {
                            Cmd::Disconnect
                        } else {
                            Cmd::Connect { ssid: n.ssid.clone(), password: None }
                        });
                    }
                }
            }
            ui.p.pop_clip();
            match action {
                Some(Cmd::Connect { ssid, password }) => {
                    self.busy = Some(ssid.clone());
                    self.prompt = None;
                    self.send(Cmd::Connect { ssid, password });
                }
                Some(c) => self.send(c),
                None => {}
            }
        }
        area
    }

    /// The password line under the cards.
    fn draw_prompt(&mut self, ui: &mut Ui, r: Rect, area: Rect) {
        let accent = palette::BLUE;
        let Some(ssid) = self.prompt.clone() else { return };
        let bar = Rect::new(r.x, area.bottom() + 12.0, r.w, 36.0);
        let cancel = Rect::new(bar.right() - 34.0, bar.y + 2.0, 32.0, 32.0);
        let join = Rect::new(cancel.x - 76.0, bar.y + 4.0, 70.0, 28.0);
        let field = Rect::new(bar.x, bar.y + 2.0, join.x - bar.x - 8.0, 32.0);
        let hint = format!("Password for {ssid}");
        self.pw.draw(ui, field, &hint, accent, true);
        let can = !self.pw.is_empty();
        if crate::alcove::chip(ui, join, "Join", None, can) && can {
            let pw = self.pw.text();
            self.busy = Some(ssid.clone());
            self.send(Cmd::Connect { ssid, password: Some(pw) });
            self.prompt = None;
            self.pw.clear();
            ui.fx.keyboard_focus = Some(false);
        }
        if crate::alcove::round_button(ui, Rect::new(cancel.x, cancel.y, 30.0, 30.0), Icon::Close, false) {
            self.prompt = None;
            self.pw.clear();
            ui.fx.keyboard_focus = Some(false);
        }
    }
}
