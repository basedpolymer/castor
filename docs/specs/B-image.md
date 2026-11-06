# Spec B — ML engine and image tools

Owner: agent B (wave 1). You own `crates/engine-ml`, `crates/tools-image`, `locales/image.yml`,
this file. Build dir: `CARGO_TARGET_DIR=target-b`. Read `CLAUDE.md`, `docs/ARCHITECTURE.md`,
`docs/TOOLS.md` first, and `crates/tools-files/src/checksum.rs` (reference tool).

## Mission

Ship every image tool of `docs/TOOLS.md` § Image **except** `image.to-pdf`,
`image.images-to-pdf`, `image.extract-text` (agent A does those). 22 tools. Quality bar: as good
as or better than Hangar, Squoosh, TinyPNG, remove.bg, Photoshop's export; batch of 500 photos
must run at full CPU without blowing memory.

## Engine choices (decide, verify, record)

GPL-3.0-or-later: GPL/AGPL/LGPL fine. Recommended:
- Decoding/encoding: `image` 0.25, `zune-jpeg`, `mozjpeg` (JPEG encoding), `oxipng` (PNG),
  **`imagequant`** (libimagequant, GPL-3) for lossy PNG/GIF palettes, `webp` (libwebp, lossy +
  lossless), `ravif`/`rav1e` (AVIF), `jxl-oxide` (JPEG XL decode), `resvg` (SVG raster),
  HEIC/HEIF: `libheif-rs` (LGPL libheif; check how to build/bundle on Windows and Intel Mac) or
  the OS (ImageIO via objc2 on macOS, WIC on Windows) — choose what ships reliably;
  RAW: `rawler` (LGPL) or LibRaw; ICO/ICNS writers (`ico`, `icns` crates).
- `fast_image_resize` (SIMD) for every resize; `kamadak-exif`/`little_exif`/`img-parts` for
  metadata without recompression; XMP writing for `image.seo`.
- `vtracer` for `image.vectorize`; **`dssim`** (AGPL) for "visually identical" compression
  targets; **`gifski`** (AGPL) for `image.optimize-gif`.
- ML: ONNX models. `ort` (ONNX Runtime) is the default choice — BUT Microsoft no longer
  publishes ONNX Runtime builds for Intel Macs and our test Mac is Intel: verify `ort` on
  x86_64-apple-darwin (pyke prebuilt? `load-dynamic` with a self-built dylib?) early. If it is
  not viable, use a pure-Rust backend (`tract-onnx`, or `candle`) for every model, or for Intel
  Mac only. Decide with numbers (speed of background removal on a 12 MP photo, Windows + Mac).
- Models (licences must allow redistribution and commercial use — NOT RMBG-1.4/2.0):
  background removal: BiRefNet (MIT; lite/general variants) and/or ISNet-general-use / U²-Net
  (Apache-2.0) — measure quality on hair, product shots, logos, and keep small detached marks;
  upscaling: Real-ESRGAN x4plus + x4plus-anime (BSD-3), ×2 via downscale or RealESRGAN_x2plus;
  faces for `image.redact`: a permissively licensed detector (e.g. YuNet, Apache/MIT; or
  UltraFace). Host URLs must be stable (GitHub releases, Hugging Face resolve URLs with commit
  hash), pinned SHA-256, size known.

## Public API of engine-ml

```rust
pub mod models {
    /// Catalog: id, display name, asset (url, sha256, size, licence).
    pub fn catalog() -> &'static [Model];
    pub fn asset(id: &str) -> Option<castor_core::download::Asset>;
}
pub fn remove_background(rgba: &image::RgbaImage, model: &str, cx: &castor_core::RunCx) -> anyhow::Result<image::GrayImage /* alpha matte */>;
pub fn upscale(rgb: &image::RgbImage, scale: u32, model: &str, cx: &castor_core::RunCx) -> anyhow::Result<image::RgbImage>;
pub fn detect_faces(rgb: &image::RgbImage, cx: &castor_core::RunCx) -> anyhow::Result<Vec<[f32; 4]>>;
```
Tiled inference for large images (no OOM on 48 MP), progress per tile, cancel between tiles.
A session cache so a batch loads the model once (thread-safe; parallel jobs share it).

