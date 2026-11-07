# Changelog

## 0.9.1 — unreleased

- **Settings → Models & helpers**: every model Castor can use (all 82 OCR languages, every
  Whisper size and precision, the image models, helper programs) with readable names and the
  tools that use them; a Download button per row with progress, Delete, the disk total, a filter
  (needed by default / on this machine / all) and "Download everything for offline use".
- **macOS**: Finder Quick Actions (right-click → Quick Actions → Open in Castor or a favourite
  tool), switched on in Settings → System integration. Fixed a crash when the quick panel was
  opened with ⌥⌘Space while another app (the Finder answers that shortcut too) took the focus.
- **MCP server**: values are checked strictly in `run_chain` too (a size written as text was
  ignored, a wrong key or value ran with defaults); out-of-range numbers, unknown colours and
  languages are errors instead of being clamped; relative paths are refused (the server does
  not run in your folder); skipped files are listed; one-time downloads wait for
  `allow_downloads: true` so the agent asks you first; cancelling a request stops the run, long
  runs report progress; new `preview_tool` and `undo_run`; tool annotations (read-only /
  destructive / open world) for the client's confirmations.
- The command line checks values the same way (`--set quality=500` is an error, not 100).
- Runs remember their workspace; Explorer menu, watch folders and the PDF reviewer name have
  their own settings (moved automatically from 0.9.0).
- PDF thumbnails no longer wait for a running PDF compression (the old serialisation is gone:
  the PDF engine is thread-safe).
- Small fixes: the State column of the file table no longer overflows; "1 file" instead of
  "1 files"; decimal comma in French durations and percentages ("2,1 s"); a file that is not
  empty never shows "−100 %"; the OCR step of "Scans to searchable PDF" reads your language;
  the missing delete icon in PDF Studio.

## 0.9.0 — public beta — 2026-11-06

The first public release of **Castor**: 110 file tools in one calm app for macOS and Windows.
Free, offline, open source (GPL-3.0). *Gnaws through your files.*

- **PDF (28)**: compress, merge, split, OCR in 83 languages, redact (really removed), sign,
  encrypt and unlock, organize pages, page numbers, stamps, PDF ↔ images, Markdown and ebooks,
  Office to PDF (with LibreOffice), compare, fill forms…
- **Image (25)**: remove backgrounds and upscale with local models, convert (HEIC, AVIF, JPEG XL,
  RAW, SVG…), compress without ever making a file bigger, resize, watermark, beautify screenshots,
  blur faces, icon packs, contact sheets…
- **Video and audio (28)**: trim, compress, convert, GIF, captions, transcription with Whisper,
  loudness, denoise, cut silences, stabilize…
- **Create (20)** and **Files (9)**: QR codes that are checked to scan, barcodes, mockups,
  gradients, social cards, brand sheets, data merge, batch rename, archives, duplicates,
  “what is this file?”…
- **Studios**: a PDF studio (organize, annotate, sign, redact, fill forms) and a Trim & Caption
  studio (timeline, cut by transcript, captions that follow the cuts).
- Chains, run history with undo, workspaces, a ⌘K/Ctrl K palette that understands plain words,
  a menu-bar / tray quick panel, watch folders, the Explorer “Castor ›” menu, `castor-cli` and an
  MCP server for AI agents. In 9 languages.

This is a beta: please report anything odd at https://github.com/basedpolymer/castor/issues.
Windows builds are not code-signed yet (SmartScreen: “More info → Run anyway”).

---

**Première version publique de Castor** : 110 outils de fichiers dans une app calme pour macOS et
Windows. Gratuit, hors ligne, libre (GPL-3.0). *Ronge vos fichiers à votre place.*
C'est une bêta : signalez tout ce qui cloche sur la page GitHub du projet.
