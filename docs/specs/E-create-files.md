# Spec E — Create tools, Files tools, MCP server

Owner: agent E (wave 2). You own `crates/tools-create`, `crates/tools-files` (keep
`files.checksum`), `crates/castor-mcp` (new: add it under "Requests" — the coordinator adds it to
the workspace before you start), `locales/create.yml`, `locales/files.yml`, this file.
Build dir: `CARGO_TARGET_DIR=target-e`. Read `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/DESIGN.md`,
`docs/TOOLS.md`, and the **Reports of wave 1** in `docs/specs/A-pdf.md`, `B-image.md`,
`C-media.md` (engine APIs you can reuse: MuPDF/Typst in engine-pdf, image codecs in tools-image /
engine-ml, ffmpeg/whisper/yt-dlp in engine-media).

## Create (`docs/TOOLS.md` § Create)

All create tools implement `preview` — it is their main view (the app shows it large).

- `create.qr-code`: content types URL, text, Wi-Fi (SSID, password, security), contact (vCard),
  e-mail, phone, SMS, geo, event; 12 styles (square, dots, rounded, classy, diamond — our motif —,
  gapped…), colours/gradient, logo in the centre (error correction raised automatically),
  9 printable cards (Wi-Fi, table tent, review, business card, ticket, poster, product tag,
  social card, invitation) as PDF (Typst) or PNG; **every output is decoded back with `rqrr`
  and the run fails if it does not scan**. SVG, PNG, PDF outputs.
- `create.barcode` ★: EAN-13, UPC-A, Code 128, Code 39, ITF, DataMatrix, PDF417 (pick crates
  with permissive/GPL-compatible licences).
- `create.mockup`: device frames drawn by us in SVG (generic phone, tablet, laptop, desktop
  monitor, browser window — no Apple/Samsung trademarks or exact product shapes), 20+ scenes
  (procedural backgrounds, desk surfaces made of gradients/noise), screenshot or video input
  (video → ffmpeg overlay with perspective for flat/angled frames), export PNG/MP4.
- `create.html-to-image`, `create.html-to-pdf`: headless Chromium (detect Chrome/Edge/Brave/
  Chromium; `chromiumoxide` or `headless_chrome`); file or URL or pasted HTML; viewport, full
  page, scale; PDF page size, margins, backgrounds, header/footer; `unavailable()` if no browser.
- `create.audiogram`: audio → video with waveform styles (bars, line, circle), title, cover
  image/logo, captions from whisper (engine-media), aspect presets (1:1, 9:16, 16:9).
- `create.gradient`: mesh gradients (4–8 control points, colours, lighting, distortion, grain),
  export PNG (any size) and seamless looping MP4/WebM.
- `create.shader`: generative waves, silk, noise, aurora, topography lines (CPU rendering with
  `rayon`, or wgpu if you can keep it simple), presets + seed, PNG and looping video.
- `create.pattern`, `create.watermark-tile`, `create.placeholder` (any size, label, colours,
  "1200 × 630" text, batch of sizes), `create.social-card` (templates in Typst: title, subtitle,
  logo, colours → OG images 1200×630 etc.), `create.brand-sheet` (one-page brand PDF from logo,
  colours, fonts — Typst), `create.data-merge` (template = Typst file or a simple built-in
  layout + CSV → one PDF per row or one merged PDF; also fills PDF form templates via
  engine-pdf if available), `create.favicon` ★ (from text/emoji/letter + colours → icon pack),
  `create.formula` ★ (LaTeX-style math → SVG/PNG/PDF via Typst math; accept common LaTeX
  syntax by translation or document the Typst syntax), `create.carousel` (multi-slide social
  carousels from a list of text blocks + images, consistent template, export each slide PNG with
  editable names + PDF), `create.fonts` (library of installed + user fonts: preview text, glyph
  grid, info; convert TTF/OTF/WOFF/WOFF2 — use `fontations`/`write-fonts` + `woff2` encoder;
  optional Google Fonts download: off by default, explicit consent).
