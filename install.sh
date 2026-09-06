#!/bin/sh
# Zyplus installer:  curl -fsSL https://zyplus.sh | sh
# Or pin a version:  ZYPLUS_VERSION=0.1.0 sh install.sh
set -eu

REPO=Fankrits/zyplus-editor
VERSION="${ZYPLUS_VERSION:-}"

if [ -z "$VERSION" ]; then
  VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
    sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)
fi
[ -n "$VERSION" ] || { echo "could not determine latest version" >&2; exit 1; }

dl() { echo "Downloading $1" >&2; curl -fL --progress-bar -o "$2" \
  "https://github.com/$REPO/releases/download/v$VERSION/$1"; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; [ -n "${MNT:-}" ] && hdiutil detach "$MNT" -quiet 2>/dev/null || true' EXIT

case "$(uname -s)" in
  Darwin)
    dl "Zyplus_${VERSION}_universal.dmg" "$TMP/zyplus.dmg"
    MNT=$(hdiutil attach -nobrowse -readonly "$TMP/zyplus.dmg" |
      sed -n 's/.*\(\/Volumes\/.*\)$/\1/p' | tail -1)
    [ -d "$MNT/Zyplus.app" ] || { echo "Zyplus.app not found in dmg" >&2; exit 1; }
    rm -rf /Applications/Zyplus.app
    cp -R "$MNT/Zyplus.app" /Applications/
    xattr -dr com.apple.quarantine /Applications/Zyplus.app 2>/dev/null || true
    echo "Installed /Applications/Zyplus.app — open it with: open -a Zyplus"
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) ;;
      *) echo "only x86_64 Linux builds are published" >&2; exit 1 ;;
    esac
    BIN="${ZYPLUS_BIN_DIR:-$HOME/.local/bin}"
    mkdir -p "$BIN"
    dl "Zyplus_${VERSION}_amd64.AppImage" "$TMP/zyplus"
    chmod +x "$TMP/zyplus"
    mv "$TMP/zyplus" "$BIN/zyplus"
    echo "Installed $BIN/zyplus"
    case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to your PATH." ;; esac
    ;;
  *)
    echo "Unsupported OS. Download from https://github.com/$REPO/releases" >&2
    exit 1
    ;;
esac
