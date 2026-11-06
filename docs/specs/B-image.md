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
| id | status | notes |
|---|---|---|

### Benchmarks (machine, image, time)

### Third-party
| component | version | licence | how shipped (linked / sidecar / download) |
|---|---|---|---|

### Requests to the coordinator

### Packaging notes (for wave 2)
