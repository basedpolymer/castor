# Tool catalogue

Reference: hangar.tools catalogue as of 2026-11-06 (v0.12.3): **89 entries** — 77 shipped and
12 marked "Soon". Castor implements all of them, plus the ★ extras. Ids are stable (chains,
favorites, history, CLI): never rename one.

Legend: ★ not in Hangar · (S) "Soon" at Hangar · owner letter = spec in `docs/specs/`.

## PDF — owner A (`tools-pdf`, `engine-pdf`: MuPDF, Tesseract, optional Ghostscript)

| id | Hangar name | notes |
|---|---|---|
| `pdf.compress` | Compress PDF | presets Light / Balanced / Strong / Maximum (CMYK→RGB, B/W scans, strip bookmarks/tags; each step switchable); keeps the smallest candidate; never larger than the original |
| `pdf.merge` | Merge PDF | order = queue order, optional bookmarks per file |
| `pdf.ocr` | OCR PDF | searchable text layer, many languages (tessdata on demand), deskew, skip pages with text |
| `pdf.redact` | Redact PDF | true removal; search terms, presets (emails, phones, IBAN, card numbers), import TXT/CSV/TSV list; areas in PDF Studio (wave 2) |
| `pdf.sign` | Sign PDF | signature image or typed name (Chorus), page, position, size; saved signatures |
| `pdf.split` | Split PDF | every page / ranges / every N pages / by size / by bookmarks |
| `pdf.extract-pages` | Extract pages | ranges → new file |
| `pdf.organize` | Organize pages | batch form: reorder spec, rotate, delete; visual version in PDF Studio |
| `pdf.crop` | Crop PDF | margins or auto-trim whitespace |
| `pdf.page-numbers` | Page numbers | format ("1", "Page 1 of N", roman), position, font, start, skip first |
| `pdf.to-images` | PDF to images | PNG / JPG / WebP / TIFF, DPI |
| `pdf.to-markdown` | PDF to Markdown | headings by font size, lists, tables, images extracted |
| `pdf.encrypt` | Encrypt PDF | AES-256, owner/user passwords, permissions |
| `pdf.unlock` | Unlock PDF | known password |
| `pdf.stamp` | Stamp | text or image stamp/watermark, opacity, rotation, pages |
| `pdf.attach-to-each` | Attach to each | prepend/append a cover or terms PDF to every file |
| `pdf.flatten` | Flatten PDF (S) | forms + annotations baked in |
| `pdf.repair` | Repair PDF (S) | rebuild xref, recover objects |
| `pdf.from-office` | Office to PDF (S) | via LibreOffice if installed (detected), clear message otherwise |
| `pdf.to-epub` | PDF to Ebook (S) | reflowable EPUB |
| `pdf.from-ebook` | Ebook to PDF | EPUB, MOBI, FB2, CBZ, TXT, HTML (MuPDF layout engine), page size, font size |
| `pdf.compare` ★ | — | visual diff of two PDFs, page by page, highlighted PDF report |
| `pdf.metadata` ★ | (Metadata, S) | title, author, subject, keywords, dates; or strip all |
| `pdf.fill-form` ★ | — | fill fields from a CSV/JSON row or a form; list fields |
| `pdf.from-markdown` ★ | — | Markdown → beautiful PDF in the TeXtured style (Typst) |
| `pdf.extract-images` ★ | — | every embedded image at native resolution |
| `pdf.grayscale` ★ | — | convert to grayscale (print-friendly) |
| `pdf.rotate` ★ | — | quick batch rotate (all / odd / even / ranges) |

## Image — owner B (`tools-image`, `engine-ml`: ONNX models)

