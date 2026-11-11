#!/bin/bash
# Package the exact app already distributed in the release ZIP.
set -euo pipefail
archive=$1
scripts=$(cd "$(dirname "$0")" && pwd)
name=$(basename "$archive" .zip)
[[ $name =~ ^OpenRoom-([0-9]+\.[0-9]+\.[0-9]+)-macos-arm64$ ]]
version=${BASH_REMATCH[1]}
output=$(dirname "$archive")/$name.dmg
work=$(mktemp -d)
mounted=false
cleanup() {
  if [[ $mounted == true ]]; then hdiutil detach "$work/mount" || return 1; fi
  rm -rf "$work"
}
trap cleanup EXIT

mkdir "$work/unpack" "$work/mount"
ditto -x -k "$archive" "$work/unpack"
app=$work/unpack/OpenRoom.app
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")" = "$version"
test "$(lipo -archs "$app/Contents/MacOS/OpenRoom")" = arm64
codesign --verify --deep --strict "$app"
art="$scripts/../assets/branding/macos-install-arrow.tiff"
dmgbuild -s "$scripts/macos-dmg-settings.py" -D app="$app" -D background="$art" OpenRoom "$work/distribution.dmg"
hdiutil verify "$work/distribution.dmg"
hdiutil attach -readonly -nobrowse -mountpoint "$work/mount" "$work/distribution.dmg"
mounted=true
test -s "$work/mount/.DS_Store"
cmp "$art" "$work/mount/.background.tiff"
test "$(readlink "$work/mount/Applications")" = /Applications
test -d "$work/mount/Applications"
codesign --verify --deep --strict "$work/mount/OpenRoom.app"
cmp "$app/Contents/MacOS/OpenRoom" "$work/mount/OpenRoom.app/Contents/MacOS/OpenRoom"
hdiutil detach "$work/mount"
mounted=false
mv "$work/distribution.dmg" "$output"
