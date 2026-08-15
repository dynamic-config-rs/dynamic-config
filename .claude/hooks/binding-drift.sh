#!/usr/bin/env bash
# Names the files a change has to travel to, the moment it is made.
#
# Advisory by design: it exits 0 and never blocks a tool call.
set -euo pipefail

input=$(cat)
path=$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input", {}).get("file_path", ""))' 2>/dev/null || true)

[ -z "$path" ] && exit 0

case "$path" in
  */dynamic-config/src/lib.rs)
    cat <<'NOTE'
The core's front door moved. If a public item changed, three repositories
may need to follow and nothing in this build says so:
  · dynamic-config-remote   the stores implement RemoteSource
  · dynamic-config-python   its facade wraps this crate
  · dynamic-config-node     the same, through Node-API
  · book/src/               the chapter that describes the item
  · CHANGELOG.md            under Unreleased
NOTE
    ;;
  */dynamic-config/src/lib.rs|*/book/src/sources-and-precedence.md)
    cat <<'NOTE'
The precedence chain is written in both of these files, and `doc_surface`
compares them character for character.
NOTE
    ;;
esac

exit 0
