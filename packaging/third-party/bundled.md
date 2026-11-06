## Source code offer

Castor is free software under the **GNU General Public License v3.0 or later** (the full text is
in `LICENSE.txt` next to the application). Its complete source code, including the build and
packaging scripts used to produce this package, is at
**https://github.com/basedpolymer/castor** (each release is tagged `vX.Y.Z`; GitHub attaches
its source archives to the release).

The complete corresponding source of the third-party programs bundled with Castor — FFmpeg
exactly as built for Windows and for macOS, the build scripts that produced those binaries
(which pin every library version they compile in), and whisper.cpp — is **attached to every
Castor release on https://github.com/basedpolymer/castor/releases** as `*-source.tar.gz`
archives with a `SHA256SUMS-sources.txt` file (made by `packaging/sources.sh` from the commits
pinned in `packaging/pins.env`). The upstream locations are listed below as well. If anything
is missing, open an issue on the repository and we will provide it.

## Programs bundled with Castor

| component | version | licence | where it is | source |
|---|---|---|---|---|
| FFmpeg + ffprobe (Windows: BtbN "gpl-shared" build) | 9.0.2 (n9.0.2-39-g7adfbe40dc, autobuild 2026-10-31) | GPL-3.0-or-later (GPL build; includes x264, x265, SVT-AV1, libvpx, libaom, dav1d, Opus, LAME, Vorbis, libwebp, libass, FreeType, HarfBuzz, FriBidi, zimg, vid.stab and others, see the build scripts) | `bin\` | https://github.com/FFmpeg/FFmpeg/tree/7adfbe40dcde598f93a6963d2f7b22cd75202ccc (release/9.0) · build scripts and library versions: https://github.com/BtbN/FFmpeg-Builds/tree/autobuild-2026-10-31-12-59 |
| FFmpeg + ffprobe (macOS: martin-riedl.de static build) | 9.0.2 | GPL-3.0-or-later (`--enable-gpl --enable-version3`; includes x264, x265, SVT-AV1, libvpx, libaom, dav1d, rav1e, vvenc, OpenH264, Opus, LAME, Vorbis, Theora, libwebp, libass, FreeType, HarfBuzz, FriBidi, fontconfig, zimg, libvmaf, libbluray, SRT, OpenSSL 3, libxml2, snappy, zvbi, libklvanc) | `Contents/Resources/bin` | https://github.com/FFmpeg/FFmpeg/tree/n9.0.2 · build scripts: https://git.martin-riedl.de/ffmpeg/build-script (commit f63b8aab) · versions: https://ffmpeg.martin-riedl.de |
| whisper.cpp `whisper-cli` + ggml | v1.9.5 (`d1be6fd`) | MIT | `bin/` | https://github.com/ggml-org/whisper.cpp/tree/v1.9.5 |
| ONNX Runtime (Windows, Apple Silicon: statically linked through the `ort` crate, pyke build) | 1.31.0 | MIT | inside the executables | https://github.com/microsoft/onnxruntime · https://github.com/pykeio/ort |
| ONNX Runtime (Intel Macs, dynamic library) | 1.23.2 | MIT | `Contents/Frameworks/libonnxruntime.dylib` | https://github.com/microsoft/onnxruntime/releases/tag/v1.23.2 |
| MuPDF (through the `mupdf` / `mupdf-sys` crates) | 1.27.2 | AGPL-3.0-or-later | statically linked | https://github.com/ArtifexSoftware/mupdf · https://github.com/messense/mupdf-rs |
| MuPDF's bundled libraries: FreeType, HarfBuzz, libjpeg, OpenJPEG, jbig2dec, lcms2 (mt), zlib, Brotli, Gumbo, extract | as shipped in MuPDF 1.27.2 | FreeType License (FTL), MIT, IJG, BSD-2-Clause, AGPL-3.0, MIT, Zlib, MIT, Apache-2.0, AGPL-3.0 | statically linked | https://github.com/ArtifexSoftware/mupdf/tree/1.27.2/thirdparty |
| Tesseract OCR + Leptonica (inside MuPDF) | as shipped in MuPDF 1.27.2 | Apache-2.0 / BSD-2-Clause | statically linked | https://github.com/tesseract-ocr/tesseract · https://github.com/DanBloomberg/leptonica |
| Tesseract "GlyphLessFont" | from Tesseract 5 | Apache-2.0 | compiled in | https://github.com/tesseract-ocr/tesseract |
| Magika content-type model | from the `magika` crate 1.1.0 | Apache-2.0 | compiled in | https://github.com/google/magika |
| Velopack updater (`Update.exe`, `UpdateMac`, setup stub) | 1.3.39 | MIT | next to the application | https://github.com/velopack/velopack |

The Windows installer installs the Microsoft Visual C++ 2015–2022 Redistributable (x64) from
Microsoft if it is missing; it is not part of the Castor package.

## Fonts and artwork

| component | version | licence | source |
|---|---|---|---|
| Latin Modern (Sans, Roman, Roman Caps, Slanted, Mono) | 2.004 | GUST Font License (`assets/fonts/GUST-FONT-LICENSE.txt`) | https://www.gust.org.pl/projects/e-foundry/latin-modern |
| TeX Gyre Chorus | 2.003 | GUST Font License | https://www.gust.org.pl/projects/e-foundry/tex-gyre |
| New Computer Modern Math | as in `crates/tools-create/assets/fonts` | GUST Font License | https://ctan.org/pkg/newcomputermodern |
| Lucide icons (through gpui-kit-assets) | as in gpui-kit-assets 0.9.0 | ISC | https://lucide.dev |
| Platform badges (`assets/badges`, repository and website only) | — | MIT (`assets/badges/LICENSE`) | — |
| Castor brand icons (`assets/brand`) | — | part of Castor (GPL-3.0-or-later) | this repository |

## Downloaded on first use (never bundled; Castor asks first and shows the size)

| component | licence | source |
|---|---|---|
| Tesseract `tessdata_fast` language data (pinned commit `87416418`) | Apache-2.0 | https://github.com/tesseract-ocr/tessdata_fast |
| BiRefNet-lite ONNX (background removal) | MIT | https://huggingface.co/onnx-community/BiRefNet_lite-ONNX |
| ISNet general-use ONNX (background removal, fast) | Apache-2.0 | https://github.com/danielgatis/rembg/releases/tag/v0.0.0 |
| Real-ESRGAN x4plus / x2plus / x4plus-anime-6B, realesr-general-x4v3 ONNX (upscaling) | BSD-3-Clause | https://github.com/xinntao/Real-ESRGAN (ONNX exports pinned in `crates/engine-ml/src/models.rs`) |
| YuNet 2023mar ONNX (face detection) | MIT | https://huggingface.co/opencv/face_detection_yunet |
| Whisper ggml models (tiny … large-v3-turbo) | MIT | https://huggingface.co/ggerganov/whisper.cpp |
| Silero VAD v6.2.0 (ggml) | MIT | https://huggingface.co/ggml-org/whisper-vad |
| RNNoise model `sh.rnnn` | no copyright claimed by its author | https://github.com/GregorR/rnnoise-models |
| yt-dlp | Unlicense | https://github.com/yt-dlp/yt-dlp |

## Used if installed, never shipped

LibreOffice (MPL-2.0) for Office → PDF; Chrome / Edge / Brave / Chromium for HTML → image/PDF.
