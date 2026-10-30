//! Updates, on request only: nothing here runs until the button in Settings is pressed.
//!
//! "Check" asks GitHub for the newest release of `[update] repo` (one small request, then the
//! thread ends). "Install" downloads that release's setup program to the temp folder and starts
//! it quietly; the installer closes this app, replaces the files and starts it again.
//! There is no timer, no background service and no check at start-up.

use std::io::Write;
use std::sync::atomic::AtomicBool;

use crate::bus::Bus;
use crate::sys::http::{self, Timeouts};

/// The version this build was made as (`Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Worker -> Settings.
#[derive(Debug)]
pub enum Msg {
    UpToDate,
    Available { tag: String, installer: Option<String>, page: String },
    Installing,
    Failed(String),
}

/// Ask GitHub whether a newer release exists. The answer arrives as a `Msg`.
pub fn check(repo: &str, bus: &Bus) {
    let repo = repo.trim().to_string();
    let bus = bus.clone();
    let _ = std::thread::Builder::new().name("update".into()).spawn(move || {
        bus.to_module("settings", run_check(&repo));
    });
}

/// Download the setup program and start it quietly.
pub fn install(url: &str, bus: &Bus) {
    let url = url.to_string();
    let bus = bus.clone();
    let _ = std::thread::Builder::new().name("update".into()).spawn(move || {
        bus.to_module("settings", run_install(&url));
    });
}

fn headers() -> [(&'static str, String); 2] {
    [("Accept", "application/vnd.github+json".to_string()), ("User-Agent", "DynamicNotch".to_string())]
}

fn run_check(repo: &str) -> Msg {
    if repo.split('/').count() != 2 || repo.contains(' ') || repo.starts_with('/') || repo.ends_with('/') {
        return Msg::Failed("Set [update] repo in the config (owner/name)".into());
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let cancel = AtomicBool::new(false);
    let (status, body) = match http::fetch("GET", &url, &headers(), &[], Timeouts::DEFAULT, 512 * 1024, &cancel) {
        Ok(v) => v,
        Err(e) => return Msg::Failed(e),
    };
    match status {
        200 => {}
        404 => return Msg::Failed("No release published yet".into()),
        s => return Msg::Failed(format!("GitHub answered {s}")),
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Msg::Failed("Couldn't read GitHub's answer".into());
    };
    let tag = v["tag_name"].as_str().unwrap_or("").to_string();
    if tag.is_empty() {
        return Msg::Failed("The release has no version".into());
    }
    if !is_newer(&tag, VERSION) {
        return Msg::UpToDate;
    }
    // the setup program: an .exe with "setup" in its name
    let installer = v["assets"].as_array().and_then(|assets| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?.to_ascii_lowercase();
            if name.ends_with(".exe") && name.contains("setup") {
                a["browser_download_url"].as_str().map(str::to_string)
            } else {
                None
            }
        })
    });
    let page = v["html_url"].as_str().unwrap_or("").to_string();
    Msg::Available { tag, installer, page }
}

fn run_install(url: &str) -> Msg {
    if !url.starts_with("https://") {
        return Msg::Failed("The download link is not secure".into());
    }
    let path = std::env::temp_dir().join("DynamicNotch-Setup.exe");
    let Ok(mut file) = std::fs::File::create(&path) else {
        return Msg::Failed("Couldn't write to the temp folder".into());
    };
    let cancel = AtomicBool::new(false);
    let mut total = 0usize;
    let mut first = [0u8; 2];
    let mut write_ok = true;
    let result =
        http::request("GET", url, &headers(), &[], Timeouts(10_000, 15_000, 30_000, 300_000), &cancel, |chunk| {
            if total == 0 && chunk.len() >= 2 {
                first = [chunk[0], chunk[1]];
            }
            total += chunk.len();
            write_ok = file.write_all(chunk).is_ok();
            write_ok
        });
    drop(file);
    let status = match result {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            return Msg::Failed(e);
        }
    };
    // a real Windows program: right status, plausible size, "MZ" header
    if status != 200 || !write_ok || total < 100_000 || &first != b"MZ" {
        let _ = std::fs::remove_file(&path);
        return Msg::Failed("The download didn't look right".into());
    }
    match std::process::Command::new(&path).args(["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"]).spawn() {
        Ok(_) => Msg::Installing,
        Err(e) => Msg::Failed(format!("Couldn't start the installer: {e}")),
    }
}

/// Is release tag `tag` ("V3", "v1.2.0") newer than `current` ("3.0.0")? Compared number by number.
pub fn is_newer(tag: &str, current: &str) -> bool {
    fn nums(s: &str) -> Vec<u32> {
        s.trim_start_matches(['v', 'V'])
            .split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (t, c) = (nums(tag), nums(current));
    for i in 0..t.len().max(c.len()) {
        let (a, b) = (t.get(i).copied().unwrap_or(0), c.get(i).copied().unwrap_or(0));
        if a != b {
            return a > b;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("V4", "3.0.0"));
        assert!(is_newer("v3.1", "3.0.9"));
        assert!(is_newer("V10", "9.0.0"));
        assert!(!is_newer("V3", "3.0.0"));
        assert!(!is_newer("V2", "3.0.0"));
        assert!(!is_newer("v3.0.0", "3"));
    }
}