`image.redact` may use `engine_pdf::ocr::recognize(rgba, w, h, langs, cx)` (agent A implements
it; signature in `docs/specs/A-pdf.md`) for "redact text matching…". If it is not ready while you
work, build the rest and leave that option behind `ToolStatus::Beta`/a clear error.

## Tool notes (beyond docs/TOOLS.md)

- Option keys used by chain templates (keep them): `image.resize` → `mode` (choices include
  `fit`), `size` (Size), `preset` (choices include `instagram-portrait`); `image.convert` →
  `format` (choices include `jpg`); `image.compress` and `image.rotate` work with defaults.
- `image.compress`: per format: JPEG (mozjpeg, progressive, chroma subsampling auto), PNG
  (oxipng + optional imagequant lossy), WebP, AVIF, GIF; modes quality / target size /
  visually identical (dssim threshold); keep metadata option; **never output a bigger file**
  (copy original + note instead).
- `image.convert`: animated GIF/WebP stay animated when the target supports it; alpha handled
  (background colour option for JPG); colour profiles: convert to sRGB, keep ICC option.
- `image.resize`: presets (Instagram square/portrait/story, X/Twitter, LinkedIn, Facebook cover,
  YouTube thumbnail, Etsy, Amazon, Shopify, App Store/Play screenshots, 4K/1080p, A4@300dpi…),
  fill with focal point (9-point) or auto focus (saliency: simple edge/entropy based or model).
- `image.beautify`: generate the 20+ painted backgrounds procedurally (mesh gradients, grain) —
  no external images; window frame (macOS-like / Windows-like / none), shadow, radius, padding,
  canvas ratio presets; `preview` required.
- `image.watermark`, `image.stylize`, `image.crop`, `image.palette`, `image.beautify`,
  `image.compare`, `image.colorblind`: implement `preview`.
- `image.compare`: output HTML (self-contained, inline base64 images, slider), GIF (wipe
  animation), side-by-side PNG/JPEG; 6 looks (labels, divider styles).
- `image.contact-sheet`: grid options, header (title, date), captions (name, size, EXIF date),
  output PNG/JPEG/PDF (for PDF you may write it with `krilla` or `printpdf`; no dependency on
  agent A needed).
- `image.icon-pack`: ICO (16–256), ICNS, favicon.ico + PNGs + `site.webmanifest`, iOS AppIcon
  set (Contents.json), Android mipmaps, padding/background options.
- `image.seo`: slugified names from a pattern (`{title}-{n}`), title/alt/keywords/author/
  copyright/GPS written to XMP/IPTC/EXIF; "Write with AI" behind an opt-in toggle using the
  user's API key from Settings (`Settings::ai_api_key`; Anthropic Messages API, model
  `claude-haiku-5-5`, small image) — if no key, the option explains how to add one.
- Implement `downloads()` for ML tools.

## Testing

Fixtures in `tests/fixtures/generated/image/` (gitignored): generate synthetic images, and fetch
a few CC0 photos (Unsplash/Pexels CC0 or Wikimedia public domain: portrait with hair, product on
white, logo, screenshot, scanned text, animated GIF, HEIC sample, RAW DNG sample). Integration
tests in `crates/tools-image/tests/`. Look at your outputs (open the PNGs with the Read tool) —
background removal and upscaling quality must be judged visually. Build on the Mac once.

## Report (fill in before finishing)

### Status per tool

All 22 tools are registered in `tools_image::tools()` with en + fr strings (`locales/image.yml`,
465 keys). `cargo test -p engine-ml -p tools-image` runs 5 + 21 unit tests and 24 integration
tests (`crates/tools-image/tests/tools.rs`: every tool on synthetic files, previews, corrupt and
empty inputs, unicode names, undo of `image.duplicates`, our JPEGs re-read by our own decoder) —
all 51 pass. `castor-cli` could not be linked at the end of the session because `tools-create`
(another owner) did not compile; every tool was instead run through the real runner with
`cargo run -p tools-image --example run -- <tool-id> files… --set k=v --out dir` (same syntax as
`castor-cli run`, `… --example run -- list` lists the 22 tools) on the fixtures in
`tests/fixtures/generated/image/`.

