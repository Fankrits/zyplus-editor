#!/bin/sh
# Zyplus macOS installer:  curl -fsSL https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.sh | sh
# Or pin a version:        ZYPLUS_VERSION=0.2.4 sh install.sh
set -eu

REPO=Fankrits/zyplus-editor

if [ "$(uname -s)" != "Darwin" ]; then
  echo "Zyplus currently supports macOS only ($(uname -s) is not supported yet)." >&2
  echo "See https://github.com/$REPO/releases" >&2
  exit 1
fi
if [ "$(uname -m)" != "arm64" ]; then
  echo "Zyplus releases are Apple Silicon (arm64) only; this Mac is $(uname -m)." >&2
  exit 1
fi

API="https://api.github.com/repos/$REPO/releases"
[ -n "${ZYPLUS_VERSION:-}" ] && API="$API/tags/v${ZYPLUS_VERSION}" || API="$API/latest"

RELEASE=$(curl -fsSL "$API" 2>/dev/null) || {
  echo "No release found at $API — check ZYPLUS_VERSION, or see" >&2
  echo "https://github.com/$REPO/releases" >&2
  exit 1
}
VERSION=$(printf '%s' "$RELEASE" | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)
URLS=$(printf '%s' "$RELEASE" | tr ',' '\n' |
  sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p')

URL=$(printf '%s\n' "$URLS" | grep -E '_aarch64\.dmg$' | head -1)
if [ -z "$URL" ]; then
  echo "No Zyplus ${VERSION:-} macOS DMG published for $(uname -s) $(uname -m)." >&2
  echo "Available: https://github.com/$REPO/releases" >&2
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; [ -n "${MNT:-}" ] && hdiutil detach "$MNT" -quiet 2>/dev/null || true' EXIT

echo "Downloading Zyplus $VERSION" >&2
curl -fL --progress-bar -o "$TMP/dl.dmg" "$URL"

MNT=$(hdiutil attach -nobrowse -readonly "$TMP/dl.dmg" |
  sed -n 's/.*\(\/Volumes\/.*\)$/\1/p' | tail -1)
[ -n "$MNT" ] && [ -d "$MNT/Zyplus.app" ] || { echo "Zyplus.app not found in dmg" >&2; exit 1; }

# Copy beside the old app, then swap: a copy that fails partway (full disk)
# must leave the installed version working, not no version at all.
rm -rf /Applications/.Zyplus.app.new
cp -R "$MNT/Zyplus.app" /Applications/.Zyplus.app.new
rm -rf /Applications/Zyplus.app
mv /Applications/.Zyplus.app.new /Applications/Zyplus.app
xattr -dr com.apple.quarantine /Applications/Zyplus.app 2>/dev/null || true
echo "Installed /Applications/Zyplus.app — open it with: open -a Zyplus"
