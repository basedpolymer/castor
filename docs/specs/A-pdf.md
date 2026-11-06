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
  Report "19 MB → 212 KB" via sizes; if the result is not smaller, copy the original and say
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
| id | status | notes |
|---|---|---|

### Third-party
| component | version | licence | how shipped (linked / sidecar / download) |
|---|---|---|---|

### Requests to the coordinator

### Packaging notes (for wave 2: what must be bundled per OS, sizes)
