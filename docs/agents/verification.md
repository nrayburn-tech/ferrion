# Verification: which checks to run, and when

Pick checks based on what the diff touches, including the crates that depend on it. Run the full
suite once, at the end, as a checkpoint. Don't re-run everything after every small edit.

## While iterating

| You changed                         | Run                                                                                                                            |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Rust code or tests in one crate     | `cargo check -p <crate>`, then `cargo clippy -p <crate> --all-targets -- -D warnings` and an **unfiltered** `cargo test -p <crate>` |
| A crate that other crates use       | The above for every crate that depends on it (`cargo tree -i <crate> -e normal`)                                               |
| `java.grammar`                      | `cargo codegen`, then `git diff` to review the generated code, then the parser's and its consumers' tests                    |
| Snapshot inputs or expected outputs | The tests that read them, with `FERRION_BLESS=1` only for an intended change. Review each snapshot diff by hand.              |
| Formatter behavior                  | Its snapshots, then `cargo format-check snapshots`, `differences`, and `verify` on a corpus project if one is checked out      |
| Lexer, parser or semantic behavior  | The matching oracle comparison from [CONTRIBUTING.md](../../CONTRIBUTING.md#javac-oracle) on one project, if a JDK and the corpus are available |
| Code a fuzz target covers           | The target for 30 seconds (`-max_total_time=30`), and the crate's fuzz regression test                                       |
| Rustdoc                             | `cargo doc -p <crate> --no-deps`                                                                                               |
| `Cargo.toml` dependencies           | `cargo deny check`, and `cargo deny --manifest-path fuzz/Cargo.toml check` if the fuzz workspace uses the crate               |

## Rules

- **A filtered test run that runs zero tests is incomplete, not passing.** If
  `cargo test -p x foo` reports `0 passed`, your filter is wrong.
- **Reuse a passing result** until something it depends on changes: the code under test, its
  dependencies, test inputs, the toolchain or the build config. Making a commit doesn't invalidate
  it.
- **Run Cargo commands one at a time** when they share a `target/` directory. Running them in
  parallel causes lock contention and confusing rebuilds.
- **Keep flags consistent.** Mixing profiles or feature sets between runs rebuilds everything.
- **Corpus and oracle checks** need a JDK, Maven and a fetched corpus. Run the one that covers your
  change when they're available, and say in the PR which you ran. CI's `tools` job runs them on one
  project, and the weekly oracle workflow on all of them.

## Final checkpoint

Before you say the work is done, run **`just ready`** once. It runs CI's required checks that need
no JDK or corpus: format, clippy, tests, both `cargo deny` checks, the generated-code check and
ScanCode. It refuses to start if the working tree has uncommitted changes, and fails if any step
leaves files changed. Read its output; don't just check the exit code.

When a branch has more than one commit, check each commit, not just the last one. Every commit must
stand alone (see AGENTS.md, "Commits and PRs"). `git rebase --exec 'just ready' origin/main` runs
`just ready` on each commit in turn and stops at the first one that fails.

If you can't run `just ready`, for example because `just` or ScanCode's prerequisites aren't
installed, run the individual commands listed in AGENTS.md. In your summary, say which ones you
skipped.
