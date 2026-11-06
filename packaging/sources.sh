#!/usr/bin/env bash
# Corresponding source of the third-party programs bundled with Castor (GPL source offer).
# Fetches the exact commits pinned in packaging/pins.env (git checks every object hash) and
# writes reproducible archives + SHA256SUMS, to be attached to each GitHub release:
#   ffmpeg-<commit>-windows-source.tar.gz        FFmpeg as built by BtbN for Windows
#   ffmpeg-n9.0.2-macos-source.tar.gz            FFmpeg as built by martin-riedl.de for macOS
#   ffmpeg-builds-btbn-<commit>.tar.gz           BtbN build scripts (pin every library version)
#   ffmpeg-build-script-martin-riedl-<commit>.tar.gz
#   whisper.cpp-v1.9.5-source.tar.gz
# Castor's own source is the repository tag (GitHub adds its archives to the release).
#
#   bash packaging/sources.sh [OUT_DIR]     (default: ${CARGO_TARGET_DIR:-target-g}/release-sources)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
while IFS='=' read -r k v; do [[ "$k" =~ ^[A-Z0-9_]+$ ]] && printf -v "$k" '%s' "$v"; done < packaging/pins.env
OUT="${1:-${CARGO_TARGET_DIR:-target-g}/release-sources}"
GIT_CACHE="${CARGO_TARGET_DIR:-target-g}/deps/src-git"
mkdir -p "$OUT" "$GIT_CACHE"

# archive NAME REPO COMMIT OUTFILE — shallow fetch of one commit, then `git archive`.
archive() {
  local name="$1" repo="$2" commit="$3" out="$4" dir="$GIT_CACHE/$1"
  if [ -s "$OUT/$out" ]; then echo "exists: $out"; return; fi
  [ -d "$dir/.git" ] || git init -q "$dir"
  if ! git -C "$dir" cat-file -e "$commit^{commit}" 2>/dev/null; then
    git -C "$dir" fetch -q --depth 1 "$repo" "$commit" \
      || { echo "shallow fetch refused, full clone of $repo"; git -C "$dir" fetch -q "$repo"; }
  fi
  git -C "$dir" cat-file -e "$commit^{commit}" || { echo "commit $commit not found in $repo" >&2; exit 1; }
  # -n: no timestamp in the gzip header, so the archive is reproducible.
  git -C "$dir" archive --format=tar --prefix="${out%.tar.gz}/" "$commit" | gzip -n -6 > "$OUT/$out.part"
  mv "$OUT/$out.part" "$OUT/$out"
  echo "wrote: $out ($(du -h "$OUT/$out" | cut -f1))"
}

archive ffmpeg-win  "$FFMPEG_REPO"  "$FFMPEG_WIN_COMMIT" "ffmpeg-${FFMPEG_WIN_COMMIT:0:10}-windows-source.tar.gz"
archive ffmpeg-mac  "$FFMPEG_REPO"  "$FFMPEG_MAC_COMMIT" "ffmpeg-n$FFMPEG_MAC_VERSION-macos-source.tar.gz"
archive btbn        "$BTBN_REPO"    "$BTBN_COMMIT"       "ffmpeg-builds-btbn-${BTBN_COMMIT:0:10}.tar.gz"
archive mriedl      "$MRIEDL_REPO"  "$MRIEDL_COMMIT"     "ffmpeg-build-script-martin-riedl-${MRIEDL_COMMIT:0:10}.tar.gz"
archive whisper     "$WHISPER_REPO" "$WHISPER_COMMIT"    "whisper.cpp-$WHISPER_TAG-source.tar.gz"

(cd "$OUT" && sha256sum *.tar.gz > SHA256SUMS-sources.txt && cat SHA256SUMS-sources.txt)
