# Releasing Castor

Windows packages are built on the Windows PC, macOS packages on the Intel Mac (one package set
per architecture: Intel `x64` and Apple Silicon `arm64`). GitHub Releases on
github.com/basedpolymer/castor is the update feed, with three Velopack channels: `win`,
`osx-x64`, `osx-arm64`. Pinned third-party binaries and source commits: `packaging/pins.env`.
Scripts: `packaging/`.

One-time setup — Windows: PowerShell 7, Rust (MSVC), VS 2022 C++ + Windows SDK, LLVM, CMake,
.NET SDK 8+, `dotnet tool install -g vpk --version <VPK_VERSION>`, `cargo install cargo-about
--locked --features cli`, `gh auth login`. Mac: Xcode, Rust + both darwin targets, CMake, nasm,
.NET SDK (`dotnet-install.sh --install-dir ~/.dotnet`), the same `vpk`, the "Developer ID
Application" identity in the login keychain and a notarytool profile (`xcrun notarytool
store-credentials fouine`).

1. **Prepare** (Windows, clean tree on `main`): `bash packaging/release.sh X.Y.Z` — bumps the
   workspace version, drafts the `CHANGELOG.md` section and `packaging/notes/X.Y.Z.md`, and
   prints the exact commands of steps 3–9. Edit both notes files.
2. **Notices**: `bash packaging/third-party/generate.sh` (regenerates `THIRD_PARTY.md`; any
   crate under a licence outside the allow-list in `packaging/third-party/about.toml` stops it).
3. **Commit and tag** (printed by step 1): `git commit`, `git tag -a vX.Y.Z`.
4. **Windows build**: `vpk download github … --channel win -o target-g/releases/win` (previous
   release, for the delta), then `pwsh packaging/windows/build.ps1 -Version X.Y.Z -ReleaseNotes
   packaging/notes/X.Y.Z.md`. Read its `check` output: tool count, ffmpeg and whisper-cli run
   with a bare PATH, no missing DLL.
5. **Smoke test Windows**: run `target-g\releases\win\Castor-win-Setup.exe`, open a PDF, an
   image and a video tool, Settings → About → notices; uninstall from Settings → Apps.
6. **macOS build** (over SSH is fine): sync the tagged tree to `~/castor-mac-g`, then
   `packaging/macos/build.sh --version X.Y.Z` (both architectures: build, helpers, ad-hoc
   signed bundles, checks, unsigned DMGs for a quick look).
7. **macOS sign + notarise** (Terminal.app on the Mac — the keychain is locked over SSH): the two
   `vpk download github … --channel osx-x64 / osx-arm64` commands, then
   `packaging/macos/sign-notarize.sh --version X.Y.Z`. Copy `target/package/macos/release/`
   back to the PC.
8. **Source archives** (GPL source offer for the bundled FFmpeg and whisper.cpp):
   `bash packaging/sources.sh target-g/release-sources`.
9. **Push and upload as a draft**: `git push origin main vX.Y.Z`, the three `vpk upload github …
   --tag vX.Y.Z` commands (`win`, then `osx-x64` and `osx-arm64` with `--merge`), and
   `gh release upload` of the two DMGs and of the source archives + `SHA256SUMS-sources.txt`.
   Check the draft (Setup.exe, portable zips, DMGs, `*.nupkg`, `releases.*.json`, sources),
   set the notes, publish: `gh release edit vX.Y.Z --draft=false`.
10. **Verify updates**: an installed previous version finds X.Y.Z (Settings → Updates → Check
    now), downloads it and restarts into it — Windows, Intel Mac, Apple Silicon Mac. Update the
    site's download buttons (Windows, macOS Intel, macOS Apple Silicon).

Notes
- Windows builds are not code-signed: SmartScreen shows "Windows protected your PC" →
  "More info" → "Run anyway" on first install (say so on the download page). The installer
  creates a Start-menu shortcut only.
- Test a release without publishing: pack two versions into one local folder and point an
  installed app at it with `CASTOR_UPDATE_FEED=<folder>` (`packaging/windows/update-test.ps1`).
- Never commit downloaded binaries or archives; everything lands in `target-*/` (git-ignored).
