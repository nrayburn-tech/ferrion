#!/usr/bin/env bash
# PreToolUse hook (Edit|Write|MultiEdit|NotebookEdit).
# Blocks edits to generated, license and lock files, and asks before edits to guardrail files and
# to existing test expectations. The policy matches AGENTS.md ("Never" and "Ask first"). Keep the
# two in sync.
set -euo pipefail

input="$(cat)"

if command -v jq >/dev/null 2>&1; then
  file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.notebook_path // empty')"
elif command -v python3 >/dev/null 2>&1; then
  file="$(printf '%s' "$input" | python3 -c 'import json,sys; t=json.load(sys.stdin).get("tool_input",{}); print(t.get("file_path") or t.get("notebook_path") or "")')"
else
  echo "protect-files: neither jq nor python3 found; skipping checks" >&2
  exit 0
fi

[ -z "$file" ] && exit 0

root="${CLAUDE_PROJECT_DIR:-$PWD}"
file="${file//\\//}"   # normalize Windows separators
root="${root//\\//}"
rel="${file#"$root"/}"

decide() { # $1 = deny|ask, $2 = reason (no double quotes)
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"%s","permissionDecisionReason":"%s"}}\n' "$1" "$2"
  exit 0
}

case "$rel" in
  Cargo.lock | */Cargo.lock)
    decide deny "Cargo.lock is generated. Change it with cargo (cargo add, cargo update) after asking the human." ;;
  LICENSE-* | NOTICE)
    decide deny "License and notice files are never edited by agents." ;;
  target/* | */target/* | .git/*)
    decide deny "Build output and git internals are not edited." ;;
  crates/ferrion_syntax/src/*/generated.rs)
    decide deny "Generated from crates/ferrion_syntax/java.grammar. Change the grammar and run cargo codegen." ;;
  .github/* | deny.toml | rust-toolchain.toml | fuzz/rust-toolchain.toml | rustfmt.toml | .cargo/config.toml | Cargo.toml | tools/scancode/*)
    decide ask "Guardrail or workspace config (CI, lints, toolchain, workspace deps, provenance check). AGENTS.md says ask first." ;;
  .claude/settings.json | .claude/hooks/* | .claude/agents/* | AGENTS.md | */AGENTS.md | CLAUDE.md | SECURITY.md | .agents/* | justfile)
    decide ask "Agent guidance or policy file. AGENTS.md says ask first." ;;
  docs/SCOPE.md)
    decide ask "A change in scope starts as an issue. AGENTS.md says ask first." ;;
  tools/format-check/gjf.toml | tools/corpus/projects.toml | tools/corpus/jdk-tests.toml)
    decide ask "This moves a pin (GJF, a corpus project, javac's tests). AGENTS.md says ask first." ;;
  tools/parser-check/thresholds.toml | tools/bench/thresholds.toml | tools/format-check/gjf-diff.toml)
    decide ask "Thresholds and diff counts measure regressions. Raise them when results improve; never loosen them to pass. Explain the change to the human." ;;
  */tests/snapshots/* | */tests/block-snapshots/* | */tests/fuzz-regressions/* | */tests/*-regressions/* | fuzz/seeds/* | tools/oracle-check/differences/* | tools/oracle-check/parser-cases/* | tools/format-check/differences/*)
    # New files are fine. Changing an existing one changes what the tests treat as correct, and
    # these are rewritten with FERRION_BLESS=1 or --bless, not by hand.
    if [ -e "$file" ]; then
      decide ask "This changes an existing test expectation or recorded input. Explain why the old one was wrong; expected outputs are normally rewritten with FERRION_BLESS=1 or --bless and reviewed."
    fi ;;
esac

exit 0
