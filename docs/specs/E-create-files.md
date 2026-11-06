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

## MCP server (`crates/castor-mcp`, binary `castor-mcp`, also `castor-cli mcp`)

stdio MCP server exposing: `list_tools`, `describe_tool(id)` (options JSON schema derived from
`OptionSpec`), `run_tool(id, files[], values{}, output_dir?)` (returns outputs + notes), 
`run_chain(toml, files[])`, `history(limit)`. Use the official Rust MCP SDK (`rmcp`) if
suitable. Document setup for Claude Desktop / Claude Code in `docs/MCP.md`.

## Report (fill in before finishing)

### Status per tool
| id | status | notes |
|---|---|---|

### Third-party
| component | version | licence | how shipped |
|---|---|---|---|

### Requests to the coordinator
