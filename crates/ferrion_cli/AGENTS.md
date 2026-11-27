# ferrion_cli

The `ferrion` command-line binary. Today `main.rs` only prints the version, and `files.rs` holds
file selection (built ahead of the commands, with inline tests). Argument parsing, the commands,
the output formats and crash containment are planned in the CLI design. It depends on
`ferrion_config`.

**Design:** [cli.md](../../docs/design/cli.md), [SCOPE.md](../../docs/SCOPE.md#command-line-interface)

**Risk level:** Deep for any change to commands, flags, exit codes, output formats (`pretty`,
`json`, `github`) and their message wording (high-risk-areas.md §7), which also needs a human's yes
before it starts. Standard for internals that change none of those, such as file selection code.

## Invariants

- Commands, flags, exit codes and output formats follow cli.md and SCOPE.md. Exit code 3 means a
  crash or a `--verify` failure.
- File selection and formatting depend only on the files, the Java release and the config.

## Where tests go

| Kind of change             | Add the test here                              | Command |
| -------------------------- | ---------------------------------------------- | ------- |
| File selection             | inline `mod tests` in `src/files.rs`           | `cargo test --package ferrion_cli` |
| A command or output format | `tests/` end-to-end tests that run the built binary (cli.md, "Testing"; the directory doesn't exist yet) | `cargo test --package ferrion_cli` |
| A message's wording        | the message catalog snapshot (cli.md, "Testing"; not written yet). Config errors are in `crates/ferrion_config/tests/configs/errors.txt` today | see `ferrion_config`'s AGENTS.md |

Use cli.md's test plan. Don't add new test frameworks or helper crates without asking.

## Don't

- Don't change a command, flag, exit code, output format or message without a human's yes.
- Don't put config loading or validation here; it belongs in `ferrion_config`.
- Don't make formatting depend on a JDK, a classpath or the network.
