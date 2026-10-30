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
