#!/usr/bin/env bash
# App stills for the promo video, in English and French, from the real app (offscreen `--shot`)
# with real runs in a throw-away profile. Git Bash on Windows, from the repository root:
#
#   . crates/engine-pdf/env.sh && CARGO_TARGET_DIR=target-d cargo build -p castor-app --features shots
#   WORK=/c/tmp/castor-video bash video/scripts/make-shots.sh en   # then: … fr
#
# Inputs, prepared once in $WORK by prepare-inputs.sh: models in $WORK/downloads, two scanned
# PDFs ($WORK/scan.pdf, scan2.pdf) and the page images of the first ($WORK/pages/*.jpg), the
# two studio clips and their edit scripts ($WORK/clips, $WORK/tr).
# Output: video/public/shots/<lang>/*.png
set -euo pipefail
L=${1:-en}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CASTOR="$ROOT/target-d/debug/castor.exe"
WORK=${WORK:?set WORK}
FIX="$ROOT/tests/fixtures/generated"
OUT="$ROOT/video/public/shots/$L"
SIZE=${SIZE:-1400x875}
win() { cygpath -w "$1"; }

P="$WORK/profile-$L"          # CASTOR_HOME: settings, runs, models
F="$WORK/files-$L"            # the user's files in the demo
rm -rf "$P" "$F"; mkdir -p "$P/data" "$P/config" "$F/photos" "$F/camera" "$F/pages" "$OUT"
# Remove background: WebP keeps the cut-out light (a PNG with alpha is ~10 MB for this photo).
printf '[tool_values."image.remove-background"]\nformat = "webp"\n' > "$P/config/settings.toml"
# models (whisper, BiRefNet, tessdata…) are moved in and out instead of copied: hundreds of MB
mv "$WORK/downloads" "$P/data/downloads"
trap 'mv "$P/data/downloads" "$WORK/downloads"' EXIT
export CASTOR_HOME=$(win "$P")
export PATH="$PATH:$ROOT/target-g/package/win/stage/bin"   # whisper-cli, tesseract if staged

if [ "$L" = fr ]; then
  PDF="$F/scan-chapitre-1.pdf"; PDF2="$F/scan-chapitre-2.pdf"; PORTRAIT="$F/portrait.jpg"
  CLIP="$F/Les Anciens Canadiens.mp4"; EDIT="$WORK/tr/edit-fr.json"; QUERY="rendre ce pdf plus léger"
  CHAIN="Photos prêtes pour le web"; SCANS="Scans en PDF cherchable"; OCRLANG=fra
  RENAME="vacances-{n:02}"; LANGARG=(--lang fr)
  PH=(thé.jpg cadeau.jpg groupe.jpg chat.jpg)
else
  PDF="$F/scan-chapter-1.pdf"; PDF2="$F/scan-chapter-2.pdf"; PORTRAIT="$F/portrait.jpg"
  CLIP="$F/The Canvasser's Tale.mp4"; EDIT="$WORK/tr/edit-en.json"; QUERY="make this pdf smaller"
  CHAIN="Web-ready photos"; SCANS="Scans to searchable PDF"; OCRLANG=eng
  RENAME="holiday-{n:02}"; LANGARG=()
  PH=(tea.jpg gift.jpg group.jpg cat.jpg)
fi
cp "$WORK/scan.pdf" "$PDF"; cp "$WORK/scan2.pdf" "$PDF2"; cp "$WORK"/pages/*.jpg "$F/pages/"
cp "$FIX/image/portrait-hair.jpg" "$PORTRAIT"
cp "$WORK/clips/$(basename "$CLIP")" "$CLIP"
i=0; for src in product-tea product-gift group cat-fur; do
  cp "$FIX/image/$src.jpg" "$F/photos/${PH[$i]}"; i=$((i+1))
  cp "$FIX/image/$src.jpg" "$F/camera/IMG_2026110$i.jpg"
done

cli() { "$CASTOR" cli "${LANGARG[@]}" "$@" 2>&1 | grep -E "Complete|✗" || true; }
shot() { # shot <name> <delay> args…   (SIZE=WxH shot … for one taller shot)
  local name=$1 delay=$2; shift 2
  "$CASTOR" "$@" --theme light "${LANGARG[@]}" --size "$SIZE" --shot-delay "$delay" --shot "$(win "$OUT/$name.png")" 2>&1 | grep -aE "png \(" || echo "!! $name failed"
}

# 1. Real runs from the command line: two chains and an undoable rename.
printf 'name = "%s"\n[[steps]]\ntool = "image.resize"\nvalues = { mode = "fit", size = [2048, 2048] }\n[[steps]]\ntool = "image.compress"\n' "$CHAIN" > "$WORK/chain-$L.toml"
printf 'name = "%s"\n[[steps]]\ntool = "image.images-to-pdf"\n[[steps]]\ntool = "pdf.ocr"\nvalues = { languages = "%s" }\n[[steps]]\ntool = "pdf.compress"\n' "$SCANS" "$OCRLANG" > "$WORK/scans-$L.toml"
mkdir -p "$F/web" "$F/searchable"
cli chain "$(win "$WORK/scans-$L.toml")" $(for f in "$F"/pages/*.jpg; do win "$f"; done) --out "$(win "$F/searchable")"
cli chain "$(win "$WORK/chain-$L.toml")" $(for f in "$F"/photos/*; do win "$f"; done) --out "$(win "$F/web")"
cli run files.rename $(for f in "$F"/camera/*; do win "$f"; done) --set "pattern=$RENAME"

# 2. The app.
shot home 2.5            --page home
shot home-drop 3         --page home --files "$(win "$PDF")" "$(win "$PDF2")"
shot pdf-queued 3        --page tool:pdf.compress --files "$(win "$PDF")" "$(win "$PDF2")"
shot pdf-done 25         --page tool:pdf.compress --files "$(win "$PDF")" "$(win "$PDF2")" --run
SIZE=1400x1080 shot rmbg-done 60 --page tool:image.remove-background --files "$(win "$PORTRAIT")" --run
shot studio-captions 8   --page studio:media --files "$(win "$CLIP")" --mode captions --script "$(win "$EDIT")"
shot studio-cut 8        --page studio:media --files "$(win "$CLIP")" --mode cut --script "$(win "$EDIT")"
shot palette 3           --page palette --query "$QUERY"
shot chains 2.5          --page chains
shot chain 3             --page "chain:$CHAIN"
shot chain-scans 3       --page "chain:$SCANS"
shot runs 3              --page runs
# The quick panel shows a "Clipboard:" line when the clipboard holds an image.
powershell.exe -NoProfile -Command "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; [System.Windows.Forms.Clipboard]::SetImage([System.Drawing.Image]::FromFile('$(win "$FIX/image/screenshot.png")'))"
"$CASTOR" --page panel --theme light "${LANGARG[@]}" --shot "$(win "$OUT/panel.png")" 2>&1 | grep -aE "png \(" || echo "!! panel failed"
"$CASTOR" cli "${LANGARG[@]}" history --limit 10 2>/dev/null
