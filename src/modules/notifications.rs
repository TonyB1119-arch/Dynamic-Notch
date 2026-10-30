//! Windows notifications in the notch.
//!
//! Uses the Windows notification listener (`UserNotificationListener`): when
//! a toast arrives, the notch peeks with the app, title and text, and the tab
//! keeps a short history. The list is checked every second and a half while
//! the module is on; nothing else runs.
//!
//! Windows must allow it: Settings → Privacy & security → Notifications
//! ("Notification access"). The tab says what Windows answered.

use std::any::Any;
use std::collections::HashSet;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::Duration;

use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

use super::widgets::scroll;
use super::{Cx, Module, ModuleId, Peek, Trailing};
use crate::bus::Bus;
use crate::gfx::{Icon, TextStyle};
use crate::ui::{ButtonStyle, Ui};
use crate::util::{palette, truncate_chars, Color, Rect};

const ID: ModuleId = "notifications";
const POLL: Duration = Duration::from_millis(2000);
const MAX_KEPT: usize = 30;
const ROW_H: f32 = 46.0;
const HEADER_H: f32 = 34.0;
const LIST_H: f32 = 4.0 * ROW_H;

#[derive(Clone, Debug)]
struct Note {
    app: String,
    title: String,
    body: String,
}

enum NMsg {
    New(Note),
    /// What Windows said about notification access.
    Access(String, bool),
}

pub struct Notifications {
    quit: Option<Sender<()>>,
    items: Vec<Note>,
    status: String,
    allowed: bool,
    scroll: f32,
}

impl Notifications {
    pub fn new() -> Self {
        Self { quit: None, items: Vec::new(), status: "Checking access…".into(), allowed: false, scroll: 0.0 }
    }
}

impl Module for Notifications {
    fn id(&self) -> ModuleId {
        ID
    }
    fn title(&self) -> &str {
        "Notifications"
    }
    fn icon(&self) -> Icon {
        Icon::Bell
    }

    fn start(&mut self, cx: &mut Cx) {
        let (tx, rx) = channel::<()>();
        let bus = cx.bus.clone();
        let _ = std::thread::Builder::new().name("notifications".into()).spawn(move || worker(rx, bus));
        self.quit = Some(tx);
    }

    fn stop(&mut self) {
        if let Some(tx) = self.quit.take() {
            let _ = tx.send(());
        }
        self.items.clear();
    }

    fn on_message(&mut self, msg: Box<dyn Any + Send>, cx: &mut Cx) {
        let Ok(m) = msg.downcast::<NMsg>() else { return };
        match *m {
            NMsg::Access(text, ok) => {
                self.status = text;
                self.allowed = ok;
            }
            NMsg::New(n) => {
                let title = if n.title.is_empty() { n.app.clone() } else { n.title.clone() };
                let sub = match (n.title.is_empty(), n.body.is_empty()) {
                    (true, _) => truncate_chars(&n.body, 80),
                    (false, true) => n.app.clone(),
                    (false, false) => truncate_chars(&format!("{} · {}", n.app, n.body), 80),
                };
                // the message itself under the name; the count of kept notices as a red badge
                let body = if n.body.is_empty() { sub } else { truncate_chars(&n.body, 110) };
                let count = (self.items.len() + 1).min(9).to_string();
                cx.fx.peek(
                    Peek::new(Icon::Bell, palette::ORANGE, truncate_chars(&title, 40), body)
                        .key("notify")
                        .trailing(Trailing::Text(count, palette::RED))
                        .duration_ms(5500)
                        .page(ID),
                );
                self.items.insert(0, n);
                self.items.truncate(MAX_KEPT);
            }
        }
        cx.fx.redraw = true;
    }

    fn has_page(&self) -> bool {
        true
    }

    fn page_height(&self, _w: f32) -> f32 {
        if crate::ui::look_alcove() {
            return 40.0 + 3.0 * 66.0;
        }
        HEADER_H + 12.0 + LIST_H
    }

