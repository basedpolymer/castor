<div align="center">
  <img src="assets/brand/png/castor-icon-256.png" width="128" height="128" alt="Castor icon: an ink diamond with a calligraphic C, one corner gnawed off">

  <h1>Castor</h1>

  <p><strong>Gnaws through your files. Free, offline, yours.</strong></p>

  <p>
    <a href="https://github.com/basedpolymer/castor/releases/latest"><img src="assets/badges/macos.svg" alt="Available on macOS" height="40"></a>
    <a href="https://github.com/basedpolymer/castor/releases/latest"><img src="assets/badges/windows.svg" alt="Available on Windows" height="40"></a>
    <a href="https://www.rust-lang.org"><img src="assets/badges/rust.svg" alt="Built with Rust" height="40"></a>
  </p>
</div>

Castor is a free, open-source desktop app for macOS and Windows that gathers **110 file tools** —
PDF, image, video, audio, creation and file utilities — in one calm, fast window and in your menu
bar. Everything runs on your computer: nothing is uploaded, there is no account and no telemetry.

**0.9 is a public beta.** Presentation page: <https://basedpolymer.eu/castor>
(français : <https://basedpolymer.eu/castor/fr>).

## Download

| System | Download | Requirements |
|---|---|---|
| macOS, Apple Silicon | [Castor-macos-arm64.dmg](https://github.com/basedpolymer/castor/releases/latest/download/Castor-macos-arm64.dmg) | macOS 13.4 or later |
| macOS, Intel | [Castor-macos-x64.dmg](https://github.com/basedpolymer/castor/releases/latest/download/Castor-macos-x64.dmg) | macOS 11 or later |
| Windows, x64 | [Castor-windows-setup.exe](https://github.com/basedpolymer/castor/releases/latest/download/Castor-windows-setup.exe) | Windows 10 (2004 or later) or 11 |

The Windows installer is not code-signed yet: if SmartScreen appears, choose
*More info → Run anyway*. Castor updates itself (Settings → Updates).

## What it does

- **PDF (28)** — compress, merge, split, OCR (83 languages), redact for real, sign, encrypt and
  unlock, organize, page numbers, stamps, PDF ↔ images / Markdown / ebooks, Office to PDF (with
  LibreOffice), compare, fill forms, metadata…
- **Image (25)** — remove backgrounds and upscale with local models, convert (HEIC, AVIF, JPEG XL,
  RAW, SVG…), compress (never bigger than the original), resize, crop, watermark, beautify
  screenshots, blur faces, icon packs, contact sheets, palettes, vectorize…
- **Video (19) and audio (9)** — trim, compress, convert, GIF, captions, Whisper transcription,
  loudness, denoise, cut silences, stabilize, extract frames…
- **Create (20)** — QR codes checked to scan, barcodes, device mockups, gradients, shaders,
  social cards, brand sheets, data merge, formulas, fonts, HTML to PDF, video downloader…
- **Files (9)** — batch rename, archives, file dates, sort, duplicates, “what is this file?”,
  metadata, checksums — with undo.
- **Studios** — a PDF studio (organize, annotate, sign, redact, fill forms) and a Trim & Caption
  studio (timeline, cut by transcript, captions that follow the cuts).
- **Around the tools** — chains of tools, run history with undo, workspaces (brand, output
  folder), a ⌘K / Ctrl K palette that understands plain words, a menu-bar / tray quick panel
  with clipboard actions and a global shortcut, watch folders, the Explorer “Castor ›” menu.
- **9 languages** — English, French, German, Spanish, Italian, Brazilian Portuguese, Japanese,
  Simplified Chinese, Korean.

The full catalogue with tool ids is in [docs/TOOLS.md](docs/TOOLS.md). Some tools need a model or
a helper the first time (background removal, upscaling, OCR languages, Whisper models): Castor
asks first, shows the size, downloads it once from a pinned address and checks it; after that
everything works offline.

## Command line and AI agents

Every tool also runs from a terminal and from AI agents, with the same options as in the app:

```sh
castor-cli list                                   # every tool (castor-cli --lang fr list)
castor-cli info pdf.compress                      # options, types, defaults
castor-cli run pdf.compress report.pdf --set preset=max --out ./out
castor-cli run image.remove-background ~/Pictures/products --out ./cutouts
castor-cli chain web-photos.toml ~/Pictures/trip
castor mcp                                        # MCP server over stdio
```

`castor-cli` is installed next to the app (`Castor.app/Contents/MacOS/` on macOS, the install
folder on Windows). To use Castor from Claude Desktop, Claude Code or any MCP client, see
[docs/MCP.md](docs/MCP.md).

## Build from source

Rust stable (see `rust-toolchain.toml`). Windows: Visual Studio 2022 Build Tools, the Windows SDK
and LLVM; macOS: Xcode with the Metal toolchain. ffmpeg on the `PATH` for the media tools.

```sh
. crates/engine-pdf/env.sh                 # Windows (Git Bash) only: MuPDF bindgen settings
cargo run -p castor-app                     # the app
cargo run -p castor-app -- cli list         # the command line, same binary
cargo run -p castor-app --no-default-features --features demo-tools   # UI only, builds in minutes
cargo test --workspace
```

How the code is organised: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Visual and wording
rules: [docs/DESIGN.md](docs/DESIGN.md). Packaging and releases: [docs/RELEASING.md](docs/RELEASING.md).

## Feedback

Castor is a beta: bug reports, ideas and translation fixes are welcome in
[issues](https://github.com/basedpolymer/castor/issues). Castor was inspired by
[Hangar](https://hangar.tools), a polished paid app with the same idea.

## Licence

GPL-3.0-or-later © 2026 basedpolymer. Fonts: Latin Modern, TeX Gyre Chorus and New Computer
Modern Math (GUST Font License). Platform badges from
[JMcrafter26/badges](https://github.com/JMcrafter26/badges) (MIT, `assets/badges/LICENSE`).
Every third-party component, bundled program and downloadable model is listed with its licence
in [THIRD_PARTY.md](THIRD_PARTY.md); the exact sources of the bundled FFmpeg and whisper.cpp are
attached to each release.
