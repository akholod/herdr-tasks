#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Toggle the wide board overlay (kanban for the status grouping).
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=lib.sh
. "$DIR/lib.sh"

mapfile -t _open < <(panes_by_title herdr-tasks-board)
if [ "${#_open[@]}" -gt 0 ]; then
  for pid in "${_open[@]}"; do "$HERDR_BIN" pane close "$pid" >/dev/null 2>&1; done
  exit 0
fi
"$HERDR_BIN" plugin pane open --plugin herdr-tasks --entrypoint board --placement overlay --focus >/dev/null 2>&1
