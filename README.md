<div align="center">
  <img src="assets/brand/png/castor-icon-256.png" width="128" height="128" alt="Castor icon: an ink diamond with a calligraphic C, one corner gnawed off">

  <h1>Castor</h1>

  <p><strong>Every file tool, right on your desk. Free, offline, yours.</strong></p>

  <p>
    <a href="https://github.com/basedpolymer/castor/releases/latest"><img src="assets/badges/macos.svg" alt="Available on macOS" height="40"></a>
    <a href="https://github.com/basedpolymer/castor/releases/latest"><img src="assets/badges/windows.svg" alt="Available on Windows" height="40"></a>
    <a href="https://basedpolymer.eu/castor"><img src="assets/badges/website.svg" alt="Visit the website" height="40"></a>
    <a href="https://www.rust-lang.org"><img src="assets/badges/rust.svg" alt="Built with Rust" height="40"></a>
  </p>

  <p>
    <a href="https://github.com/basedpolymer/castor/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/basedpolymer/castor?style=for-the-badge&color=ccfaff&label=release"></a>
    <a href="https://github.com/basedpolymer/castor/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/basedpolymer/castor/total?style=for-the-badge&color=ade1ff"></a>
    <a href="LICENSE"><img alt="Licence GPL-3.0" src="https://img.shields.io/badge/licence-GPL--3.0-fbe2ff?style=for-the-badge"></a>
    <a href="docs/TOOLS.md"><img alt="Tools" src="https://img.shields.io/badge/tools-100%2B-aafaba?style=for-the-badge"></a>
    <a href="https://github.com/basedpolymer/castor/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/basedpolymer/castor?style=for-the-badge&color=ffeca4"></a>
  </p>
</div>

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
License). Platform badges from [JMcrafter26/badges](https://github.com/JMcrafter26/badges) (MIT,
`assets/badges/LICENSE`). Third-party components are listed in `THIRD_PARTY.md`.
