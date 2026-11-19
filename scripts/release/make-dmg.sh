#!/usr/bin/env bash
# Builds a compressed read-only macOS DMG containing Zyplus.app and an
# /Applications symlink for drag-and-drop installation.
#
# Usage:
#   ./make-dmg.sh <path/to/Zyplus.app> <output/Zyplus_<version>_aarch64.dmg> [volume_name]
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "make-dmg.sh: this script only runs on macOS" >&2
  exit 1
fi

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "Usage: $0 <path/to/Zyplus.app> <output.dmg> [volume_name]" >&2
  exit 2
fi

APP_PATH="$1"
OUT_DMG="$2"
VOL_NAME="${3:-Zyplus}"

if [ ! -d "$APP_PATH" ] || [ ! -f "$APP_PATH/Contents/Info.plist" ]; then
  echo "make-dmg.sh: invalid app bundle: $APP_PATH" >&2
  exit 1
fi

APP_NAME="$(basename "$APP_PATH")"
if [ "$APP_NAME" != "Zyplus.app" ]; then
  echo "make-dmg.sh: expected bundle named Zyplus.app, got: $APP_NAME" >&2
  exit 1
fi

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/zyplus-dmg.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

echo "==> staging DMG contents"
ditto "$APP_PATH" "$STAGE/Zyplus.app"
ln -s /Applications "$STAGE/Applications"

mkdir -p "$(dirname "$OUT_DMG")"
rm -f "$OUT_DMG"

echo "==> creating $OUT_DMG"
hdiutil create \
  -volname "$VOL_NAME" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  "$OUT_DMG"

echo "==> verifying $OUT_DMG"
hdiutil verify "$OUT_DMG"
echo "==> done: $OUT_DMG"