| id | status | notes |
|---|---|---|
| `image.remove-background` | ✅ | BiRefNet-lite ("Best", default) or ISNet ("Fast"); 1024² prediction + full-resolution guided-filter refinement + colour decontamination (no halo); transparent / colour / blurred background; crop to subject; small detached parts kept (no largest-blob filter); preview once the model is present. Checked visually on a 20 MP portrait with long hair: BiRefNet clean (hair strands, white sweater on a white sky); ISNet loses the sweater (model limit). |
| `image.upscale` | ✅ | ×2/×3/×4; "Fast" = realesr-general-x4v3 (default), "Photo" = RealESRGAN x4plus (x2plus for ×2), "Art" = x4plus-anime-6B. Even tiles ≤ 384 px (256 on tract) with 16 px reflected context, progress per tile, cancel between tiles, alpha upscaled separately, output capped at 400 MP. No visible seams. |
| `image.convert` | ✅ | In: JPEG/PNG/APNG/GIF/WebP/BMP/TIFF/ICO/ICNS/TGA/QOI/DDS/PNM/EXR/HDR (image crate), HEIC + AVIF (`heic`, pure Rust, AV1 via rav1d-safe), JXL (jxl-oxide), SVG (resvg), RAW (rawler, untested: no DNG sample fetched). Out: JPG (zenjpeg), PNG (oxipng), WebP (libwebp), AVIF (ravif), GIF (imagequant; gifski for animations), TIFF, BMP, ICO, + HEIC on macOS via `sips`. Animated GIF/WebP stay animated; ICC → sRGB (moxcms) or kept; EXIF/XMP kept with orientation reset to 1; alpha flattened on a chosen colour for JPG. |
| `image.vectorize` | ✅ | vtracer; auto / logo / B&W / illustration / photo presets, colour precision, detail, smooth or sharp curves; preview. |
| `image.compress` | ✅ | 12 MP photos: −55…−58 % in "visually identical" (13–16 s each), −48…−58 % at the default quality (2 s). Modes: quality / visually identical (DSSIM ≤ 0.0012, binary search) / target size; PNG palette (libimagequant) + oxipng; animated GIF through gifski; never bigger (original copied + note); orientation and ICC always kept, other metadata optional. AVIF "identical" uses a fixed q ≥ 70 (no AVIF decode in the measuring loop). |
| `image.palette` | ✅ | k-means++ in CIE Lab on a 200 px sample; swatches PNG + ASE + GPL + CSS + JSON; order by share / hue / lightness; preview. |
| `image.resize` | ✅ | fit / fill (auto focus = saliency, or 9-point anchor) / pad / exact / percent; 22 presets incl. `instagram-portrait`; never enlarges unless asked; SIMD resize; JPEG re-saved at its estimated original quality. |
| `image.crop` | ✅ | ratio presets with auto focus or anchor, margins in px or %, auto-trim of uniform borders (tolerance, padding); preview dims what goes. |
| `image.rotate` | ✅ | 90/180/270, flips, "upright only" (EXIF), straighten −45…45° (crop to fill or expand); orientation tag reset after rotating pixels. Re-encodes (no lossless JPEG transform). |
| `image.strip-metadata` | ✅ | JPEG/PNG/WebP without recompression (scan data byte-identical, tested); keeps orientation (minimal EXIF), ICC, optionally author/copyright; TIFF/BMP/TGA/QOI re-encoded losslessly; other formats → clear error. Note lists what was removed (GPS flagged). |
| `image.seo` | ✅ | slug names from a pattern (`{title}`, `{name}`, `{n}`, `{date}`), XMP (dc, Iptc4xmpCore alt text, exif GPS, CreatorTool = Castor) + IPTC-IIM for JPEG, no recompression. "Write with AI" is opt-in, uses `Settings::ai_api_key` (Messages API, `claude-haiku-5-5`, 768 px JPEG, JSON-schema output); not exercised against the live API (no key here). |
| `image.redact` | ✅ | Faces (YuNet, multi-scale tiles: 8 of ~10 people on a 3840 px group photo, profile/hat-shaded faces can be missed), manual regions, blur / pixelate / box, metadata always dropped. Text terms + e-mails/phones through `engine_pdf::ocr::recognize` (agent A): on a test page "Falcon", an e-mail and a phone number were boxed (3 areas). |
| `image.watermark` | ✅ | text (system fonts via fontdb/resvg) or logo (PNG/SVG…), single anchored or tiled, size, opacity, angle, soft shadow; preview with or without a file. |
| `image.beautify` | ✅ | 22 procedural painted backgrounds (warped mesh gradients + grain) + solid + transparent; padding, radius, soft/strong shadow, macOS / Windows frame light/dark, canvas ratios; preview with or without a file. |
| `image.icon-pack` | ✅ | favicon.ico + PNGs + `site.webmanifest` + HTML snippet, icon.ico (16–256), icon.icns (10 RGBA types), iOS `AppIcon.appiconset` (18 opaque PNGs + Contents.json), Android mipmaps + Play icon; padding, background, radius. Output is a folder. |
| `image.contact-sheet` | ✅ | N → 1; A4/Letter grid, title + date header, captions (name / name + size + EXIF date), watermark; PDF (minimal hand-written PDF, one JPEG per page at 300 dpi), PNG or JPG pages. |
| `image.compare` | ✅ | pairs (1-2, 3-4…); self-contained HTML slider (inline base64, range input), GIF wipe animation (gifski), PNG/JPG side-by-side / stacked / split; 6 looks. |
| `image.stylize` | ✅ | 11 looks (film, vintage, noir, duotone, halftone, sketch, posterize, cool, warm, fade, pixel) with intensity; preview. |
| `image.duplicates` | ✅ | SHA-256 + 256-bit gradient hash, union-find groups, aspect-ratio guard; HTML report with thumbnails; optional move of extra copies to a "Duplicates" folder, undoable (`in_place`, journal). Pairwise compare is O(n²) on hashes: fine to a few thousand files. |
| `image.optimize-gif` | ✅ | gifski re-encode, max width, frame-rate cap (durations preserved), never bigger. |
| `image.optimize-svg` | ✅ | drops editor namespaces (Inkscape, Sodipodi, Illustrator, Sketch, Figma), metadata, comments, empty groups; rounds numbers in geometry attributes; minifies; the result must re-render with resvg or the original is kept. |
| `image.colorblind` | ✅ | Machado 2009 simulations (protan/deutan/tritan) + achromatopsia, severity, labelled overview grid; text report of dominant colour pairs that become confusable (ΔE). |

