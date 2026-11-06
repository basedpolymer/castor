#!/bin/sh
# Capture the Castor window of a process to a PNG on macOS.
#   ./shot.sh <pid> <out.png> [wait-seconds]
# Needs the Screen Recording permission for the terminal that runs it.
set -eu
PID="$1"; OUT="$2"; WAIT="${3:-0}"
sleep "$WAIT"
WID=$(osascript -l JavaScript -e "
ObjC.import('CoreGraphics');
var list = ObjC.deepUnwrap($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0));
var best = null, area = 0;
for (var w of list) {
  if (w.kCGWindowOwnerPID == $PID && w.kCGWindowLayer == 0) {
    var a = w.kCGWindowBounds.Width * w.kCGWindowBounds.Height;
    if (a > area) { area = a; best = w.kCGWindowNumber; }
  }
}
best;")
[ -n "$WID" ] || { echo "no window for pid $PID" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"
screencapture -x -o -l "$WID" "$OUT"
echo "$OUT"
