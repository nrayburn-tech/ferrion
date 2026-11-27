# ferrion_java_version

The Java releases Ferrion supports (`OLDEST_RELEASE` to `LATEST_RELEASE`), the LTS releases and
`PRIMARY_RELEASES`, which test matrices, oracle comparisons and fuzzing run at. No dependencies;
the parser, formatter and tools read it.

**Design:** [SCOPE.md](../../docs/SCOPE.md); [ROADMAP.md](../../docs/ROADMAP.md) ("Cross-cutting
work")

**Risk level:** Standard for tests. Raising `LATEST_RELEASE` is a cross-cutting work item (a new GA
release), not a local edit.

## Invariants

- `PRIMARY_RELEASES` is `LTS_RELEASES` plus `LATEST_RELEASE` (if it isn't an LTS); the LTS list
  starts at `OLDEST_RELEASE`.
- The lexer's Unicode tables and the workflows' JDK agree with `LATEST_RELEASE`.

## Where tests go

| Kind of change       | Add the test here                      | Command                                                   |
| -------------------- | -------------------------------------- | --------------------------------------------------------- |
| Release lists        | unit tests in `src/lib.rs`             | `cargo test --package ferrion_java_version --lib`         |
| Consistency checks   | `tests/latest_release.rs`              | `cargo test --package ferrion_java_version --test latest_release` |

## Don't

- Don't raise `LATEST_RELEASE` on its own: regenerate the lexer's Unicode tables with that JDK,
  install it in `.github/workflows`, and update `PRIMARY_RELEASES` and `LTS_RELEASES`.
