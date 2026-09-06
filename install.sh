#!/bin/sh
# Zyplus installer:  curl -fsSL https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.sh | sh
# Or pin a version:  ZYPLUS_VERSION=0.1.0 sh install.sh
set -eu

REPO=Fankrits/zyplus-editor
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

# Match the published asset rather than guessing its filename.
pick() { printf '%s\n' "$URLS" | grep -E "$1" | head -1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; [ -n "${MNT:-}" ] && hdiutil detach "$MNT" -quiet 2>/dev/null || true' EXIT

case "$(uname -s)" in
  Darwin) URL=$(pick '\.dmg$') ;;
  Linux)
    case "$(uname -m)" in
      x86_64|amd64)  URL=$(pick '(x86_64|amd64|x64)[^/]*\.AppImage$') ;;
      aarch64|arm64) URL=$(pick '(aarch64|arm64)[^/]*\.AppImage$') ;;
      *) URL="" ;;
    esac
    ;;
  *)
    echo "Unsupported OS. On Windows run install.ps1; otherwise see" >&2
    echo "https://github.com/$REPO/releases" >&2
    exit 1
    ;;
esac

if [ -z "$URL" ]; then
  echo "No Zyplus ${VERSION:-} build published for $(uname -s) $(uname -m)." >&2
  echo "Available: https://github.com/$REPO/releases" >&2
  exit 1
fi

echo "Downloading Zyplus $VERSION" >&2
curl -fL --progress-bar -o "$TMP/dl" "$URL"

if [ "$(uname -s)" = Darwin ]; then
  MNT=$(hdiutil attach -nobrowse -readonly "$TMP/dl" |
    sed -n 's/.*\(\/Volumes\/.*\)$/\1/p' | tail -1)
  [ -d "$MNT/Zyplus.app" ] || { echo "Zyplus.app not found in dmg" >&2; exit 1; }
  rm -rf /Applications/Zyplus.app
  cp -R "$MNT/Zyplus.app" /Applications/
  xattr -dr com.apple.quarantine /Applications/Zyplus.app 2>/dev/null || true
  echo "Installed /Applications/Zyplus.app — open it with: open -a Zyplus"
else
  BIN="${ZYPLUS_BIN_DIR:-$HOME/.local/bin}"
  mkdir -p "$BIN"
  chmod +x "$TMP/dl"
  mv "$TMP/dl" "$BIN/zyplus"
  echo "Installed $BIN/zyplus"
  case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to your PATH." ;; esac
fi