### Benchmarks (machine, image, time)

Windows: 6 cores / 32 GB, shared with three other agents compiling during the runs (pessimistic
numbers), dev profile with dependencies at opt-level 2. Intel Mac: i5, 8 GB, release, also loaded
by another agent's build. Times include pre/post-processing, exclude decode and model download.

| task | machine / backend | input | time |
|---|---|---|---|
| remove-background ISNet | Windows / ONNX Runtime 1.31 | 3648×5472 (20 MP) | 2.4 s (3.2 s first run) |
| remove-background BiRefNet-lite | Windows / ONNX Runtime 1.31 | 20 MP | 11.5 s (26 s first run incl. session creation) |
| remove-background ISNet | Windows / tract | 20 MP | 7.1 s |
| remove-background BiRefNet-lite | Windows / tract | 20 MP | 86 s |
| remove-background ISNet | Intel Mac / ONNX Runtime 1.23.2 (load-dynamic) | 20 MP | 8.6 s |
| remove-background ISNet | Intel Mac / tract | 20 MP | 8.3 s |
| remove-background BiRefNet-lite | Intel Mac / ONNX Runtime 1.23.2 | 20 MP | 73 s (193 s on a second run under memory pressure) |
| upscale ×4 general-v3 ("Fast") | Windows / ORT | 480×319 | 12.7 s |
| upscale ×4 general-v3 ("Fast") | Intel Mac / ORT 1.23.2 | 320×480 | 4.7 s |
| upscale ×4 x4plus ("Photo") | Windows / ORT (first tiling, ~2× overlap waste, fixed since) | 480×319 | 193 s |
| upscale ×2 x2plus | Windows / ORT (first tiling) | 480×319 | 57 s |
| upscale ×4 anime-6B | Windows / ORT (first tiling) | 536×211 | 31 s |
| detect faces (YuNet + tiles) | Windows / ORT | 3840×2560, 8 faces | 1.4 s |
| detect faces | Intel Mac / ORT 1.23.2 | same | 2.7 s |
| compress "visually identical" JPEG | Windows | 3840×2560 photo | 13–16 s, −55…−58 % |
| compress default (quality 78) JPEG | Windows | 3840×2560 photo | 1.8–2 s, −48…−58 % |

