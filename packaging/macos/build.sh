#!/usr/bin/env bash
# Castor for macOS: one Castor.app PER ARCHITECTURE (Intel x64, Apple Silicon arm64), each with
# its own helpers, unsigned / ad-hoc signed, + DMG. (Separate downloads instead of a ~320 MB
# universal bundle: the static FFmpeg alone is ~95 MB x64 / ~66 MB arm64 per tool.)
#
#   packaging/macos/build.sh [--dry-run] [--version X.Y.Z] [--arch x64|arm64|all]
#                            [--steps build,deps,bundle,sign,check,dmg[,vpk]]
#
# Steps (all but vpk by default, in this order; each is idempotent and can be re-run alone):
#   build   cargo build --release --target x86_64-apple-darwin / aarch64-apple-darwin
#   deps    FFmpeg/ffprobe 9.0.2 (martin-riedl static GPL), whisper-cli v1.9.5 (built here:
#           arm64 with Metal, x64 CPU + Accelerate), ONNX Runtime 1.23.2 dylib (x64 only) — all
#           pinned + SHA-256 checked (packaging/pins.env), cached in $TARGET/deps
#   bundle  $TARGET/package/macos/<arch>/Castor.app (Info.plist, ICNS, helpers in
#           Contents/Resources/bin, ORT in Contents/Frameworks on x64, fonts, licences)
#   sign    ad-hoc signature (codesign -s -) so the bundle runs locally
#   check   arch/otool checks; run checks (CLI tools with a minimal PATH) for the host arch
#   dmg     Castor-<version>-macos-<arch>-unsigned.dmg, house-style background (dmgbuild)
#   vpk     unsigned Velopack packages (channel osx-<arch>), for local tests only
#
# The real release (Developer ID signature, notarisation, stapling, Velopack packages, signed
# DMGs) is packaging/macos/sign-notarize.sh, run afterwards in a GUI Terminal session on the
# Mac (the login keychain is locked over SSH). See docs/RELEASING.md.
#
# Prerequisites (on the build Mac): Xcode or Command Line Tools (clang, lipo, codesign,
# iconutil, hdiutil, tiffutil), Rust stable with `rustup target add x86_64-apple-darwin
# aarch64-apple-darwin`, CMake + git, nasm (rav1e), python3 (dmgbuild goes in a venv under
# $TARGET/deps), .NET SDK + vpk for the vpk step, ~25 GB free. Optional env: CARGO_TARGET_DIR
# (default ./target), CARGO_BUILD_JOBS (default 2: the 8 GB Intel Mac swaps with more),
# MACOSX_DEPLOYMENT_TARGET (default 11.0).

source "$(dirname "$0")/common.sh"

STEPS="build,deps,bundle,sign,check,dmg"
VERSION=""
ARCH_ARG="all"
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --steps) STEPS="$2"; shift ;;
    --version) VERSION="$2"; shift ;;
    --arch) ARCH_ARG="$2"; shift ;;
    -h|--help) sed -n '2,32p' "$0"; exit 0 ;;
    *) die "unknown argument $1" ;;
  esac
  shift
done
has() { [[ ",$STEPS," == *",$1,"* ]]; }
case "$ARCH_ARG" in all) ARCHS="x64 arm64" ;; x64|arm64) ARCHS="$ARCH_ARG" ;; *) die "--arch x64|arm64|all" ;; esac
VERSION="${VERSION:-$(workspace_version)}"
SHORT_VERSION="${VERSION%%-*}"           # CFBundleShortVersionString: digits only
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
HOST_ARCH="$(uname -m | sed 's/x86_64/x64/')"
note "Castor $VERSION (macOS ≥ $MACOSX_DEPLOYMENT_TARGET) arch=$ARCHS target=$TARGET steps=$STEPS dry-run=$DRY_RUN"

FF="$DEPS/ffmpeg-mac-$FFMPEG_MAC_VERSION"
WH="$DEPS/whisper.cpp-$WHISPER_TAG"
ORT="$DEPS/onnxruntime-osx-x86_64-$ORT_MAC_X64_VERSION"

