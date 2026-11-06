#!/usr/bin/env bash
# Sign, notarise and staple Castor for macOS, then make the Velopack packages and the DMGs —
# one set per architecture (Intel x64, Apple Silicon arm64).
#
#   packaging/macos/sign-notarize.sh [--dry-run] [--version X.Y.Z] [--arch x64|arm64|all]
#                                    [--releases DIR] [--profile fouine] [--no-vpk]
#
# Input: the bundles made by packaging/macos/build.sh ($TARGET/package/macos/<arch>/Castor.app).
# Output ($TARGET/package/macos/release/ unless --releases), for each arch:
#   osx-<arch>/Castor-osx-<arch>-Portable.zip, osx-<arch>/Castor-<ver>-osx-<arch>-full.nupkg
#       (+ delta), osx-<arch>/releases.osx-<arch>.json — Velopack channel `osx-<arch>`; the app
#       inside is signed (Developer ID, hardened runtime), notarised and stapled by `vpk pack`
#   Castor-<ver>-macos-<arch>.dmg — drag-to-Applications DMG made from that same app, signed,
#       notarised, stapled
#
# Needs the login keychain UNLOCKED, with the "Developer ID Application" identity and the
# notarytool keychain profile (default `fouine`, created once with
# `xcrun notarytool store-credentials fouine …`). Over SSH the keychain is locked:
#   a) run this script in Terminal.app on the Mac (GUI session), or
#   b) over SSH: `security unlock-keychain ~/Library/Keychains/login.keychain-db` first (asks
#      for the password interactively; never put it in a file or on the command line). If
#      codesign then fails with errSecInternalComponent, use a).
# The identity is read from the keychain at run time and never written anywhere.
# --no-vpk: sign + notarise + staple with codesign/notarytool directly (no Velopack packages;
# such installs cannot self-update).

source "$(dirname "$0")/common.sh"

VERSION=""; OUT="$PKG/release"; PROFILE="fouine"; USE_VPK=1; ARCH_ARG="all"
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --version) VERSION="$2"; shift ;;
    --arch) ARCH_ARG="$2"; shift ;;
    --releases) OUT="$2"; shift ;;
    --profile) PROFILE="$2"; shift ;;
    --no-vpk) USE_VPK=0 ;;
    -h|--help) sed -n '2,27p' "$0"; exit 0 ;;
    *) die "unknown argument $1" ;;
  esac
  shift
done
case "$ARCH_ARG" in all) ARCHS="x64 arm64" ;; x64|arm64) ARCHS="$ARCH_ARG" ;; *) die "--arch x64|arm64|all" ;; esac
VERSION="${VERSION:-$(workspace_version)}"
ENT="$HERE/castor.entitlements"

step "preflight"
for a in $ARCHS; do [ -d "$(app_path "$a")" ] || die "$(app_path "$a") not found: run packaging/macos/build.sh first"; done
ID="$(developer_id)"
[ -n "$ID" ] || die "no 'Developer ID Application' identity in the keychain"
note "identity: Developer ID Application (from the keychain)   notary profile: $PROFILE   version: $VERSION   arch: $ARCHS"
if [ "$DRY_RUN" != 1 ]; then
  # A throw-away signature tells whether the private key is usable (locked keychain → fails).
  t="$(mktemp -d)"; cp /usr/bin/true "$t/probe"
  codesign --force --sign "$ID" --timestamp "$t/probe" 2>"$t/err" \
    || { cat "$t/err"; rm -rf "$t"; die "codesign cannot use the identity: unlock the login keychain (see the header of this script)"; }
  rm -rf "$t"
  xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null \
    || die "notarytool profile '$PROFILE' unusable (keychain locked or profile missing)"
fi
run mkdir -p "$OUT"

for a in $ARCHS; do
  WORK="$PKG/signing-$a"
  step "[$a] copy the bundle"
  run rm -rf "$WORK"; run mkdir -p "$WORK"
  run ditto "$(app_path "$a")" "$WORK/Castor.app"
  A="$WORK/Castor.app"; C="$A/Contents"

  step "[$a] sign nested code (hardened runtime, timestamp)"
  # Inside out. `vpk pack` signs again (deep) after adding its updater; pre-signing keeps the
  # --no-vpk path working and makes failures show up per file.
  for f in "$C/Frameworks/"*.dylib "$C/Resources/bin/"* "$C/MacOS/castor-cli"; do
    [ -e "$f" ] || continue
    run codesign --force --sign "$ID" --timestamp --options runtime --entitlements "$ENT" "$f"
  done

  if [ "$USE_VPK" = 1 ]; then
    step "[$a] vpk pack (channel osx-$a): updater, deep signature, notarisation, staple"
    run vpk pack --packId "$APP_ID" --packVersion "$VERSION" --packDir "$A" --mainExe castor \
      --packTitle "$APP_TITLE" --packAuthors "$APP_AUTHORS" --bundleId "$MAC_BUNDLE_ID" \
      --icon "$C/Resources/castor.icns" --runtime "osx-$a" --channel "osx-$a" \
      --outputDir "$OUT/osx-$a" --noInst \
      --signAppIdentity "$ID" --signEntitlements "$ENT" --notaryProfile "$PROFILE" --skip-updates
    # The DMG carries exactly the app of the portable package (with Velopack's updater inside,
    # so DMG installs can update themselves).
    run rm -rf "$WORK/dmg-src"; run mkdir -p "$WORK/dmg-src"
    run ditto -x -k "$OUT/osx-$a/$APP_ID-osx-$a-Portable.zip" "$WORK/dmg-src"
    A="$WORK/dmg-src/Castor.app"
  else
    step "[$a] sign + notarise + staple the app (no Velopack)"
    run codesign --force --sign "$ID" --timestamp --options runtime --entitlements "$ENT" "$A"
    run ditto -c -k --keepParent "$A" "$WORK/Castor-notarize.zip"
    run xcrun notarytool submit "$WORK/Castor-notarize.zip" --keychain-profile "$PROFILE" --wait
    run xcrun stapler staple "$A"
  fi

  step "[$a] verify the app"
  run codesign --verify --deep --strict --verbose=2 "$A"
  run xcrun stapler validate "$A"
  # Gatekeeper assessment only works for the host architecture's binaries.
  run spctl --assess --type execute --verbose=2 "$A" || note "(spctl: see above; expected to work on a $a Mac)"

  step "[$a] DMG: build, sign, notarise, staple"
  DMG="$OUT/Castor-$VERSION-macos-$a.dmg"
  dmg_build "$A" "$DMG"
  run codesign --force --sign "$ID" --timestamp "$DMG"
  run xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
  run xcrun stapler staple "$DMG"
  run spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
done

[ "$DRY_RUN" = 1 ] || { ls -lhR "$OUT"; (cd "$OUT" && shasum -a 256 ./*.dmg); }
