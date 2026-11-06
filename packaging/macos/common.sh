# Shared helpers for packaging/macos/*.sh (sourced, not executed).
# shellcheck shell=bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/macos"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
case "$TARGET" in /*) ;; *) TARGET="$ROOT/$TARGET" ;; esac
PKG="$TARGET/package/macos"          # everything this script produces
DEPS="$TARGET/deps"                  # cached downloads + helper builds
DRY_RUN="${DRY_RUN:-0}"

export PATH="$HOME/.cargo/bin:$HOME/.dotnet:$HOME/.dotnet/tools:/usr/local/bin:/opt/homebrew/bin:$PATH"
export DOTNET_ROOT="${DOTNET_ROOT:-$HOME/.dotnet}"
export DOTNET_CLI_TELEMETRY_OPTOUT=1

# Pins (packaging/pins.env): KEY=VALUE lines.
while IFS='=' read -r k v; do
  [[ "$k" =~ ^[A-Z0-9_]+$ ]] && printf -v "$k" '%s' "$v"
done < "$ROOT/packaging/pins.env"

step() { printf '\n\033[1;36m=== %s ===\033[0m\n' "$*"; }
note() { printf '%s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
# Run a command, or only print it in dry-run mode.
run() {
  printf '+ %s\n' "$*"
  [ "$DRY_RUN" = 1 ] || "$@"
}

# Architectures: Castor ships one app per arch ("x64" Intel, "arm64" Apple Silicon).
rust_target() { case "$1" in x64) echo x86_64-apple-darwin ;; arm64) echo aarch64-apple-darwin ;; *) die "bad arch $1" ;; esac; }
app_path() { echo "$PKG/$1/Castor.app"; }
# Oldest macOS each app runs on (Info.plist LSMinimumSystemVersion). Intel: 11.0 (the binaries
# are built for it). Apple Silicon: 13.4, because the static ONNX Runtime linked on arm64 (ort's
# prebuilt 1.31) is compiled for macOS 13.4; every Apple Silicon Mac can run macOS 14 or later.
min_macos() { case "$1" in x64) echo 11.0 ;; arm64) echo 13.4 ;; *) die "bad arch $1" ;; esac; }

# Workspace version from the root Cargo.toml ([workspace.package] version).
workspace_version() {
  awk '/^\[workspace\.package\]/{p=1;next} /^\[/{p=0} p && /^version *=/{gsub(/[" ]/,"",$0); split($0,a,"="); print a[2]; exit}' "$ROOT/Cargo.toml"
}

# fetch URL SHA256 DEST — download once, verify, keep in the cache.
fetch() {
  local url="$1" sha="$2" dest="$3"
  if [ -f "$dest" ] && [ "$(shasum -a 256 "$dest" | cut -d' ' -f1)" = "$sha" ]; then
    note "cached: $(basename "$dest")"; return
  fi
  run mkdir -p "$(dirname "$dest")"
  run curl -fL --retry 3 -o "$dest.part" "$url"
  if [ "$DRY_RUN" = 1 ]; then return; fi
  local got; got="$(shasum -a 256 "$dest.part" | cut -d' ' -f1)"
  [ "$got" = "$sha" ] || { rm -f "$dest.part"; die "SHA-256 mismatch for $url: expected $sha, got $got"; }
  mv "$dest.part" "$dest"
}

# Code-signing identity "Developer ID Application: …" read from the keychain at run time
# (never written to any file).
developer_id() {
  security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) [0-9A-F]* "\(Developer ID Application: .*\)"$/\1/p' | head -1
}

dmg_build() {  # dmg_build APP OUT.dmg — house-style DMG (dmgbuild, no Finder scripting: works over SSH)
  local app="$1" out="$2" venv="$DEPS/dmgbuild-venv"
  if [ ! -x "$venv/bin/dmgbuild" ]; then
    run python3 -m venv "$venv"
    run "$venv/bin/pip" install --quiet "dmgbuild==$DMGBUILD_VERSION"
  fi
  local bg="$PKG/dmg-background.tiff"
  run tiffutil -cathidpicheck "$HERE/dmg/background.png" "$HERE/dmg/background@2x.png" -out "$bg"
  run rm -f "$out"
  run "$venv/bin/dmgbuild" -s "$HERE/dmg/settings.py" -D app="$app" -D background="$bg" \
    -D icon="$app/Contents/Resources/castor.icns" "Castor" "$out"
}
