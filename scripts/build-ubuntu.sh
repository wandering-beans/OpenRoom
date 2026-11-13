#!/usr/bin/env bash
# Build from the repository root with the dependencies in ubuntu-release-build.yml.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -sm) == 'Linux x86_64' ]]
# Match the metadata reader used by the macOS release (notably Fuji MakerNotes).
deps="$PWD/.tools/linux-deps"
mkdir -p "$deps"
if [[ ! -f "$deps/lib/pkgconfig/exiv2.pc" ]]; then
  archive="$deps/exiv2-0.28.9.tar.gz"
  curl -fLsS --retry 3 https://github.com/Exiv2/exiv2/archive/refs/tags/v0.28.9.tar.gz -o "$archive"
  echo "700b76b97695b2fab4ef8c79619c68ae57d09e0c130724791cafbd39e0eb4aef  $archive" | sha256sum -c -
  tar -xf "$archive" -C "$deps"
  cmake -S "$deps/exiv2-0.28.9" -B "$deps/build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$deps" -DCMAKE_INSTALL_LIBDIR=lib -DEXIV2_BUILD_EXIV2_COMMAND=OFF \
    -DEXIV2_BUILD_SAMPLES=OFF -DEXIV2_BUILD_UNIT_TESTS=OFF
  cmake --build "$deps/build" --parallel 4
  cmake --install "$deps/build"
fi
export PKG_CONFIG_PATH="$deps/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LD_LIBRARY_PATH="$deps/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
python3 scripts/build-rtengine.py
export LD_LIBRARY_PATH="$PWD/.tools/rtengine/build-linux${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
notices=.tools/linux-notices
mkdir -p "$notices" dist/ubuntu
mkdir -p .tools/linux-heif "$notices/ubuntu"
for pkg in libheif-plugin-aomenc libheif-plugin-dav1d libheif-plugin-libde265; do
  plugin=$(dpkg-query -L "$pkg" | grep -E '/libheif/[^/]+\.so$')
  cp "$plugin" .tools/linux-heif/
  cp -L "/usr/share/doc/$pkg/copyright" "$notices/ubuntu/$pkg-copyright"
done
cp .tools/rtengine/fmt/LICENSE "$notices/fmt-LICENSE.txt"
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
