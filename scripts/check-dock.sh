#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# AC-1b: the dock of the current tab sits at the right edge and spans
# 0.30-0.36 of the tab width. Prints `ratio=0.33 ok` and exits 0, or prints
# the measured values and exits 1. When the dock is not at the right edge
# because the right column was too narrow to split (D7 fallback), it reports
# `fallback: not right edge` and checks the ratio only.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=lib.sh
. "$DIR/lib.sh"

DOCK="$(panes_by_title herdr-tasks-dock | head -n1)"
if [ -z "$DOCK" ]; then
  echo "no herdr-tasks-dock pane in this tab"
  exit 1
fi
"$HERDR_BIN" pane layout --pane "$DOCK" 2>/dev/null | DOCK="$DOCK" python3 -c '
import json, os, sys
L = (json.load(sys.stdin).get("result") or {}).get("layout") or {}
area = L["area"]
dock = next(p for p in L["panes"] if p["pane_id"] == os.environ["DOCK"])
r = dock["rect"]
ratio = r["width"] / float(area["width"])
edge = r["x"] + r["width"] == area["x"] + area["width"]
ok_ratio = 0.30 <= ratio <= 0.36
if edge:
    ok = ok_ratio
else:
    right = [p for p in L["panes"] if p["rect"]["x"] >= r["x"] + r["width"]]
    narrow = any(p["rect"]["width"] < 0.36 * area["width"] for p in right)
    if narrow:
        print("fallback: not right edge")
    ok = ok_ratio and narrow
if ok:
    print("ratio=%.2f ok" % ratio)
else:
    print("ratio=%.3f right_edge=%s dock=%s area=%s" % (ratio, edge, r, area))
sys.exit(0 if ok else 1)
'