| id | Hangar name | notes |
|---|---|---|
| `image.remove-background` | Remove background | local model, keeps small detached marks; PNG/WebP with alpha, or colour/blur background |
| `image.upscale` | AI Upscale | ×2 / ×4, photo / art models |
| `image.convert` | Convert images | in: RAW, HEIC, SVG, ICO, TGA, QOI, DDS, PNM, JFIF, AVIF, JXL, animated GIF/WebP… out: JPG, PNG, WebP, AVIF, GIF, TIFF, BMP, ICO, HEIC (where possible); option `format` |
| `image.vectorize` | Vectorize image | vtracer; presets logo / illustration / photo; auto |
| `image.compress` | Compress images | per-format best encoder; quality or "visually identical" target (dssim); never larger than the original |
| `image.palette` | Color palette | k-means in Lab; swatches PNG + ASE/GPL/CSS/JSON |
| `image.resize` | Resize images | options `mode` (fit / fill / exact / percent), `size`, `preset` (social, marketplace, device canvases), focal point / auto focus |
| `image.crop` | Crop image | ratio presets, px / %, auto-trim borders |
| `image.rotate` | Rotate & flip | 90/180/270, flip, auto (EXIF), straighten angle |
| `image.strip-metadata` | Strip metadata | EXIF/GPS/XMP/IPTC without recompression (JPEG/PNG/WebP lossless path) |
| `image.seo` | SEO image | slug file names, title/alt/keywords/author/copyright/GPS into XMP/IPTC; optional "Write with AI" (user's API key, opt-in) |
| `image.redact` | Redact image | blur / pixelate / box; auto faces (model) + regions by text (OCR) ; metadata stripped |
| `image.watermark` | Watermark | text or logo, tiled or anchored, opacity, scale |
| `image.beautify` | Beautify shot | background (painted gradients, solid, image), padding, radius, shadow, canvas ratio, window frame |
| `image.to-pdf` | Image to PDF | scan/photo → cleaned (deskew, crop, contrast) searchable PDF |
| `image.images-to-pdf` | Images to PDF | many images → one PDF, page size, margins, optional OCR |
| `image.extract-text` | Extract text | OCR → TXT / Markdown / copy |
| `image.icon-pack` | Icon pack | ICO, ICNS, favicon set, iOS/Android/PWA sizes from one master |
| `image.contact-sheet` | Contact sheet | grid, header, captions, watermark → PDF or PNG |
| `image.compare` | Image comparison | before/after: interactive HTML slider, GIF, PNG/JPEG side-by-side |
| `image.stylize` | Stylize | looks: film, duotone, halftone, sketch, posterize, vintage, noir… |
| `image.duplicates` ★ | — | perceptual-hash grouping, report, move duplicates aside |
| `image.optimize-gif` ★ | — | smaller animated GIFs (gifski / lossy palette) |
| `image.optimize-svg` ★ | — | minify/clean SVG |
| `image.colorblind` ★ | — | simulate protanopia/deuteranopia/tritanopia + contrast report |

## Video — owner C (`tools-media`, `engine-media`: ffmpeg/ffprobe, whisper.cpp, yt-dlp)

| id | Hangar name |
|---|---|
| `video.trim` | Trim video (batch: start/end/duration; Trim Studio in wave 2) |
| `video.convert` | Convert video |
| `video.extract-audio` | Extract audio |
| `video.to-gif` | Video to GIF (Quality / Balanced / Small, colours, dithering, fps, width) |
| `video.compress` | Compress video (target quality, target size, codec-preserving mode) |
| `video.split` | Split video (parts / length / scenes) |
| `video.resize` | Resize video |
| `video.crop` | Crop video (aspect ratio, auto black-bar removal) |
| `video.rotate` | Rotate video |
| `video.speed` | Change speed (pitch kept) |
| `video.mute` | Mute video |
| `video.join` | Join videos (re-encode when needed, explicit quality) |
| `video.insert-clips` | Insert clips (intro, clip at first pause, outro) |
| `video.watermark` | Watermark video |
| `video.captions` | Caption video (burn-in from speech or SRT; styles) |
| `video.add-audio` | Add audio (replace / mix, volume, loop) |
| `video.frames` ★ | Extract frames / thumbnails / contact strip |
| `video.remove-silences` ★ | Cut silences automatically |
| `video.stabilize` ★ | Stabilize shaky footage |

## Audio — owner C

| id | Hangar name |
|---|---|
| `audio.transcribe` | Transcribe (video or audio → TXT, SRT, VTT, DOCX, PDF; models tiny→large-v3-turbo; language auto) |
| `audio.merge` | Merge audio (stitch or layer) |
| `audio.convert` | Convert audio (bit depth preserved when possible) |
| `audio.compress` | Compress audio |
| `audio.trim` | Trim audio |
| `audio.split` | Split audio (pauses / length / parts) |
| `audio.speed` | Audio speed (pitch kept or shifted) |
| `audio.normalize` | Volume & normalize (EBU R128 target, single gain) |
| `audio.denoise` ★ | Denoise (RNNoise / ffmpeg arnndn) |

## Create — wave 2 (`tools-create`)

`create.qr-code` QR code · `create.mockup` Device mockup · `create.html-to-image` HTML to Image ·
`create.html-to-pdf` HTML to PDF · `create.audiogram` Audiogram · `create.gradient` Gradients ·
`create.shader` Shaders · `create.pattern` Pattern & texture (S) · `create.video-download` Video
downloader · `create.carousel` Carousel · `create.fonts` Fonts · `create.motion` Motion (S) ·
`create.social-card` Social card (S) · `create.placeholder` Placeholders (S) ·
`create.watermark-tile` Watermark tile (S) · `create.brand-sheet` Brand sheet (S) ·
`create.data-merge` Data merge (S) · ★ `create.barcode` · ★ `create.favicon` · ★ `create.formula`

## Files — wave 2 (`tools-files`)

`files.rename` Batch rename · `files.archive` Create archive · `files.extract` Extract archive ·
`files.dates` Change file dates · `files.sort` Sort & route · `files.metadata` Metadata (S) ·
★ `files.checksum` (done, reference example) · ★ `files.duplicates` ·
★ `files.identify` (what is this file? content-based type detection, confidence, wrong-extension
finder and fixer — idea from TridUI)

## Around the tools (Hangar features → Castor)

| Hangar | Castor |
|---|---|
| Studios: PDF, Trim, Caption, Sign | wave 2 |
| Chains (templates, folders) | ✓ core; UI wave 1 (D); watch folders ★ wave 2 |
| Runs history | ✓ core; UI wave 1 (D); undo for in-place tools |
| Workspaces (brand, output, history) | wave 2 |
| ⌘K smart search (127 MB model) | wave 1 keyword search in 9 languages; optional small embedding model ★ later |
| Menu bar / tray quick panel | wave 1 (D) + clipboard actions ★ + global shortcut ★ |
| In-app updates | Velopack (D: in-app flow; packaging in wave 2) |
| Licence key, 3-day trial | none: free, GPL-3.0 |
| — | ★ CLI `castor-cli`, ★ MCP server, ★ Finder/Explorer context menu (wave 2) |
