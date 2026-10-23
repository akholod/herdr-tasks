#!/usr/bin/env bash
# Shared helpers for the herdr-tasks launcher scripts (adapted from
# herdr-beads scripts/lib.sh). herdr starts plugin processes with a minimal
# PATH, so extend it and resolve herdr. JSON is parsed with python3 -c (the
# JSON arrives on stdin, so the program cannot come from a heredoc).

export PATH="${PATH:-}:/opt/homebrew/bin:/home/linuxbrew/.linuxbrew/bin:/usr/local/bin:/usr/bin:/bin"
HERDR_BIN="${HERDR_BIN_PATH:-herdr}"

# pane_id of the focused pane.
focused_pane_id() {
  "$HERDR_BIN" pane list 2>/dev/null | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for p in (d.get("result") or {}).get("panes") or []:
    if p.get("focused") and p.get("pane_id"):
        print(p["pane_id"])
        break
' 2>/dev/null || true
}

# cwd of the focused pane.
focused_cwd() {
  "$HERDR_BIN" pane list 2>/dev/null | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
panes = (d.get("result") or {}).get("panes") or []
p = ([p for p in panes if p.get("focused")] or panes or [None])[0]
if p:
    print(p.get("cwd") or p.get("foreground_cwd") or "")
' 2>/dev/null || true
}

# pane_ids whose terminal title contains $1, limited to the current tab
# ($HERDR_TAB_ID, else the focused pane's tab). The dock is per tab: a
# workspace-wide match would close another tab's dock.
panes_by_title() {
  "$HERDR_BIN" pane list 2>/dev/null | MARKER="$1" python3 -c '
import json, os, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
marker = os.environ.get("MARKER", "")
panes = (d.get("result") or {}).get("panes") or []
tab = os.environ.get("HERDR_TAB_ID", "")
if not tab:
    tab = next((p.get("tab_id", "") for p in panes if p.get("focused")), "")
for p in panes:
    title = (p.get("terminal_title") or "") + " " + (p.get("terminal_title_stripped") or "")
    if marker in title and (not tab or p.get("tab_id") == tab) and p.get("pane_id"):
        print(p["pane_id"])
' 2>/dev/null || true
}

# First "pane_id" found anywhere in the JSON on stdin.
json_pane_id() {
  python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
def find(o):
    if isinstance(o, dict):
        if isinstance(o.get("pane_id"), str):
            return o["pane_id"]
        o = list(o.values())
    if isinstance(o, list):
        for v in o:
            r = find(v)
            if r:
                return r
    return None
p = find(d)
if p:
    print(p)
' 2>/dev/null || true
}

# From `herdr pane layout` JSON on stdin: the pane to split for the dock.
# A pane at the tab's right edge, preferring full height (a stacked right
# column gives a partial-height dock, then the tallest one); if that column
# is narrower than 0.36 x tab width, the widest full-height pane instead
# (D7 fallback: the dock is then not at the right edge).
pick_dock_target() {
  python3 -c '
import json, sys
try:
    L = (json.load(sys.stdin).get("result") or {}).get("layout") or {}
    area, panes = L["area"], L["panes"]
except Exception:
    sys.exit(0)
if not panes:
    sys.exit(0)
def full(p):
    return p["rect"]["y"] == area["y"] and p["rect"]["height"] == area["height"]
edge = max(p["rect"]["x"] + p["rect"]["width"] for p in panes)
right = [p for p in panes if p["rect"]["x"] + p["rect"]["width"] == edge]
target = max(right, key=lambda p: (full(p), p["rect"]["height"], -p["rect"]["y"]))
if target["rect"]["width"] < 0.36 * area["width"]:
    wide = [p for p in panes if full(p)]
    if wide:
        target = max(wide, key=lambda p: p["rect"]["width"])
print(target["pane_id"])
' 2>/dev/null || true
}

# From `herdr pane layout` JSON on stdin: "<ratio> <right_edge 0|1>" for $1.
dock_metrics() {
  DOCK="$1" python3 -c '
import json, os, sys
try:
    L = (json.load(sys.stdin).get("result") or {}).get("layout") or {}
    area = L["area"]
    p = next(p for p in L["panes"] if p["pane_id"] == os.environ["DOCK"])
except Exception:
    sys.exit(0)
r = p["rect"]
edge = int(r["x"] + r["width"] == area["x"] + area["width"])
print("%.4f %d" % (r["width"] / float(area["width"]), edge))
' 2>/dev/null || true
}

# Closed-loop resize of dock $1 to HERDR_TASKS_DOCK_RATIO (default 0.33):
# at most 4 steps, stop inside 0.30-0.36, flip direction when a step did not
# bring the ratio closer.
resize_dock() {
  local dock="$1" target="${HERDR_TASKS_DOCK_RATIO:-0.33}" dir="" prev="" m ratio step
  for _ in 1 2 3 4; do
    m="$("$HERDR_BIN" pane layout --pane "$dock" 2>/dev/null | dock_metrics "$dock")"
    [ -n "$m" ] || return 0
    ratio="${m%% *}"
    step="$(RATIO="$ratio" TARGET="$target" PREV="$prev" DIR="$dir" python3 -c '
import os
r, t = float(os.environ["RATIO"]), float(os.environ["TARGET"])
if 0.30 <= r <= 0.36:
    print("ok"); raise SystemExit
dist = abs(r - t)
d = os.environ["DIR"] or ("right" if r > t else "left")
prev = os.environ["PREV"]
if prev and dist >= float(prev) - 1e-6:
    d = "left" if d == "right" else "right"
print("%s %.4f %.4f" % (d, dist, dist))
')"
    [ "$step" = "ok" ] && return 0
    read -r dir prev amount <<<"$step"
    "$HERDR_BIN" pane resize --pane "$dock" --direction "$dir" --amount "$amount" >/dev/null 2>&1 || true
  done
}