- `create.video-download`: yt-dlp through engine-media (`ytdlp::ensure/update/download`), URL
  list input (text option), quality presets, audio-only MP3/M4A, playlist toggle, subtitles,
  output naming; status `Requirement::Network`; legal note in help text ("only download what you
  have the right to").
- `create.motion` (S at Hangar): text reveal / lower third / logo sting as short MP4 with alpha
  (ProRes 4444 / WebM VP9 alpha) — do it if time allows, else keep `Soon`.

## Files (`docs/TOOLS.md` § Files)

In-place tools (`meta.in_place = true`) must write an undo journal (`Output::undo`) and
implement `Tool::undo`; previews must show the exact result before anything moves.
- `files.rename`: tokens ({name}, {n:03}, {date:%Y-%m-%d}, {exif.date}, {ext}, {parent}),
  find/replace (plain/regex), case, remove characters, change extension, natural sort,
  numbering start/step; preview list (implement `preview` as a rendered table image or extend
  via request); collision-safe.
- `files.archive`: ZIP (deflate/zstd), 7z (`sevenz-rust2`), tar.gz/tar.zst; password (AES-256
  zip / 7z); split volumes; exclude patterns.
- `files.extract`: ZIP, 7z, TAR(+gz/bz2/xz/zst), RAR (only if a GPL-compatible extractor exists —
  `unrar` licence is NOT compatible: skip RAR or use libarchive if feasible), password; zip-slip
  protection; into a folder named after the archive.
- `files.dates`: set created/modified to a date, to EXIF/QuickTime capture date, or shift by
  ±N; preview; undo.
- `files.sort`: by type / year / month / camera / rule (extension, size, name pattern) into
  subfolders; dry-run counts; undo.
- `files.metadata`: view/edit common metadata (PDF info via engine-pdf, image EXIF/XMP via
  tools-image helpers, audio tags via `lofty`, video tags via ffmpeg), strip.
- `files.duplicates` ★: size → partial hash → BLAKE3 full hash; report; move duplicates to a
  "Duplicates" folder (undo) — never delete.
- `files.identify` ★ ("What is this file?", idea from TridUI/TrID): detect the real type of files
  with no or wrong extensions. Engines, in order: **Magika** (Google, Apache-2.0, `magika` crate,
  ~1 MB ONNX model, 200+ content types with confidence — reuse agent B's ML backend decision,
  incl. the Intel-Mac question), **`file-format`** crate (Apache-2.0, ~1 000 signatures), `infer`.
  Optional long tail: TrID definitions (22 000+ types) downloaded only on explicit consent from
  mark0.net, never bundled (TrID is "free for personal / non-commercial use" — show that notice;
  write our own TRD reader, do not port TridUI's AGPL Go code). Output: report (TXT/CSV/JSON)
  with the top matches and confidence; option "fix extensions" renames mismatched files
  (preview + undo). Also offer the coordinator a fast `identify(path)` helper so the app can
  route extension-less files to the right tools.

## MCP server (`crates/castor-mcp`, binary `castor-mcp`, also `castor-cli mcp`)

stdio MCP server exposing: `list_tools`, `describe_tool(id)` (options JSON schema derived from
`OptionSpec`), `run_tool(id, files[], values{}, output_dir?)` (returns outputs + notes), 
`run_chain(toml, files[])`, `history(limit)`. Use the official Rust MCP SDK (`rmcp`) if
suitable. Document setup for Claude Desktop / Claude Code in `docs/MCP.md`.

## Report (fill in before finishing)

Agent E, wave 2. 20 create tools (`tools_create::tools()`) and 9 files tools
(`tools_files::tools()`, `files.checksum` unchanged) are registered with `en` + `fr` strings
(`locales/create.yml`, `locales/files.yml`); `castor-mcp` is a working stdio MCP server.

**How it was verified** (Windows 11, `. crates/engine-pdf/env.sh`, `CARGO_TARGET_DIR=target-e`):
`cargo test -p tools-create -p tools-files -p castor-mcp`.
- `tools-create`: 30 unit tests (every name, option and choice label present in en + fr; every QR style × eye style decodes, every barcode type
  round-trips through rxing, every Typst template compiles, LaTeX → Typst translations
  compile, loops are seamless, homography, FFT…) + `tests/create.rs` (every create tool
  through the real runner: QR PNG/SVG/PDF/logo/9 cards decoded back, barcodes, placeholders,
  favicon pack, gradients, shaders, patterns, watermark, mockup (image and screen-recording →
  MP4 with perspective), social cards (incl. CSV batch), data merge (merged + per-row),
  brand sheet, carousel, formula, fonts (WOFF2 ↔ OTF ↔ WOFF, specimen, info), gradient MP4,
  motion WebM/MOV with alpha, audiogram with SRT captions (probed: duration, audio), HTML →
  PNG/PDF with the installed Chrome). `CASTOR_KEEP=1` keeps the outputs in
  `tests/fixtures/generated/create/`. The yt-dlp test is `#[ignore]` (network).
- `tools-files`: unit tests + `tests/files.rs` (archive → extract for ZIP, ZIP-AES, 7z,
  7z-AES, TAR, TAR.GZ/ZST/XZ with unicode names and exclusions; wrong passwords fail; split
  volumes round trip; zip-slip entries land inside; rename with tokens + natural order + undo;
  dates shift/set + undo (created date checked on Windows); sort by type and by rules + undo;
  duplicates (same size and same head/tail but different middle are told apart) moved aside +
  undo; identify PNG/ZIP/Python without or with wrong extensions + fix + undo; audio tag
  edit/strip round trip; checksum).
- `castor-mcp`: `tests/stdio.rs` starts the binary and speaks JSON-RPC over stdio like Claude
  Desktop: initialize, tools/list, list_tools, describe_tool (schema), run_tool (QR file
  written), a bad option → readable tool error, history.
- Visual review: every preview and many outputs were rendered and inspected, then iterated
  (QR cards, shaders, social cards, carousel, audiogram, brand sheet, mockups). Preview
  renderer: `cargo run -p tools-create --example create-cli -- preview <tool> --set k=v --out x.png`
  (`create-cli` also does `run`/`info`/`list`/`undo` like `castor-cli`, without the ONNX build);
  the batch script is `tests/fixtures/generated/create/look.sh`.

### Status per tool
| id | status | notes |
|---|---|---|
| `create.qr-code` | ✅ | URL, text, Wi-Fi, vCard, e-mail, phone, SMS, geo, calendar event; 12 module styles (square, gapped, dots, rounded, fluid, classy, leaves, diamonds, sparkles, vertical/horizontal bars, mosaic) × 6 corner-marker styles, colours, linear/radial gradient, separate marker colour, transparent background, centred logo (EC forced to H, modules cleared under it), quiet zone; PNG (any size) / SVG / PDF; 9 printable Typst cards (Wi-Fi, table tent, review, business card, ticket, A4 poster, product tag, social, invitation) as PDF/PNG/SVG with editable texts and accent. **Every output is decoded back** (rqrr, then rxing if rqrr cannot: it rejects rounded finder patterns that phones read fine); cards decoded at 200 dpi; a code that scans with neither is refused. Preview caption: version, EC level, modules, physical card size, "scans ✓". |
| `create.barcode` ★ | ✅ | EAN-13, EAN-8, UPC-A (check digit added or verified, proper retail layout with guard bars and split digits), Code 128, Code 39, ITF, Codabar, Data Matrix, PDF417, Aztec (rxing, Apache-2.0); batch (one per line); PNG/SVG (text outlined)/PDF at true size (0.33 mm modules); every code read back by rxing. Non-Latin-1 text is refused for 1D/Data Matrix/PDF417 (scanners would garble it) with a pointer to QR. |
| `create.placeholder` | ✅ | Any sizes ("1200x630, 800x600") or preset sets (social 6, web/ads 8, devices 6, thumbnails 4); label, "W × H" text, 5 light patterns, colours; PNG/JPG/WebP/SVG. |
| `create.favicon` ★ | ✅ | Letter / word / emoji (system colour-emoji fonts) / logo image; 6 shapes incl. the diamond; bundled or installed fonts; gradient background; pack: favicon.ico (16/32/48), favicon.svg (text outlined), 16/32 PNG, apple-touch-icon (opaque 180), Android 192/512, maskable 512, site.webmanifest, HTML snippet. |
| `create.gradient` | ✅ | Mesh gradients: 4–8 points blended in OKLab, 8 palettes or custom colours, domain-warp distortion, softness, lighting, grain, dithering (no banding); PNG/JPG/WebP any size; seamless looping MP4/WebM (points on closed orbits; first and last frames match). |
| `create.shader` | ✅ | CPU (rayon): silk (lit height field), waves, clouds, aurora (curtains + stars), contour lines (anti-aliased, major every 5th), plasma, dunes; palettes, seed, scale, detail, grain; PNG/JPG/WebP and seamless loops (MP4/WebM). |
| `create.pattern` | ✅ | 14 seamless SVG patterns (dots, grid, diamonds, checker, stripes, zigzag, waves, hexagons, triangles, crosses, terrazzo, confetti, scales, herringbone); tile export (truly seamless, wrapped shapes) or filled canvas with rotation; PNG/SVG/JPG. Was "Soon" at Hangar. |
| `create.watermark-tile` | ✅ | Repeating text (1–2 lines, bundled or installed font, colour with alpha, angle, spacing, staggered rows) as a transparent seamless tile or full sheet; PNG/SVG. |
| `create.social-card` | ✅ | 6 Typst templates (classic with ghost initial and page rule, pastel, split, quote, photo, minimal), 7 size presets + custom, title auto-fits, logo, photo (or a painted gradient fallback), colours, title font (bundled or installed); PNG/JPG/PDF; **CSV batch** (one card per row, columns title/subtitle/eyebrow/footer/file). |
| `create.mockup` | ✅ | 5 generic devices drawn here (phone, tablet, laptop, monitor, browser window — no brand shapes), 3 finishes, 22 scenes (8 painted gradients, paper, night, pastel, wood, walnut, concrete, marble, linen, felt, terrazzo, oak desk, studio, custom, transparent), front / turned left / turned right / lying back (true perspective warp), soft or strong shadow, fill/fit; screenshot → PNG/JPG; **screen recording → MP4** (ffmpeg: scaled, perspective-mapped, masked, frame overlaid, sound kept). |
| `create.audiogram` | ✅ | Audio or video → MP4 (1:1, 9:16, 16:9, 4:5); bars (48 log-spaced FFT bands, attack/release), line (oscilloscope) or circle (radial spectrum around the cover); title (default = file name), subtitle, cover image, 22 backgrounds, colours, progress line; captions from speech (whisper via engine-media, model on demand, VAD) or from an SRT/VTT/ASS file; start + length; original sound muxed (AAC). |
| `create.motion` | ⚠️ Beta | Lower third, text reveal (words rise from a mask), logo sting (diamond draws itself, logo/initial pops in), title card; eased keyframes; transparent ProRes 4444 MOV or VP9 WebM with alpha, or MP4 on a colour. Works and tested; marked Beta because there is no timeline editing (fixed choreography per preset). |
| `create.brand-sheet` | ✅ | One A4 page (Typst): name + tagline with ghost initial, logo on light and dark with clear-space outline, swatches with HEX/RGB/HSL/CMYK (naive device CMYK, stated), heading/text specimens (bundled or installed fonts), WCAG contrast table (best 8 pairs, AA/AAA/AA large), notes; PDF or 300 dpi PNG. |
| `create.carousel` | ✅ | Slides from text blocks separated by `---` (first line = title), dropped images assigned in order, 4 templates (classic, bold alternating, minimal, full-bleed photo), 4 sizes, handle, numbers "2 / 5", "Swipe →" on the cover; every slide PNG named `{name}-01.png`… + PDF of the set. |
| `create.data-merge` | ✅ | CSV/TSV (delimiter sniffed, BOM) + layout: letter, certificate, name badges (10 per A4), address labels (3 × 8 per A4), postcard — texts with `{column}` placeholders, logo, accent, A4/Letter; **or your own Typst template** (`sys.inputs.<column>` and `json("/row.json")`, images next to the template usable); one merged PDF or one PDF per row with a name pattern. ⚠️ Filling PDF *form* templates is left to `pdf.fill-form` (agent A, CSV-capable); not duplicated here. |
| `create.formula` ★ | ✅ | Typst math with New Computer Modern Math; LaTeX translated (fractions, roots, scripts, 40+ Greek/operators/arrows, accents, \mathbb/\mathcal/…, \text, \operatorname, \left/\right, pmatrix/bmatrix/vmatrix/matrix/array, cases, aligned) or native Typst; display/inline, size, colour, transparent or filled background; SVG/PNG (dpi)/PDF. Unknown commands give a clear error naming them. |
| `create.fonts` | ✅ | Specimen PDF/PNG (name in the font, info table, waterfall 9–44 pt, glyph grid of up to 320 characters), conversion TTF/OTF ↔ WOFF (own encoder) ↔ WOFF2 (ttf2woff2; decoding with wuff), info report TXT/JSON (names, version, weight, glyphs, characters, variable axes, designer, licence), catalogue PDF of installed families (each line in its own font), ⚠️ Beta: "Get from Google Fonts" (explicit opt-in toggle, github.com/google/fonts, open licences only, files parsed before keeping; no SHA pin possible for user-chosen fonts). |
| `create.html-to-image` | ✅ | Files, URLs (one per line) or pasted HTML → PNG/JPG/WebP via installed Chrome/Edge/Brave/Chromium (headless, throw-away profile, closed reliably even when Chrome relaunches itself); viewport, full page (capped at 16 000 px), 1×/2×/3×, dark mode, transparent background, wait after load (fonts awaited). `unavailable()` without a browser. |
| `create.html-to-pdf` | ✅ | Same sources → PDF: A4/Letter/A3/A5/Legal/Tabloid or CSS page size, landscape, margins, backgrounds, title/date/page-number header & footer, scale, screen vs print CSS, document outline, tagged PDF. |
| `create.video-download` | ✅ | yt-dlp via engine-media (`ensure`/`update`/`download`): several links, best/4K/1080p/720p/480p or audio only (MP3/M4A/Opus), playlists, embedded subtitles, names from `{title}` `{id}` `{n}`, "update yt-dlp first"; requirements YtDlp + Ffmpeg + Network; legal note in the links help text. Not run end-to-end in the test suite (network, `#[ignore]`). |
| `files.rename` | ✅ | Tokens `{name}` `{n}` `{n:03}` `{n:auto}` `{date:%Y-%m-%d}` `{created:…}` `{exif.date:…}` `{exif.camera}` `{parent}` `{ext}` `{size}`, find/replace (plain or regex with `$1`, case option), 7 cases incl. kebab/snake, remove characters, accents → ASCII, extension keep/lower/upper/change/remove, numbering order (natural, modified, created, date taken, size), start/step, subfolders; preview = booktabs table "Now → After"; collision-safe two-phase moves (swaps and case-only renames work), duplicate targets refused, rollback on error, undo journal. |
| `files.archive` | ✅ | ZIP (Deflate or Zstandard, AES-256), 7z (LZMA2, AES-256 with encrypted names), TAR, TAR.GZ, TAR.ZST, TAR.XZ; levels store/fast/normal/max; exclude patterns; one archive per item option; dropped folders keep their name; symlinks never followed; split volumes `.001`…; atomic write; note "N files · before → after (−x %)". |
| `files.extract` | ✅ | Format sniffed from content: ZIP (AES and ZipCrypto), 7z (AES), TAR, TAR.GZ/BZ2/XZ/ZST, single .gz/.bz2/.xz/.zst, split sets (`.001` + following parts joined on the fly); zip-slip safe (no `..`, no absolute/drive paths, no links/devices); "no folder inside a folder"; dates kept; clear errors for wrong/missing password, damaged archive, RAR (not supported: licence). |
| `files.dates` | ✅ | Set to date taken (EXIF DateTimeOriginal, or QuickTime/MP4 `mvhd` creation time), to a chosen date, shift by ± days/hours/minutes, modified = created / created = modified; created + modified (Windows and macOS), preview table, undo journal with nanosecond times; files without a date are skipped and counted. |
| `files.sort` | ✅ | By type, year, month, day (date taken/modified/created), camera, extension, size, or rules (`*.pdf, *.docx => Documents`, tokens `{year}` `{month}` `{ext}` `{type}` `{camera}`), into the dropped folder or a chosen one; preview = folder/count/size table; unique names; undo removes created folders. |
| `files.duplicates` ★ | ✅ | Size → partial BLAKE3 (first + last 64 KB) → full BLAKE3 (parallel); keep oldest/newest/shortest path/first; report CSV/TXT/JSON with wasted space; optional move of copies into "Duplicates" (undo); never deletes. Preview for folders up to 5,000 files. |
| `files.identify` ★ | ✅ | Magika (bundled model, tract) → file-format (~1,000 signatures) → infer; label, description, MIME, group, confidence, engine, second opinion when signatures disagree; report TXT/CSV/JSON (all or mismatches only); "fix extensions" renames binary files whose extension is missing or wrong above a confidence threshold (text files keep theirs), undoable; public `tools_files::identify(path)` helper. ❌ TrID long tail: not implemented (non-commercial licence; see decisions). |
| `files.metadata` | ✅ | View (preview table + TXT/JSON report): PDF info (engine-pdf), EXIF + XMP of photos, audio tags + properties (lofty), video/stream tags (ffprobe). Edit (copy written): title/author/subject/keywords/year/comment/copyright for PDF, audio, video. Remove all: PDF (info + XMP + PieceInfo), audio (every tag type), video (stream copy, `-map_metadata -1`), JPEG/PNG/WebP losslessly (pixels untouched; JPEG keeps orientation as a minimal EXIF and the ICC profile). ⚠️ Editing photo metadata is left to `image.seo` (agent B). |
| `files.checksum` | ✅ | Unchanged (reference tool); covered by a test. |

### MCP server (`crates/castor-mcp`)
✅ `castor-mcp` binary (stdio, `rmcp` 3.9 — the official SDK) + library (`castor_mcp::run_stdio()`,
`serve_stdio()`, `options_schema()`, `parse_values()`). MCP tools: `list_tools` (family, query
through the registry's multilingual search), `describe_tool` (JSON Schema from `OptionSpec`:
types, enums with labels, min/max, units, defaults, "used when" conditions, password =
writeOnly, plus inputs, requirements, `unavailable()`, missing downloads), `run_tool` (values
checked against the schema, unknown keys rejected with the valid list, files expanded, output
next to inputs or `output_dir`, runs recorded in history, notes returned; failures are tool
errors with the run record, not protocol errors), `run_chain` (chain TOML), `history`. Calls run
on Tokio's blocking pool. Setup for Claude Desktop / Claude Code: `docs/MCP.md`.

### Decisions taken (please confirm)
1. **Typst 0.15.1 embedded in tools-create** (`render/typ.rs`): agent A did not put Typst in
   engine-pdf, so tools-create has its own in-memory `World` (bundled Latin Modern, Chorus and
   New Computer Modern Math; installed families loaded on demand through fontdb; `sys.inputs`;
   no packages, no network). Cards, brand sheet, data merge, carousel, formula and specimens use
   it (PDF via krilla, PNG via typst-render, SVG via typst-svg). Cost ≈ 300 crates on a cold
   build and roughly +8–10 MB in release. It can move as-is into a shared `engine-typst` if the
   PDF tools want it too (e.g. math in `pdf.from-markdown`).
2. **Magika on tract, model bundled**: the `magika` crate pins `ort =2.0.0-rc.12`, which cannot
   coexist with engine-ml's `ort =2.0.0-rc.15` (`links = "onnxruntime"`). `files.identify` runs
   the same model (magika 1.1.0 `model.onnx`, 3.1 MB, Apache-2.0, SHA-256
   `fe2d2eb49c5f88a9e0a6c048e15d6ffdf86235519c2afc535044de433169ec8c`, in
   `crates/tools-files/assets/magika/` with its LICENSE) through **tract**, engine-ml's pure-Rust
   backend. Same model and thresholds; feature extraction and label tables ported
   (`magika_table.rs` generated from the crate, notice kept). Consequence: identical on Windows,
   Apple Silicon and Intel Macs, no ONNX Runtime involved, works offline from the first run.
3. **QR checking**: rqrr first, rxing as second reader (see qr-code row). Pure rqrr would have
   forced square finder patterns only.
4. **TrID not offered** (definitions are free for personal / non-commercial use only).
5. **RAR not supported** (unrar licence); everything else extracts.
6. **Own Chrome DevTools client** (≈ 300 lines, tungstenite, localhost only) instead of
   `headless_chrome`/`chromiumoxide`: no async runtime, no bundled/downloaded browser.
7. Only rename, dates, sort, duplicates (move) and identify (fix) are in place, all with undo
   journals; metadata/extract/archive always write new files.
8. Barcodes that cannot carry the text (non-Latin-1 in 1D/Data Matrix/PDF417) are refused
   rather than written with an ECI that most scanners ignore.

### Third-party
| component | version | licence | how shipped |
|---|---|---|---|
| typst, typst-layout, typst-pdf, typst-render, typst-svg (+ typst-library, typst-syntax…, krilla, krilla-svg, comemo, ecow) | 0.15.1 (krilla 0.8.2) | Apache-2.0 (krilla MIT OR Apache-2.0) | linked |
| New Computer Modern Math (`NewCMMath-Regular.otf`, from typst-assets 0.15.1) | 7.x as shipped by typst-assets | GUST Font License | bundled (`crates/tools-create/assets/fonts/`, 1.3 MB, `include_bytes!`) |
| resvg / usvg / tiny-skia / fontdb | 0.47.0 / 0.47.0 / 0.12.0 / 0.23.0 | Apache-2.0 OR MIT; tiny-skia BSD-3-Clause | linked |
| qrcode | 0.14.1 | MIT OR Apache-2.0 | linked |
| rqrr | 0.11.0 | (MIT OR Apache-2.0) AND ISC | linked |
| rxing (oned, datamatrix, pdf417, qrcode, aztec, encoding_rs) | 0.9.3 | Apache-2.0 | linked |
| csv | 1.4.0 | MIT OR Unlicense | linked |
| tungstenite (no TLS) | 0.29.0 | MIT OR Apache-2.0 | linked |
| ttf2woff2 / wuff / ttf-parser | 0.14.1 / 0.2.10 / 0.25.1 | MIT OR Apache-2.0 / MIT / MIT OR Apache-2.0 | linked |
| flate2 | 1.1.11 | MIT OR Apache-2.0 | linked |
| Magika model `standard_v3_3` (magika crate 1.1.0) + generated type table | 1.1.0 | Apache-2.0 (© Google LLC) | bundled (`crates/tools-files/assets/magika/model.onnx`, 3.1 MB) |
| tract-onnx | 0.23.10 | MIT OR Apache-2.0 | linked (shared with engine-ml) |
| file-format | 0.29.0 | MIT OR Apache-2.0 | linked |
| zip (aes-crypto, deflate, deflate64, bzip2, zstd, lzma) | 2.6.1 | MIT | linked |
| sevenz-rust2 (aes256, compress, bzip2, ppmd, zstd, deflate) | 0.24.2 | Apache-2.0 | linked |
| tar | 0.4.46 | MIT OR Apache-2.0 | linked |
| zstd (libzstd) | 0.13.4 | MIT (libzstd BSD-3-Clause) | linked (C, built by cc) |
| bzip2 (libbz2-rs-sys) | 0.6.1 | MIT OR Apache-2.0 | linked |
| lzma-rust2 (xz encoder/decoder) | 0.23.0 | Apache-2.0 | linked |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 | linked |
| kamadak-exif | 0.6.2 | BSD-2-Clause | linked |
| lofty | 0.25.5 | MIT OR Apache-2.0 | linked |
| chrono (clock) | 0.4.45 | MIT OR Apache-2.0 | linked |
| ab_glyph | 0.2.32 | Apache-2.0 | linked (preview tables) |
| regex | 1.13.1 | MIT OR Apache-2.0 | linked |
| rmcp (server, macros, transport-io) + tokio | 3.9.0 / 1.54.1 | Apache-2.0 / MIT | linked (castor-mcp) |
| Latin Modern, TeX Gyre Chorus | repository `assets/fonts` | GUST Font License | already bundled |
| yt-dlp, ffmpeg, whisper models | see C-media | Unlicense / GPL / MIT | via engine-media |
| Chrome / Edge / Brave / Chromium | user-installed | proprietary / BSD | **not shipped**: detected, used headless |
| Google Fonts families | user-chosen | OFL / Apache-2.0 | **not shipped**: optional user download, explicit toggle |

### Requests to the coordinator
1. **`castor-cli mcp`**: add `castor-mcp` as a dependency of `castor-cli` and a subcommand
   calling `castor_mcp::run_stdio()` (sets the language from `CASTOR_LANG`, default English).
   Meanwhile the `castor-mcp` binary works on its own.
2. **Routing extension-less files**: call `tools_files::identify(path)` (fast: 4 KB read, model
   loaded once) in `FileInfo::from_path` when the extension is missing/unknown and `infer` finds
   nothing, mapping `Identity::mime`/`label` to `FileKind`; or in the app's drop handler.
3. **Folders in `castor-cli run`**: the CLI expands folders before planning, so
   `files.archive` loses the dropped folder's name and `files.sort`/`files.rename` cannot use
   the folder as their base. Please pass inputs unexpanded to tools whose `inputs.folders` is
   true (as the app does); the tools expand themselves.
4. **Structured previews**: rename/dates/sort/duplicates/identify/metadata draw their preview
   table as a PNG (booktabs, Latin Modern). A `Preview::Table { caption, header, rows }`
   variant in castor-core would let the app show a real, scrollable, themed table.
5. **Consent before downloads**: `create.audiogram` (captions from speech → whisper model) and
   `create.video-download` (yt-dlp) implement `downloads()`; please make sure the app asks
   before running them. `create.fonts` → "Get from Google Fonts" and the HTML tools with a web
   address use the network by design; consider showing `Requirement::Network` dynamically
   (only when those options are chosen).
6. **Typst**: if another crate needs Typst (e.g. math in `pdf.from-markdown`), move
   `crates/tools-create/src/render/typ.rs` + `assets/fonts/NewCMMath-Regular.otf` into an
   `engine-typst` crate rather than adding a second copy.
7. **rxing features**: rxing must be built with `encoding_rs` (or `legacy_encoding`), otherwise
   it fails to compile — already set in tools-create's manifest; keep it if the workspace ever
   declares rxing at the root.
8. **Disk**: `target-a/b/d` reached 20–28 GB each during this wave (free space fell to 13 GB);
   `target-e` stays smaller with line-tables-only debug info, but builds the whole workspace for `castor-mcp`.
9. **`Requirement::Pdfium`** (stale, as A noted) and a missing `Requirement::Chromium` UI flow:
   the HTML tools return `unavailable()` when no browser is installed.

### Packaging notes (macOS checks before publication — nothing was built on the Mac)
1. **Typst / fonts**: `cargo test -p tools-create` (pure Rust; Typst, resvg, rxing build
   everywhere). System fonts come from CoreText paths through fontdb: check the favicon emoji
   (Apple Color Emoji is an `sbix` font — resvg must draw it; if not, fall back message) and
   `create.fonts` catalogue speed with many installed fonts.
2. **Dates**: `files.dates` sets the creation date with `std::os::macos::fs::FileTimesExt`
   (stable since 1.75; compiled only on macOS — not compiled here). Check created and modified
   dates in Finder after a run and after undo; local time conversion uses chrono's `Local`
   (reads /etc/localtime).
3. **Magika/tract** on Intel and Apple Silicon: `tools_files::identify` on a PNG, a ZIP and a
   Python file (`tests/files.rs::identify_finds_real_types`).
4. **Chrome detection**: `/Applications/Google Chrome.app/…`, Edge, Brave, Chromium, Vivaldi,
   also `~/Applications`; run `html_with_installed_browser`. Gatekeeper is not involved (we run
   the user's signed browser). Check that the headless browser quits (no orphan
   `Google Chrome Helper` processes) and that the temp profile is removed.
5. **ffmpeg-based create tools** (gradient/shader loops, motion ProRes 4444 + VP9 alpha,
   audiogram, mockup video with `perspective`/`alphamerge`): need the bundled ffmpeg to have
   `libvpx-vp9`, `prores_ks`, `libx264`, `aac`; run `tests/create.rs::videos` with
   `CASTOR_FFMPEG` pointing at it.
6. **7z/zip/zstd C parts**: `zstd-sys` (cc) builds on macOS; `bzip2` uses the Rust backend.
   Run `cargo test -p tools-files` (archives, split volumes, zip-slip, undo).
7. **MCP**: `castor-mcp` must be inside the app bundle (`Contents/MacOS/castor-mcp`), signed and
   notarised with the app; check `claude mcp add castor -- /Applications/Castor.app/Contents/MacOS/castor-mcp`
   and the Claude Desktop config from `docs/MCP.md`.
8. **Video downloader**: yt-dlp from engine-media on macOS (quarantine attribute, see C-media).
