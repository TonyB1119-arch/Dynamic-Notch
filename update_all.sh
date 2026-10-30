cd /workspaces/Notch
cat > /tmp/v157.py <<'PYEOF'
import re
def rd(p): return open(p).read()
def sub(s,a,b,n=1):
    assert a in s, a[:70]
    return s.replace(a,b,n)
NEW="TonyB1119-arch/Notch"

# ---- config: new default repo
p='src/config.rs'; s=rd(p)
s=sub(s,'Self { repo: "shreyasb1119-arch/Notch".into() }',f'Self {{ repo: "{NEW}".into() }}')
s=sub(s,'repo = "shreyasb1119-arch/Notch"',f'repo = "{NEW}"')
open(p,'w').write(s)

# ---- updater
p='src/updater.rs'; s=rd(p)
s=sub(s,"""//! it quietly; the installer closes this app, replaces the files and starts it again.
//! There is no timer, no background service and no check at start-up.""","""//! it quietly; the installer closes this app, replaces the files and starts it again.
//! A release that carries the bare `dynamic-notch.exe` is even simpler: it is downloaded next to
//! the running exe, swapped in (a running exe can be renamed on Windows) and started again.
//! There is no timer, no background service and no check at start-up.""")
s=sub(s,"Available { tag: String, installer: Option<String>, page: String },","Available { tag: String, installer: Option<String>, page: String, notes: String },")
s=sub(s,"""    let repo = repo.trim().to_string();
    let bus = bus.clone();""","""    let repo = canonical(repo);
    let bus = bus.clone();""")
s=sub(s,"""fn headers() ->""","""/// Older configs still name the repos this app was forked from; they all mean this project's.
fn canonical(repo: &str) -> String {
    let r = repo.trim();
    if r.eq_ignore_ascii_case("shreyasb1119-arch/Notch") || r.eq_ignore_ascii_case("eigenes/DynamicNotch") || r.is_empty() {
        "TonyB1119-arch/Notch".to_string()
    } else {
        r.to_string()
    }
}

/// Remove what a finished self-update left behind. Cheap, silent, and only touches our own files.
pub fn tidy() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
        let _ = std::fs::remove_file(exe.with_extension("exe.new"));
    }
}

fn headers() ->""")
# asset choice
a=s.index("    // the setup program")
b=s.index("    let page = v[")
s=s[:a]+"""    // the bare exe wins (swapped in place); a setup program is the fallback
    let mut exe = None;
    let mut setup = None;
    for a in v["assets"].as_array().into_iter().flatten() {
        let (Some(name), Some(url)) = (a["name"].as_str(), a["browser_download_url"].as_str()) else { continue };
        let name = name.to_ascii_lowercase();
        if !name.ends_with(".exe") {
            continue;
        }
        if name.contains("setup") {
            setup.get_or_insert(url.to_string());
        } else {
            exe.get_or_insert(url.to_string());
        }
    }
    let installer = exe.or(setup);
    let notes = v["body"].as_str().unwrap_or("").to_string();
"""+s[b:]
s=sub(s,"Msg::Available { tag, installer, page }\n}","Msg::Available { tag, installer, page, notes }\n}")
# install
a=s.index("fn run_install(url: &str) -> Msg {")
b=s.index("/// Is release tag")
s=s[:a]+'''fn run_install(url: &str) -> Msg {
    if !url.starts_with("https://") {
        return Msg::Failed("The download link is not secure".into());
    }
    let setup = url.rsplit('/').next().unwrap_or("").to_ascii_lowercase().contains("setup");
    if setup {
        let path = std::env::temp_dir().join("DynamicNotch-Setup.exe");
        if let Err(e) = download(url, &path) {
            return Msg::Failed(e);
        }
        return match std::process::Command::new(&path)
            .args(["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"])
            .spawn()
        {
            Ok(_) => Msg::Installing,
            Err(e) => Msg::Failed(format!("Couldn't start the installer: {e}")),
        };
    }
    swap_in(url)
}

/// Download `url` to `path` and check it looks like a Windows program.
fn download(url: &str, path: &std::path::Path) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|_| "Couldn't write next to the app".to_string())?;
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
    let ok = matches!(result, Ok(200)) && write_ok && total >= 100_000 && &first == b"MZ";
    if !ok {
        let _ = std::fs::remove_file(path);
        return Err(match result {
            Err(e) => e,
            _ => "The download didn't look right".to_string(),
        });
    }
    Ok(())
}

/// Replace the running exe with the downloaded one, start it again and quit this copy.
fn swap_in(url: &str) -> Msg {
    use std::os::windows::process::CommandExt;
    let Ok(cur) = std::env::current_exe() else { return Msg::Failed("Couldn't find the running app".into()) };
    let new = cur.with_extension("exe.new");
    let old = cur.with_extension("exe.old");
    if let Err(e) = download(url, &new) {
        return Msg::Failed(e);
    }
    let _ = std::fs::remove_file(&old);
    if std::fs::rename(&cur, &old).is_err() {
        let _ = std::fs::remove_file(&new);
        return Msg::Failed("Couldn't replace the app (is it in a protected folder?)".into());
    }
    if std::fs::rename(&new, &cur).is_err() {
        let _ = std::fs::rename(&old, &cur);
        let _ = std::fs::remove_file(&new);
        return Msg::Failed("Couldn't put the new version in place".into());
    }
    // start the new copy a moment from now, once this one has let go of the single-instance lock
    let started = std::process::Command::new("cmd")
        .arg("/C")
        .raw_arg(format!("ping -n 3 127.0.0.1 >nul & start \\"\\" \\"{}\\"", cur.display()))
        .creation_flags(0x0800_0000)
        .spawn();
    if started.is_err() {
        return Msg::Failed("Updated, please start the notch again".into());
    }
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(400));
        crate::ipc::send_to_running(&["quit".to_string()]);
    });
    Msg::Installing
}

'''+s[b:]
open(p,'w').write(s)

