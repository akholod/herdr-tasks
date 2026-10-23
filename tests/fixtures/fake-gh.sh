#!/usr/bin/env bash
# Fake `gh` for tests: logs argv to $FAKE_GH_LOG and serves/mutates issues
# from $FAKE_GH_DIR/issues.json. `auth status` exits ${FAKE_GH_AUTH:-0}.
exec python3 "$(dirname "$(readlink -f "$0")")/fake-gh.py" "$@"
