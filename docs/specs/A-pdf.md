# Spec A — PDF engine and PDF tools

Owner: agent A (wave 1). You own `crates/engine-pdf`, `crates/tools-pdf`, `locales/pdf.yml`,
this file. Build dir: `CARGO_TARGET_DIR=target-a`. Read `CLAUDE.md`, `docs/ARCHITECTURE.md`,
`docs/TOOLS.md` first, and `crates/tools-files/src/checksum.rs` (reference tool).

## Mission

Ship every PDF tool of `docs/TOOLS.md` § PDF (28 tools) **plus three image-family tools that are
really PDF/OCR work**: `image.to-pdf`, `image.images-to-pdf`, `image.extract-text` (they live in
`tools-pdf` with `family: Family::Image`). Quality bar: as good as or better than Hangar /
iLovePDF / Acrobat on real-world files; fast on 500-page documents; never corrupts a file.

## Engine choices (decide, verify, record)

Castor is GPL-3.0-or-later, so AGPL is allowed. Recommended:
- **MuPDF** through the `mupdf` crate (messense/mupdf-rs, AGPL) — rendering, text with
  positions (stext), page tree editing, grafting pages between documents (merge/split),
  annotations/redactions (`pdf_redact_page` with real content removal), forms, encryption
  (AES-256 on save), clean/garbage/compress, repair on load, and its HTML/EPUB/FB2/MOBI/CBZ
  layout engine for `pdf.from-ebook`. Use `mupdf-sys` FFI for what the safe API lacks (e.g.
  `pdf_rewrite_images` for image downsampling/recompression in `pdf.compress`,
  `pdf_subset_fonts`). Verify it builds on Windows (MSVC) and macOS Intel (`ssh macmac`).
- **OCR**: Tesseract. Preferred: MuPDF's built-in OCR device if `mupdf-sys` can be built with
  Tesseract; else the `tesseract` C++ library through a maintained crate; else bundle the
  `tesseract` executable as a sidecar (`castor_core::paths::find_program`). Traineddata
  (`tessdata_fast` or `_best`, Apache-2.0) downloaded on demand per language with
  `castor_core::download::Asset` (pinned SHA-256). Provide ≥ 40 languages including en, fr, de,
  es, it, pt, nl, pl, ru, uk, ja, zh (simplified + traditional), ko, ar, hi.
- **Ghostscript** (AGPL) is optional: only if it clearly beats MuPDF for `pdf.compress`
  "Maximum". If you use it, it must be a sidecar found via `find_program("gs")` (dev: install it
  locally), and the tool must still work (other presets) without it.
- **Typst** (`typst` + `typst-pdf` crates, Apache-2.0; `typst-as-lib` may help) for
  `pdf.from-markdown`: Markdown → Typst markup → PDF, house style = TeXtured look
  (Latin Modern from `assets/fonts`, numbered headings, captions). Options: paper size,
  font size, table of contents, numbered headings, title page.
- **LibreOffice** for `pdf.from-office`: detect `soffice` (Windows: Program Files, macOS:
  /Applications/LibreOffice.app) → `soffice --headless --convert-to pdf`; `unavailable()`
  explains when it is missing (with the download link in the help text).

## Public API of engine-pdf (others depend on it — keep these signatures)

```rust
pub mod ocr {
    pub struct Word { pub text: String, pub bbox: [f32; 4] /* x0,y0,x1,y1 px */, pub conf: f32 }
    pub struct Page { pub words: Vec<Word>, pub text: String /* reading order, \n between lines */ }
    /// (tesseract code, endonym), e.g. ("fra", "Français"). Used by `OptionSpec::language`.
    pub fn languages() -> &'static [(&'static str, &'static str)];
    /// Assets to download for "fra+eng" (not yet present ones are what the app asks consent for).
    pub fn assets(langs: &str) -> Vec<castor_core::download::Asset>;
    /// OCR an RGBA8 image. Downloads missing languages through `download::ensure`.
    pub fn recognize(rgba: &[u8], width: u32, height: u32, langs: &str, cx: &castor_core::RunCx) -> anyhow::Result<Page>;
}
/// PNG thumbnail of page `page` (0-based), longest side ≤ max_side.
pub fn thumbnail(path: &std::path::Path, page: usize, max_side: u32) -> anyhow::Result<Vec<u8>>;
/// Number of pages (fast).
pub fn page_count(path: &std::path::Path) -> anyhow::Result<usize>;
```
Agent B calls `ocr::recognize` for `image.redact` (text regions) — agree by the signature,
nothing else. The coordinator wires `thumbnail` into the app.