# ---- main: tidy
p='src/main.rs'; s=rd(p)
s=sub(s,"    util::log_init();\n","    util::log_init();\n    updater::tidy();\n")
open(p,'w').write(s)

# ---- settings
p='src/modules/settings.rs'; s=rd(p)
s=sub(s,"Available { tag: String, installer: Option<String>, page: String },","Available { tag: String, installer: Option<String>, page: String, notes: String },")
s=sub(s,"updater::Msg::Available { tag, installer, page } => Update::Available { tag, installer, page },","updater::Msg::Available { tag, installer, page, notes } => Update::Available { tag, installer, page, notes },")
s=sub(s,"""            Update::Available { tag, installer, .. } => {
                let verb = if installer.is_some() { "Install" } else { "Open page" };
                (format!("{tag} is available"), verb.to_string())
            }""","""            Update::Available { tag, installer, notes, .. } => {
                let verb = if installer.is_some() { "Install" } else { "Open page" };
                // the first real line of the release notes: what's new
                let line = notes
                    .lines()
                    .map(|l| l.trim().trim_start_matches(['#', '-', '*', ' ']).trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or("");
                let short: String = if line.chars().count() > 44 {
                    line.chars().take(43).collect::<String>() + "…"
                } else {
                    line.to_string()
                };
                let what = if short.is_empty() { format!("{tag} is available") } else { format!("{tag}: {short}") };
                (what, verb.to_string())
            }""")
s=re.sub(r"Build v\d+ ·","Build v157 ·",s)
open(p,'w').write(s)
PYEOF
python3 /tmp/v157.py && echo patched
cat > release.sh <<'EOF'
#!/usr/bin/env bash
# Build, package and publish a new Dynamic Notch release.
#   ./release.sh "What's new in this update"
set -euo pipefail
cd "$(dirname "$0")"

NOTES="${1:-Improvements and fixes.}"
cur=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([0-9]+)\.([0-9]+)\.([0-9]+)".*/\1 \2 \3/')
read -r MA MI PA <<<"$cur"
VER="$MA.$MI.$((PA + 1))"
TAG="v$VER"

echo "Releasing $TAG"
sed -i -E "0,/^version = \".*\"/s//version = \"$VER\"/" Cargo.toml
sed -i -E "s/Build v[0-9]+ ·/Build $TAG ·/" src/modules/settings.rs || true

cargo build --release --target x86_64-pc-windows-gnu
EXE=target/x86_64-pc-windows-gnu/release/dynamic-notch.exe

rm -rf dist && mkdir dist
cp "$EXE" dist/dynamic-notch.exe
if [ -f DynamicNotch.nsi ]; then
  makensis DynamicNotch.nsi >/dev/null
  [ -f DynamicNotch-Setup.exe ] && cp DynamicNotch-Setup.exe dist/
fi

printf '%s\n' "$NOTES" > dist/notes.md
git add -A
git commit -m "Release $TAG: $NOTES" >/dev/null || true
git push
gh release create "$TAG" dist/*.exe --title "Dynamic Notch $TAG" --notes-file dist/notes.md
echo "Done. Press Check for updates in Settings to get $TAG."
EOF
chmod +x release.sh
