#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
  echo "This helper packages macOS only. On Windows/Linux run cargo tauri build from desktop/." >&2
  exit 1
fi
export MACOSX_DEPLOYMENT_TARGET=14.0
npm --prefix desktop run build
python3 scripts/cargo-release.py build --manifest-path desktop/Cargo.toml --release --features custom-protocol --locked
target_dir=$(./scripts/cargo-local.sh metadata --manifest-path desktop/Cargo.toml --format-version 1 --no-deps --locked | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
mkdir -p dist
staging=$(mktemp -d dist/.openroom-build.XXXXXX)
# Preserve the previous bundle if installation is interrupted during replacement.
trap 'if [ ! -d "$staging/previous.app" ]; then rm -rf "$staging"; fi' EXIT
app="$staging/OpenRoom.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target_dir/release/advanced-explorer-desktop" "$app/Contents/MacOS/OpenRoom"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>OpenRoom</string>
<key>CFBundleIdentifier</key><string>local.advanced-explorer.desktop</string>
<key>CFBundleName</key><string>OpenRoom</string>
<key>CFBundleDisplayName</key><string>OpenRoom</string>
<key>CFBundleVersion</key><string>18</string>
<key>CFBundleShortVersionString</key><string>0.7.8</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleIconName</key><string>AppIcon</string>
</dict></plist>
PLIST
cp desktop/icons/icon.icns "$app/Contents/Resources/AppIcon.icns"
cp desktop/icons/Assets.car "$app/Contents/Resources/Assets.car"
mkdir -p "$app/Contents/Resources/rtengine"
cp -R .tools/rtengine/data/. "$app/Contents/Resources/rtengine/"
python3 scripts/bundle-macos-libs.py "$app"
python3 scripts/check-license-inventory.py "$app"
mkdir -p "$app/Contents/Resources/licenses"
cp LICENSE COPYRIGHT.md DISTRIBUTION.md "$app/Contents/Resources/licenses/"
cp -R licenses "$app/Contents/Resources/licenses/third-party"
cp desktop/node_modules/react/LICENSE "$app/Contents/Resources/licenses/React-MIT.txt"
cp desktop/node_modules/react-dom/LICENSE "$app/Contents/Resources/licenses/ReactDOM-MIT.txt"
cp desktop/node_modules/scheduler/LICENSE "$app/Contents/Resources/licenses/ReactScheduler-MIT.txt"
cp native/rawler/LICENSE "$app/Contents/Resources/licenses/Rawler-LGPL-2.1.txt"
cp native/rawler/OPENROOM_PATCH.txt native/rawler/no-prefetch.patch native/rawler/camera-previews.patch "$app/Contents/Resources/licenses/"
cp THIRD_PARTY.md "$app/Contents/Resources/licenses/"
cp native/rtengine/COPYING.GPL-3.0 "$app/Contents/Resources/licenses/RawTherapee-GPL-3.0.txt"
cp native/rtengine/source.json native/rtengine/float-preview.patch "$app/Contents/Resources/licenses/"
codesign --force --deep --sign - "$app"
# Refresh the bundle timestamp so macOS invalidates its cached app icon.
touch "$app"
codesign --verify --deep --strict "$app"
python3 scripts/install-macos-app.py "$app" dist/OpenRoom.app
printf 'Built %s\n' dist/OpenRoom.app