Visual checks (opened with the Read tool): BiRefNet cut-out of the 20 MP portrait clean on both
Windows and the Mac (identical results); ISNet loses the white sweater; x4plus and general-v3 ×4
on a fur close-up far sharper than bicubic, no tile seams; anime-6B ×4 on a logo crisp.

### Intel-Mac ML backend decision

Microsoft's last x86_64 macOS build is ONNX Runtime **1.23.2** (`onnxruntime-osx-x86_64-1.23.2.tgz`,
SHA-256 `d10359e16347b57d9959f7e80a225a5b4a66ed7d7e007274a15cae86836485a6`); pyke's prebuilt
`ort` has no `x86_64-apple-darwin` entry. That dylib is self-contained (only system frameworks,
checked with `otool -L`). Implemented in `engine-ml/src/backend.rs`:
- Windows, Apple Silicon, Linux: `ort` 2.0.0-rc.15 with ONNX Runtime 1.31 statically linked
  (pyke prebuilt, `download-binaries`), API level `api-23`.
- Intel Mac: same `ort` with `load-dynamic` + `api-23`, loading `libonnxruntime.dylib` 1.23.2
  from `Contents/Frameworks/` (or `CASTOR_ORT_DYLIB`, or `downloads/runtime/`).
- Fallback everywhere: `tract-onnx` 0.23 (pure Rust), automatic if ONNX Runtime cannot
  initialise, or forced with `CASTOR_ML_BACKEND=tract`. Same results as ORT; as fast for
  ISNet/YuNet/general-v3, ~7× slower for BiRefNet.
Checked once on the Intel Mac before the no-Mac-build rule: release build of
`engine-ml --example bench`, ORT 1.23.2 loaded dynamically, ISNet / BiRefNet / YuNet /
general-v3 ran with the same images as Windows, models downloaded through
`castor_core::download::ensure`. No Mac build was made of `tools-image` or the CLI.

### Third-party
| component | version | licence | how shipped (linked / sidecar / download) |
|---|---|---|---|
| ONNX Runtime (via `ort` 2.0.0-rc.15, pyke build) | 1.31.0 | MIT | statically linked (Windows, Apple Silicon) |
| ONNX Runtime osx-x86_64 | 1.23.2 | MIT | dylib in `Contents/Frameworks` (Intel Mac; packaging step) — github.com/microsoft/onnxruntime/releases/tag/v1.23.2 |
| tract-onnx / tract-linalg | 0.23.10 | MIT OR Apache-2.0 | linked |
| fast_image_resize | 6.1.0 | MIT OR Apache-2.0 | linked |
| image | 0.25.10 | MIT OR Apache-2.0 | linked |
| zenjpeg (JPEG encoder, jpegli AQ + trellis) | 0.8.4 | AGPL-3.0-only OR commercial | linked |
| oxipng | 10.2.1 | MIT | linked |
| imagequant (libimagequant) | 4.4.1 | GPL-3.0-or-later | linked |
| webp / libwebp-sys (libwebp) | 0.3.1 / 0.9.6 | MIT OR Apache-2.0 / MIT (libwebp BSD-3-Clause) | linked |
| ravif / rav1e | 0.13.0 / 0.8.1 | BSD-3-Clause / BSD-2-Clause | linked |
| gif | 0.14.2 | MIT OR Apache-2.0 | linked |
| gifski | 1.34.0 | AGPL-3.0-or-later | linked |
| dssim-core | 3.5.1 | AGPL-3.0 | linked |
| heic (HEIC/AVIF decoder) + rav1d-safe | 0.1.6 + 0.5.7 | AGPL-3.0-only OR commercial | linked |
| jxl-oxide | 0.12.6 | MIT OR Apache-2.0 | linked |
| resvg / usvg / fontdb | 0.46.0 | Apache-2.0 OR MIT | linked |
| rawler | 0.8.0 | LGPL-2.1 | linked (full source shipped under GPL satisfies LGPL relinking) |
| icns | 0.5.0 | MIT | linked |
| img-parts | 0.4.2 | MIT OR Apache-2.0 | linked |
| kamadak-exif | 0.6.2 | BSD-2-Clause | linked |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 | linked |
| vtracer / visioncortex | 0.6.5 / 0.8.10 | MIT OR Apache-2.0 | linked (pulls image 0.23 + clap 2 internally) |
| image_hasher | 3.1.1 | MIT OR Apache-2.0 | linked |
| roxmltree | 0.20.0 | MIT OR Apache-2.0 | linked |
| BiRefNet-lite ONNX (onnx-community, rev de15b22) | — | MIT | download 224 MB `models/birefnet-lite.onnx` |
| ISNet general-use ONNX (rembg release v0.0.0) | — | Apache-2.0 | download 179 MB |
| Real-ESRGAN x4plus / x2plus ONNX (SceneWorks, rev 09f741b) | — | BSD-3-Clause | download 67 MB each |
| Real-ESRGAN x4plus anime 6B ONNX (RekluzLabs, rev d5377ec) | — | BSD-3-Clause | download 18 MB |
| realesr-general-x4v3 ONNX (CoderViking, rev c6a9717) | — | BSD-3-Clause | download 4.9 MB |
| YuNet 2023mar ONNX (opencv on HF, rev 3cc26e7) | — | MIT | download 233 KB |

