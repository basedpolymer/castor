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

**Status: INTERIM (paused 2026-11-06 at the coordinator's request while agent D finishes).**
Scope of this wave as briefed by the coordinator: Windows first (local update test, no GitHub
release), macOS built + bundled + ad-hoc signed over SSH, real signing/notarisation as a
separate script for a GUI session. Channel names: `win` / `osx` (Velopack defaults), not `stable`.

### Produced (all under `packaging/` unless noted)

| file | what |
|---|---|
| `pins.env` | every bundled third-party binary pinned (URL + SHA-256 from the publisher / GitHub asset digest), vpk + dmgbuild versions, app id/title/authors/bundle id |
| `windows/build.ps1` | steps `build, deps, stage, check, pack`: cargo build → FFmpeg (BtbN GPL shared 9.0.2, verified) + whisper-cli v1.9.5 built from the verified tag commit (MSVC, shared ggml with runtime CPU dispatch SSE4.2…AVX-512, static CRT, no OpenMP) → stage (`castor.exe`, `castor-cli.exe`, `castor-mcp.exe`, `bin\`, `assets\fonts\`, `LICENSE.txt`, `THIRD_PARTY.md`, `licenses\`) → checks with a bare `PATH` + isolated `CASTOR_HOME` (CLI list, ffmpeg/ffprobe/whisper-cli run, audio/PDF/checksum tool runs, `dumpbin` import scan) → `vpk pack` (Setup.exe, portable zip, full/delta nupkg, icon, splash, Start-menu shortcut, `vcredist143-x64` prerequisite) |
| `windows/update-test.ps1` | local end-to-end update test: pack A → silent install from a local feed → pack B (delta) → run A with `CASTOR_UPDATE_FEED=<feed>` and auto-install on → wait for the download → quit-to-apply (or click "Restart to update") → verify B runs; `-PatchedBuild` builds castor.exe from a patched *copy* of the sources (see request 1) without touching the tree; `-Uninstall` |
| `macos/build.sh` | **one app per architecture** (`--arch x64|arm64|all`), steps `build, deps, bundle, sign, check, dmg[, vpk]`, `--dry-run`, idempotent: release build per target → FFmpeg 9.0.2 (martin-riedl static, verified) + whisper-cli v1.9.5 (arm64: Metal embedded; x64: CPU + Accelerate, AVX2) + ONNX Runtime 1.23.2 dylib (x64 only) → `target/package/macos/<arch>/Castor.app` (Info.plist from `Info.plist.in`; ICNS via iconutil from the brand PNGs; thin helpers in `Contents/Resources/bin`; ORT in `Contents/Frameworks` on x64; fonts; licences) → ad-hoc signature → checks (archs, otool non-system deps; CLI runs with `env -i` for the host arch) → `Castor-<v>-macos-<arch>-unsigned.dmg` (dmgbuild, house-style background) → optional unsigned `vpk pack` (channel `osx-<arch>`) |
| `macos/sign-notarize.sh` | for Terminal.app on the Mac (keychain unlocked), per arch: preflight (identity read from the keychain at run time, never written; probe signature; notarytool profile `fouine`) → hardened-runtime signatures inside-out → `vpk pack --runtime osx-<arch> --channel osx-<arch> --signAppIdentity … --signEntitlements … --notaryProfile fouine --noInst` (signs, notarises, staples) → `Castor-<v>-macos-<arch>.dmg` built from the app *inside the Velopack portable zip* (so DMG installs can self-update) → DMG signed, notarised, stapled, `spctl` checks; `--no-vpk` fallback, `--dry-run` |
| `macos/common.sh`, `macos/entitlements.plist` (empty: no JIT, library validation on), `macos/Info.plist.in` (bundle id `eu.basedpolymer.castor`, productivity category, "© basedpolymer — GPL-3.0", "Open with" document types at rank Alternate: PDF, images, audio/video, archives, fonts, text/office), `macos/dmg/settings.py`, `macos/dmg/background.svg` | |
| `release.sh` | SemVer check, bumps `[workspace.package] version`, refreshes Cargo.lock offline, drafts the CHANGELOG section + `packaging/notes/<v>.md` from `git log`, then **prints** the commit/tag/build/`vpk download`/`sources.sh`/`vpk upload github` (one draft release: `win`, then `osx-x64` and `osx-arm64` with `--merge`)/`gh release upload` (DMGs, source archives) commands — runs none of them; `--dry-run` |
| `sources.sh` | GPL source offer: `git archive` of the exact pinned commits (FFmpeg as built for Windows `7adfbe40dc` and macOS `n9.0.2`, BtbN and martin-riedl build scripts, whisper.cpp v1.9.5) → reproducible `*.tar.gz` + `SHA256SUMS-sources.txt`, attached to each release. Run once: 5 archives, 46 MB, in `target-g/release-sources/` |
| `third-party/{generate.sh,about.toml,about.hbs,bundled.md}` + **`THIRD_PARTY.md`** (repo root, generated) | cargo-about 0.9.3 (`--locked --fail`, shipped targets only, no build/dev deps, own crates filtered): 896 crates, every licence in the GPL-compatible allow-list; hand-written part: source offer, bundled programs (FFmpeg Win/Mac with their library lists, whisper.cpp, ONNX Runtime, MuPDF + its libraries, Tesseract/Leptonica, Magika model, Velopack), fonts/artwork, on-demand downloads, detected-only programs. `generate.sh --check` fails when stale. |
| `homebrew/castor.rb`, `winget/basedpolymer.Castor.installer.yaml` | drafts (no `basedpolymer/homebrew-tap` repo exists yet) |
| `.github/workflows/ci.yml` | Windows-only build + tests (core, PDF, files) + CLI smoke; manual `package` job running `build.ps1` and keeping Setup/portable as artifacts. No secrets. |
| `docs/RELEASING.md` | the release in 10 steps |

Tools installed: `vpk 1.3.39-g8d2844f` (= the velopack crate version) on the PC and on the Mac
(user-local .NET 8 in `~/.dotnet` via Microsoft's `dotnet-install.sh`, no sudo), `cargo-about
0.9.3` on the PC.

### Verified so far
- FFmpeg Windows zip: SHA-256 = GitHub asset digest = BtbN `checksums.sha256`; imports only
  UCRT/system DLLs + its own DLLs.
- whisper-cli Windows: first build imported `VCOMP140.DLL` (MSVC OpenMP, not in Windows) from
  the ggml CPU backends → rebuilt with `-DGGML_OPENMP=OFF` + static CRT; runs with a bare PATH.
- macOS helpers (on the Mac): checksums OK; x86_64 ffmpeg has subtitles, drawtext, zscale,
  arnndn, loudnorm, scdet, palettegen, deshake and libx264/x265/svtav1/vpx-vp9/opus/mp3lame/
  vorbis/webp + VideoToolbox encoders; **no vid.stab** (C's `deshake` fallback applies);
  ffmpeg, whisper-cli (both arches) depend only on system frameworks/libs.
- Signing identities on the Mac: one "Developer ID Application" (+ "Apple Development"), **no
  "Developer ID Installer"** → Velopack's macOS `.pkg` installer cannot be signed; we ship DMG
  + portable zip + nupkg (`--noInst`).
- **macOS Intel (x64) package, built and run on the Mac** (snapshot of the tree at 16:59;
  release profile; build 77 min wall with `-j 2` while the Mac was in use):
  - executables before `strip -x`: `castor` 195 MB, `castor-cli` 169 MB, `castor-mcp` 171 MB;
    after `strip -x` (now done in the bundle step): 151 / 132 / 134 MB. `__TEXT` alone is
    148 MB per executable: every binary statically links all engines and tools.
  - `Castor.app` 641 MB installed (3 × ~140 MB executables, ffmpeg + ffprobe 90 MB each, ONNX
    Runtime 38 MB, whisper-cli 3 MB); `Castor-0.1.0-macos-x64-unsigned.dmg` 240 MB;
    Velopack `Castor-osx-x64-Portable.zip` and `Castor-0.1.0-osx-x64-full.nupkg` 248 MB each
    (`vpk pack` unsigned: OK, 53 s; file names as expected by `sign-notarize.sh`).
  - checks with `env -i PATH=/usr/bin:/bin`: `castor-cli list` 110 lines; ffmpeg, whisper-cli
    run; `audio.convert` (bundled ffmpeg), `files.checksum`, `pdf.compress` (MuPDF, IRS W-9
    −13 %), `image.redact` (YuNet downloaded on first use, **ONNX Runtime 1.23.2 loaded from
    `Contents/Frameworks`** in 80 ms) all OK; otool: only system libraries.
  - GUI: `open -n --env CASTOR_HOME=… Castor.app` → main window (1180×760) on screen after
    **1.40 / 1.51 / 1.56 s** (three launches, measured with CGWindowList, aarch64 build running
    in the background); log clean, Latin Modern fonts registered. **Screenshot not possible
    over SSH**: macOS denies window capture without the Screen Recording permission (a
    full-screen capture shows only the wallpaper). Needs the user (Terminal.app with Screen
    Recording, or a manual ⌘⇧4). Note for D: `crates/castor-app/tools/shot.sh` fails on this
    macOS 15.8 (`for…of` in JXA → TypeError, and `$.kCGWindowList…` constants are undefined in
    JXA); a compiled Swift helper works (`/tmp/castor-g-wait.swift` on the Mac).
  - DMG background rendered **by Castor itself** (`castor-cli run image.convert background.svg
    --set format=png --set svg_size=660|1320`) → `packaging/macos/dmg/background{,@2x}.png`.
- **macOS Apple Silicon (arm64)**: first attempt failed (no aarch64 std for the stable 1.99
  toolchain; fixed by the coordinator), rebuild started 18:18 (`build-arm.log`);
  `pack-arm-after-build.sh` runs `build.sh --arch arm64 --steps deps,bundle,sign,check,dmg,vpk`
  automatically when it ends (`pack-arm.log`). Run checks are skipped on the Intel Mac (arch
  and otool checks only).
- `packaging/sources.sh` ran: 5 source archives (46 MB) + `SHA256SUMS-sources.txt` in
  `target-g/release-sources/`.

### Not done yet (exact next commands when resumed)
1. Windows release build was stopped at ~600/900 crates (coordinator, CPU):
   `pwsh packaging/windows/build.ps1` (from the repo root; build → deps (cached) → stage → check
   → pack; output in `target-g\releases\win`). Then measure sizes + cold start.
2. Update flow: `pwsh packaging/windows/update-test.ps1 -PatchedBuild -Mode quit`, then
   `-Mode restart` (click "Restart to update"), `-Verify`, `-Uninstall`.
3. Mac arm64: check `~/castor-mac-g/pack-arm.log`; then a GUI screenshot of both apps needs the
   user (Screen Recording). Re-sync the tree first if agents changed it, then rebuild both arches.
4. Size reduction (see request 7) once decided; re-measure.
5. Re-read D's and E's specs, regenerate `THIRD_PARTY.md`, final report.

### Requests to the coordinator
1. **(agent D) Local update feed for testing** — `crates/castor-app/src/updates.rs`, `manager()`:
   ```rust
   fn manager() -> Result<UpdateManager, String> {
       // Dev/test only: CASTOR_UPDATE_FEED=<local folder or URL> replaces the GitHub feed.
       let feed = std::env::var("CASTOR_UPDATE_FEED").ok().filter(|s| !s.trim().is_empty());
       let source = velopack::sources::AutoSource::new(feed.as_deref().unwrap_or(REPO));
       UpdateManager::new(source, None, None).map_err(|e| e.to_string())
   }
   ```
   (`AutoSource::new(REPO)` resolves to `GithubSource::new(REPO, None, false)`, i.e. today's
   behaviour; a non-URL string becomes a `FileSource`.) Drop the `GithubSource` import.
2. **(agent D) Explorer "Open with Castor" via Velopack hooks** — in `main.rs` (the
   `on_*_fast_callback` methods exist only on Windows in velopack 1.3.39):
   ```rust
   let mut app = velopack::VelopackApp::build();
   #[cfg(windows)]
   {
       app = app
           .on_after_install_fast_callback(|_| explorer::register())
           .on_after_update_fast_callback(|_| explorer::register())
           .on_before_uninstall_fast_callback(|_| explorer::unregister());
   }
   app.run();
   ```
   `explorer::register()` writes per-user keys (no admin) for `HKCU\Software\Classes\*\shell\Castor`
   and `HKCU\Software\Classes\Directory\shell\Castor`: default value / `MUIVerb` = localized
   "Open with Castor", `Icon` = `"<exe>",0`, subkey `command` default = `"<exe>" "%1"`, with
   `<exe>` = `std::env::current_exe()` (`%LOCALAPPDATA%\Castor\current\castor.exe`, stable across
   updates); `unregister()` deletes both `Castor` keys. Multi-selection launches one process per
   file; the single-instance forwarding already merges them. The "Castor ›" sub-menu with
   favourites can later replace the flat verb (`SubCommands` / `ExtendedSubCommandsKey`).
   Hooks must finish within 15–30 s and must not show UI.
3. **Root `Cargo.toml`**: no profile change needed so far (packaging uses `release`: thin LTO,
   cg 4; `dist` = fat LTO, cg 1 is much slower to build — measure before choosing for releases).
4. **Decided (coordinator)**: GPL source offer = source archives attached to each release
   (`packaging/sources.sh`, in `release.sh`, RELEASING step 8, THIRD_PARTY source-offer text).
5. **Decided**: macOS ships separate Intel (`x64`) and Apple Silicon (`arm64`) downloads, each
   with its own helpers and Velopack channel (`osx-x64`, `osx-arm64`); scripts, cask draft and
   RELEASING updated. The site needs three buttons: Windows, macOS Intel, macOS Apple Silicon.
6. **Decided**: Windows Start-menu shortcut only (already the case in `build.ps1`).
7. **Size — three ~140 MB copies of the same code** (agents D/E + root `Cargo.toml`): `castor`,
   `castor-cli` and `castor-mcp` each statically link every engine and tool (148 MB `__TEXT`
   each on macOS x64), so ~60 % of the 641 MB app is duplicated code. Options, biggest first:
   (a) **`castor-cli mcp`** (E's spec already plans it; `castor_mcp` is a library) and stop
   shipping the `castor-mcp` binary — MCP configs call `castor-cli mcp` (−134 MB);
   (b) on macOS, one multi-call binary (`castor` dispatches to the CLI when invoked as
   `castor-cli`, shipped as a symlink) — not possible on Windows, where the GUI exe uses the
   `windows` subsystem; (c) `strip = true` (symbols) in `[profile.release]`/`dist` — the
   packaging already does `strip -x` on macOS; (d) measure `dist` (fat LTO, cg 1) and
   `opt-level = "s"` for the heaviest non-hot crates (typst, tract, gpui).
