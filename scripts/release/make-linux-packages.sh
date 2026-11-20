#!/usr/bin/env bash
#
# Builds the Linux installers from a release binary: a .deb (double-click or
# `apt install ./x.deb`) and a single-file AppImage (chmod +x and run).
#
#   scripts/release/make-linux-packages.sh <version> <path/to/zyplus-gpui> <out-dir>
#
# Neither bundles system libraries (Vulkan, X11/Wayland, fontconfig, ...): the
# .deb declares them as dependencies and the AppImage uses the host's.
# The AppImage step downloads appimagetool, so it needs network access; set
# SKIP_APPIMAGE=1 to build only the .deb.
set -euo pipefail

VERSION="${1:?usage: make-linux-packages.sh <version> <binary> <out-dir>}"
BIN="${2:?missing binary}"
OUT="${3:?missing out dir}"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DESKTOP="$ROOT/packaging/linux/zyplus.desktop"
ICON="$ROOT/assets/icon.png"
[ -x "$BIN" ] || { echo "binary not found or not executable: $BIN" >&2; exit 1; }

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- .deb -------------------------------------------------------------------
# Debian versions sort a pre-release before its release with `~`.
TILDE='~'  # kept in a variable: a literal ~ in the substitution would expand to $HOME
DEB_VERSION="${VERSION/-/$TILDE}"
PKG="$WORK/deb"
install -Dm755 "$BIN" "$PKG/usr/bin/zyplus-gpui"
install -Dm644 "$DESKTOP" "$PKG/usr/share/applications/zyplus.desktop"
install -Dm644 "$ICON" "$PKG/usr/share/icons/hicolor/512x512/apps/zyplus.png"
mkdir -p "$PKG/DEBIAN"
SIZE_KB=$(( ($(wc -c < "$BIN") + 1023) / 1024 ))
cat > "$PKG/DEBIAN/control" <<EOF
Package: zyplus
Version: $DEB_VERSION
Section: editors
Priority: optional
Architecture: amd64
Installed-Size: $SIZE_KB
Maintainer: Fankrits <noreply@users.noreply.github.com>
Homepage: https://github.com/Fankrits/zyplus-editor
Depends: libc6, libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libx11-6, libx11-xcb1, libxcb1, libvulkan1
Description: A markdown editor that gets out of the way
 Zyplus is a local-first markdown editor: a project tree, tabs, a rendered
 preview, and your files stay on your machine.
EOF
DEB="$OUT/Zyplus_${VERSION}_linux-amd64.deb"
dpkg-deb --root-owner-group --build "$PKG" "$DEB" >/dev/null
echo "built $DEB"

# --- AppImage ---------------------------------------------------------------
if [ "${SKIP_APPIMAGE:-0}" != "1" ]; then
  APPDIR="$WORK/Zyplus.AppDir"
  install -Dm755 "$BIN" "$APPDIR/usr/bin/zyplus-gpui"
  install -Dm644 "$DESKTOP" "$APPDIR/zyplus.desktop"
  install -Dm644 "$ICON" "$APPDIR/zyplus.png"
  install -Dm644 "$ICON" "$APPDIR/usr/share/icons/hicolor/512x512/apps/zyplus.png"
  ln -s usr/bin/zyplus-gpui "$APPDIR/AppRun"
  ln -s zyplus.png "$APPDIR/.DirIcon"

  TOOL="$WORK/appimagetool"
  curl -fsSL -o "$TOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "$TOOL"
  APPIMAGE="$OUT/Zyplus_${VERSION}_linux-x86_64.AppImage"
  # No FUSE on CI runners: run the tool by extracting itself first.
  ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" "$APPIMAGE" >/dev/null
  chmod +x "$APPIMAGE"
  echo "built $APPIMAGE"
fi
