# ferrion_semantic

Symbols, name resolution, type relations and attribution, kept up to date as files change. The
`Engine` holds a project's inputs and everything derived from them in layers, caches results by
what they read, and answers queries through a `Snapshot`. It does not decide findings. It depends on
`ferrion_classfile` and `ferrion_project`, and the lint rules build on it.

**Design:** [semantic.md](../../docs/design/semantic.md),
[semantic-names.md](../../docs/design/semantic-names.md),
[semantic-types.md](../../docs/design/semantic-types.md),
[semantic-type-relations.md](../../docs/design/semantic-type-relations.md),
[semantic-attribution.md](../../docs/design/semantic-attribution.md),
[semantic-incremental.md](../../docs/design/semantic-incremental.md),
[semantic-oracle-normalization.md](../../docs/design/semantic-oracle-normalization.md),
[types-oracle-normalization.md](../../docs/design/types-oracle-normalization.md)

**Risk level:** Deep: see [docs/agents/high-risk-areas.md](../../docs/agents/high-risk-areas.md)
§4 for name resolution, type relations, attribution and inference, incomplete analysis, and the
engine's dependencies and invalidation; also §8 for recursion that follows the input's depth.
Standard for test support (`testing.rs`), docs, and changes that touch none of those.

## Invariants

- After any edit, every answer equals a fresh engine's over the same inputs. A dependency the
  engine fails to record serves a stale entry.
- Queries return `Cancelled` once an input changes under them; they never return a mix.
- When unsure, report incomplete analysis (error types, unresolved symbols) rather than guess
  ([SCOPE.md](../../docs/SCOPE.md#incomplete-analysis)). Relations answer `Yes`, `No` or `Unknown`.
- Recursion whose depth follows the input calls `ferrion_stack::ensure_stack` on entry.
- Cite the JLS section a rule comes from.

## Where tests go

| Kind of change             | Add the test here                              | Command                                                                  |
| -------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------ |
| Name resolution            | a `.case` in `tests/resolution/` and its `.names` | `cargo test --package ferrion_semantic --test resolution`             |
| Declaration summaries      | a `.java` in `tests/summaries/` and `.summary` | `cargo test --package ferrion_semantic --test summaries`                 |
| Bless a snapshot           | review the diff by hand                        | `FERRION_BLESS=1 cargo test --package ferrion_semantic --test resolution` (or `--test summaries`) |
| Engine recomputation, edits | `tests/engine.rs`                             | `cargo test --package ferrion_semantic --test engine`                    |
| Attribution and its edits  | `tests/attribution.rs`                         | `cargo test --package ferrion_semantic --test attribution`               |
| Deep nesting, long chains  | `tests/deep.rs` (256 KiB stack)                | `cargo test --package ferrion_semantic --test deep`                      |
| Project model to inputs    | `tests/project_input.rs`                       | `cargo test --package ferrion_semantic --test project_input`             |
| Relations, constants, names | inline `#[cfg(test)]` in `src/types.rs`, `src/attr/tests.rs`, `src/constants/tests.rs`, `src/names.rs` | `cargo test --package ferrion_semantic --lib` |
| Corpus-level agreement     | no test file; run the oracles                  | `cargo oracle names jedis`, `types`, `removed`, `attribute`              |
| Incremental fuzz failure   | extend `invariants::check_edits` and a test in `tests/engine.rs`; seeds are in `fuzz/seeds/semantic_edits` | `cargo test --package ferrion_semantic --test engine` |

Each `.case` was checked against javac when written, and its first lines say how. Use this crate's
existing test setup. Don't add new test frameworks, helper crates or test styles without asking.

## Don't

- Don't bless a snapshot without reading every diff.
- Don't add a known difference without a category and an explanation, and update the
  normalization document in the same change. Never lower a check to make it pass.
- Don't guess a type or a symbol. Report incomplete analysis.
- Don't copy class files from a JDK; `testing.rs`'s stand-in platform is written in this crate.
- Don't study javac or OpenJDK source for a rule; use the JLS and javac's observable output.
- Don't recurse on input depth without `ensure_stack`.
