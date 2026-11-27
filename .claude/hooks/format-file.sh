#!/usr/bin/env bash
# PostToolUse hook (Edit|Write|MultiEdit): run rustfmt on an edited .rs file. rustfmt reads
# rustfmt.toml from the file's parent directories, so it formats as `cargo fmt` does.
# Never fails the tool call. A syntax error mid-edit, or a missing tool, just skips formatting.
set -uo pipefail

input="$(cat)"

if command -v jq >/dev/null 2>&1; then
  file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // empty')"
elif command -v python3 >/dev/null 2>&1; then
  file="$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input",{}).get("file_path") or "")')"
else
  exit 0
fi

[ -n "$file" ] && [ -f "$file" ] || exit 0

case "$file" in
  */tests/fuzz-regressions/* | */fuzz/seeds/*) ;; # test data, never formatted
  *.rs)
    command -v rustfmt >/dev/null 2>&1 && rustfmt --quiet "$file" >/dev/null 2>&1
    ;;
esac

exit 0
