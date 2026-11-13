#!/usr/bin/env bash
# Build from the repository root with the dependencies in ubuntu-release-build.yml.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -sm) == 'Linux x86_64' ]]
python3 scripts/build-rtengine.py
export LD_LIBRARY_PATH="$PWD/.tools/rtengine/build-linux${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
notices=.tools/linux-notices
mkdir -p "$notices" dist/ubuntu
cp LICENSE COPYRIGHT.md DISCLAIMER.md TERMS.md THIRD_PARTY.md "$notices/"
cp -R licenses "$notices/"
cp native/rtengine/{COPYING.GPL-3.0,source.json,*.patch} "$notices/"
cp native/rawler/{OPENROOM_PATCH.txt,*.patch} "$notices/"
cp native/rawler/LICENSE "$notices/Rawler-LGPL-2.1.txt"
for pkg in react react-dom scheduler; do
  cp "desktop/node_modules/$pkg/LICENSE" "$notices/$pkg-LICENSE.txt"
done
(cd desktop && npm run tauri -- build --bundles appimage -- --locked)
target=$(cargo metadata --manifest-path desktop/Cargo.toml --no-deps --format-version 1 | jq -r .target_directory)
version=$(node -p "require('./desktop/tauri.conf.json').version")
cp "$target/release/bundle/appimage/OpenRoom_${version}_amd64.AppImage" "dist/ubuntu/OpenRoom-$version-linux-x86_64.AppImage"
(cd dist/ubuntu && sha256sum "OpenRoom-$version-linux-x86_64.AppImage" > SHA256SUMS.txt)
