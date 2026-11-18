#!/usr/bin/env bash
#
# Builds Zyplus (the gpui app) for macOS and assembles Zyplus.app without
# cargo-bundle. It needs only cargo and the tools macOS ships with
# (plutil, codesign).
#
#   ./bundle-macos.sh                 -> <target>/release/bundle/osx/Zyplus.app
#   ./bundle-macos.sh /some/dir       -> /some/dir/Zyplus.app
#
# Environment overrides:
#   CARGO_TARGET_DIR  build output directory      (default: <crate>/target)
#   ZYPLUS_ICONS_DIR  directory holding icon.icns (default: <crate>/../src-tauri/icons)
#   SIGN_IDENTITY     codesign identity           (default: "-", ad-hoc)
#   SKIP_BUILD=1      reuse the release binary already in the target directory
#
# The CFBundleDocumentTypes block below mirrors bundle/macos-document-types.plist,
# which cargo-bundle merges via osx_info_plist_exts. Change both together.
# Both mirror fileAssociations in src-tauri/tauri.conf.json.
#
# Gatekeeper and default-app claims: an ad-hoc-signed app runs on the machine
# that built it. LaunchServices only accepts a default-app claim for an app it
# can find, so copy the bundle to /Applications before using Settings > Default app.
set -euo pipefail

APP_NAME="Zyplus"
BIN_NAME="zyplus-gpui"
BUNDLE_ID="com.fankrits.zyplus-editor"
MIN_MACOS="10.15"
CATEGORY="public.app-category.productivity"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "bundle-macos.sh: this script only runs on macOS" >&2
  exit 1
fi

CRATE_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$CRATE_DIR/target}"
ICONS_DIR="${ZYPLUS_ICONS_DIR:-$CRATE_DIR/../src-tauri/icons}"
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
OUT_ROOT="${1:-$TARGET_DIR/release/bundle/osx}"

# The version is the one under [package], not the one in [package.metadata.bundle].
VERSION="$(awk -F'"' '/^\[/{sec=$0} /^version = /{ if (sec == "[package]") { print $2; exit } }' "$CRATE_DIR/Cargo.toml")"
if [ -z "$VERSION" ]; then
  echo "bundle-macos.sh: could not read the package version from Cargo.toml" >&2
  exit 1
fi

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  echo "==> cargo build --release"
  cargo build --release --manifest-path "$CRATE_DIR/Cargo.toml"
fi

BIN="$TARGET_DIR/release/$BIN_NAME"
if [ ! -x "$BIN" ]; then
  echo "bundle-macos.sh: release binary not found: $BIN" >&2
  exit 1
fi
ICNS="$ICONS_DIR/icon.icns"
if [ ! -f "$ICNS" ]; then
  echo "bundle-macos.sh: icon not found: $ICNS (set ZYPLUS_ICONS_DIR)" >&2
  exit 1
fi

APP="$OUT_ROOT/$APP_NAME.app"
echo "==> assembling $APP (version $VERSION)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/$BIN_NAME"
chmod 755 "$APP/Contents/MacOS/$BIN_NAME"
cp "$ICNS" "$APP/Contents/Resources/icon.icns"
printf 'APPL????' > "$APP/Contents/PkgInfo"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>English</string>
	<key>CFBundleDisplayName</key>
	<string>$APP_NAME</string>
	<key>CFBundleExecutable</key>
	<string>$BIN_NAME</string>
	<key>CFBundleIconFile</key>
	<string>icon.icns</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>$APP_NAME</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
	<key>LSApplicationCategoryType</key>
	<string>$CATEGORY</string>
	<key>LSMinimumSystemVersion</key>
	<string>$MIN_MACOS</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Markdown</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>md</string>
				<string>markdown</string>
				<string>mdown</string>
				<string>mkd</string>
			</array>
			<key>LSItemContentTypes</key>
			<array>
				<string>net.daringfireball.markdown</string>
			</array>
			<key>LSHandlerRank</key>
			<string>Default</string>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Plain Text</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>txt</string>
			</array>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.plain-text</string>
			</array>
			<key>LSHandlerRank</key>
			<string>Default</string>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>CSV</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>csv</string>
			</array>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.comma-separated-values-text</string>
			</array>
			<key>LSHandlerRank</key>
			<string>Default</string>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>TSV</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>tsv</string>
			</array>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.tab-separated-values-text</string>
			</array>
			<key>LSHandlerRank</key>
			<string>Default</string>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>JSON</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>json</string>
			</array>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.json</string>
			</array>
			<key>LSHandlerRank</key>
			<string>Default</string>
		</dict>
	</array>
</dict>
</plist>
PLIST

plutil -lint "$APP/Contents/Info.plist"

echo "==> codesign ($SIGN_IDENTITY)"
if [ "$SIGN_IDENTITY" = "-" ]; then
  # Sign the executable first, then the bundle that contains it.
  codesign --force --sign - "$APP/Contents/MacOS/$BIN_NAME"
  codesign --force --sign - "$APP"
else
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP/Contents/MacOS/$BIN_NAME"
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"

echo "==> done: $APP"
echo "    run:     open \"$APP\""
echo "    install: cp -R \"$APP\" /Applications/   (needed before a default-app claim works)"
