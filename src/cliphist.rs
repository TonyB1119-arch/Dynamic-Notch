//! What was copied: text, links, images and files, newest first. The clipboard module feeds it every
//! time something is copied (it never sees content that password managers mark as private), and the
//! launcher's clipboard mode reads it. It lives in memory only and is gone when the notch closes.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP};
use windows::Win32::UI::Shell::DROPFILES;

use crate::gfx::ImageData;
use crate::sys::clip;

/// How many things are kept.
const MAX_ITEMS: usize = 60;
/// An image bigger than this is listed but cannot be put back on the clipboard.
const MAX_IMAGE: usize = 24 * 1024 * 1024;
/// All the images together: the oldest ones lose their data first (their small picture stays).
const MAX_IMAGE_TOTAL: usize = 96 * 1024 * 1024;

#[derive(Clone, PartialEq)]
pub enum Content {
    Text(String),
    Files(Vec<String>),
    Image { w: u32, h: u32 },
}

struct Rec {
    id: u64,
    content: Content,
    at: Instant,
    thumb: Option<ImageData>,
    /// An image as a BMP file (the clipboard's DIB with a 14-byte header in front).
    bmp: Option<Arc<Vec<u8>>>,
    /// Tells two images of the same size apart.
    sig: u64,
}

struct Store {
    recs: Vec<Rec>,
    next: u64,
}

static STORE: Mutex<Store> = Mutex::new(Store { recs: Vec::new(), next: 1 });

fn store() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

/// One line of the history, as the launcher shows it.
pub struct Row {
    pub id: u64,
    /// 0 text, 1 link, 2 image, 3 files.
    pub code: u8,
    pub summary: String,
    pub age_secs: u64,
    pub thumb: Option<ImageData>,
}

fn fingerprint(b: &[u8]) -> u64 {
    // a few thousand bytes spread over the whole picture are enough to tell pictures apart
    let step = (b.len() / 4096).max(1);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ b.len() as u64;
    for &x in b.iter().step_by(step) {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

fn is_link(t: &str) -> bool {
    let t = t.trim();
    let l = t.to_ascii_lowercase();
    !t.contains(char::is_whitespace) && (l.starts_with("http://") || l.starts_with("https://") || l.starts_with("www."))
}

/// Something was copied. Returns its id (the old one when the same thing is copied again).
pub fn add(content: Content, bmp: Option<&[u8]>) -> u64 {
    let sig = match (&content, bmp) {
        (Content::Image { .. }, Some(b)) => fingerprint(b),
        _ => 0,
    };
    let mut s = store();
    if let Some(pos) = s.recs.iter().position(|r| r.content == content && r.sig == sig) {
        let mut r = s.recs.remove(pos);
        r.at = Instant::now();
        let id = r.id;
        s.recs.insert(0, r);
        return id;
    }
    let bmp = bmp.filter(|b| b.len() <= MAX_IMAGE).map(|b| Arc::new(b.to_vec()));
    let id = s.next;
    s.next += 1;
    s.recs.insert(0, Rec { id, content, at: Instant::now(), thumb: None, bmp, sig });
    s.recs.truncate(MAX_ITEMS);
    let mut total = 0usize;
    for r in s.recs.iter_mut() {
        if let Some(b) = &r.bmp {
            total += b.len();
            if total > MAX_IMAGE_TOTAL {
                r.bmp = None;
            }
        }
    }
    id
}

/// The small picture of an image item, once it has been made.
pub fn set_thumb(id: u64, img: ImageData) {
    if let Some(r) = store().recs.iter_mut().find(|r| r.id == id) {
        r.thumb = Some(img);
    }
}

/// The history, newest first; `filter` keeps only what contains those letters (any case).
pub fn rows(filter: &str) -> Vec<Row> {
    let f = filter.trim().to_lowercase();
    let s = store();
    let mut out = Vec::with_capacity(s.recs.len());
    for r in &s.recs {
        let (code, summary, hay) = match &r.content {
            Content::Text(t) => {
                let one: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
                let short: String = one.chars().take(120).collect();
                (u8::from(is_link(t)), short, if f.is_empty() { String::new() } else { t.to_lowercase() })
            }
            Content::Files(fl) => {
                let name = match fl.len() {
                    0 => String::new(),
                    1 => file_name(&fl[0]).to_string(),
                    n => format!("{} and {} more", file_name(&fl[0]), n - 1),
                };
                let hay = if f.is_empty() { String::new() } else { fl.join("\n").to_lowercase() };
                (3, name, hay)
            }
            Content::Image { w, h } => (2, format!("Image  {w} × {h}"), "image".to_string()),
        };
        if !f.is_empty() && !hay.contains(&f) {
            continue;
        }
        out.push(Row { id: r.id, code, summary, age_secs: r.at.elapsed().as_secs(), thumb: r.thumb.clone() });
    }
    out
}

/// The words of a text item (or the names of files) for the preview box.
pub fn preview(id: u64) -> Option<String> {
    let s = store();
    let r = s.recs.iter().find(|r| r.id == id)?;
    match &r.content {
        Content::Text(t) => Some(t.chars().take(600).collect()),
        Content::Files(f) => Some(f.iter().map(|p| file_name(p)).collect::<Vec<_>>().join("\n")),
        Content::Image { .. } => None,
    }
}

pub fn remove(id: u64) {
    store().recs.retain(|r| r.id != id);
}

pub fn clear() {
    store().recs.clear();
}

/// Put an item back on the clipboard. False when that is not possible (a very big image).
pub fn copy_back(hwnd: HWND, id: u64) -> bool {
    enum Out {
        Text(String),
        Files(Vec<String>),
        Image(Arc<Vec<u8>>),
    }
    let out = {
        let s = store();
        let Some(r) = s.recs.iter().find(|r| r.id == id) else { return false };
        match &r.content {
            Content::Text(t) => Out::Text(t.clone()),
            Content::Files(f) => Out::Files(f.clone()),
            Content::Image { .. } => match &r.bmp {
                Some(b) => Out::Image(b.clone()),
                None => return false,
            },
        }
    };
    match out {
        Out::Text(t) => clip::set_text(hwnd, &t),
        Out::Files(f) => set_files(hwnd, &f),
        // the clipboard wants the picture without the file header
        Out::Image(b) if b.len() > 14 => clip::set_data(hwnd, CF_DIB.0 as u32, &b[14..]),
        Out::Image(_) => false,
    }
}

fn set_files(hwnd: HWND, files: &[String]) -> bool {
    let header = std::mem::size_of::<DROPFILES>();
    let mut list: Vec<u16> = Vec::new();
    for f in files {
        list.extend(f.encode_utf16());
        list.push(0);
    }
    list.push(0);
    let mut bytes = vec![0u8; header];
    let df = DROPFILES { pFiles: header as u32, fWide: true.into(), ..Default::default() };
    unsafe { std::ptr::copy_nonoverlapping(&df as *const _ as *const u8, bytes.as_mut_ptr(), header) };
    bytes.extend(list.iter().flat_map(|c| c.to_le_bytes()));
    clip::set_data(hwnd, CF_HDROP.0 as u32, &bytes)
}
