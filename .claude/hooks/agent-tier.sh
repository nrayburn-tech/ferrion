#!/usr/bin/env bash
# PreToolUse hook (Agent|Task).
# Asks the human before a call to a ferrion-* subagent that passes its own model or effort. Each
# ferrion-* agent in .claude/agents/ sets the model and reasoning effort of its tier
# (docs/agents/model-tiers.md), and a per-call model or effort replaces them, which would run the
# task at a different tier. It asks rather than denies, because a human may want that on purpose.
# The rule matches CLAUDE.md ("Subagents and model choice"). Keep the two in sync.
set -euo pipefail

input="$(cat)"

if command -v jq >/dev/null 2>&1; then
  field() { printf '%s' "$input" | jq -r --arg k "$1" '.tool_input[$k] // empty'; }
elif command -v python3 >/dev/null 2>&1; then
  field() { printf '%s' "$input" | python3 -c 'import json,sys; v=json.load(sys.stdin).get("tool_input",{}).get(sys.argv[1]); print("" if v is None else v)' "$1"; }
else
  echo "agent-tier: neither jq nor python3 found; skipping checks" >&2
  exit 0
fi

agent="$(field subagent_type)"

case "$agent" in
  ferrion-*) ;;
  *) exit 0 ;;
esac

passed=""
[ -n "$(field model)" ] && passed="model"
if [ -n "$(field effort)" ]; then
  passed="${passed:+$passed and }effort"
fi

[ -z "$passed" ] && exit 0

printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"%s sets the model and effort of its tier, and this call passes %s, which would replace them. Allow only if this task should run outside the tier in .claude/agents/%s.md."}}\n' \
  "$agent" "$passed" "$agent"
exit 0
