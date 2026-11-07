# Roadmap

## 0.9.1 (next, through the auto-update) — done in the tree, to be released

- [x] Settings → Models & helpers: Download per row, "Download everything for offline use",
  readable names with the tools that use each file, the full list (every OCR language, every
  Whisper model), progress, delete, the disk total (`castor_tools::downloads`).
- [x] `pdf_gate` removed: PDF thumbnails no longer wait for a running compression.
- [x] `Settings::explorer_menu`, `watch_folders`, `pdf_reviewer` (moved from `tool_values` at
  load) and `RunRecord.workspace` (`runner::start_in`).
- [x] macOS Quick Actions: template in `packaging/macos/quick-action`, embedded in the app, which
  writes "Open in Castor" and one per favourite in `~/Library/Services` when switched on.
- [x] Windows update test: `update-test.ps1` checks the Explorer menu after install, after the
  update and after uninstall.
- [x] Small fixes: "1 file", decimal comma in French durations, OCR template in the UI language.
- To check on the release machines: build on Windows and macOS, `cargo test`, the Quick Actions
  in Finder, the quick panel on ⌥⌘Space, `update-test.ps1`.

## Later

- Real-time playback in Trim & Caption Studio (today: scrub + play the selection in the system
  player).
- PDF/A-2b output for OCR, validated with veraPDF.
- Code-signed Windows builds.
- Homebrew tap and winget manifest (drafts in `packaging/`).
- HEIC decoding through the OS codecs (ImageIO / Windows HEVC extension) where available.
