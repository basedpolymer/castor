# Castor

**Every file tool, right on your desk. Free, offline, yours.**

Castor is a free, open-source desktop app for macOS and Windows that gathers about a hundred file
tools — PDF, image, video, audio, creation and file utilities — in one calm, fast window and in
your menu bar. Nothing leaves your machine.

> Work in progress. First release coming soon at <https://basedpolymer.eu/castor>.

- ~100 tools: compress, merge, OCR, redact and sign PDFs; convert, compress, resize, remove
  backgrounds and upscale images; trim, compress, caption and transcribe videos; QR codes,
  mockups, gradients and more — see [docs/TOOLS.md](docs/TOOLS.md).
- Chains of tools, run history with undo, a ⌘K palette that understands plain words, a menu bar
  quick panel, a CLI (`castor-cli`) and an MCP server for AI agents.
- Rust + [GPUI Kit](https://github.com/longbridge/gpui-kit). Style inspired by
  [TeXtured](https://github.com/jdujava/TeXtured).

## Build

```sh
cargo run -p castor-cli -- list          # every tool, from the terminal
cargo run -p castor-app                  # the app
```

## Licence

GPL-3.0-or-later © 2026 basedpolymer. Fonts: Latin Modern and TeX Gyre Chorus (GUST Font
License). Third-party components are listed in `THIRD_PARTY.md`.
