#!/usr/bin/env bash
# Inputs of make-shots.sh, made once in $WORK from the repository's test fixtures, with Castor's
# own CLI (git bash, repository root, after the `shots` build of castor-app):
#   - scan.pdf / scan2.pdf: pages 1–6 / 7–10 of tests/fixtures/generated/pdf/arxiv-attention.pdf
#     rendered at 300 dpi, given a scanner's look (slight tilt, paper tint, grain, JPEG q≈90)
#     and wrapped into an A4 PDF with image.images-to-pdf: what an office scanner produces;
#     pages/*.jpg are the page images of scan.pdf (for the "Scans to searchable PDF" chain);
#   - clips/*.mp4: 70 s of the LibriVox public-domain readings in tests/fixtures/generated/media/
#     speech over a pastel gradient (the Trim & Caption Studio demo);
#   - tr/*.json: their whisper transcripts (audio.transcribe), tr/edit-*.json: the studio edits;
#   - downloads/: the models (whisper, BiRefNet, tessdata) — copied from the fixture profiles,
#     BiRefNet is fetched by Castor itself on the first background removal.
#   WORK=/c/tmp/castor-video bash video/scripts/prepare-inputs.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CASTOR="$ROOT/target-d/debug/castor.exe"
FIX="$ROOT/tests/fixtures/generated"
WORK=${WORK:?set WORK}
win() { cygpath -w "$1"; }
mkdir -p "$WORK"/{scan/png,scan/jpg,scan2/png,scan2/jpg,pages,clips,tr/en,tr/fr,downloads,home}
export CASTOR_HOME=$(win "$WORK/home")
export PATH="$PATH:$ROOT/target-g/package/win/stage/bin"   # whisper-cli

scan() { # scan <dir> <pages>
  "$CASTOR" cli run pdf.to-images "$(win "$FIX/pdf/arxiv-attention.pdf")" --set format=png --set dpi=300 --set "pages=$2" --out "$(win "$WORK/$1/png")"
  local i=0
  for f in "$WORK/$1"/png/*.png; do
    i=$((i+1))
    ffmpeg -v error -y -i "$f" -vf "rotate=0.004*($i-3):fillcolor=white,colorchannelmixer=rr=0.97:gg=0.955:bb=0.92,noise=alls=7:allf=u,gblur=sigma=0.6" -q:v 3 "$WORK/$1/jpg/page-$i.jpg"
  done
  "$CASTOR" cli run image.images-to-pdf $(for f in "$WORK/$1"/jpg/*.jpg; do win "$f"; done) --set page_size=a4 --out "$(win "$WORK/$1")"
  cp "$WORK/$1/page-1-images.pdf" "$WORK/$1.pdf"
}
scan scan 1-6
scan scan2 7-10
cp "$WORK"/scan/jpg/*.jpg "$WORK/pages/"

GRAD="gradients=s=1280x720:r=30:c0=#fbe2ff:c1=#ade1ff:c2=#aafaba:c3=#ffeca4:nb_colors=4:speed=0.004:type=linear"
ffmpeg -v error -y -f lavfi -i "$GRAD" -ss 24 -t 70 -i "$FIX/media/speech/en-canvassers-tale.mp3" -map 0:v -map 1:a -t 70 \
  -c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p -g 60 -c:a aac -b:a 128k -shortest "$(win "$WORK/clips")\\The Canvasser's Tale.mp4"
ffmpeg -v error -y -f lavfi -i "$GRAD" -ss 66.8 -t 70 -i "$FIX/media/speech/fr-anciens-canadiens-01.mp3" -map 0:v -map 1:a -t 70 \
  -c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p -g 60 -c:a aac -b:a 128k -shortest "$(win "$WORK/clips")\\Les Anciens Canadiens.mp4"

cp -r "$FIX"/castor-home/data/downloads/* "$WORK/downloads/"
cp -r "$FIX/castor-home-a/data/downloads/tessdata" "$WORK/downloads/"
mkdir -p "$WORK/home/data" && rm -rf "$WORK/home/data/downloads" && cp -r "$WORK/downloads" "$WORK/home/data/downloads"
"$CASTOR" cli run audio.transcribe "$(win "$WORK/clips")\\The Canvasser's Tale.mp4" --set format=json --set vad=false --set detect=false --set language=en --out "$(win "$WORK/tr/en")"
"$CASTOR" cli run audio.transcribe "$(win "$WORK/clips")\\Les Anciens Canadiens.mp4" --set format=json --set vad=false --set detect=false --set language=fr --out "$(win "$WORK/tr/fr")"
(cd "$WORK/tr" && py -I "$ROOT/video/scripts/studio_edit.py" "en/The Canvasser's Tale.json" edit-en.json en \
  && py -I "$ROOT/video/scripts/studio_edit.py" "fr/Les Anciens Canadiens.json" edit-fr.json fr)
