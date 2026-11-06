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
| `macos/common.sh`, `macos/castor.entitlements` (empty: no JIT, library validation on), `macos/Info.plist.in` (bundle id `eu.basedpolymer.castor`, productivity category, "© basedpolymer — GPL-3.0", "Open with" document types at rank Alternate: PDF, images, audio/video, archives, fonts, text/office), `macos/dmg/settings.py`, `macos/dmg/background.svg` | |
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

## Update — 2026-11-06 (evening): single binary, Windows packages, update flow, both Mac arches

Picked up from the interim report above (history kept). Tree: commit `1aec8ec` (single binary)
plus the app agent's work in progress at ~20:25 (it already includes the `CASTOR_UPDATE_FEED`
override, so no patched build was needed). Request 7 (a) is done by the coordinator's single
binary.

### Changed in `packaging/` and around
- **Single-binary layout everywhere.** Builds are `-p castor-app -p castor-launcher` (`--locked`);
  shipped: `castor(.exe)` (app + `castor cli` + `castor mcp`) and the launcher
  `castor-cli-launcher` renamed to `castor-cli(.exe)` next to it. No `castor-mcp`, no full
  `castor-cli` any more: `windows/build.ps1`, `windows/update-test.ps1`, `macos/build.sh`,
  `macos/sign-notarize.sh` (nested code = dylibs, helpers, `castor-cli`), CI, cask, winget
  draft, RELEASING.
- **Checks** (Windows and Mac, bare PATH + isolated `CASTOR_HOME`): `castor-cli list` through
  the launcher = `castor cli list` direct (110 lines each); `castor mcp` gets `initialize` +
  `tools/list` over stdio (Windows: started with redirected pipes like an MCP client —
  PowerShell does not pipe stdin into GUI-subsystem programs); Windows also times the main
  window (3 launches); Mac also tries the launcher through a symlink (Homebrew).
- `build.ps1`: `-Steps a,b` works with `pwsh -File`; no positional binding (an empty splat
  bound `$null` to `-CargoProfile`); `_CL_=-GL-` like `env.sh`; `vpk pack --runtime win-x64`
  (vpk defaulted to x86); the DLL check knows the system DLLs FFmpeg/castor import (USP10,
  AVICAP32, UIAutomationCore, icuuc, gdiplus, DirectML) and `MSVCP140_1` (VC++ redist).
- `update-test.ps1`: waits for Setup/Update (`Start-Process -Wait`: GUI programs), waits for the
  **full** `*-<B>-full.nupkg` (the delta is first rebuilt as `.partial`; quitting earlier
  abandons the download), waits for `Update.exe` to finish before relaunching, fails if the
  relaunched app exits; the test profile turns off keep-in-tray and the global hotkey.
- **macOS arm64 link fix**: pyke's static ONNX Runtime (arm64) references
  `___isPlatformVersionAtLeast` (Objective-C `@available`, clang runtime); rustc links with
  `-nodefaultlibs`, so `castor` is linked with `cargo rustc … -p castor-app --bin castor --
  -C link-arg=<clang resource dir>/lib/darwin/libclang_rt.osx.a` (final link only, no
  dependency rebuild). Those ORT objects are built for macOS 13.4, so the Apple Silicon app
  declares `LSMinimumSystemVersion` 13.4 (Intel stays 11.0): `min_macos()` in
  `macos/common.sh`, `@MINOS@` in `Info.plist.in`.
- `release.sh` uploads **stable-named copies** `Castor-windows-setup.exe`,
  `Castor-macos-x64.dmg`, `Castor-macos-arm64.dmg` (the site links to
  `…/releases/latest/download/<name>`) and prints a `curl -I` loop checking the three redirects
  after publishing; RELEASING steps 9–10 say so (dry run checked).
