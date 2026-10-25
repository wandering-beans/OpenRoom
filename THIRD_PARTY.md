# Media dependencies

## Desktop frontend

- React, React DOM and Scheduler: https://github.com/facebook/react (MIT). Versions are pinned in `desktop/package-lock.json`; the macOS bundle includes their license texts in `Contents/Resources/licenses/`.
- Vite (MIT), TypeScript (Apache-2.0), and Playwright (Apache-2.0) are development and test tools. They are not shipped as application runtimes.

- rawler 0.8.0 (source in `native/rawler`, local changes in `OPENROOM_PATCH.txt` and `no-prefetch.patch`): https://github.com/dnglab/dnglab/tree/main/rawler — LGPL-2.1 (see upstream LICENSE).
- libheif / libheif-rs: https://github.com/strukturag/libheif and https://github.com/Cykooz/libheif-rs . Rust wrapper uses the system libheif API (minimum 1.17).
- HEVC decode: libde265 https://github.com/strukturag/libde265 .

macOS local packages copy libheif and its linked dependencies into Contents/Frameworks. The exact libraries depend on the installed libheif build. Homebrew's build can also link encoders and additional codecs (x265, libaom, libsharpyuv). Upstream sources and licenses: https://bitbucket.org/multicoreware/x265_git/ , https://aomedia.googlesource.com/aom/ , https://chromium.googlesource.com/webm/libwebp/ .

Cargo.lock files pin Rust dependency versions. This local development package is not a completed public distribution; redistribution must include the license texts and source/relinking materials required by the chosen builds.

- MCP protocol and Streamable HTTP: rmcp 3.6.0 https://github.com/modelcontextprotocol/rust-sdk/ (Apache-2.0).
- Local HTTP routing: Axum https://github.com/tokio-rs/axum (MIT).
- OS credential storage: keyring 3.6.3 https://github.com/hwchen/keyring-rs (MIT/Apache-2.0).

- Raster image decoding: image https://github.com/image-rs/image (MIT/Apache-2.0), with PNG/JPEG/TIFF features.
- Library PSD/PSB previews: ag-psd 0.3 https://github.com/Vasyanator/ag-psd-rs (MIT).
- AVIF decode: libheif with AV1 decoder libaom; same packaged native dependency boundary as HEIF.

- Development resize: fast_image_resize 6.1.0 https://github.com/Cykooz/fast_image_resize (MIT/Apache-2.0).
- ICC conversion: moxcms 0.8.1 https://github.com/awxkee/moxcms (BSD-3-Clause/Apache-2.0).
- Geometric interpolation: imageproc 0.27.0 https://github.com/image-rs/imageproc (MIT).
- Parallel development: Rayon https://github.com/rayon-rs/rayon (MIT/Apache-2.0).

Windows/Linux development uses rawler sensor decoding and demosaicing, followed by the Rust camera-profile processing described below. macOS development uses the pinned rtengine described below. No Adobe profiles, algorithms, models, icons, or logos are bundled. The UI reference and official documentation describe intended behavior; the processing results differ from Lightroom Classic.

- Catalog recipe and Photoshop composite ZIP compression: flate2 https://github.com/rust-lang/flate2-rs (MIT/Apache-2.0). Development reads precision-preserving RGB/grayscale saved composites separately from ag-psd.

RAW color conversion implements the public DNG matrix, ForwardMatrix and HSV-table model in Rust. Installed DCP files are read locally and are not redistributed. The measured default RAW tone table contains only numeric responses to a synthetic neutral ramp.

Robertson (1968) isotemperature data: Wyszecki & Stiles, *Color Science*, table 1(3.11), as tabulated by Colour (https://github.com/colour-science/colour/blob/develop/colour/temperature/robertson1968.py), Copyright 2013 Colour Developers, BSD-3-Clause:

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

## macOS rtengine integration

RawTherapee 5.13, commit `1924179107995b01b73cdc7f4fe91e44fc8227a7`:
[upstream source](https://github.com/RawTherapee/RawTherapee/tree/1924179107995b01b73cdc7f4fe91e44fc8227a7),
[engine license notice](https://github.com/RawTherapee/RawTherapee/blob/1924179107995b01b73cdc7f4fe91e44fc8227a7/rtengine/rtengine.h).
The engine is GPL-3.0-or-later. The unmodified license text is retained in
[native/rtengine/COPYING.GPL-3.0](native/rtengine/COPYING.GPL-3.0).
The C ABI and Rust FFI crate are part of this GPL-covered integration.

`native/rtengine/source.json` pins the upstream revision.
`native/rtengine/float-preview.patch` records the engine-only CMake target,
scene-linear float access without unused display stages or buffers, explicit preview scale,
extended-color retention, a four-thread OpenMP limit inside the preview worker,
initialized X-Trans homogeneity-map halos, and ordered accumulation of overlapping
denoise tiles with serial per-tile reductions. The bridge uses tiled denoising to bound scratch memory, with at most four simultaneous noise-reduction tiles. Other full-size filters
use up to eight threads; previews use four.
`native/rtengine/bridge.cc` provides native sessions, manual detail processing,
and an initial RAW tone fitted to the embedded camera JPEG using the upstream
auto-matched curve. The bridge keeps the fitted curve monotonic and preserves
extended highlight values when applying it. Missing reference curves fall back
to the existing default RAW tone.
Camera constants and upstream ICC/DCP data are copied from the same pinned source.
No Adobe installation profiles are copied into the package.

The local app includes the license, pin and patch in `Contents/Resources/licenses`,
and data in `Contents/Resources/rtengine`. It dynamically links the native bridge
and its required dependencies from `Contents/Frameworks`. Dependencies include
GLib/GLibmm/Gio (LGPL), Little CMS 2 (MIT), FFTW (GPL), LibRaw (LGPL/CDDL),
Exiv2 (GPL), libiptcdata (LGPL), libomp (Apache-2.0 with LLVM exceptions),
and the codecs selected by the installed Homebrew builds.
Upstream notices remain authoritative: [FFTW](https://www.fftw.org/),
[LibRaw](https://www.libraw.org/), [Exiv2](https://exiv2.org/),
[Little CMS](https://www.littlecms.com/), [LLVM OpenMP](https://openmp.llvm.org/).
GTK build dependencies support upstream common code; no RawTherapee GUI target is built.

Public distribution is outside this local migration. Before distributing a linked
application, provide the complete corresponding source for that application and
covered libraries, the exact upstream revisions and local modifications, build
and installation scripts, required license notices, and any applicable relinking
materials under their respective terms. The bundled pin and patch alone are not
complete corresponding source. Public signing/notarization and a complete
redistribution inventory have not been completed.
