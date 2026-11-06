# Spec G — packaging, signing, updates, release

Owner: agent G (wave 2). You own `packaging/`, `.github/workflows/` (optional CI for checks,
not for signing), `THIRD_PARTY.md`, `docs/RELEASING.md`, this file. Read `CLAUDE.md`,
`docs/ARCHITECTURE.md`, and the "Packaging notes" + "Third-party" sections of every spec in
`docs/specs/`.

## Goals

1. **macOS**: universal (x86_64 + arm64) `Castor.app` with `castor` (GUI) and `castor-cli`,
   bundled helpers (ffmpeg, ffprobe, whisper runtime, Tesseract/MuPDF data, Ghostscript if used —
   whatever the wave-1 reports require) in `Contents/Resources/bin` and dylibs in
   `Contents/Frameworks`, Info.plist (bundle id `eu.basedpolymer.castor`, document types for
   "Open with", LSApplicationCategoryType public.app-category.productivity, NSHumanReadable-
   Copyright "© basedpolymer — GPL-3.0"), icon from `assets/brand/` (ICNS), hardened runtime +
   entitlements (only what is needed), **signed with the Developer ID Application identity of the
   Mac keychain and notarised** (`notarytool` keychain profile `fouine` — ask the coordinator to
   have the user unlock the keychain first; never ask for or store passwords), stapled, in a DMG
   (drag-to-Applications background in the house style) and in Velopack's macOS package.
   Build on the Intel Mac (`~/castor-mac-g/`): x86_64 natively, aarch64 by cross-compiling
   (`rustup target add aarch64-apple-darwin`), `lipo` to universal; third-party binaries per arch.
2. **Windows**: x64 build, Velopack `Setup.exe` + portable zip, icon (ICO), file associations
   optional, Start menu shortcut, Explorer context menu "Open with Castor" (per-user registry,
   installed/removed by Velopack hooks), unsigned (SmartScreen note in the README/page).
3. **Updates**: `vpk pack` for both OS, release channel `stable`, GitHub Releases on
   github.com/basedpolymer/castor as the feed; verify the in-app flow end to end: install
   v0.9.0-test → publish v0.9.1-test to a **draft/pre-release** → app finds it, downloads,
   restarts into it (then delete the test releases).
4. **Licences**: `THIRD_PARTY.md` generated (`cargo-about` for crates + hand-written section for
   bundled binaries/models/fonts with licences and source links; GPL source offer = this repo
   + upstream URLs). Shown from Settings → About.
5. **Scripts**: `packaging/macos/build.sh` (build → bundle → sign → notarise → dmg → vpk),
   `packaging/windows/build.ps1` (build → bundle → vpk), `packaging/release.sh` (version bump,
   changelog, tag, upload). `docs/RELEASING.md` explains the whole release in ≤ 10 steps.
6. Homebrew cask (`basedpolymer/tap` if it exists, else document how to create it) and a
   `winget` manifest draft — optional, report what is needed.

Report sizes (download, installed) and startup time of release builds on both machines.

## Report (fill in before finishing)
