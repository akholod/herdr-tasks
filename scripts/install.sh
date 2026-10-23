#!/usr/bin/env bash
# Install herdr-tasks: build, link the plugin, install the agent skill, put
# the binary on PATH and bind the dock toggle (default prefix+t).
#
# Safe by default: without --yes it only prints what it would do.
#   --yes        apply
#   --key KEY    keybinding for the dock toggle (default prefix+t)
#   --no-build   skip `cargo build --release`
#   --no-link    skip everything outside ~/.claude, ~/.agents and the herdr
#                config.toml: no `herdr plugin link`, no reload, no
#                ~/.local/bin symlink
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
YES=0 KEY="prefix+t" BUILD=1 LINK=1
while [ $# -gt 0 ]; do
  case "$1" in
    --yes) YES=1 ;;
    --key) KEY="$2"; shift ;;
    --no-build) BUILD=0 ;;
    --no-link) LINK=0 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
  shift
done
export PATH="${PATH:-}:/opt/homebrew/bin:/home/linuxbrew/.linuxbrew/bin:/usr/local/bin:/usr/bin:/bin"
HERDR_BIN="${HERDR_BIN_PATH:-herdr}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/herdr/config.toml"

run() {
  if [ "$YES" = 1 ]; then
    "$@"
  else
    echo "would run: $*"
  fi
}

if [ "$BUILD" = 1 ]; then
  run cargo build --release --manifest-path "$REPO/Cargo.toml"
fi

if [ "$LINK" = 1 ]; then
  run "$HERDR_BIN" plugin link "$REPO"
fi

install_skill() {
  local dest="$1"
  run mkdir -p "$dest"
  run cp "$REPO/skill/SKILL.md" "$dest/SKILL.md"
}
install_skill "$HOME/.claude/skills/herdr-tasks"
if [ -d "$HOME/.agents/skills" ]; then
  install_skill "$HOME/.agents/skills/herdr-tasks"
fi

if [ "$LINK" = 1 ]; then
  run mkdir -p "$HOME/.local/bin"
  run ln -sf "$REPO/target/release/herdr-tasks" "$HOME/.local/bin/herdr-tasks"
fi

if [ -f "$CONFIG" ] && grep -q 'herdr-tasks.open-dock' "$CONFIG"; then
  echo "keybinding already present in $CONFIG"
else
  BLOCK="$(printf '\n[[keys.command]]\nkey = "%s"\ntype = "plugin_action"\ncommand = "herdr-tasks.open-dock"\n' "$KEY")"
  if [ "$YES" = 1 ]; then
    mkdir -p "$(dirname "$CONFIG")"
    if [ -f "$CONFIG" ]; then
      cp "$CONFIG" "$CONFIG.bak.$(date +%Y%m%d%H%M%S)"
    fi
    printf '%s\n' "$BLOCK" >> "$CONFIG"
    echo "added $KEY -> herdr-tasks.open-dock to $CONFIG"
  else
    echo "would append to $CONFIG:$BLOCK"
  fi
fi

if [ "$LINK" = 1 ]; then
  run "$HERDR_BIN" server reload-config || true
fi
[ "$YES" = 1 ] || echo "dry run: re-run with --yes to apply"
