# Roadmap

## 0.9.1 (next, through the auto-update)

- **Settings → Models & helpers**: a "Download" button per row, "Download everything for offline
  use", readable names ("Background removal — BiRefNet lite") with the tools that use each one,
  and the full list (every OCR language, every Whisper model, not only the defaults), with
  progress, delete, and the disk total.
- Remove the app's `pdf_gate` serialisation now that engine-pdf shares mupdf-rs's locks
  (`crates/engine-pdf/tests/concurrency.rs`), so PDF thumbnails no longer wait for a running
  compression.
- Settings fields for the Explorer menu switch, watch folders and the PDF reviewer name (now kept
  in `tool_values`), and `RunRecord.workspace`.
- macOS Quick Actions (`.workflow` bundles: "Open in Castor", favourite tools) at packaging time.
- Windows update test: check that the Explorer "Castor ›" menu is present after install and gone
  after uninstall.

## Later

- Real-time playback in Trim & Caption Studio (today: scrub + play the selection in the system
  player).
- PDF/A-2b output for OCR, validated with veraPDF.
- Code-signed Windows builds.
- Homebrew tap and winget manifest (drafts in `packaging/`).
- HEIC decoding through the OS codecs (ImageIO / Windows HEVC extension) where available.
