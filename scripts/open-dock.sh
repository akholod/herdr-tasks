#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Toggle the herdr-tasks dock in the current tab: a right split at ~33% of
# the tab width (D7: split the rightmost full-height pane, then a closed-loop
# resize verified from `herdr pane layout`).
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=lib.sh
. "$DIR/lib.sh"

mapfile -t _open < <(panes_by_title herdr-tasks-dock)
if [ "${#_open[@]}" -gt 0 ]; then
  for pid in "${_open[@]}"; do "$HERDR_BIN" pane close "$pid" >/dev/null 2>&1; done
  exit 0
fi

FOCUSED="$(focused_pane_id)"
[ -n "$FOCUSED" ] || exit 0
TARGET="$("$HERDR_BIN" pane layout --pane "$FOCUSED" 2>/dev/null | pick_dock_target)"
[ -n "$TARGET" ] || TARGET="$FOCUSED"

OUT="$("$HERDR_BIN" plugin pane open \
  --plugin herdr-tasks \
  --entrypoint dock \
  --placement split \
  --direction right \
  --target-pane "$TARGET" \
  --focus 2>/dev/null)"
DOCK="$(printf '%s' "$OUT" | json_pane_id)"
[ -n "$DOCK" ] || exit 0
resize_dock "$DOCK"