Model URLs are commit-pinned Hugging Face `resolve` URLs (ISNet: GitHub release) with SHA-256 and
size in `engine-ml/src/models.rs`. The ONNX files are third-party exports of the official
weights; outputs were checked visually, not bit-compared with PyTorch. `mozjpeg` was tried first
and dropped: `mozjpeg-sys` and MuPDF's bundled libjpeg both define the `jpeg_*` symbols, so any
binary with both fails to link (LNK2005). zenjpeg is configured without restart markers:
with them, `image`/zune-jpeg 0.25.8 cannot read its progressive output (now covered by a test).
`Cargo.lock` was moved from `image` 0.25.10 to 0.25.8 by someone else during the session; the
code only uses APIs present in 0.25.8 (XMP is read with img-parts).

### Requests to the coordinator

1. `.cargo/config.toml` `[env]` for the MuPDF bindgen variables (same as A's request): without
   `. crates/engine-pdf/env.sh`, `tools-image` (which links `engine-pdf` for OCR) does not build.
2. Settings UI (agent D): an "AI" section storing `Settings::ai_api_key`; `image.seo` points users
   to "Settings → AI" when "Write with AI" is on and no key is set.
3. `castor-tools::thumbnail` could call `tools_image::codec::decode` (HEIC, AVIF, JXL, RAW, SVG,
   ICNS) instead of `image::ImageReader`, so the app shows thumbnails for every format Castor reads.
4. `Requirement::Model` is static per tool (remove-background → `birefnet-lite`, upscale →
   `realesr-general-x4v3`, redact → `yunet`); `Tool::downloads(values)` returns the model of the
   chosen option and should drive the consent dialog.
5. Decision needed: BiRefNet is the default background model everywhere (best quality) but takes
   1–3 min per 20 MP photo on the 8 GB Intel Mac, where ISNet takes ~9 s. Option: default to
   "Fast" on Intel Macs / when the tract backend is in use.
6. HEIC decoding ships a pure-Rust HEVC decoder (`heic`); check the patent position before
   release (macOS could use ImageIO instead; Windows has no free system decoder).

### Packaging notes (for wave 2)

- Windows: nothing extra, ONNX Runtime is static. Check `castor.exe` imports no
  `onnxruntime.dll` / `DirectML.dll` (pyke's Windows build is the DirectML variant;
  `dumpbin /dependents`).
- Intel Mac: copy `onnxruntime-osx-x86_64-1.23.2/lib/libonnxruntime.1.23.2.dylib` to
  `Castor.app/Contents/Frameworks/libonnxruntime.dylib` and sign it with the app. To verify on the
  Mac at the final build: (1) `CASTOR_LOG=info castor-cli run image.remove-background …` logs
  "loaded … with onnxruntime"; (2) with the dylib removed it falls back to tract and still works;
  (3) notarised hardened-runtime app can `dlopen` the bundled dylib (same team signature, library
  validation); (4) BiRefNet memory/time on 8 GB; (5) `tools-image` builds there (nasm for
  rav1e, libclang + env for MuPDF) and HEIC export via `sips` works.
- Apple Silicon: static ONNX Runtime 1.31 (CoreML EP available, not enabled).
- Build tools: `nasm` (rav1e/ravif assembly; on the Mac it is already installed via Homebrew) and
  libclang for MuPDF bindgen.
- Models download on first use into `paths::downloads_dir()/models/` (~560 MB if every model is
  used).
- HEIC export exists only on macOS (`sips`, part of the OS); the choice is hidden elsewhere.