    fn draw_page(&mut self, ui: &mut Ui, r: Rect) {
        if crate::ui::look_alcove() {
            return self.draw_alcove(ui, r);
        }
        let head = Rect::new(r.x, r.y, r.w, HEADER_H);
        ui.p.icon(Icon::Bell, Rect::new(head.x + 2.0, head.cy() - 9.0, 18.0, 18.0), palette::TEXT_DIM);
        let clear = Rect::new(head.right() - 78.0, head.y + 3.0, 78.0, 28.0);
        ui.p.text(
            &self.status,
            Rect::new(
                head.x + 28.0,
                head.y,
                (clear.x - head.x - 36.0 - if self.items.is_empty() { 0.0 } else { 84.0 }).max(0.0),
                head.h,
            ),
            &TextStyle::new(12.5, if self.allowed { palette::TEXT } else { palette::ORANGE }).medium(),
        );
        if !self.items.is_empty() {
            // how full the kept list is
            let fill = self.items.len() as f32 / MAX_KEPT as f32;
            ui.ticks(Rect::new(clear.x - 12.0 - 72.0, head.cy() - 5.0, 72.0, 10.0), fill, Color::white(0.85));
            if ui.pill_button(clear, "Clear", None, ButtonStyle::filled(Color::white(0.08)).fg(palette::TEXT_DIM)) {
                self.items.clear();
            }
        }

        let area = Rect::new(r.x, head.bottom() + 12.0, r.w, LIST_H);
        if self.items.is_empty() {
            let text =
                if self.allowed { "No notifications yet" } else { "Waiting for Windows to allow notification access" };
            ui.empty_state(area, Icon::Bell, text);
            return;
        }
        scroll(ui, area, &mut self.scroll, self.items.len() as f32 * ROW_H);
        ui.p.push_clip(area);
        for (i, n) in self.items.iter().enumerate() {
            let row = Rect::new(area.x, area.y + i as f32 * ROW_H - self.scroll, area.w, ROW_H - 6.0);
            if row.bottom() < area.y || row.y > area.bottom() {
                continue;
            }
            ui.row_bg(row, false, None);
            ui.p.icon(Icon::Bell, Rect::new(row.x + 12.0, row.cy() - 8.0, 16.0, 16.0), palette::ORANGE);
            let tx = row.x + 38.0;
            let tw = (row.w - 50.0).max(0.0);
            let head_line = if n.title.is_empty() { n.app.clone() } else { n.title.clone() };
            ui.p.text(&head_line, Rect::new(tx, row.y + 4.0, tw, 17.0), &TextStyle::new(13.0, palette::TEXT).medium());
            let sub = if n.title.is_empty() { n.body.clone() } else { format!("{} · {}", n.app, n.body) };
            ui.p.text(&sub, Rect::new(tx, row.y + 21.0, tw, 15.0), &TextStyle::new(11.5, palette::TEXT_FAINT));
        }
        ui.p.pop_clip();
    }
}

