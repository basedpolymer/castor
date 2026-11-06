#!/usr/bin/env bash
# Runs every PDF tool once through castor-cli on the fixtures in tests/fixtures/generated/pdf.
# Usage (repo root, Git Bash or macOS):  bash crates/tools-pdf/tests/cli-smoke.sh [castor-cli path]
# (or the faster-to-build example: target-a/debug/examples/run_tool.exe)
# Outputs land in tests/fixtures/generated/out-a/<tool>/ (gitignored).
set -u
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CLI="${1:-$ROOT/target-a/debug/castor-cli}"
FX="$ROOT/tests/fixtures/generated/pdf"
OUT="$ROOT/tests/fixtures/generated/out-a"
export CASTOR_HOME="${CASTOR_HOME:-$ROOT/tests/fixtures/generated/castor-home-a}"
mkdir -p "$CASTOR_HOME/data/downloads/tessdata" "$OUT"
for l in eng fra; do
  [ -f "$ROOT/tests/fixtures/generated/tessdata/$l.traineddata" ] && cp -n "$ROOT/tests/fixtures/generated/tessdata/$l.traineddata" "$CASTOR_HOME/data/downloads/tessdata/" 2>/dev/null
done
PAPER="$FX/arxiv-attention.pdf"; FORM="$FX/irs-fw9.pdf"; SCAN="$FX/yearbook-1890-scan.pdf"; JPG="$FX/scan-page.jpg"
pass=0; fail=0
run() { # run <name> <tool> <args...>
  local name="$1"; shift; local tool="$1"; shift
  local start=$(date +%s)
  local verb=run
  case "$(basename "$CLI")" in run_tool*) verb="";; esac  # the tools-pdf example takes no verb
  if "$CLI" $verb "$tool" "$@" --out "$OUT/$name" > "$OUT/$name.log" 2>&1; then
    pass=$((pass+1)); echo "ok   $name ($(( $(date +%s) - start )) s)"
  else
    fail=$((fail+1)); echo "FAIL $name"; tail -5 "$OUT/$name.log"
  fi
}
rm -rf "$OUT"/*
run compress-paper pdf.compress "$PAPER"
run compress-scan-max pdf.compress "$SCAN" --set preset=max
run compress-scan-balanced pdf.compress "$SCAN"
run merge pdf.merge "$PAPER" "$FORM" "$JPG"
run split-every pdf.split "$PAPER"
run split-size pdf.split "$SCAN" --set mode=size --set max_mb=1
run extract pdf.extract-pages "$PAPER" --set pages=2-3
run organize pdf.organize "$PAPER" --set "order=3,1-2" --set delete=5 --set rotate_pages=1
run rotate pdf.rotate "$PAPER" --set angle=180 --set pages=even
run crop pdf.crop "$PAPER"
run numbers pdf.page-numbers "$PAPER" --set format=page_n_of_total --set position=bottom-right
run stamp pdf.stamp "$PAPER" --set text=DRAFT --set opacity=0.2
run sign pdf.sign "$FORM" --set name=basedpolymer --set date=true
run encrypt pdf.encrypt "$PAPER" --set user_password=castor
run unlock pdf.unlock "$OUT/encrypt/arxiv-attention-protected.pdf" --set password=castor
run redact pdf.redact "$PAPER" --set "terms=Transformer" --set emails=true
run to-images pdf.to-images "$PAPER" --set pages=1-2 --set dpi=100
run to-markdown pdf.to-markdown "$PAPER"
run to-epub pdf.to-epub "$PAPER"
run from-ebook pdf.from-ebook "$OUT/to-epub/arxiv-attention.epub"
run compare pdf.compare "$PAPER" "$OUT/stamp/arxiv-attention-stamped.pdf"
run metadata pdf.metadata "$PAPER" --set "title=Attention — test" --set author=basedpolymer
run fields pdf.fill-form "$FORM" --set mode=list
run flatten pdf.flatten "$FORM"
run repair pdf.repair "$PAPER"
run grayscale pdf.grayscale "$PAPER"
run extract-images pdf.extract-images "$PAPER"
run attach pdf.attach-to-each "$PAPER" --set "file=$FORM" --set position=after
run ocr-scan pdf.ocr "$SCAN" --set pages=9-11 --set skip_text=false --set sidecar=true
run images-to-pdf image.images-to-pdf "$JPG" "$OUT/to-images"
run scan-to-pdf image.to-pdf "$JPG"
run extract-text image.extract-text "$JPG"
MD="$ROOT/tests/fixtures/generated/md/sample.md"; OFFICE="$ROOT/tests/fixtures/generated/office"
if [ -f "$MD" ]; then
  run from-markdown pdf.from-markdown "$MD" --set toc=true --set title_page=true
  run from-markdown-pages pdf.to-images "$OUT/from-markdown/sample.pdf" --set dpi=110
fi
for f in report.docx budget.xlsx slides.pptx; do
  [ -f "$OFFICE/$f" ] && run "from-office-${f%%.*}" pdf.from-office "$OFFICE/$f"
done
echo "passed $pass, failed $fail"