- `THIRD_PARTY.md` regenerated (898 crates; picks up `winreg`, just added by the app agent);
  `bundled.md` gained "Main libraries compiled into Castor" (Typst + krilla, GPUI Kit, tract,
  rxing, qrcode/rqrr, resvg/tiny-skia, libimagequant, gifski/dssim, zenjpeg/heic, rawler,
  libwebp/rav1e/libzstd/libdeflate, archive crates incl. libbzip2's own licence, rmcp/tokio),
  the Magika model row (`standard_v3_3`, © Google) and New Computer Modern Math from
  typst-assets 0.15.1 (now in `assets/fonts`). `generate.sh --check` is green against the
  current `Cargo.lock`; re-run `generate.sh` whenever the lock changes (RELEASING step 2).

### Results — Windows x64 (release profile, `target-g`)
| | |
|---|---|
| cargo build (castor-app + launcher, 4 jobs, MuPDF rebuilt) | 26 min 35 s |
| `castor.exe` / `castor-cli.exe` | 190.1 MB / 0.22 MB |
| staged app folder (= installed `current\`) | 386.6 MB |
| `target-g\releases\win\Castor-win-Setup.exe` | **163.2 MB** |
| `Castor-win-Portable.zip` / `Castor-0.1.0-full.nupkg` | 155.1 MB / 155.2 MB |
| installed on disk (`%LOCALAPPDATA%\Castor`: `current` + Velopack's package cache + Update.exe) | 548 MB |
| main window after launch (staged app, bare PATH, fresh profile) | **0.84 s** first launch after staging, then 0.60 / 0.63 s |
| checks | 110 tools via launcher and direct; `castor mcp`: server "castor" 0.1.0, protocol 2025-06-18, 5 MCP tools; ffmpeg/ffprobe/whisper-cli run; audio.convert, files.checksum, pdf.compress OK; no missing DLL |

`castor.exe` hard-imports `DirectML.dll` (ordinal 2) and `icuuc.dll` from System32 (pyke's ORT
is the DirectML build): present on Windows 10 2004+ and 11, the effective minimum. Installed
app: `castor-cli.exe list` → 110 lines; Start-menu shortcut created, removed by the uninstall.
A truly cold start (empty file cache) was not measured (flushing the standby list needs admin).

**Update flow (local feed, nothing published): PASS.** Silent Setup of 0.9.0-test (9–18 s) →
0.9.1-test packed into the same folder (full + 37 KB delta) → installed app started with
`CASTOR_UPDATE_FEED=<folder>` and auto-install on → finds 0.9.1-test, downloads the delta,
rebuilds the full package → window closed → `Update.exe apply --waitPid … --silent --norestart`
→ `sq.version` = 0.9.1-test → the relaunched app runs from `current\` → uninstall clean.
Not tested: the "Restart to update" button (needs a click: `-Mode restart`, then `-Verify`).
Seen: a launch while `Update.exe` is still finishing exits at once (the user clicks again).

### Results — macOS (Intel Mac, `~/castor-mac-g`, `-j 2`, Mac in use)
| | x64 (Intel) | arm64 (Apple Silicon) |
|---|---|---|
| cargo build (workspace crates + final link) | 37 min 43 s | 31 min 46 s |
| `castor` (after `strip -x`) / `castor-cli` | 156 MB / 400 KB | 156 MB / 416 KB |
| `Castor.app` | **382 MB** (was 641 MB) | **294 MB** |
| `Castor-0.1.0-macos-<arch>-unsigned.dmg` | **138 MB** (was 240 MB) | **120 MB** |
| Velopack (unsigned) portable zip = full nupkg | 145 MB | 125 MB |
| minimum macOS | 11.0 | 13.4 |

Paths: `~/castor-mac-g/target/package/macos/{x64,arm64}/Castor.app`, the DMGs in
`~/castor-mac-g/target/package/macos/`, unsigned Velopack sets in `…/releases-unsigned/`. Both
apps ad-hoc signed (`codesign --verify --strict` OK); otool: only system libraries (+ the
bundled ORT dylib on x64). x64 run checks with `env -i PATH=/usr/bin:/bin…`: 110 tools via the
launcher and direct, `castor mcp` initialize + 5 tools, ffmpeg, whisper-cli, audio.convert,
files.checksum, pdf.compress (IRS W-9 −13 %) OK. **`castor-cli` through a symlink fails** ("the
Castor app binary was not found next to this program": macOS `current_exe()` returns the
link's path) → request 9. GUI: `open -n` starts the Intel app (log clean, Latin Modern fonts
registered ~1 s after "Castor starting"); the window time could not be measured this time —
the display was asleep/locked, CGWindowList lists no on-screen window (earlier today:
1.40–1.56 s). arm64 run checks need an Apple Silicon Mac. `sign-notarize.sh --dry-run` over
SSH: both arches, single-binary nested list, `vpk pack --signAppIdentity … --notaryProfile
fouine`, DMGs made from the signed portable app.

**For the user, at release time, in Terminal.app on the Mac (keychain unlocked):**
`cd ~/castor-mac-g && packaging/macos/sign-notarize.sh`
(both arches, version from `Cargo.toml`; `--version X.Y.Z` to override). Output:
`~/castor-mac-g/target/package/macos/release/` (two signed, notarised, stapled DMGs + the
`osx-x64` / `osx-arm64` Velopack folders).

### Requests to the coordinator (new)
8. **(app agent) Explorer "Open with Castor"** (request 2) was not in the tested build: no
   `HKCU\Software\Classes\*\shell\Castor` after install (`winreg` just appeared in
   `Cargo.lock`, so it may be in progress).
9. **(castor-launcher) Resolve symlinks**: in `app_binary()`, take
   `std::fs::canonicalize(std::env::current_exe()?)` before `.parent()`. Homebrew's
   `binary "…/Contents/MacOS/castor-cli"` (and any `ln -s` into PATH) installs a symlink, and
   macOS `current_exe()` returns the link's path. `build.sh` prints a WARNING until fixed.
10. **(agent E / docs)** `docs/specs/E-create-files.md` packaging note 7 still says
    `Contents/MacOS/castor-mcp`; `docs/MCP.md` is right (`castor mcp`).
11. **(app agent, minor)** macOS: after the app is killed (SIGTERM/crash) its single-instance
    socket stays; the next launch logs "single-instance socket unavailable: Address already in
    use" and runs without forwarding. Remove a stale socket when connecting to it fails.
12. **Decide**: Apple Silicon minimum macOS 13.4 (forced by the static ORT objects; every Apple
    Silicon Mac can run 14+) and Windows 10 version 2004 minimum — say so on the download page.
13. **Before publishing**: rebuild both OSes from the final committed tree (these packages are a
    snapshot of a tree the app agent was still editing), then RELEASING steps 1–10.
