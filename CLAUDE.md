@AGENTS.md

## Claude Code specifics

The instructions above are the source of truth. This section only covers Claude Code features.

- **Settings:** `.claude/settings.json` holds the shared permissions and hooks. Do not edit it to
  loosen permissions. Personal overrides go in `.claude/settings.local.json`, which is gitignored.
- **Hooks:**
  - `.claude/hooks/protect-files.sh` blocks edits to generated, license and lock files, and asks
    before edits to guardrail files (CI, lint and toolchain config, agent guidance) and to existing
    test expectations: snapshots, fuzz regressions and seeds, oracle differences and thresholds.
  - `.claude/hooks/agent-tier.sh` asks the human before a call to a `ferrion-*` agent that passes a
    `model` or `effort`, because that would replace the agent's tier.
  - `.claude/hooks/format-file.sh` runs `rustfmt` on each `.rs` file you edit. Expect a file to be
    reformatted right after you edit it. If you read it again, work from the formatted version.
  - If a hook blocks or questions an edit, do not route around it (shell redirection, `sed -i`,
    writing a copy). Ask the human.
- **Subagents and model choice:** when you delegate, use the agent in `.claude/agents/` that matches
  the task's tier in [docs/agents/model-tiers.md](docs/agents/model-tiers.md). Each one sets its
  tier's model and reasoning effort.

  | Tier     | Agent              | Model  | Effort |
  | -------- | ------------------ | ------ | ------ |
  | Light    | `ferrion-light`    | Haiku  | low    |
  | Standard | `ferrion-standard` | Sonnet | medium |
  | Deep     | `ferrion-deep`     | Opus   | high   |

  Don't pass a `model` or `effort` when you call one of these agents, because they override what the
  agent sets. Typical use is read-only exploration and log summaries with `ferrion-light`, and
  implementation with the agent for the task's tier. Risk-area reviews always use `ferrion-deep`.
  Tell the user when the session model is below the tier the task needs.

- **Parallel agents:** run at most about four at once, and give each a disjoint area. Worktree
  agents start from `main`: when the work belongs on another branch, have the agent check that its
  worktree is clean and reset onto that branch before it starts. Subagents don't start subagents of
  their own.
- **Plan mode:** use it for every Deep-tier task before editing code.
- **Skills:** the repo's skills live in `.agents/skills/`, so every agent tool can share them.
  Claude Code doesn't load them automatically from there, so read the matching `SKILL.md` yourself
  when a task fits the table in AGENTS.md.