## Tool notes (beyond docs/TOOLS.md)

- `pdf.compress`: presets light/balanced/strong/max + custom (image DPI, JPEG quality,
  grayscale, strip metadata/bookmarks/tags/embedded files, subset fonts, object streams).
  Report "2.2 MB → 1.6 MB" via sizes; if the result is not smaller, copy the original and say
  so in `Output::note`. Test on scanned, vector-heavy, image-heavy and already-optimised PDFs.
- `pdf.ocr`: skip pages that already have text (option), deskew/rotate option, output PDF/A-2b
  if feasible, plus optional `.txt` sidecar. Must handle 300-page scans with progress per page.
- `pdf.redact`: search terms (multiline, case/whole-word), presets (email, phone, IBAN, credit
  card, French NIR/SIRET, dates), list import (TXT/CSV/TSV path). Real removal of text, images
  under the boxes and metadata; verify by re-extracting text in tests.
- `pdf.sign`: signature = image file or typed name rendered with TeX Gyre Chorus
  (`assets/fonts/texgyrechorus-mediumitalic.otf`); page (first/last/all/n), anchor, width,
  date line option. Saved signatures in `paths::data_dir()/signatures/`.
- `pdf.split`: plan returns one job per input; outputs many files named `name-p1-3.pdf`.
- `pdf.merge`: plan returns one job with all inputs (N → 1).
- `pdf.to-markdown`: headings by size clusters, bold/italic, lists, simple tables, images to a
  sibling folder, page breaks optional. Test on a real paper and a slide deck.
- `pdf.from-ebook`: EPUB/MOBI/FB2/CBZ/TXT/HTML via MuPDF layout; page size, margins, font size.
- `pdf.compare`: two PDFs → report PDF with changed regions highlighted + summary note.
- `image.to-pdf`: auto-crop the document, deskew, enhance (contrast, white balance, B/W option),
  then OCR layer. `image.images-to-pdf`: JPEG passthrough (no re-encode) when possible; page size
  fit/A4/Letter/original; margins; optional OCR. `image.extract-text`: TXT (and Markdown) output,
  languages option.
- Implement `preview` where it helps (stamp, page numbers, sign: render page 1 with the overlay).
- Implement `downloads()` for OCR tools (traineddata of selected languages).

## Testing

Generate fixtures into `tests/fixtures/generated/pdf/` (gitignored) from code (MuPDF/Typst) and
collect a few real public-domain PDFs (arXiv paper, a scanned public-domain book page from
Wikimedia Commons, a government form with fields). Integration tests per tool in
`crates/tools-pdf/tests/`. Run every tool once from the CLI on Windows; build `engine-pdf` +
`tools-pdf` on the Mac at least once (`~/castor-mac/`), and report.

## Report (fill in before finishing)

### Status per tool
✅ done and tested (integration test and/or CLI run on real files) · ⚠️ works with limits ·
❌ not done.

