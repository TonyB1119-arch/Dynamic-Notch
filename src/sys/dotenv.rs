//! Minimal `.env` reader for secrets (API keys).
//!
//! Lookup order for a variable:
//!   1. the real process environment
//!   2. `.env` next to the executable, then up to 3 parent folders
//!      (so `target\release\dynamic-notch.exe` finds the project's `.env`)
//!   3. `.env` in the current working directory
//!   4. `%APPDATA%\DynamicNotch\.env`
//!
//! `.env` is git-ignored; `.env.example` documents the expected keys.

use std::path::{Path, PathBuf};

use crate::util::app_dir;

pub fn get(key: &str) -> Option<String> {
    if let Ok(v) = std::env::var(key) {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    for path in candidates() {
        if let Some(v) = read_key(&path, key) {
            return Some(v);
        }
    }
    None
}

/// Like `get`, but the key saved from the settings screen (in
/// `%APPDATA%\\DynamicNotch\\.env`) wins over any other `.env` file.
pub fn get_saved_first(key: &str) -> Option<String> {
    read_key(&app_dir().join(".env"), key).or_else(|| get(key))
}

/// Save `key=value` in `%APPDATA%\\DynamicNotch\\.env`, keeping the other lines.
/// An empty value removes the key.
pub fn set_saved(key: &str, value: &str) -> bool {
    let path = app_dir().join(".env");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let is_key = t.split_once('=').is_some_and(|(k, _)| k.trim() == key);
        if !is_key {
            out.push(line.to_string());
        }
    }
    if !value.is_empty() {
        out.push(format!("{key}={value}"));
    }
    std::fs::write(&path, out.join("\n") + "\n").is_ok()
}

/// Where we would look, for error messages.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..4 {
            let Some(d) = dir else { break };
            out.push(d.join(".env"));
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join(".env"));
    }
    out.push(app_dir().join(".env"));
    out.dedup();
    out
}

fn read_key(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    parse(&text).into_iter().find(|(k, _)| k == key).map(|(_, v)| v).filter(|v| !v.is_empty())
}

pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim().to_string();
        let mut v = v.trim();
        if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
            || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
        {
            v = &v[1..v.len() - 1];
        } else if let Some(i) = v.find(" #") {
            v = v[..i].trim_end();
        }
        out.push((k, v.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_common_forms() {
        let p = parse("# c\nA=1\nexport B = \"two words\"\nC='x'\nD=val # comment\n\nbad line\n");
        assert_eq!(p[0], ("A".into(), "1".into()));
        assert_eq!(p[1], ("B".into(), "two words".into()));
        assert_eq!(p[2], ("C".into(), "x".into()));
        assert_eq!(p[3], ("D".into(), "val".into()));
        assert_eq!(p.len(), 4);
    }
}
