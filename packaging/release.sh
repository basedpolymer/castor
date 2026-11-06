#!/usr/bin/env bash
# Prepare a Castor release: bump the workspace version, add a CHANGELOG section, write the
# release notes used by the update dialog, then PRINT (never run) the commands that build,
# tag and publish. Publishing is done by hand by the coordinator.
#
#   packaging/release.sh 0.2.0 [--dry-run]
#
# Writes: Cargo.toml ([workspace.package] version), Cargo.lock (workspace entries only),
# CHANGELOG.md (new section on top, drafted from `git log` since the last tag — edit it),
# packaging/notes/<version>.md (the same section, passed to `vpk pack --releaseNotes`).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
DRY_RUN=0
NEW=""
for a in "$@"; do
  case "$a" in
    --dry-run) DRY_RUN=1 ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) NEW="$a" ;;
  esac
done
[ -n "$NEW" ] || { echo "usage: packaging/release.sh <version> [--dry-run]" >&2; exit 2; }
# SemVer 2 (Velopack requires it): 1.2.3 or 1.2.3-beta.1
[[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "not a SemVer version: $NEW" >&2; exit 2; }

OLD="$(awk '/^\[workspace\.package\]/{p=1;next} /^\[/{p=0} p && /^version *=/{gsub(/[" ]/,"",$0); split($0,a,"="); print a[2]; exit}' Cargo.toml)"
TAG="v$NEW"
DATE="$(date +%Y-%m-%d)"
LAST_TAG="$(git describe --tags --abbrev=0 2>/dev/null || true)"
NOTES="packaging/notes/$NEW.md"
echo "Castor $OLD -> $NEW (tag $TAG, previous tag: ${LAST_TAG:-none})"
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then echo "tag $TAG already exists" >&2; exit 1; fi

# Draft notes from the commit subjects since the last tag (to be edited by a human).
if [ -n "$LAST_TAG" ]; then RANGE="$LAST_TAG..HEAD"; else RANGE="HEAD"; fi
DRAFT="$(git log --no-merges --format='- %s' "$RANGE" 2>/dev/null | grep -v -i -E '^- (wip|fixup!|squash!)' || true)"
[ -n "$DRAFT" ] || DRAFT="- (describe the changes)"
SECTION="## $NEW — $DATE

$DRAFT"

if [ "$DRY_RUN" = 1 ]; then
  echo "--- dry run: would set version $NEW in Cargo.toml, refresh Cargo.lock, prepend to CHANGELOG.md:"
  printf '%s\n' "$SECTION"
else
  # 1. Version (only the [workspace.package] table).
  awk -v v="$NEW" '/^\[workspace\.package\]/{p=1} /^\[/{if(!/^\[workspace\.package\]/)p=0}
       p && /^version *=/ && !done {print "version = \"" v "\""; done=1; next} {print}' Cargo.toml > Cargo.toml.new
  mv Cargo.toml.new Cargo.toml
  # Cargo.lock: re-resolve offline so the workspace crates carry the new version (deps untouched).
  cargo metadata --format-version 1 --offline >/dev/null
  # 2. Changelog + notes.
  mkdir -p packaging/notes
  printf '%s\n' "$SECTION" > "$NOTES"
  if [ -f CHANGELOG.md ]; then
    { head -n 2 CHANGELOG.md; printf '%s\n\n' "$SECTION"; tail -n +3 CHANGELOG.md; } > CHANGELOG.md.new
  else
    printf '# Changelog\n\n%s\n' "$SECTION" > CHANGELOG.md.new
  fi
  mv CHANGELOG.md.new CHANGELOG.md
  echo "updated: Cargo.toml Cargo.lock CHANGELOG.md $NOTES  (edit the notes now)"
fi

REPO=https://github.com/basedpolymer/castor
cat <<EOF

================ next steps (run them yourself, in this order) ================
# 0. Edit CHANGELOG.md and $NOTES (same text), then regenerate the notices:
bash packaging/third-party/generate.sh

# 1. Commit and tag:
git add Cargo.toml Cargo.lock CHANGELOG.md $NOTES THIRD_PARTY.md
git commit -m "Castor $NEW"
git tag -a $TAG -m "Castor $NEW"

# 2. Windows (PowerShell 7, this PC). Fetch the previous release first so a delta is made:
vpk download github --repoUrl $REPO --channel win -o target-g/releases/win
pwsh packaging/windows/build.ps1 -Version $NEW -ReleaseNotes $NOTES

# 3. macOS (on the Mac, repo checked out at $TAG), one package set per architecture:
#    over SSH (build, unsigned bundles, checks):   packaging/macos/build.sh --version $NEW
#    in Terminal.app, keychain unlocked (previous releases first, for the deltas):
vpk download github --repoUrl $REPO --channel osx-x64   -o target/package/macos/release/osx-x64
vpk download github --repoUrl $REPO --channel osx-arm64 -o target/package/macos/release/osx-arm64
packaging/macos/sign-notarize.sh --version $NEW
#    then copy target/package/macos/release/ back to this PC as MAC_REL (scp -r).

# 4. Corresponding source of the bundled FFmpeg / whisper.cpp (GPL source offer):
bash packaging/sources.sh target-g/release-sources

# 5. Push, then put everything into ONE draft GitHub release (check it, then publish):
git push origin main $TAG
vpk upload github --repoUrl $REPO --token "\$(gh auth token)" --tag $TAG --releaseName "Castor $NEW" --channel win -o target-g/releases/win
vpk upload github --repoUrl $REPO --token "\$(gh auth token)" --tag $TAG --releaseName "Castor $NEW" --channel osx-x64 -o \$MAC_REL/osx-x64 --merge
vpk upload github --repoUrl $REPO --token "\$(gh auth token)" --tag $TAG --releaseName "Castor $NEW" --channel osx-arm64 -o \$MAC_REL/osx-arm64 --merge
gh release upload $TAG \$MAC_REL/Castor-$NEW-macos-x64.dmg \$MAC_REL/Castor-$NEW-macos-arm64.dmg
gh release upload $TAG target-g/release-sources/*.tar.gz target-g/release-sources/SHA256SUMS-sources.txt
#    Stable-named copies: the site's buttons link to
#    $REPO/releases/latest/download/<name>, which always serves the newest published release.
rm -rf target-g/release-stable && mkdir -p target-g/release-stable
cp target-g/releases/win/Castor-win-Setup.exe        target-g/release-stable/Castor-windows-setup.exe
cp \$MAC_REL/Castor-$NEW-macos-x64.dmg   target-g/release-stable/Castor-macos-x64.dmg
cp \$MAC_REL/Castor-$NEW-macos-arm64.dmg target-g/release-stable/Castor-macos-arm64.dmg
gh release upload $TAG target-g/release-stable/Castor-windows-setup.exe target-g/release-stable/Castor-macos-x64.dmg target-g/release-stable/Castor-macos-arm64.dmg
gh release edit $TAG --notes-file $NOTES          # then publish: gh release edit $TAG --draft=false

# 6. After publishing: the site's download links must answer 302 -> the new assets.
for n in Castor-windows-setup.exe Castor-macos-x64.dmg Castor-macos-arm64.dmg; do
  curl -sI "$REPO/releases/latest/download/\$n" | grep -i -m1 '^location:' | grep -q "/download/$TAG/\$n" && echo "ok \$n" || echo "NOT OK \$n"
done
EOF