| id | status | notes |
|---|---|---|
| `pdf.compress` | ✅ | Presets light / balanced (150 dpi) / strong (120 dpi, lossless→JPEG) / max (96 dpi, CMYK→RGB, tags & PieceInfo dropped) / custom; grayscale, strip metadata / bookmarks / tags / attachments, font subsetting, object streams, max deflate effort; page thumbnails always dropped. Never larger: the original is copied with a note. If an image cannot be decoded (a damaged JBIG2 stream in the 1890 scan), it retries without bitonal recompression and recolouring, then without touching images, and the note says “some images are damaged and were left as they were” — verified end to end on that file with `max` (5.8 MB → 5.6 MB, 3–12 s; `balanced` gets −10 %). MuPDF `pdf_rewrite_images` + `pdf_subset_fonts` through the C shim. No Ghostscript. |
| `pdf.merge` | ✅ | N→1 in queue order, PDFs **and images**; bookmarks: one per file (default, original outlines nested) / keep / none. Own page grafting with one graft map: shared fonts copied once, **links between copied pages, annotations and form fields kept** (MuPDF's `graft_page` drops them). |
| `pdf.split` | ✅ | Every page / ranges ("1-3, 4-10") / every N / by size (estimated per page, parts never above the limit) / by top-level bookmarks (files named `01-Title.pdf`). Names `name-p1-3.pdf`. Outlines remapped per part. |
| `pdf.ocr` | ✅ | Tesseract inside MuPDF; invisible text layer with Tesseract's glyph-less font (Identity-H + ToUnicode, words stretched to their boxes; verified: search hits land on the visible words). Skip pages with text, deskew (text layer follows the tilt; page untouched), automatic 90/180/270° detection when confidence is poor, 200/300/400 dpi, `.txt` sidecar, page range, progress per page, cancel. 83 languages on demand. ⚠️ No PDF/A-2b output: claiming conformance needs an sRGB output intent, XMP `pdfaid`, every font of the *original* page embedded, and a validator (veraPDF) in the tests — not done in wave 2 (time), rather than ship an unverified “PDF/A” label. One OCR engine per process (see requests). |
| `pdf.redact` | ✅ | Terms (multiline, case, whole word; whitespace inside a term matches line breaks), presets: emails, phones, IBAN (mod-97 checked), cards (Luhn), French NIR / SIRET / SIREN (Luhn), dates (numeric + EN/FR month names); TXT/CSV/TSV list. MuPDF redaction: text removed, image pixels under boxes blanked, covered line art removed; boxes painted in the chosen colour; metadata stripped; saved with garbage=4; **re-extracts the text and refuses to write if anything matches**. |
| `pdf.sign` | ✅ | Typed (TeX Gyre Chorus, ink colour) or image (white background knocked out); last / first / all / chosen pages, 9 anchors, width, margin, date line, "keep for next time" → `data_dir()/signatures/`. Preview. Visual signature only (not a PKI certificate). |
| `pdf.encrypt` | ✅ | AES-256, open password and/or permissions (print, copy, edit, comment, forms, assemble); random owner password when only restrictions are set. |
| `pdf.unlock` | ✅ | With the known password; refuses non-protected files. |
| `pdf.to-images` | ✅ | PNG / JPG (quality) / WebP (lossless) / TIFF, 36–1200 dpi (capped at ~120 MP per page), gray option, page range. |
| `pdf.organize` | ✅ | Batch form: new order ("3, 1-2, 4-"), reverse, delete, rotate pages, blank page after each. Visual version = PDF Studio (wave 2). |
| `pdf.rotate` | ✅ | 90/180/270, all / odd / even / ranges. |
| `pdf.extract-pages` | ✅ | One file, or one file per range. |
| `pdf.page-numbers` | ✅ | 1 / Page 1 / 1 / N / Page 1 of N / i / I / custom `{n} {total} {roman}`; 9 positions, 9 fonts, size, colour, start, unnumbered first pages, mirror on even pages, margin, page range. Preview. |
| `pdf.stamp` | ✅ | Text (multi-line, any angle, auto-shrink to fit, font, colour) or image (width % of page); opacity, 9 positions, over/under content, page range. Preview. |
| `pdf.crop` | ✅ | Auto (content box from a 50 dpi render + padding, one common box by default) or fixed margins in mm; sets CropBox (TrimBox/ArtBox/BleedBox dropped). |
| `pdf.to-markdown` | ✅ | Headings from font-size clusters (4 levels), bold/italic/code runs, bullet & numbered lists, simple tables (gap-aligned columns), code blocks (monospace), images to `name-images/`, running headers/footers and page numbers removed, paragraphs re-joined across pages, optional page breaks. Tested on an arXiv paper. |
| `pdf.attach-to-each` | ✅ | Prepend / append / both, per input file. |
| `pdf.flatten` | ✅ | Forms and/or annotations baked (appearances regenerated first), AcroForm removed. |
| `pdf.repair` | ✅ | MuPDF rebuilds the xref on load, then a garbage-collected, cleaned save. Tested on a file with its xref cut off. |
| `pdf.from-office` | ✅ | LibreOffice `--headless --convert-to pdf` with a private profile (works while LibreOffice is open). Verified end to end with LibreOffice 25 (winget) on DOCX (table, list), XLSX (formula) and PPTX (2 slides): 15–17 s each, mostly LibreOffice start-up. `unavailable()` explains when it is missing (with the download site). |
| `pdf.to-epub` | ⚠️ Beta | EPUB 3 (nav, chapters split at top headings, images, CSS) from the same structure analysis as Markdown. Good on text documents; complex layouts (multi-column magazines) come out in MuPDF's reading order. |
| `pdf.from-ebook` | ✅ | Fixed (wave 2), see Performance below: 15 min → 1.3 s for a 35-page EPUB, and the text of the produced PDF is real text again (it came out as glyph ids). EPUB / MOBI / FB2 / CBZ / HTML / TXT via MuPDF layout; A5 / A4 / Letter / pocket / e-reader, font size, margins; outline, title and author carried over; generic CSS families rendered in Latin Modern. |
| `pdf.compare` | ✅ | Renders both versions (72–110 dpi by sensitivity), groups changed pixels into regions, report = summary page + pages of the new version with translucent highlight boxes; added/removed pages flagged. |
| `pdf.metadata` | ✅ | Fixed (wave 2): `set_info` held the trailer across an edit, but the first change to a loaded file opens a new xref section whose trailer is a copy — the Info dictionary went to the stale one. Now re-reads the trailer (as MuPDF's `pdf_set_metadata` does); test edits a file twice and reads back Title/Author/Subject. Title, author, subject, keywords, application (UTF-16 when needed); “remove all” also drops XMP and PieceInfo; note lists the old values. |
| `pdf.fill-form` | ✅ | List fields → JSON (type, value, options); fill from CSV/TSV (one PDF per row, file names from a column) / JSON (object or array) / "name = value" lines; checkboxes accept yes/no/1/x…; optional flatten. Tested on the IRS W-9. |
| `pdf.from-markdown` | ✅ | **Typst 0.15.1** (wave 2): pulldown-cmark → Typst markup → `typst-pdf`. House style: Latin Modern Roman 11 pt justified with hyphenation (language option), headings in Latin Modern Sans bold numbered 1 / 1.1 / 1.1.1, emphasis in Latin Modern italic (the *Slanted* face cannot be selected: Typst folds “Slanted” into the regular family), code in Latin Modern Mono (no syntax colours), figures from images with alt text → “**Figure n** / caption”, booktabs tables (1.2 pt top/bottom, 0.6 pt under the header, column alignment from Markdown), footnotes, block quotes, task lists, links, title page from front matter (title / author / date + diamond), table of contents, page numbers, A4 / Letter / A5, 8–14 pt. **Math**: `$…$` and `$$…$$` in LaTeX converted to Typst math (New Computer Modern Math): fractions, roots, sub/superscripts, Greek, big operators, relations, arrows, `\mathbb \mathcal \mathbf \mathrm \text \operatorname`, accents, `\left…
ight`, spacing, `cases`, `matrix/pmatrix/bmatrix/vmatrix`, `aligned`; unknown commands show as text, and if Typst still rejects a formula the document is rebuilt with formulas as code and the note says so. Text extracts as real text (verified with MuPDF and poppler; fonts embedded as subsets with ToUnicode). 0.1–0.3 s for the 4-page sample (debug). |
| `pdf.extract-images` | ✅ | JPEG / JPEG 2000 streams written byte for byte; everything else decoded at native size to PNG; duplicates skipped; minimum size. |
| `pdf.grayscale` | ✅ | `pdf_recolor_page` (vector + images) and output intents removed. |
| `image.to-pdf` | ⚠️ Beta | EXIF orientation, paper detection crop (bright sheet on darker background), deskew (projection profile ±10°), clean-up (white balance + contrast stretch, or Sauvola black & white), page size, OCR layer on by default. Crop is a heuristic, not a learned document detector. |
| `image.images-to-pdf` | ✅ | **JPEG passthrough** (verified byte-identical round trip), EXIF rotation applied without re-encoding, other formats via MuPDF or the `image` crate; page = image / A4 / Letter / A5 / A3 / Legal, orientation, margins, upscale, one PDF or one per image, optional OCR. |
| `image.extract-text` | ✅ | TXT or Markdown (paragraphs joined), languages, deskew, small screenshots upscaled ×2 before OCR. |

### Third-party
| component | version | licence | how shipped (linked / sidecar / download) |
|---|---|---|---|
| MuPDF (via `mupdf` / `mupdf-sys` crates, github.com/messense/mupdf-rs) | crates 0.8.0, MuPDF 1.27.2 | AGPL-3.0-or-later | statically linked (built from source by `mupdf-sys`) |
| MuPDF third-party libs: freetype, harfbuzz, libjpeg, openjpeg, jbig2dec, lcms2 (mt), zlib, brotli, gumbo, extract | bundled with MuPDF 1.27.2 | FTL/GPL-2, MIT, IJG, BSD-2, AGPL-3 (jbig2dec), MIT, Zlib, MIT, Apache-2.0, AGPL-3 | statically linked |
| Tesseract OCR (inside MuPDF `thirdparty/`) | 5.x as vendored by MuPDF 1.27.2 | Apache-2.0 | statically linked |
| Leptonica (inside MuPDF `thirdparty/`) | as vendored by MuPDF 1.27.2 | BSD-2-Clause | statically linked |
| Tesseract trained data `tessdata_fast` (83 languages, pinned commit 87416418657359cb625c412a48b6e1d6d41c29bd, SHA-256 per file in `engine-pdf/src/ocr.rs`) | 4.1 models | Apache-2.0 | downloaded on first use per language (0.4–8 MB each, eng 4.1 MB) into `downloads_dir()/tessdata/` |
| Tesseract "GlyphLessFont" (`pdf_ttf.h` → `engine-pdf/data/glyphless.ttf`, 573 B) | from Tesseract 5 | Apache-2.0 | compiled in (`include_bytes!`) |
| Latin Modern fonts (`assets/fonts`) | 2.004 | GUST Font License | compiled in (`include_bytes!`), embedded/subset in output PDFs |
| TeX Gyre Chorus (`assets/fonts`) | 2.003 | GUST Font License | compiled in, embedded in signed PDFs |
| `pulldown-cmark` | 0.13 | MIT | linked (Markdown parsing) |
| Typst (`typst`, `typst-pdf`, `typst-layout`) | 0.15.1 | Apache-2.0 | linked (`pdf.from-markdown`; shared with tools-create) |
| New Computer Modern Math (`tools-pdf/data/NewCMMath-Regular.otf`, from typst-assets 0.15.1) | 7.x | GUST Font License | compiled in (math in `pdf.from-markdown`) |
| `ttf-parser` | 0.25 | MIT/Apache-2.0 | linked (system font index, `engine-pdf/src/sysfonts.rs`) |
| `regex` | 1 | MIT/Apache-2.0 | linked (redaction presets) |
| `zip` | 2 | MIT | linked (EPUB writing) |
| `cc` (build), `bindgen` (build, via mupdf-sys) | 1 / 0.72.1 | MIT/Apache-2.0 / BSD-3 | build-time only |
| LibreOffice | user-installed | MPL-2.0 | not shipped: detected (`soffice`) for `pdf.from-office` |

### Verification done (Windows, wave 2)

- `cargo test -p engine-pdf -p tools-pdf`: **all green** — 4 engine unit tests, 2 engine
  tests, 7 tools unit tests (incl. LaTeX→Typst math, Typst escaping), 13 integration tests in
  `crates/tools-pdf/tests/tools.rs` covering all 31 tools: OCR with the real `eng.traineddata`,
  the e-book test re-enabled (with a < 60 s guard), Markdown→PDF text read back, metadata edited
  twice and read back, LibreOffice DOCX / XLSX / PPTX → PDF (skipped with a message when
  LibreOffice is absent). Test run: 39 s. Always test the two packages *together*:
  `-p engine-pdf` alone unifies features differently and triggers a separate MuPDF build.
- `bash crates/tools-pdf/tests/cli-smoke.sh target-a/debug/castor-cli.exe`: **37 runs over the
  31 tools, 37 passed, 3 min 31 s** (outputs in `tests/fixtures/generated/out-a/`), spot-checked
  with poppler (`pdftotext`, `pdffonts`, `pdfinfo`): redacted words gone, AES-256 reported,
  page numbers and signature present, Typst output fonts embedded + subset + Unicode maps,
  e-book PDF reads as clean text. Real-world inputs: arXiv paper, IRS W-9 form, 1890 scanned
  book (Wikimedia Commons), generated DOCX/XLSX/PPTX, a Markdown sample with math, table,
  figure, footnote, code (`tests/fixtures/generated/md/sample.md`).
- Rendered and inspected the Markdown→PDF pages (title page, contents, body with math and
  booktabs table, figure with “Figure 1 /” caption and code).
- Per-tool times in that smoke run (debug, shared CPU): most tools 0–3 s; balanced compress of
  the 168-page JBIG2 scan 87 s; OCR of 3 scanned pages 16 s; scan-to-PDF with OCR 14 s;
  LibreOffice 6–25 s per document.

### Performance (wave 2)

Root causes found and fixed:

1. **System fonts (to-images 13–16 s for one page, e-book minutes):** the `mupdf` crate's
   `system-fonts` loader (font-kit) enumerated every installed font on *each* lookup and never
   remembered a miss; a paper with a few symbols triggers dozens of lookups per page, an EPUB
   thousands. Replaced by `engine-pdf/src/sysfonts.rs`: installed fonts indexed once from their
   `name`/`head` tables only (≈0.3 s for all Windows fonts, cached on disk in
   `cache_dir()/fonts-index-2.tsv`), every answer memoised, files read once; generic CSS
   families (`serif`, `sans-serif`, `monospace`) map to the bundled Latin Modern.
2. **E-book / image conversion:** `Document::convert_to_pdf` of mupdf-rs 0.8.0 reuses freed
   page buffers after the first page (dangling `contents`/`resources` in its C wrapper), so
   page *n* carried pages 1…n on top of each other — slow, huge, and unreadable text. Replaced
   by `engine_pdf::convert_to_pdf` (MuPDF's PDF document writer, page by page). Should be
   reported upstream.
3. **Link time:** MuPDF built with `/GL` made every link re-run whole-program code generation
   (2 min 37 s to link the small example binary). With `_CL_=-GL-` in env.sh: 20–30 s.

| measurement (Windows, debug build, shared 6-core PC) | before | after |
|---|---|---|
| `pdf.to-images` arXiv paper, page 1 at 60 dpi | 13–16 s (castor-cli, coordinator) | **castor-cli: 0.69 s** with an empty `CASTOR_HOME` (font index built), **0.23 s** after (`castor-cli list`: 0.18 s) |
| `pdf.from-ebook`, 35-page EPUB | ~15 min (tiny EPUB) | 1.3 s |
| `pdf.from-markdown`, 4-page sample with math, TOC, figure | — (MuPDF HTML) | 0.07–0.3 s |
| `cargo test -p engine-pdf -p tools-pdf` (run, excl. build) | 514–952 s | 39 s (13 integration tests incl. OCR and 3 LibreOffice conversions) |
| `cli-smoke.sh`, all 31 tools (37 runs) | stopped after 19 tools | 3 min 31 s, 37/37 |
| link of a test binary / example | 2–3 min | 20–30 s |


Release vs debug (tools-pdf example `run_tool`, same machine, `cargo build --release`: 22 min
from scratch incl. the MuPDF C build):

| run | debug | release |
|---|---|---|
| to-images, arXiv page 1, 60 dpi | 0.30 s | 0.11 s |
| to-images, arXiv 15 pages, 150 dpi | 1.27 s | 1.08 s |
| compress (balanced), arXiv 2.2 MB | 2.31 s | 2.19 s |
| OCR, 3 scanned pages, 300 dpi | 6.31 s | 5.86 s |
| from-markdown, sample with TOC + math | 0.47 s | 0.05 s |
| redact (term + emails), 15 pages | 1.79 s | 1.66 s |
| split every page, 15 pages | 0.62 s | 0.63 s |

(MuPDF's C code is built in Release mode even in debug builds, so only the Rust-heavy steps —
Typst, image encoding — gain much.)

### Requests to the coordinator

1. **Build environment for `mupdf-sys` (blocking for anyone building the workspace).**
   - `bindgen` needs libclang *with its resource headers*: on Windows install LLVM
     (`winget install LLVM.LLVM`, done on this PC: LLVM 23.1.5 in `C:\Program Files\LLVM`) or set
     `LIBCLANG_PATH`. macOS: Xcode / Command Line Tools libclang works.
   - `mupdf-sys 0.8.0` has a bug on MSVC (and Apple): clang declares `max_align_t` as a typedef
     of a primitive there, bindgen drops it, and the `mupdf` crate fails with
     "cannot find type `max_align_t`". Work-around shipped in
     `crates/engine-pdf/bindgen/max_align_t.h`; builds need
     `BINDGEN_EXTRA_CLANG_ARGS="-D__CLANG_MAX_ALIGN_T_DEFINED -include <abs path>/crates/engine-pdf/bindgen/max_align_t.h"`.
     `crates/engine-pdf/env.sh` sets it (and `LIBCLANG_PATH` on Windows) for Git Bash / macOS
     shells. Please add it to the root `.cargo/config.toml` `[env]` (absolute path: the variable
     is read by the mupdf-sys build script, whose working directory is the registry) or to the
     CI/dev scripts. Long term: report upstream (messense/mupdf-rs) and drop the work-around.
   - Changing `LIBCLANG_PATH` / `BINDGEN_EXTRA_CLANG_ARGS` re-runs the MuPDF C build (~10 min on
     this PC, much longer on the Mac). Keep them stable.
2. **Thumbnails**: wire `engine_pdf::thumbnail(path, 0, max_side)` into
   `castor_tools::thumbnail` for `FileKind::Pdf` (also works for EPUB/XPS/CBZ via MuPDF).
3. `Requirement::Pdfium` in `castor-core/src/tool.rs` is stale (the engine is MuPDF, statically
   linked; nothing to locate). It can be removed or renamed.
4. **`pdf.from-markdown` now uses Typst 0.15.1** (same version as `tools-create`, so no extra
   crates in the workspace): pulldown-cmark → Typst markup → `typst-pdf`. Fonts are compiled
   in (Latin Modern ×7 from `assets/fonts`, New Computer Modern Math from typst-assets, GUST
   licence) — no system fonts, no network.
5. **Link time** (wave 2): MuPDF's Visual Studio projects build with `/GL`, so *every* link of a
   binary containing MuPDF re-ran whole-program code generation over the 600 MB library
   (2–3 min per test binary, CLI or app). `crates/engine-pdf/env.sh` now exports `_CL_=-GL-`
   (dash form on purpose: Git Bash rewrites `/GL-` into a path — that version, briefly in
   env.sh, makes the MuPDF build fail with `C1083 Cannot open source file 'C:/Program'`; if a
   build failed like that, re-source env.sh and rebuild). It only takes effect when mupdf-sys
   is rebuilt (delete `target-*/debug/build/mupdf-sys-*` and `.fingerprint/mupdf-sys-*` once).
   Please put the same variable in CI.
6. Ghostscript was not used: MuPDF's image rewriter + font subsetting + object streams reach the
   same sizes on the test files without a sidecar.
7. OCR runs one engine at a time per process (MuPDF's Tesseract glue keeps Leptonica's
   allocator in a global): OCR tools are `serial`, and parallel OCR jobs from different tools
   queue (with a "waiting" message, cancellable).

### Packaging notes (for wave 2: what must be bundled per OS, sizes)

- **Nothing to bundle for PDF**: MuPDF, its third-party libraries, Tesseract and Leptonica are
  compiled and statically linked into the binaries by `mupdf-sys` (MSBuild on Windows, make on
  macOS). Expect roughly +25–35 MB per executable (`castor`, `castor-cli`) in release; fonts
  (Latin Modern ×6, Chorus, glyph-less) add ~1 MB. MuPDF's own CJK/Noto fonts are *not*
  compiled in (mupdf-sys default); fonts a document names but does not embed, CJK and fallback
  glyphs come from the installed system fonts through Castor's own indexed loader
  (`engine-pdf/src/sysfonts.rs`; the font-kit based `system-fonts` feature is off). Typst
  (`pdf.from-markdown`) adds the New Computer Modern Math font (1.3 MB) and shares the Typst
  crates with tools-create.
- **Downloads** (first use, with consent through `Tool::downloads`): Tesseract `tessdata_fast`
  per language, 0.4–8 MB each (eng 4.1 MB, fra 1.1 MB, deu 1.5 MB, chi_sim 2.5 MB), stored in
  `downloads_dir()/tessdata/<lang>.traineddata`, SHA-256 pinned.
- **Optional, user-installed**: LibreOffice (`pdf.from-office`), detected in Program Files /
  `/Applications/LibreOffice.app` / PATH.
- **Build machines** need: MSVC 2022 + Windows SDK + LLVM (libclang) on Windows; Xcode or the
  Command Line Tools on macOS (make, clang, libclang). Plus the `BINDGEN_EXTRA_CLANG_ARGS`
  work-around above. The MuPDF C build takes ~10 min on the 6-core PC; the Rust side is normal.
- **Not yet verified on macOS** (Mac builds were stopped by the coordinator for this wave; the
  sources are in `~/castor-mac-a`, no successful build). To verify before publication:
  `mupdf-sys` make build with Xcode CLT, bindgen with
  `LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib` and the `max_align_t`
  work-around (the Apple branch of `bindgen/max_align_t.h`, `long double`), the C shim
  (`csrc/castor_mupdf.c`, needs the mupdf-sys headers found by `build.rs`), the internal
  Tesseract symbols `ocr_init/ocr_recognise/ocr_fin` + `TessBaseAPI*` C API being exported from
  the static libs, the system font index on macOS font folders (`/System/Library/Fonts`,
  `/Library/Fonts`, `~/Library/Fonts`; `.ttc` collections) and the CJK/fallback family names
  in `sysfonts.rs` (PingFang, Hiragino, Apple SD Gothic Neo…), LibreOffice detection in
  `/Applications/LibreOffice.app`, then `cargo test -p engine-pdf -p tools-pdf` and
  `bash crates/tools-pdf/tests/cli-smoke.sh target/debug/castor-cli`.
- **Windows builds**: source `crates/engine-pdf/env.sh` (libclang, `max_align_t`, `_CL_=-GL-`).
  The wave-1 one-off LNK1169 at `castor-cli` link did not come back in wave 2 (castor-cli built
  and linked cleanly, 37/37 smoke runs).
- Licence: shipping MuPDF makes the binaries AGPL-3.0-compatible GPL-3.0-or-later; the About
  box / third-party notices must list MuPDF (AGPL-3.0) with a source offer (the GitHub repo).