impl Notifications {
    /// Alcove: each notification is a soft bubble with the app's first letter on a coloured tile.
    fn draw_alcove(&mut self, ui: &mut Ui, r: Rect) {
        let (head, body) = r.split_top(32.0, 8.0);
        let note = if self.items.is_empty() { String::new() } else { format!("{}", self.items.len()) };
        crate::alcove::title(ui, head, Icon::Bell, palette::ORANGE, "Notifications", &note);
        if !self.items.is_empty() {
            let cw = crate::alcove::chip_w(ui, "Clear", Some(Icon::Trash));
            if crate::alcove::chip(
                ui,
                Rect::new(head.right() - cw, head.y + 2.0, cw, 28.0),
                "Clear",
                Some(Icon::Trash),
                false,
            ) {
                self.items.clear();
                return;
            }
        }
        if self.items.is_empty() {
            let text =
                if self.allowed { "No notifications yet" } else { "Waiting for Windows to allow notification access" };
            crate::alcove::empty(ui, body, Icon::Bell, text);
            if !self.allowed {
                ui.p.text(
                    &self.status,
                    Rect::new(body.x + 16.0, body.bottom() - 30.0, body.w - 32.0, 22.0),
                    &TextStyle::new(11.5, palette::ORANGE).center(),
                );
            }
            return;
        }
        let pitch = 66.0;
        scroll(ui, body, &mut self.scroll, self.items.len() as f32 * pitch);
        ui.p.push_clip(body);
        for (i, n) in self.items.iter().enumerate() {
            let row = Rect::new(body.x, body.y + i as f32 * pitch - self.scroll, body.w, pitch - 8.0);
            if row.bottom() < body.y || row.y > body.bottom() {
                continue;
            }
            let hover = ui.hovered(row) && ui.hovered(body);
            if hover {
                ui.p.fill_rounded(row, 22.0, Color::white(0.08));
            }
            // a round avatar with the first letter, the name in bold, the message under it
            let av = Rect::centered(row.x + 30.0, row.cy(), 42.0, 42.0);
            ui.p.fill_circle(av.cx(), av.cy(), 21.0, Color::white(0.13));
            let who = if n.title.is_empty() { &n.app } else { &n.title };
            let letter: String = who.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            ui.p.text(&letter, av, &TextStyle::new(18.0, Color::white(0.95)).semibold().center());
            let tx = av.right() + 14.0;
            let tw = (row.right() - tx - 16.0).max(0.0);
            ui.p.text(who, Rect::new(tx, row.y + 6.0, tw, 22.0), &TextStyle::new(15.0, Color::white(1.0)).semibold());
            let sub = if n.title.is_empty() { n.body.clone() } else { format!("{} · {}", n.app, n.body) };
            ui.p.text(&sub, Rect::new(tx, row.y + 29.0, tw, 20.0), &TextStyle::new(13.0, palette::TEXT_DIM));
        }
        ui.p.pop_clip();
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

fn worker(rx: std::sync::mpsc::Receiver<()>, bus: Bus) {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let listener = match UserNotificationListener::Current() {
        Ok(l) => l,
        Err(e) => {
            bus.to_module(ID, NMsg::Access(format!("Notifications aren't available here ({e})"), false));
            return;
        }
    };
    let allowed = |l: &UserNotificationListener| {
        l.GetAccessStatus().is_ok_and(|s| s == UserNotificationListenerAccessStatus::Allowed)
    };
    // asks once (Windows may show its own prompt), then only re-checks
    let first = listener.RequestAccessAsync().and_then(|op| op.join());
    let mut was_allowed = matches!(&first, Ok(s) if *s == UserNotificationListenerAccessStatus::Allowed);
    match first {
        Ok(_) if was_allowed => bus.to_module(ID, NMsg::Access("Showing Windows notifications".into(), true)),
        Ok(_) => bus.to_module(
            ID,
            NMsg::Access(
                "Windows hasn't allowed notification access (Settings → Privacy & security → Notifications)".into(),
                false,
            ),
        ),
        Err(e) => bus.to_module(ID, NMsg::Access(format!("Windows refused notification access ({e})"), false)),
    }

    let mut seen: HashSet<u32> = HashSet::new();
    let mut primed = false;
    loop {
        let ok = allowed(&listener);
        if ok != was_allowed {
            was_allowed = ok;
            let text = if ok { "Showing Windows notifications" } else { "Notification access was turned off" };
            bus.to_module(ID, NMsg::Access(text.into(), ok));
        }
        if ok {
            if let Ok(list) = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.join()) {
                let mut current = HashSet::new();
                for i in 0..list.Size().unwrap_or(0) {
                    let Ok(item) = list.GetAt(i) else { continue };
                    let id = item.Id().unwrap_or(0);
                    current.insert(id);
                    // the first pass only learns what is already there
                    if primed && !seen.contains(&id) {
                        if let Some(note) = read(&item) {
                            bus.to_module(ID, NMsg::New(note));
                        }
                    }
                }
                seen = current;
                primed = true;
            }
        }
        match rx.recv_timeout(POLL) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }
    }
}

fn read(n: &UserNotification) -> Option<Note> {
    let app = n.AppInfo().ok()?.DisplayInfo().ok()?.DisplayName().ok()?.to_string();
    let visual = n.Notification().ok()?.Visual().ok()?;
    let binding = visual.GetBinding(&KnownNotificationBindings::ToastGeneric().ok()?).ok()?;
    let texts = binding.GetTextElements().ok()?;
    let mut lines: Vec<String> = Vec::new();
    for i in 0..texts.Size().ok()? {
        if let Ok(t) = texts.GetAt(i).and_then(|t| t.Text()) {
            let s = t.to_string();
            if !s.trim().is_empty() {
                lines.push(s.trim().to_string());
            }
        }
    }
    let title = lines.first().cloned().unwrap_or_default();
    let body = lines.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(Note { app, title, body })
}