# ------------------------------------------------------------------------------------ build
if has build; then
  # MuPDF bindgen work-around + libclang from the Command Line Tools (see CLAUDE.md, spec A).
  # shellcheck disable=SC1091
  source "$ROOT/crates/engine-pdf/env.sh"
  if [ -z "${LIBCLANG_PATH:-}" ] && [ -f /Library/Developer/CommandLineTools/usr/lib/libclang.dylib ]; then
    export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
  fi
  for a in $ARCHS; do
    t="$(rust_target "$a")"
    step "build (release, $t)"
    run rustup target add "$t"
    run cargo build --release --target "$t" -p castor-app -p castor-cli -p castor-mcp
  done
fi

# ------------------------------------------------------------------------------------ deps
if has deps; then
  common_flags=(-DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF
    -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF -DWHISPER_SDL2=OFF -DWHISPER_CURL=OFF
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET")
  for a in $ARCHS; do
    step "deps ($a): FFmpeg $FFMPEG_MAC_VERSION (martin-riedl static)"
    A_UP="$(echo "$a" | tr a-z A-Z)"
    for t in ffmpeg ffprobe; do
      T_UP="$(echo "$t" | tr a-z A-Z)"
      url_var="FFMPEG_MAC_${A_UP}_${T_UP}_URL"; sha_var="FFMPEG_MAC_${A_UP}_${T_UP}_SHA256"
      zip="$DEPS/dl/ffmpeg-mac-$FFMPEG_MAC_VERSION-$a-$t.zip"
      fetch "${!url_var}" "${!sha_var}" "$zip"
      if [ ! -x "$FF/$a/$t" ]; then
        run rm -rf "$FF/$a/$t.unpack"
        run mkdir -p "$FF/$a/$t.unpack"
        run unzip -q -o "$zip" -d "$FF/$a/$t.unpack"
        run mv "$FF/$a/$t.unpack/$t" "$FF/$a/$t"
        run rm -rf "$FF/$a/$t.unpack"
      fi
    done

    step "deps ($a): whisper.cpp $WHISPER_TAG"
    if [ ! -d "$WH/.git" ]; then
      run git -c advice.detachedHead=false clone --depth 1 --branch "$WHISPER_TAG" "$WHISPER_REPO" "$WH"
    fi
    if [ "$DRY_RUN" != 1 ]; then
      head="$(git -C "$WH" rev-parse HEAD)"
      [ "$head" = "$WHISPER_COMMIT" ] || die "whisper.cpp $WHISPER_TAG is $head, expected $WHISPER_COMMIT"
    fi
    if [ "$a" = x64 ] && [ ! -x "$WH/build-x86_64/bin/whisper-cli" ]; then
      # Intel: CPU + Accelerate, no Metal (Intel GPUs are slower than the CPU path). AVX2/FMA
      # baseline: every Intel Mac supported by macOS 11+ has it except the 2013 Mac Pro.
      run cmake -S "$WH" -B "$WH/build-x86_64" "${common_flags[@]}" -DCMAKE_OSX_ARCHITECTURES=x86_64 \
        -DGGML_METAL=OFF -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON
      run cmake --build "$WH/build-x86_64" --config Release -j "$CARGO_BUILD_JOBS" --target whisper-cli
    fi
    if [ "$a" = arm64 ] && [ ! -x "$WH/build-arm64/bin/whisper-cli" ]; then
      # Apple Silicon: Metal with the shader library embedded in the binary + Accelerate.
      run cmake -S "$WH" -B "$WH/build-arm64" "${common_flags[@]}" -DCMAKE_OSX_ARCHITECTURES=arm64 \
        -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON
      run cmake --build "$WH/build-arm64" --config Release -j "$CARGO_BUILD_JOBS" --target whisper-cli
    fi

    if [ "$a" = x64 ]; then
      step "deps (x64): ONNX Runtime $ORT_MAC_X64_VERSION"
      tgz="$DEPS/dl/$(basename "$ORT_MAC_X64_URL")"
      fetch "$ORT_MAC_X64_URL" "$ORT_MAC_X64_SHA256" "$tgz"
      if [ ! -f "$ORT/lib/libonnxruntime.$ORT_MAC_X64_VERSION.dylib" ]; then
        run mkdir -p "$DEPS/ort.unpack"
        run tar xzf "$tgz" -C "$DEPS/ort.unpack"
        run rm -rf "$ORT"
        run mv "$DEPS/ort.unpack/$(basename "$ORT")" "$ORT"
        run rm -rf "$DEPS/ort.unpack"
      fi
    fi
  done
fi

# ------------------------------------------------------------------------------------ bundle
if has bundle; then
  for a in $ARCHS; do
    APP="$(app_path "$a")"; C="$APP/Contents"; t="$(rust_target "$a")"
    step "bundle ($a) -> $APP"
    run rm -rf "$APP"
    run mkdir -p "$C/MacOS" "$C/Resources/bin" "$C/Resources/assets/fonts" "$C/Resources/licenses"
    for b in castor castor-cli castor-mcp; do
      run cp "$TARGET/$t/release/$b" "$C/MacOS/$b"
      # Drop local symbols (~40 MB per executable; panics still name public functions).
      run strip -x "$C/MacOS/$b"
    done
    if [ "$DRY_RUN" != 1 ]; then
      sed -e "s/@VERSION@/$SHORT_VERSION/" -e "s/@BUILD@/$SHORT_VERSION/" "$HERE/Info.plist.in" > "$C/Info.plist"
      plutil -lint "$C/Info.plist" >/dev/null
      printf 'APPL????' > "$C/PkgInfo"
    fi
    # Icon: the PNG renders of assets/brand/castor-icon.svg.
    set_dir="$PKG/castor.iconset"
    run rm -rf "$set_dir"; run mkdir -p "$set_dir"
    P="$ROOT/assets/brand/png"
    for s in 16 32 128 256 512; do
      run cp "$P/castor-icon-$s.png" "$set_dir/icon_${s}x${s}.png"
      run cp "$P/castor-icon-$((s * 2)).png" "$set_dir/icon_${s}x${s}@2x.png"
    done
    run iconutil -c icns -o "$C/Resources/castor.icns" "$set_dir"
    run rm -rf "$set_dir"
    # Helpers where find_program looks (Contents/Resources/bin).
    run cp "$FF/$a/ffmpeg" "$FF/$a/ffprobe" "$C/Resources/bin/"
    if [ "$a" = x64 ]; then
      run cp "$WH/build-x86_64/bin/whisper-cli" "$C/Resources/bin/"
      # ONNX Runtime for Intel Macs (Apple Silicon links it statically).
      run mkdir -p "$C/Frameworks"
      run cp "$ORT/lib/libonnxruntime.$ORT_MAC_X64_VERSION.dylib" "$C/Frameworks/libonnxruntime.dylib"
      run cp "$ORT/LICENSE" "$C/Resources/licenses/onnxruntime-LICENSE.txt"
      [ ! -f "$ORT/ThirdPartyNotices.txt" ] || run cp "$ORT/ThirdPartyNotices.txt" "$C/Resources/licenses/onnxruntime-ThirdPartyNotices.txt"
    else
      run cp "$WH/build-arm64/bin/whisper-cli" "$C/Resources/bin/"
    fi
    run cp "$WH/LICENSE" "$C/Resources/licenses/whisper.cpp-LICENSE.txt"
    run cp "$ROOT/assets/fonts/"* "$C/Resources/assets/fonts/"
    run cp "$ROOT/LICENSE" "$C/Resources/LICENSE.txt"
    if [ -f "$ROOT/THIRD_PARTY.md" ]; then run cp "$ROOT/THIRD_PARTY.md" "$C/Resources/"; else note "warning: THIRD_PARTY.md missing"; fi
    [ "$DRY_RUN" = 1 ] || du -sh "$APP"
  done
fi

# ------------------------------------------------------------------------------------ sign (ad-hoc)
if has sign; then
  for a in $ARCHS; do
    APP="$(app_path "$a")"; C="$APP/Contents"
    step "sign ($a, ad-hoc, local runs only)"
    # Inside out: nested code first, the bundle last.
    for f in "$C/Frameworks/"*.dylib "$C/Resources/bin/"* "$C/MacOS/castor-cli" "$C/MacOS/castor-mcp"; do
      [ -e "$f" ] || continue
      run codesign --force --sign - --timestamp=none "$f"
    done
    run codesign --force --sign - --timestamp=none "$APP"
    run codesign --verify --strict --verbose=1 "$APP"
  done
fi

# ------------------------------------------------------------------------------------ check
if has check; then
  for a in $ARCHS; do
    APP="$(app_path "$a")"; C="$APP/Contents"
    step "check ($a)"
    for f in "$C/MacOS/"* "$C/Resources/bin/"* "$C/Frameworks/"*; do
      [ -e "$f" ] || continue
      printf '%-28s %-8s %6s\n' "$(basename "$f")" "$(lipo -archs "$f")" "$(du -h "$f" | cut -f1)"
      # Every dynamic dependency must be part of macOS (or the bundle).
      otool -L "$f" | tail -n +2 | awk '{print $1}' \
        | grep -v -E '^(/usr/lib/|/System/Library/|@rpath/libonnxruntime|@executable_path/|@loader_path/)' \
        | sed "s/^/    NOT SYSTEM: /" || true
    done
    [ "$DRY_RUN" != 1 ] || continue
    if [ "$a" != "$HOST_ARCH" ]; then note "(not runnable on this $HOST_ARCH Mac: run checks skipped)"; continue; fi
    work="$PKG/check-$a"; rm -rf "$work"; mkdir -p "$work"
    # Minimal environment: only the OS on PATH, isolated profile.
    E=(env -i HOME="$HOME" PATH=/usr/bin:/bin:/usr/sbin:/sbin CASTOR_HOME="$work/home" TMPDIR="${TMPDIR:-/tmp}")
    "${E[@]}" "$C/MacOS/castor-cli" list | wc -l | sed 's/^ */castor-cli list: /;s/$/ lines/'
    "${E[@]}" "$C/Resources/bin/ffmpeg" -hide_banner -loglevel error -f lavfi -i sine=frequency=440:duration=2 -y "$work/tone.wav"
    "${E[@]}" "$C/Resources/bin/whisper-cli" --help > "$work/whisper-help.txt" 2>&1 || true
    note "whisper-cli --help: $(wc -l < "$work/whisper-help.txt") lines"
    "${E[@]}" "$C/MacOS/castor-cli" run audio.convert "$work/tone.wav" --set format=mp3 --out "$work/out"
    "${E[@]}" "$C/MacOS/castor-cli" run files.checksum "$work/tone.wav" --out "$work/out"
    pdf="$(find "$ROOT/tests/fixtures" -name '*.pdf' 2>/dev/null | head -1 || true)"
    [ -z "$pdf" ] || "${E[@]}" "$C/MacOS/castor-cli" run pdf.compress "$pdf" --out "$work/out"
    ls -l "$work/out"
  done
fi

# ------------------------------------------------------------------------------------ dmg
if has dmg; then
  for a in $ARCHS; do
    step "dmg ($a, unsigned)"
    dmg_build "$(app_path "$a")" "$PKG/Castor-$VERSION-macos-$a-unsigned.dmg"
    [ "$DRY_RUN" = 1 ] || ls -lh "$PKG/Castor-$VERSION-macos-$a-unsigned.dmg"
  done
fi

# ------------------------------------------------------------------------------------ vpk (test)
# Not in the default steps: unsigned Velopack packages, only to test packing/updating locally.
# Releases are made by sign-notarize.sh.
if has vpk; then
  for a in $ARCHS; do
    APP="$(app_path "$a")"
    step "vpk pack ($a, channel osx-$a, UNSIGNED test packages)"
    run vpk pack --packId "$APP_ID" --packVersion "$VERSION" --packDir "$APP" --mainExe castor \
      --packTitle "$APP_TITLE" --packAuthors "$APP_AUTHORS" --bundleId "$MAC_BUNDLE_ID" \
      --icon "$APP/Contents/Resources/castor.icns" --runtime "osx-$a" --channel "osx-$a" \
      --outputDir "$PKG/releases-unsigned/osx-$a" --noInst --skip-updates
    [ "$DRY_RUN" = 1 ] || ls -lh "$PKG/releases-unsigned/osx-$a"
  done
fi
